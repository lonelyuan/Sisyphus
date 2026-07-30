//! 无极时间线的预聚合层（`time_rollups`）。
//!
//! **为什么需要**：时间线要能从"分钟"连续缩放到"一生"。若每次查询都在 `raw_events` 上
//! `GROUP BY strftime(...)`，年尺度就是一次全表扫描 + 每行函数调用，而且用不上任何索引。
//! 缩放的代价必须是 **O(可见桶数)** 而不是 O(事件数)。
//!
//! 设计要点：
//! - 桶按**逻辑日**切（本地时区 + 换日点，见 [`crate::clock`]），周/月桶由日桶再聚合，
//!   保证三个尺度的口径完全一致（否则"这周 = 7 天之和"会对不上）。
//! - 增量：`rollup_state.watermark_ms` 记已处理到的 `ingested_at`。重建只针对水位之后
//!   新事件**触碰到的那些日桶**，且是"删掉该桶重算"——幂等，可安全重跑。
//! - 纯 rusqlite，无副作用，安卓可编。

use rusqlite::{params, Connection, Result};
use serde::Serialize;
use std::collections::BTreeSet;

use crate::clock;

const SCOPE: &str = "behavior";

/// 一个桶内某个维度键的聚合值。
#[derive(Debug, Clone, Serialize)]
pub struct RollupSlice {
    pub bucket_start_ms: i64,
    pub key: String,
    pub duration_ms: i64,
    pub event_count: i64,
}

/// 桶粒度。`from_span` 按可见跨度自动选，保证返回的桶数量在可绘制范围内。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Bucket {
    Day,
    Week,
    Month,
}

impl Bucket {
    pub fn as_str(self) -> &'static str {
        match self {
            Bucket::Day => "day",
            Bucket::Week => "week",
            Bucket::Month => "month",
        }
    }

    /// 按可见跨度选粒度：≤60 天用日桶，≤18 个月用周桶，更长用月桶。
    pub fn from_span(span_ms: i64) -> Self {
        const DAY: i64 = 86_400_000;
        if span_ms <= 60 * DAY {
            Bucket::Day
        } else if span_ms <= 550 * DAY {
            Bucket::Week
        } else {
            Bucket::Month
        }
    }

    /// 把任意时刻对齐到本粒度的桶起点。
    pub fn start_of(self, ms: i64, boundary_hour: u32) -> i64 {
        match self {
            Bucket::Day => clock::day_start_at(ms, boundary_hour),
            Bucket::Week => clock::week_start_at(ms, boundary_hour),
            Bucket::Month => clock::month_start_at(ms, boundary_hour),
        }
    }
}

