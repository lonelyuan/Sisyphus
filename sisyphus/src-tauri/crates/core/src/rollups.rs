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
use std::collections::{BTreeSet, HashMap};

use crate::clock;

const SCOPE: &str = "behavior";
const HOUR_MS: i64 = 3_600_000;

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
        rebuild_hour_dimension(conn, *day_start, day_end, now)?;
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
        // hour 维度只在日桶有意义（"日内第几小时"跨周聚合就没有含义了），不往上滚。
        conn.execute(
            "INSERT INTO time_rollups
               (bucket_kind,bucket_start_ms,dimension,key,duration_ms,event_count,updated_at_ms)
             SELECT ?1, ?2, dimension, key, SUM(duration_ms), SUM(event_count), ?4
             FROM time_rollups
             WHERE bucket_kind='day' AND bucket_start_ms >= ?2 AND bucket_start_ms < ?3
               AND dimension != 'hour'
             GROUP BY dimension, key",
            params![bucket.as_str(), start, end, now],
        )?;
    }

    set_watermark(conn, max_ingested)?;
    Ok(days.len())
}

/// `hour` 维度：`key = "HH|category"`，`HH` 是**逻辑日内小时序号**（0 = 日起点那一小时）。
///
/// 为什么在 Rust 侧算：按本地小时切分会话在 SQL 里需要生成序列且要处理 DST，
/// 而这里只是对当天的会话做一次线性扫描。序号从日起点算起 —— 这正好就是
/// 折叠视图（actogram）的横轴位置，前端不需要再做任何时区换算。
///
/// DST 长日（25 小时）会出现序号 24，落在 24 列网格之外；前端按行宽裁剪即可。
fn rebuild_hour_dimension(
    conn: &Connection,
    day_start: i64,
    day_end: i64,
    now: i64,
) -> Result<()> {
    let mut acc: HashMap<(i64, String), (i64, i64)> = HashMap::new();
    {
        let mut stmt = conn.prepare(
            "SELECT start_time, end_time, COALESCE(category, '(unknown)')
             FROM raw_events
             WHERE layer='raw' AND type='app_foreground'
               AND start_time IS NOT NULL AND end_time IS NOT NULL
               AND start_time < ?2 AND end_time > ?1",
        )?;
        let mut rows = stmt.query(params![day_start, day_end])?;
        while let Some(row) = rows.next()? {
            let start: i64 = row.get(0)?;
            let end: i64 = row.get(1)?;
            let category: String = row.get(2)?;
            let from = start.max(day_start);
            let to = end.min(day_end);
            if to <= from {
                continue;
            }
            let first = (from - day_start) / HOUR_MS;
            let last = (to - 1 - day_start) / HOUR_MS;
            for hour in first..=last {
                let slot_start = day_start + hour * HOUR_MS;
                let slot_end = (slot_start + HOUR_MS).min(day_end);
                let overlap = to.min(slot_end) - from.max(slot_start);
                if overlap <= 0 {
                    continue;
                }
                let entry = acc.entry((hour, category.clone())).or_insert((0, 0));
                entry.0 += overlap;
                entry.1 += 1;
            }
        }
    }
    if acc.is_empty() {
        return Ok(());
    }
    let mut stmt = conn.prepare(
        "INSERT INTO time_rollups
           (bucket_kind,bucket_start_ms,dimension,key,duration_ms,event_count,updated_at_ms)
         VALUES ('day',?1,'hour',?2,?3,?4,?5)
         ON CONFLICT(bucket_kind,bucket_start_ms,dimension,key) DO UPDATE SET
           duration_ms=excluded.duration_ms,
           event_count=excluded.event_count,
           updated_at_ms=excluded.updated_at_ms",
    )?;
    for ((hour, category), (duration, count)) in acc {
        stmt.execute(params![
            day_start,
            format!("{hour:02}|{category}"),
            duration,
            count,
            now
        ])?;
    }
    Ok(())
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

/// 折叠视图（actogram）的小时单元格：一天 × 24 小时，带专注/娱乐/中性拆分与主导分类。
///
/// 这是**长跨度折叠唯一可行的数据源**：一年的原始会话有上万条，
/// 而小时单元格最多 366 × 24 ≈ 8.8k 个，且已经是聚合值。
#[derive(Debug, Clone, Serialize)]
pub struct HourCell {
    pub day_start_ms: i64,
    /// 日内小时序号（0 = 逻辑日起点）。
    pub hour: i32,
    pub duration_ms: i64,
    pub focus_ms: i64,
    pub entertainment_ms: i64,
    pub neutral_ms: i64,
    pub top_category: Option<String>,
}

/// 读窗口内的小时单元格（按日桶的 `hour` 维度组装）。
pub fn hour_cells(conn: &Connection, start_ms: i64, end_ms: i64) -> Result<Vec<HourCell>> {
    let mut stmt = conn.prepare(
        "SELECT bucket_start_ms, key, duration_ms
         FROM time_rollups
         WHERE bucket_kind='day' AND dimension='hour'
           AND bucket_start_ms >= ?1 AND bucket_start_ms < ?2
         ORDER BY bucket_start_ms ASC, key ASC",
    )?;
    let mut rows = stmt.query(params![start_ms, end_ms])?;
    let mut out: Vec<HourCell> = Vec::new();
    let mut top: i64 = 0;
    while let Some(row) = rows.next()? {
        let day: i64 = row.get(0)?;
        let key: String = row.get(1)?;
        let duration: i64 = row.get(2)?;
        let (hour_part, category) = match key.split_once('|') {
            Some((h, c)) => (h, c),
            None => continue,
        };
        let hour: i32 = match hour_part.parse() {
            Ok(h) => h,
            Err(_) => continue,
        };
        let fresh = !matches!(out.last(), Some(c) if c.day_start_ms == day && c.hour == hour);
        if fresh {
            out.push(HourCell {
                day_start_ms: day,
                hour,
                duration_ms: 0,
                focus_ms: 0,
                entertainment_ms: 0,
                neutral_ms: 0,
                top_category: None,
            });
            top = 0;
        }
        let cell = out.last_mut().expect("刚推入");
        cell.duration_ms += duration;
        if category.starts_with("entertainment") {
            cell.entertainment_ms += duration;
        } else if category == "(unknown)" {
            cell.neutral_ms += duration;
        } else {
            cell.focus_ms += duration;
        }
        if duration > top {
            top = duration;
            cell.top_category = Some(category.to_string());
        }
    }
    Ok(out)
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

    #[test]
    fn hour_dimension_splits_session_across_hours() {
        let conn = db::open(":memory:").unwrap();
        let day_start = clock::day_start_at(clock::now_ms(), 0);
        // 09:30 → 11:15：第 9 小时 30min、第 10 小时 60min、第 11 小时 15min。
        let start = day_start + 9 * HOUR_MS + 1_800_000;
        session(&conn, start, start + 6_300_000, "work", "com.apple.Terminal");
        rebuild(&conn, false).unwrap();

        let cells = hour_cells(&conn, day_start, day_start + 86_400_000).unwrap();
        let at = |hour: i32| {
            cells
                .iter()
                .find(|c| c.hour == hour)
                .map(|c| c.duration_ms)
                .unwrap_or(0)
        };
        assert_eq!(at(9), 1_800_000);
        assert_eq!(at(10), 3_600_000);
        assert_eq!(at(11), 900_000);
        // 与 category 维度同口径：小时之和 = 当天该分类总时长。
        let day = slices(&conn, Bucket::Day, "category", day_start, day_start + 1).unwrap();
        let total: i64 = cells.iter().map(|c| c.duration_ms).sum();
        assert_eq!(total, day.iter().map(|s| s.duration_ms).sum::<i64>());
    }

    #[test]
    fn hour_cell_carries_split_and_top_category() {
        let conn = db::open(":memory:").unwrap();
        let day_start = clock::day_start_at(clock::now_ms(), 0);
        let hour = day_start + 14 * HOUR_MS;
        // 同一小时内：娱乐 40min + 工作 10min。
        session(&conn, hour, hour + 2_400_000, "entertainment.video", "bili");
        session(&conn, hour + 2_400_000, hour + 3_000_000, "work", "term");
        rebuild(&conn, false).unwrap();

        let cells = hour_cells(&conn, day_start, day_start + 86_400_000).unwrap();
        let cell = cells.iter().find(|c| c.hour == 14).unwrap();
        assert_eq!(cell.entertainment_ms, 2_400_000);
        assert_eq!(cell.focus_ms, 600_000);
        assert_eq!(cell.top_category.as_deref(), Some("entertainment.video"));
    }

    #[test]
    fn hour_dimension_does_not_roll_up_to_week() {
        // "日内第几小时"跨周聚合没有含义，周/月桶里不该出现 hour 维度。
        let conn = db::open(":memory:").unwrap();
        let day_start = clock::day_start_at(clock::now_ms(), 0);
        session(&conn, day_start + HOUR_MS, day_start + 2 * HOUR_MS, "work", "t");
        rebuild(&conn, false).unwrap();
        let week_start = clock::week_start_at(clock::now_ms(), 0);
        let hours = slices(&conn, Bucket::Week, "hour", week_start, week_start + 1).unwrap();
        assert!(hours.is_empty(), "周桶不该有 hour 维度");
        assert!(!hour_cells(&conn, day_start, day_start + 86_400_000)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn hour_dimension_is_idempotent_on_rebuild() {
        let conn = db::open(":memory:").unwrap();
        let day_start = clock::day_start_at(clock::now_ms(), 0);
        session(&conn, day_start + HOUR_MS, day_start + 2 * HOUR_MS, "work", "t");
        rebuild(&conn, false).unwrap();
        let first = hour_cells(&conn, day_start, day_start + 86_400_000).unwrap();
        rebuild(&conn, true).unwrap();
        let second = hour_cells(&conn, day_start, day_start + 86_400_000).unwrap();
        assert_eq!(first.len(), second.len());
        assert_eq!(first[0].duration_ms, second[0].duration_ms);
    }
}
