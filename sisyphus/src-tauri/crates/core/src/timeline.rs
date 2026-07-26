//! 记录视图读模型（感知平面 App「记录」页）：行为会话时间轴 + 干预历史。
//! 只读投影，供 Tauri 命令展示 App 已采集/产出的数据。

use std::collections::HashMap;

use chrono::{Local, NaiveDate, TimeZone};
use rusqlite::{params, Connection};
use serde::Serialize;

/// 一条干预记录。
#[derive(Debug, Serialize)]
pub struct InterventionRow {
    pub id: String,
    pub rule_id: String,
    pub shown_at: i64,
    pub severity: String,
    pub message: String,
    pub user_response: Option<String>,
    pub responded_at: Option<i64>,
    pub outcome: Option<String>,
}

/// 无极时间轴上的统一可视事件。所有坐标都用 epoch ms，前端只做投影和绘制。
#[derive(Debug, Clone, Serialize)]
pub struct TimelineEvent {
    pub id: String,
    /// behavior | intervention | system
    pub kind: String,
    pub start_ms: i64,
    pub end_ms: i64,
    pub title: String,
    pub category: Option<String>,
    pub detail: Option<String>,
    pub severity: Option<String>,
}

/// 周尺度及以上使用的每日聚合。原始区间仍留在 Event log，不在前端重复聚合。
#[derive(Debug, Clone, Serialize)]
pub struct DaySummary {
    pub date: String,
    pub start_ms: i64,
    pub observed_ms: i64,
    pub focus_ms: i64,
    pub entertainment_ms: i64,
    pub neutral_ms: i64,
    pub intervention_count: i64,
    /// 0–100 的可解释启发式状态分；无观测时为 50，不伪装成健康结论。
    pub state_score: i64,
}

#[derive(Debug, Serialize)]
pub struct TimelineResponse {
    pub start_ms: i64,
    pub end_ms: i64,
    pub detail: String,
    pub events: Vec<TimelineEvent>,
    pub days: Vec<DaySummary>,
    pub truncated: bool,
    /// 长期计划应来自用户只读信息源；未连接时前端展示真实空态。
    pub has_long_term_source: bool,
}

/// 按可见窗口查询时间轴。`detail` 由前端根据连续 zoom 推导，而不是切换页面。
pub fn query_timeline(
    conn: &Connection,
    start_ms: i64,
    end_ms: i64,
    detail: &str,
    max_items: i64,
) -> Result<TimelineResponse, String> {
    if end_ms <= start_ms {
        return Err("时间窗口无效".to_string());
    }
    let limit = max_items.clamp(50, 5_000);
    let include_raw = matches!(detail, "minute" | "day");
    let mut events = if include_raw {
        query_behavior_events(conn, start_ms, end_ms, detail, limit + 1)
            .map_err(|e| e.to_string())?
    } else {
        Vec::new()
    };
    let mut truncated = events.len() as i64 > limit;
    if truncated {
        events.truncate(limit as usize);
    }

    // 主动提醒 / 干预在所有尺度都是关键事件，数量单独设上限，避免被高频行为挤掉。
    let remaining = (limit as usize).saturating_sub(events.len()).max(20) as i64;
    let mut interaction_events = query_intervention_events(conn, start_ms, end_ms, remaining + 1)
        .map_err(|e| e.to_string())?;
    if interaction_events.len() as i64 > remaining {
        interaction_events.truncate(remaining as usize);
        truncated = true;
    }
    events.extend(interaction_events);

    // artifact 里程碑（目标/任务/提醒/知识/规则）：稀疏，所有尺度都展示。
    let artifact_cap = (limit / 4).clamp(40, 400);
    let mut artifact_events = query_artifact_events(conn, start_ms, end_ms, artifact_cap + 1)
        .map_err(|e| e.to_string())?;
    if artifact_events.len() as i64 > artifact_cap {
        artifact_events.truncate(artifact_cap as usize);
        truncated = true;
    }
    events.extend(artifact_events);
    events.sort_by_key(|e| e.start_ms);

    let days = if matches!(detail, "week" | "life") || end_ms - start_ms >= 86_400_000 {
        query_day_summaries(conn, start_ms, end_ms).map_err(|e| e.to_string())?
    } else {
        Vec::new()
    };

    Ok(TimelineResponse {
        start_ms,
        end_ms,
        detail: detail.to_string(),
        events,
        days,
        truncated,
        has_long_term_source: false,
    })
}