/// 增量重建。返回本次重算的日桶数量。
///
/// `full = true` 时忽略水位、重算全部历史（首次建表或换了换日点后调用）。
pub fn rebuild(conn: &Connection, full: bool) -> Result<usize> {
    let boundary = clock::boundary_hour(conn);
    let watermark = if full { 0 } else { watermark_ms(conn)? };

    // 1. 找出水位之后新写入的事件所触碰的日桶（用区间两端各取一次，跨午夜的会话两天都算）。
    let mut days: BTreeSet<i64> = BTreeSet::new();
    let mut max_ingested = watermark;
    {
        let mut stmt = conn.prepare(
            "SELECT COALESCE(start_time, event_time, produced_at),
                    COALESCE(end_time, start_time, event_time, produced_at),
                    ingested_at
             FROM raw_events
             WHERE ingested_at > ?1 AND layer = 'raw' AND type = 'app_foreground'",
        )?;
        let mut rows = stmt.query(params![watermark])?;
        while let Some(row) = rows.next()? {
            let start: i64 = row.get(0)?;
            let end: i64 = row.get(1)?;
            let ingested: i64 = row.get(2)?;
            max_ingested = max_ingested.max(ingested);
            let mut cursor = clock::day_start_at(start, boundary);
            let last = clock::day_start_at(end.max(start), boundary);
            // 跨天会话可能覆盖多个日桶；给个上限防脏数据（时间戳异常）拖死循环。
            for _ in 0..400 {
                days.insert(cursor);
                if cursor >= last {
                    break;
                }
                cursor = clock::day_end_at(cursor, boundary);
            }
        }
    }
    if days.is_empty() {
        return Ok(0);
    }

    // 2. 逐日重算（删+插，幂等）。会话与桶取交集，跨午夜的时长按天拆分。
    let now = clock::now_ms();
    for day_start in &days {
        let day_end = clock::day_end_at(*day_start, boundary);
        conn.execute(
            "DELETE FROM time_rollups WHERE bucket_kind='day' AND bucket_start_ms=?1",
            params![day_start],
        )?;
        for dimension in ["category", "app"] {
            let col = if dimension == "category" {
                "category"
            } else {
                "entity"
            };
            let sql = format!(
                "INSERT INTO time_rollups
                   (bucket_kind,bucket_start_ms,dimension,key,duration_ms,event_count,updated_at_ms)
                 SELECT 'day', ?1, ?4, COALESCE({col}, '(unknown)'),
                        SUM(MAX(0, MIN(COALESCE(end_time, start_time), ?2)
                                   - MAX(start_time, ?1))),
                        COUNT(*), ?3
                 FROM raw_events
                 WHERE layer='raw' AND type='app_foreground'
                   AND start_time IS NOT NULL AND end_time IS NOT NULL
                   AND start_time < ?2 AND end_time > ?1
                 GROUP BY COALESCE({col}, '(unknown)')"
            );
            conn.execute(&sql, params![day_start, day_end, now, dimension])?;
        }
    }

    // 3. 受影响的周/月桶由日桶再聚合（口径一致）。
    let mut coarse: BTreeSet<(Bucket, i64)> = BTreeSet::new();
    for day_start in &days {
        coarse.insert((Bucket::Week, clock::week_start_at(*day_start, boundary)));
        coarse.insert((Bucket::Month, clock::month_start_at(*day_start, boundary)));
    }
    for (bucket, start) in coarse {
        let end = match bucket {
            Bucket::Week => clock::week_start_at(start + 8 * 86_400_000, boundary),
            Bucket::Month => clock::month_start_at(start + 32 * 86_400_000, boundary),
            Bucket::Day => clock::day_end_at(start, boundary),
        };
        conn.execute(
            "DELETE FROM time_rollups WHERE bucket_kind=?1 AND bucket_start_ms=?2",
            params![bucket.as_str(), start],
        )?;
        conn.execute(
            "INSERT INTO time_rollups
               (bucket_kind,bucket_start_ms,dimension,key,duration_ms,event_count,updated_at_ms)
             SELECT ?1, ?2, dimension, key, SUM(duration_ms), SUM(event_count), ?4
             FROM time_rollups
             WHERE bucket_kind='day' AND bucket_start_ms >= ?2 AND bucket_start_ms < ?3
             GROUP BY dimension, key",
            params![bucket.as_str(), start, end, now],
        )?;
    }

    set_watermark(conn, max_ingested)?;
    Ok(days.len())
}

/// 追平：只在**确实有新事件**时才重建（一次索引查询的代价）。
///
/// 让读路径自愈——否则时间线要等 app ticker 跑过才有数据，MCP 单独打开库时更是空的。
pub fn catch_up(conn: &Connection) -> Result<usize> {
    let watermark = watermark_ms(conn)?;
    let pending: i64 = conn.query_row(
        "SELECT COUNT(*) FROM raw_events
         WHERE ingested_at > ?1 AND layer='raw' AND type='app_foreground'",
        params![watermark],
        |r| r.get(0),
    )?;
    if pending == 0 {
        return Ok(0);
    }
    rebuild(conn, false)
}

pub fn watermark_ms(conn: &Connection) -> Result<i64> {
    conn.query_row(
        "SELECT watermark_ms FROM rollup_state WHERE scope=?1",
        params![SCOPE],
        |r| r.get(0),
    )
    .or_else(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => Ok(0),
        other => Err(other),
    })
}

fn set_watermark(conn: &Connection, ms: i64) -> Result<()> {
    conn.execute(
        "INSERT INTO rollup_state (scope, watermark_ms, updated_at_ms) VALUES (?1,?2,?3)
         ON CONFLICT(scope) DO UPDATE SET watermark_ms=MAX(watermark_ms, excluded.watermark_ms),
                                          updated_at_ms=excluded.updated_at_ms",
        params![SCOPE, ms, clock::now_ms()],
    )?;
    Ok(())
}

/// 换日点变更后必须整体重算——桶边界变了，旧桶口径失效。
pub fn invalidate_all(conn: &Connection) -> Result<()> {
    conn.execute("DELETE FROM time_rollups", [])?;
    conn.execute("DELETE FROM rollup_state WHERE scope=?1", params![SCOPE])?;
    Ok(())
}

