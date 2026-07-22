//! 主动触发：待办动作队列（[docs/spec/proactive-triggers.md](../../../../../docs/spec/proactive-triggers.md)）。
//!
//! **core 只做纯数据逻辑**：入队 / 取到期 / 标状态 / 周期重排。纯 `rusqlite`、无副作用，安卓可编。
//! 副作用（弹通知 / 拉起 codex / 回写 Notion）由 **app 层**按 `kind` 派发——tokio/进程/通知绝不进 core。
//!
//! 心智：一条队列，`due_at_ms = now` 即"立即"，`= now+Δ` 即"延后"，同一条路径。

use rusqlite::{params, Connection};
use serde::Serialize;
use uuid::Uuid;

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

#[derive(Debug, Clone, Serialize)]
pub struct ScheduledAction {
    pub id: String,
    pub kind: String,
    pub payload_json: String,
    pub due_at_ms: i64,
    pub recurrence: Option<String>,
    pub status: String,
    pub dedup_key: Option<String>,
    pub origin_event_id: Option<String>,
    pub created_by: String,
    pub created_at_ms: i64,
    pub fired_at_ms: Option<i64>,
}

/// 一条待入队的动作。`due_at_ms = now` 即"立即"。
#[derive(Debug, Clone)]
pub struct NewAction<'a> {
    pub kind: &'a str,
    pub payload_json: &'a str,
    pub due_at_ms: i64,
    pub recurrence: Option<&'a str>,
    pub dedup_key: Option<&'a str>,
    pub origin_event_id: Option<&'a str>,
    pub created_by: &'a str,
}

impl<'a> NewAction<'a> {
    /// 便捷构造：立即执行的一次性动作。
    pub fn immediate(kind: &'a str, payload_json: &'a str, created_by: &'a str) -> Self {
        NewAction {
            kind,
            payload_json,
            due_at_ms: now_ms(),
            recurrence: None,
            dedup_key: None,
            origin_event_id: None,
            created_by,
        }
    }
}

/// 入队一个动作。若带 `dedup_key` 且已存在同 key 的 `pending`，跳过（返回 `None`）——防重复打扰。
pub fn enqueue_action(conn: &Connection, a: &NewAction) -> Result<Option<String>, String> {
    if let Some(key) = a.dedup_key {
        let existing: Option<String> = conn
            .query_row(
                "SELECT id FROM scheduled_actions WHERE dedup_key=?1 AND status='pending'",
                params![key],
                |r| r.get(0),
            )
            .ok();
        if existing.is_some() {
            return Ok(None);
        }
    }
    let id = Uuid::new_v4().to_string();
    conn.execute(
        "INSERT INTO scheduled_actions
           (id,kind,payload_json,due_at_ms,recurrence,status,dedup_key,origin_event_id,created_by,created_at_ms,fired_at_ms)
         VALUES (?1,?2,?3,?4,?5,'pending',?6,?7,?8,?9,NULL)",
        params![
            id,
            a.kind,
            a.payload_json,
            a.due_at_ms,
            a.recurrence,
            a.dedup_key,
            a.origin_event_id,
            a.created_by,
            now_ms()
        ],
    )
    .map_err(|e| format!("入队失败: {e}"))?;
    Ok(Some(id))
}

