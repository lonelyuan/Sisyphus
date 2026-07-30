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

/// 合法的采集源。新增采集端时在这里登记（协议 SPEC §1）。
pub const SOURCES: &[&str] = &[
    "manual",
    "agent",
    "desktop_agent",
    "android_usage",
    "browser_ext",
    "system",
];
pub const LAYERS: &[&str] = &["raw", "derived"];
pub const TIME_MODES: &[&str] = &["point", "interval"];
pub const PRIVACY_LEVELS: &[&str] = &["L0", "L1", "L2", "L3"];

/// 信封校验。`ingest_event` 是唯一写入路径，因此这里是**唯一**能挡住脏事件的地方。
pub fn validate(ev: &NewEvent) -> Result<(), String> {
    if !SOURCES.contains(&ev.source.as_str()) {
        return Err(format!("source '{}' 不在 {SOURCES:?}", ev.source));
    }
    if !LAYERS.contains(&ev.layer.as_str()) {
        return Err(format!("layer '{}' 不在 {LAYERS:?}", ev.layer));
    }
    if !TIME_MODES.contains(&ev.time_mode.as_str()) {
        return Err(format!("time_mode '{}' 不在 {TIME_MODES:?}", ev.time_mode));
    }
    if !PRIVACY_LEVELS.contains(&ev.privacy_level.as_str()) {
        return Err(format!(
            "privacy_level '{}' 不在 {PRIVACY_LEVELS:?}",
            ev.privacy_level
        ));
    }
    if ev.event_type.trim().is_empty() {
        return Err("type 不能为空".to_string());
    }
    match ev.time_mode.as_str() {
        "interval" => {
            let (Some(start), Some(end)) = (ev.start_time, ev.end_time) else {
                return Err("time_mode=interval 必须同时有 start_time 与 end_time".to_string());
            };
            if end < start {
                return Err(format!("区间事件 end_time({end}) 早于 start_time({start})"));
            }
        }
        _ => {
            if ev.event_time.is_none() && ev.start_time.is_none() {
                return Err("time_mode=point 必须有 event_time".to_string());
            }
        }
    }
    // category 是**行为分类命名空间**（entertainment.video / work / …），
    // 不允许塞进 capture 种类等其它语义——混用会让规则的 LIKE 前缀匹配踩雷。
    if let Some(cat) = ev.category.as_deref() {
        if cat.trim().is_empty() {
            return Err("category 若提供则不能为空串".to_string());
        }
    }
    Ok(())
}

/// 写 Event log（raw_events，幂等）+ 入 outbox（同步为 Phase 2，仅排队）。返回 event_id。
///
/// **这是闸门，不是直通管道**：先校验信封（见 [`validate`]），不合法直接拒绝。
/// 架构文档一直写着"校验信封与 privacy_level"，此前实现是纯透传——枚举随便填、
/// interval 事件可以没有 start/end，脏事件会一路流到规则引擎和时间线里。
pub fn ingest_event(
    conn: &Connection,
    user_id: &str,
    device_id: &str,
    ev: NewEvent,
) -> rusqlite::Result<String> {
    if let Err(msg) = validate(&ev) {
        return Err(rusqlite::Error::InvalidParameterName(format!(
            "非法事件信封: {msg}"
        )));
    }
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
    capture_note(conn, user_id, device_id, text, "manual", "note_text")
}

/// 便捷封装：把一份素材写成 `agent/material_text` 事件（第二大脑 ingest_document）。
///
/// 用**独立的 type** 而不是 `category="material"` 区分：`category` 是行为分类命名空间
/// （`entertainment.video` / `work`…），塞进 capture 种类会让规则的前缀匹配和时间线口径踩雷。
/// 原声笔记收件箱只看 `note_text`，素材天然不进去。
pub fn capture_material(
    conn: &Connection,
    user_id: &str,
    device_id: &str,
    text: &str,
) -> rusqlite::Result<String> {
    capture_note(conn, user_id, device_id, text, "agent", "material_text")
}

fn capture_note(
    conn: &Connection,
    user_id: &str,
    device_id: &str,
    text: &str,
    source: &str,
    event_type: &str,
) -> rusqlite::Result<String> {
    ingest_event(
        conn,
        user_id,
        device_id,
        NewEvent {
            event_id: None,
            source: source.to_string(),
            layer: "raw".to_string(),
            event_type: event_type.to_string(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    fn base() -> NewEvent {
        NewEvent {
            event_id: None,
            source: "desktop_agent".into(),
            layer: "raw".into(),
            event_type: "app_foreground".into(),
            time_mode: "interval".into(),
            event_time: None,
            start_time: Some(1_000),
            end_time: Some(2_000),
            entity: Some("com.x".into()),
            category: Some("work".into()),
            payload: serde_json::json!({}),
            parent_event_ids: vec![],
            privacy_level: "L0".into(),
        }
    }

    #[test]
    fn rejects_unknown_enums_and_broken_intervals() {
        let mut ev = base();
        ev.source = "wat".into();
        assert!(validate(&ev).is_err());

        let mut ev = base();
        ev.privacy_level = "L9".into();
        assert!(validate(&ev).is_err());

        let mut ev = base();
        ev.end_time = None;
        assert!(validate(&ev).is_err(), "interval 缺 end_time 必须拒绝");

        let mut ev = base();
        ev.end_time = Some(500);
        assert!(validate(&ev).is_err(), "end 早于 start 必须拒绝");

        let mut ev = base();
        ev.time_mode = "point".into();
        ev.start_time = None;
        ev.end_time = None;
        assert!(validate(&ev).is_err(), "point 缺时间戳必须拒绝");

        assert!(validate(&base()).is_ok());
    }

    #[test]
    fn ingest_rejects_invalid_envelope_instead_of_writing() {
        let conn = db::open(":memory:").unwrap();
        let mut ev = base();
        ev.layer = "nonsense".into();
        assert!(ingest_event(&conn, "u", "d", ev).is_err());
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM raw_events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0, "脏事件不得落库");
    }

    #[test]
    fn material_uses_its_own_type_not_the_category_namespace() {
        let conn = db::open(":memory:").unwrap();
        capture_material(&conn, "u", "d", "一篇文章").unwrap();
        let (t, cat): (String, Option<String>) = conn
            .query_row(
                "SELECT type, category FROM raw_events LIMIT 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(t, "material_text");
        assert_eq!(cat, None);
    }
}
