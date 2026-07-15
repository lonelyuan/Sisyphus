//! 记录视图读模型（感知平面 App「记录」页）：行为会话时间轴 + 干预历史。
//! 只读投影，供 Tauri 命令展示 App 已采集/产出的数据。

use rusqlite::{params, Connection};
use serde::Serialize;

/// 一段前台行为会话（app_foreground 事件投影）。
#[derive(Debug, Serialize)]
pub struct SessionRow {
    pub entity: Option<String>,
    pub category: Option<String>,
    pub start_time: i64,
    pub end_time: Option<i64>,
    /// 时长（ms）；进行中（end 为空）为 None。
    pub duration_ms: Option<i64>,
}

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

/// 最近的前台行为会话（倒序）。
pub fn list_recent_sessions(conn: &Connection, limit: i64) -> rusqlite::Result<Vec<SessionRow>> {
    let mut stmt = conn.prepare(
        "SELECT entity, category, start_time, end_time
         FROM raw_events
         WHERE type = 'app_foreground' AND start_time IS NOT NULL
         ORDER BY start_time DESC LIMIT ?1",
    )?;
    let rows = stmt
        .query_map(params![limit], |r| {
            let start_time: i64 = r.get(2)?;
            let end_time: Option<i64> = r.get(3)?;
            Ok(SessionRow {
                entity: r.get(0)?,
                category: r.get(1)?,
                start_time,
                end_time,
                duration_ms: end_time.map(|e| e - start_time),
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