/// 读某个维度在窗口内的桶切片（时间线绘制的主数据）。
pub fn slices(
    conn: &Connection,
    bucket: Bucket,
    dimension: &str,
    start_ms: i64,
    end_ms: i64,
) -> Result<Vec<RollupSlice>> {
    let mut stmt = conn.prepare(
        "SELECT bucket_start_ms, key, duration_ms, event_count
         FROM time_rollups
         WHERE bucket_kind=?1 AND dimension=?2 AND bucket_start_ms >= ?3 AND bucket_start_ms < ?4
         ORDER BY bucket_start_ms ASC, duration_ms DESC",
    )?;
    let rows = stmt
        .query_map(
            params![bucket.as_str(), dimension, start_ms, end_ms],
            |r| {
                Ok(RollupSlice {
                    bucket_start_ms: r.get(0)?,
                    key: r.get(1)?,
                    duration_ms: r.get(2)?,
                    event_count: r.get(3)?,
                })
            },
        )?
        .collect::<Result<Vec<_>>>()?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use crate::ingest::{ingest_event, NewEvent};

    fn session(conn: &Connection, start: i64, end: i64, category: &str, app: &str) {
        ingest_event(
            conn,
            "u",
            "d",
            NewEvent {
                event_id: None,
                source: "desktop_agent".into(),
                layer: "raw".into(),
                event_type: "app_foreground".into(),
                time_mode: "interval".into(),
                event_time: None,
                start_time: Some(start),
                end_time: Some(end),
                entity: Some(app.into()),
                category: Some(category.into()),
                payload: serde_json::json!({}),
                parent_event_ids: vec![],
                privacy_level: "L0".into(),
            },
        )
        .unwrap();
    }

    #[test]
    fn rollup_aggregates_by_day_and_rolls_up_to_week() {
        let conn = db::open(":memory:").unwrap();
        let now = clock::now_ms();
        let day_start = clock::day_start_at(now, 0);
        // 今天两段：娱乐 30min + 工作 20min。
        session(
            &conn,
            day_start + 3_600_000,
            day_start + 3_600_000 + 1_800_000,
            "entertainment.video",
            "tv.danmaku.bili",
        );
        session(
            &conn,
            day_start + 7_200_000,
            day_start + 7_200_000 + 1_200_000,
            "work",
            "com.apple.Terminal",
        );

        let touched = rebuild(&conn, false).unwrap();
        assert_eq!(touched, 1, "只应重算今天这一个日桶");

        let day = slices(&conn, Bucket::Day, "category", day_start, day_start + 1).unwrap();
        let ent = day.iter().find(|s| s.key == "entertainment.video").unwrap();
        assert_eq!(ent.duration_ms, 1_800_000);
        let work = day.iter().find(|s| s.key == "work").unwrap();
        assert_eq!(work.duration_ms, 1_200_000);

        // 周桶 = 日桶之和（同口径）。
        let week_start = clock::week_start_at(now, 0);
        let week = slices(&conn, Bucket::Week, "category", week_start, week_start + 1).unwrap();
        let week_ent = week.iter().find(|s| s.key == "entertainment.video").unwrap();
        assert_eq!(week_ent.duration_ms, 1_800_000);

        // app 维度也在。
        let apps = slices(&conn, Bucket::Day, "app", day_start, day_start + 1).unwrap();
        assert!(apps.iter().any(|s| s.key == "tv.danmaku.bili"));

        // 幂等：无新事件时不重算，重复调用不改变结果。
        assert_eq!(rebuild(&conn, false).unwrap(), 0);
        let again = slices(&conn, Bucket::Day, "category", day_start, day_start + 1).unwrap();
        assert_eq!(again.len(), day.len());
    }

    #[test]
    fn cross_midnight_session_is_split_across_days() {
        let conn = db::open(":memory:").unwrap();
        let now = clock::now_ms();
        let today = clock::day_start_at(now, 0);
        let yesterday = clock::day_start_at(today - 1, 0);
        // 昨天 23:00 起两小时，跨到今天 01:00。
        let start = today - 3_600_000;
        session(&conn, start, start + 7_200_000, "entertainment.video", "x");

        rebuild(&conn, false).unwrap();
        let y = slices(&conn, Bucket::Day, "category", yesterday, yesterday + 1).unwrap();
        let t = slices(&conn, Bucket::Day, "category", today, today + 1).unwrap();
        assert_eq!(y[0].duration_ms, 3_600_000, "昨天应只算到日界");
        assert_eq!(t[0].duration_ms, 3_600_000, "今天从日界起算");
    }

    #[test]
    fn bucket_granularity_follows_span() {
        const DAY: i64 = 86_400_000;
        assert_eq!(Bucket::from_span(7 * DAY), Bucket::Day);
        assert_eq!(Bucket::from_span(120 * DAY), Bucket::Week);
        assert_eq!(Bucket::from_span(3000 * DAY), Bucket::Month);
    }
}