fn query_behavior_events(
    conn: &Connection,
    start_ms: i64,
    end_ms: i64,
    detail: &str,
    limit: i64,
) -> rusqlite::Result<Vec<TimelineEvent>> {
    let mut stmt = conn.prepare(
        "SELECT event_id, type,
                COALESCE(start_time, event_time, produced_at),
                COALESCE(end_time, start_time, event_time, produced_at),
                entity, category, payload_json
         FROM raw_events
         WHERE COALESCE(start_time, event_time, produced_at) <= ?2
           AND COALESCE(end_time, start_time, event_time, produced_at) >= ?1
         ORDER BY COALESCE(start_time, event_time, produced_at) ASC
         LIMIT ?3",
    )?;
    let mut rows = stmt
        .query_map(params![start_ms, end_ms, limit], |r| {
            let event_type: String = r.get(1)?;
            let entity: Option<String> = r.get(4)?;
            let raw_detail: String = r.get(6)?;
            Ok(TimelineEvent {
                id: r.get(0)?,
                kind: "behavior".to_string(),
                start_ms: r.get(2)?,
                end_ms: r.get(3)?,
                title: entity.unwrap_or_else(|| event_type.clone()),
                category: r.get(5)?,
                detail: if detail == "minute" && raw_detail != "{}" {
                    Some(raw_detail)
                } else {
                    Some(event_type)
                },
                severity: None,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    // note_text 事件（capture / 素材）单独标为 capture，标题取正文摘要，便于时间轴区分图层。
    for ev in rows.iter_mut() {
        if ev.detail.as_deref() == Some("note_text") || ev.title == "note_text" {
            ev.kind = "capture".to_string();
        }
    }
    Ok(rows)
}

/// artifact 里程碑（点事件）：目标 / 任务 / 提醒 / 知识卡片 / 检测规则的创建。
/// 稀疏、低频，所有尺度都展示，作为时间轴上的分层标记。
fn query_artifact_events(
    conn: &Connection,
    start_ms: i64,
    end_ms: i64,
    limit: i64,
) -> rusqlite::Result<Vec<TimelineEvent>> {
    // (SQL, kind, category 标签)。每条表结构：id + 时间戳 + 标题。
    let queries: [(&str, &str); 5] = [
        (
            "SELECT id, created_at, raw_text FROM daily_goals WHERE created_at BETWEEN ?1 AND ?2",
            "goal",
        ),
        (
            "SELECT id, created_at, title FROM tasks WHERE created_at BETWEEN ?1 AND ?2",
            "task",
        ),
        (
            "SELECT id, created_at, text FROM reminders WHERE created_at BETWEEN ?1 AND ?2",
            "reminder",
        ),
        (
            "SELECT id, created_at, title FROM knowledge_notes WHERE created_at BETWEEN ?1 AND ?2",
            "knowledge",
        ),
        (
            "SELECT id, created_at, name FROM detection_rules WHERE created_at BETWEEN ?1 AND ?2",
            "rule",
        ),
    ];
    let mut out = Vec::new();
    for (sql, kind) in queries {
        let mut stmt = conn.prepare(sql)?;
        let rows = stmt.query_map(params![start_ms, end_ms], |r| {
            let at: i64 = r.get(1)?;
            Ok(TimelineEvent {
                id: r.get(0)?,
                kind: kind.to_string(),
                start_ms: at,
                end_ms: at,
                title: r.get(2)?,
                category: Some("milestone".to_string()),
                detail: Some(kind.to_string()),
                severity: None,
            })
        })?;
        for row in rows {
            out.push(row?);
            if out.len() as i64 >= limit {
                return Ok(out);
            }
        }
    }
    Ok(out)
}

fn query_intervention_events(
    conn: &Connection,
    start_ms: i64,
    end_ms: i64,
    limit: i64,
) -> rusqlite::Result<Vec<TimelineEvent>> {
    let mut stmt = conn.prepare(
        "SELECT id, shown_at, intensity, message, rule_id
         FROM interventions
         WHERE shown_at BETWEEN ?1 AND ?2
         ORDER BY shown_at ASC LIMIT ?3",
    )?;
    let rows = stmt
        .query_map(params![start_ms, end_ms, limit], |r| {
            let at: i64 = r.get(1)?;
            Ok(TimelineEvent {
                id: r.get(0)?,
                kind: "intervention".to_string(),
                start_ms: at,
                end_ms: at,
                title: r.get(3)?,
                category: Some("interaction".to_string()),
                detail: r.get(4)?,
                severity: r.get(2)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn query_day_summaries(
    conn: &Connection,
    start_ms: i64,
    end_ms: i64,
) -> rusqlite::Result<Vec<DaySummary>> {
    let mut counts = HashMap::<String, i64>::new();
    {
        let mut stmt = conn.prepare(
            "SELECT strftime('%Y-%m-%d', shown_at / 1000, 'unixepoch', 'localtime'), COUNT(*)
             FROM interventions WHERE shown_at BETWEEN ?1 AND ?2 GROUP BY 1",
        )?;
        for row in stmt.query_map(params![start_ms, end_ms], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        })? {
            let (day, count) = row?;
            counts.insert(day, count);
        }
    }

    let mut stmt = conn.prepare(
        "SELECT strftime('%Y-%m-%d', COALESCE(start_time,event_time,produced_at) / 1000, 'unixepoch', 'localtime') AS day,
                COALESCE(SUM(CASE WHEN start_time IS NOT NULL AND end_time IS NOT NULL
                                  THEN MAX(0, end_time-start_time) ELSE 0 END), 0) AS observed,
                COALESCE(SUM(CASE WHEN category IS NOT NULL AND category NOT LIKE 'entertainment%'
                                       AND start_time IS NOT NULL AND end_time IS NOT NULL
                                  THEN MAX(0, end_time-start_time) ELSE 0 END), 0) AS focus,
                COALESCE(SUM(CASE WHEN category LIKE 'entertainment%'
                                       AND start_time IS NOT NULL AND end_time IS NOT NULL
                                  THEN MAX(0, end_time-start_time) ELSE 0 END), 0) AS entertainment
         FROM raw_events
         WHERE COALESCE(start_time,event_time,produced_at) BETWEEN ?1 AND ?2
         GROUP BY day ORDER BY day ASC",
    )?;
    let rows = stmt
        .query_map(params![start_ms, end_ms], |r| {
            let date: String = r.get(0)?;
            let observed: i64 = r.get(1)?;
            let focus: i64 = r.get(2)?;
            let entertainment: i64 = r.get(3)?;
            let neutral = (observed - focus - entertainment).max(0);
            let intervention_count = counts.get(&date).copied().unwrap_or(0);
            let score = if observed <= 0 {
                50
            } else {
                let focus_share = focus as f64 / observed as f64;
                let entertainment_share = entertainment as f64 / observed as f64;
                (50.0 + focus_share * 40.0
                    - entertainment_share * 30.0
                    - intervention_count as f64 * 3.0)
                    .round()
                    .clamp(0.0, 100.0) as i64
            };
            let start = NaiveDate::parse_from_str(&date, "%Y-%m-%d")
                .ok()
                .and_then(|d| d.and_hms_opt(0, 0, 0))
                .and_then(|d| Local.from_local_datetime(&d).earliest())
                .map(|d| d.timestamp_millis())
                .unwrap_or(start_ms);
            Ok(DaySummary {
                date,
                start_ms: start,
                observed_ms: observed,
                focus_ms: focus,
                entertainment_ms: entertainment,
                neutral_ms: neutral,
                intervention_count,
                state_score: score,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// 最近的干预记录（倒序）。
pub fn list_interventions(conn: &Connection, limit: i64) -> rusqlite::Result<Vec<InterventionRow>> {
    let mut stmt = conn.prepare(
        "SELECT id, rule_id, shown_at, intensity, message, user_response, responded_at, outcome
         FROM interventions ORDER BY shown_at DESC LIMIT ?1",
    )?;
    let rows = stmt
        .query_map(params![limit], |r| {
            Ok(InterventionRow {
                id: r.get(0)?,
                rule_id: r.get(1)?,
                shown_at: r.get(2)?,
                severity: r.get(3)?,
                message: r.get(4)?,
                user_response: r.get(5)?,
                responded_at: r.get(6)?,
                outcome: r.get(7)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}
