//! 唯一写入契约 `ingest_event`：所有采集源 / 反思平面 capture 都经此写入 Event log。
//! 见 docs/spec/architecture.md §3。

use rusqlite::Connection;
use serde::Deserialize;
use uuid::Uuid;
use crate::db;

fn default_privacy() -> String {
    "L0".to_string()
}

/// 一条待写入的事件（BehaviorEvent 信封，时间用本地 epoch ms）。
/// 端侧可不填 `event_id`（自动生成）；`seq_no` / `produced_at` 由本模块补齐。
#[derive(Debug, Deserialize)]
pub struct NewEvent {
    #[serde(default)]
    pub event_id: Option<String>,
    pub source: String,
    pub layer: String,
    #[serde(rename = "type")]
    pub event_type: String,
    pub time_mode: String,
    #[serde(default)]
    pub event_time: Option<i64>,
    #[serde(default)]
    pub start_time: Option<i64>,
    #[serde(default)]
    pub end_time: Option<i64>,
    #[serde(default)]
    pub entity: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub payload: serde_json::Value,
    #[serde(default)]
    pub parent_event_ids: Vec<String>,
    #[serde(default = "default_privacy")]
    pub privacy_level: String,
}

/// 写 Event log（raw_events，幂等）+ 入 outbox（同步为 Phase 2，仅排队）。返回 event_id。
pub fn ingest_event(
    conn: &Connection,
    user_id: &str,
    device_id: &str,
    ev: NewEvent,
) -> rusqlite::Result<String> {
    let event_id = ev.event_id.unwrap_or_else(|| Uuid::new_v4().to_string());
    let produced_at = chrono::Utc::now().timestamp_millis();
    let seq_no = db::next_seq_no(conn, device_id)?;

    let payload = if ev.payload.is_null() {
        serde_json::json!({})
    } else {
        ev.payload
    };
    let payload_json = payload.to_string();
    let parent_json = serde_json::to_string(&ev.parent_event_ids)
        .unwrap_or_else(|_| "[]".to_string());

    db::insert_behavior_event(
        conn,
        &event_id,
        user_id,
        device_id,
        seq_no,
        &ev.source,
        &ev.layer,
        &ev.event_type,
        &ev.time_mode,
        ev.event_time,
        ev.start_time,
        ev.end_time,
        ev.entity.as_deref(),
        ev.category.as_deref(),
        &payload_json,
        &parent_json,
        &ev.privacy_level,
        produced_at,
    )?;

    let envelope = serde_json::json!({
        "schema_version": "1.0",
        "event_id": event_id,
        "user_id": user_id,
        "device_id": device_id,
        "seq_no": seq_no,
        "source": ev.source,
        "layer": ev.layer,
        "type": ev.event_type,
        "time_mode": ev.time_mode,
        "event_time": ev.event_time,
        "start_time": ev.start_time,
        "end_time": ev.end_time,
        "entity": ev.entity,
        "category": ev.category,
        "payload": payload,
        "parent_event_ids": ev.parent_event_ids,
        "privacy_level": ev.privacy_level,
        "produced_at": produced_at,
    });
    db::enqueue_outbox(conn, &event_id, &envelope.to_string(), produced_at)?;

    Ok(event_id)
}

/// 便捷封装：把一句自然语言写成 `manual/note_text` capture 事件（L1）。
/// 反思平面 `capture` 工具用它。见 protocol SPEC §4。
pub fn capture_text(
    conn: &Connection,
    user_id: &str,
    device_id: &str,
    text: &str,
) -> rusqlite::Result<String> {
    ingest_event(
        conn,
        user_id,
        device_id,
        NewEvent {
            event_id: None,
            source: "manual".to_string(),
            layer: "raw".to_string(),
            event_type: "note_text".to_string(),
            time_mode: "point".to_string(),
            event_time: Some(chrono::Utc::now().timestamp_millis()),
            start_time: None,
            end_time: None,
            entity: None,
            category: None,
            payload: serde_json::json!({ "text": text }),
            parent_event_ids: vec![],
            privacy_level: "L1".to_string(),
        },
    )
}