/// 取所有到期（`due_at<=now`）的 pending 动作，**并原子置为 `fired`**（防重启/并发重复执行）。
/// 返回快照供 app 层按 `kind` 派发；执行完 app 再调 [`mark_done`]/[`mark_failed`]，
/// 周期动作由 app 调 [`reschedule`] 排下一次。
pub fn due_actions(conn: &Connection, now: i64) -> Result<Vec<ScheduledAction>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id,kind,payload_json,due_at_ms,recurrence,status,dedup_key,origin_event_id,created_by,created_at_ms,fired_at_ms
             FROM scheduled_actions WHERE status='pending' AND due_at_ms<=?1 ORDER BY due_at_ms",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![now], row_to_action)
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    for a in &rows {
        conn.execute(
            "UPDATE scheduled_actions SET status='fired', fired_at_ms=?2 WHERE id=?1",
            params![a.id, now],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(rows)
}

pub fn mark_done(conn: &Connection, id: &str) -> Result<(), String> {
    set_status(conn, id, "done")
}

pub fn mark_failed(conn: &Connection, id: &str) -> Result<(), String> {
    set_status(conn, id, "failed")
}

pub fn cancel(conn: &Connection, id: &str) -> Result<(), String> {
    set_status(conn, id, "cancelled")
}

fn set_status(conn: &Connection, id: &str, status: &str) -> Result<(), String> {
    conn.execute(
        "UPDATE scheduled_actions SET status=?2 WHERE id=?1",
        params![id, status],
    )
    .map_err(|e| format!("改状态失败: {e}"))?;
    Ok(())
}

/// 周期动作触发后排下一次：按 `action.recurrence` 算出**严格晚于 now** 的下一次 `due_at`，
/// 复制 kind/payload/recurrence/dedup_key/created_by 入队一条新的 pending。非周期或无法解析则不排、返回 `None`。
/// MVP 只支持 `"daily@HH:MM"`（本地时区）。
pub fn reschedule(conn: &Connection, action: &ScheduledAction, now: i64) -> Result<Option<String>, String> {
    let rec = match &action.recurrence {
        Some(r) => r,
        None => return Ok(None),
    };
    let next = match next_due(rec, now) {
        Some(t) => t,
        None => return Ok(None),
    };
    enqueue_action(
        conn,
        &NewAction {
            kind: &action.kind,
            payload_json: &action.payload_json,
            due_at_ms: next,
            recurrence: Some(rec),
            dedup_key: action.dedup_key.as_deref(),
            origin_event_id: action.origin_event_id.as_deref(),
            created_by: &action.created_by,
        },
    )
}

/// 列出 pending 动作（供 app 播种去重 / 调试）。
pub fn list_pending(conn: &Connection) -> Result<Vec<ScheduledAction>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id,kind,payload_json,due_at_ms,recurrence,status,dedup_key,origin_event_id,created_by,created_at_ms,fired_at_ms
             FROM scheduled_actions WHERE status='pending' ORDER BY due_at_ms",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], row_to_action)
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    Ok(rows)
}

/// 解析 recurrence，返回严格晚于 `after_ms` 的下一次触发（epoch ms）。MVP：`"daily@HH:MM"`。
pub fn next_due(recurrence: &str, after_ms: i64) -> Option<i64> {
    let hm = recurrence.strip_prefix("daily@")?;
    let (h, m) = hm.split_once(':')?;
    let hh: u32 = h.trim().parse().ok()?;
    let mm: u32 = m.trim().parse().ok()?;
    if hh > 23 || mm > 59 {
        return None;
    }
    next_daily_due(after_ms, hh, mm)
}

fn next_daily_due(after_ms: i64, hh: u32, mm: u32) -> Option<i64> {
    use chrono::{Duration, Local, TimeZone, Timelike};
    let after = Local.timestamp_millis_opt(after_ms).single()?;
    let mut candidate = after
        .with_hour(hh)?
        .with_minute(mm)?
        .with_second(0)?
        .with_nanosecond(0)?;
    if candidate <= after {
        candidate += Duration::days(1);
    }
    Some(candidate.timestamp_millis())
}

fn row_to_action(r: &rusqlite::Row) -> rusqlite::Result<ScheduledAction> {
    Ok(ScheduledAction {
        id: r.get(0)?,
        kind: r.get(1)?,
        payload_json: r.get(2)?,
        due_at_ms: r.get(3)?,
        recurrence: r.get(4)?,
        status: r.get(5)?,
        dedup_key: r.get(6)?,
        origin_event_id: r.get(7)?,
        created_by: r.get(8)?,
        created_at_ms: r.get(9)?,
        fired_at_ms: r.get(10)?,
    })
}
