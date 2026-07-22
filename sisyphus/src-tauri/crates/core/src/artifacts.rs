//! 反思平面 Artifact store（Phase 1.2 原声笔记）。
//!
//! 唯一写入 artifact 的路径：`propose_intents`（持久化 Codex 生成的候选）→
//! `accept_intent`（把候选落成对应 per-type 表）。保证每个 artifact 都有
//! **来源（capture 事件）+ 置信度 + 可回滚状态**（roadmap Phase 1.0 验收）。
//!
//! Core 不做任何 LLM 推断：分类/意图提取是 Codex（反思平面）的活，
//! 本模块只负责「数据结构保存」（见 docs/spec/architecture.md §2.2、§4）。

use rusqlite::{params, Connection};
use serde::Serialize;
use serde_json::{json, Value};
use uuid::Uuid;

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

// ── 读模型结构 ──────────────────────────────────────────────────────────────

/// 一条原始 capture（note_text 事件的投影）。
#[derive(Debug, Serialize)]
pub struct Capture {
    pub event_id: String,
    pub text: String,
    pub created_at: i64,
}

#[derive(Debug, Serialize)]
pub struct IntentCandidate {
    pub id: String,
    pub capture_event_id: String,
    pub kind: String,
    pub proposed: Value,
    pub confidence: f64,
    pub status: String,
    pub created_at: i64,
}

#[derive(Debug, Serialize)]
pub struct Task {
    pub id: String,
    pub title: String,
    pub status: String,
    pub due_ms: Option<i64>,
    pub priority: i64,
    pub note: Option<String>,
    pub created_at: i64,
}

#[derive(Debug, Serialize)]
pub struct Note {
    pub id: String,
    pub title: Option<String>,
    pub body: String,
    pub tags: Vec<String>,
    pub status: String,
    pub created_at: i64,
}

#[derive(Debug, Serialize)]
pub struct Reminder {
    pub id: String,
    pub text: String,
    pub remind_at_ms: i64,
    pub status: String,
    pub recurrence: Option<String>,
    pub created_at: i64,
}

/// 合法意图种类。
pub const KINDS: &[&str] = &["goal", "task", "reminder", "note"];

// ── Capture inbox ───────────────────────────────────────────────────────────

/// 列出最近的 capture（note_text 事件）。`unprocessed=true` 时排除已生成候选的。
pub fn list_captures(
    conn: &Connection,
    unprocessed: bool,
    limit: i64,
) -> rusqlite::Result<Vec<Capture>> {
    let sql = if unprocessed {
        "SELECT event_id, payload_json, COALESCE(event_time, produced_at)
         FROM raw_events
         WHERE type = 'note_text'
           AND (category IS NULL OR category != 'material')
           AND event_id NOT IN (SELECT capture_event_id FROM intent_candidates)
         ORDER BY COALESCE(event_time, produced_at) DESC LIMIT ?1"
    } else {
        "SELECT event_id, payload_json, COALESCE(event_time, produced_at)
         FROM raw_events
         WHERE type = 'note_text'
           AND (category IS NULL OR category != 'material')
         ORDER BY COALESCE(event_time, produced_at) DESC LIMIT ?1"
    };
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt
        .query_map(params![limit], |r| {
            let payload_json: String = r.get(1)?;
            let text = serde_json::from_str::<Value>(&payload_json)
                .ok()
                .and_then(|v| v.get("text").and_then(|t| t.as_str()).map(String::from))
                .unwrap_or_default();
            Ok(Capture {
                event_id: r.get(0)?,
                text,
                created_at: r.get(2)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

// ── 意图候选（桥） ──────────────────────────────────────────────────────────

/// 持久化一条 Codex 生成的意图候选，返回 candidate id。
pub fn insert_intent_candidate(
    conn: &Connection,
    capture_event_id: &str,
    kind: &str,
    proposed: &Value,
    confidence: f64,
    source: &str,
) -> rusqlite::Result<String> {
    let id = Uuid::new_v4().to_string();
    conn.execute(
        "INSERT INTO intent_candidates
         (id, capture_event_id, kind, proposed_json, confidence, source, status, created_at)
         VALUES (?1,?2,?3,?4,?5,?6,'proposed',?7)",
        params![
            id,
            capture_event_id,
            kind,
            proposed.to_string(),
            confidence,
            source,
            now_ms()
        ],
    )?;
    Ok(id)
}

/// 批量持久化候选（单事务，全成或全不成）：先校验 capture 存在 + 每个 kind 合法，
/// 再在一个事务里插入。防止半成品候选 + 悬空溯源（capture_event_id 不存在）。返回 id 列表。
pub fn insert_intent_candidates(
    conn: &Connection,
    capture_event_id: &str,
    candidates: &[(String, Value, f64)],
    source: &str,
) -> Result<Vec<String>, String> {
    // 溯源不悬空：capture 必须是一条真实的 note_text 事件。
    let exists: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM raw_events WHERE event_id = ?1 AND type = 'note_text'",
            params![capture_event_id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if exists == 0 {
        return Err(format!(
            "capture_event_id 不存在或不是 note_text 事件: {capture_event_id}"
        ));
    }
    for (kind, _, _) in candidates {
        if !KINDS.contains(&kind.as_str()) {
            return Err(format!("非法 kind '{kind}'（应为 {KINDS:?}）"));
        }
    }

    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    let mut ids = Vec::with_capacity(candidates.len());
    for (kind, proposed, confidence) in candidates {
        let id = insert_intent_candidate(&tx, capture_event_id, kind, proposed, *confidence, source)
            .map_err(|e| e.to_string())?;
        ids.push(id);
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(ids)
}

/// 列出意图候选（可按状态过滤）。
pub fn list_intent_candidates(
    conn: &Connection,
    status: Option<&str>,
) -> rusqlite::Result<Vec<IntentCandidate>> {
    let mut stmt = conn.prepare(
        "SELECT id, capture_event_id, kind, proposed_json, confidence, status, created_at
         FROM intent_candidates
         WHERE (?1 IS NULL OR status = ?1)
         ORDER BY created_at DESC",
    )?;
    let rows = stmt
        .query_map(params![status], |r| {
            let proposed_json: String = r.get(3)?;
            Ok(IntentCandidate {
                id: r.get(0)?,
                capture_event_id: r.get(1)?,
                kind: r.get(2)?,
                proposed: serde_json::from_str(&proposed_json).unwrap_or_else(|_| json!({})),
                confidence: r.get(4)?,
                status: r.get(5)?,
                created_at: r.get(6)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// 接受一条意图候选：按 kind 落成对应 artifact，候选转 accepted/edited。返回 artifact id。
/// `edits` 为可选 JSON 对象，覆盖候选中的同名字段（用户在对话里就地修改）。
pub fn accept_intent(
    conn: &Connection,
    intent_id: &str,
    edits: Option<&str>,
) -> Result<String, String> {
    let (kind, proposed_json, capture_event_id, status): (String, String, String, String) = conn
        .query_row(
            "SELECT kind, proposed_json, capture_event_id, status
             FROM intent_candidates WHERE id = ?1",
            params![intent_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .map_err(|e| format!("候选不存在或读取失败: {e}"))?;

    if status == "accepted" || status == "edited" {
        return Err(format!("候选 {intent_id} 已处理（{status}），不重复落库"));
    }
    if status == "ignored" {
        return Err(format!("候选 {intent_id} 已忽略，无法接受"));
    }

    let mut payload: Value = serde_json::from_str(&proposed_json).unwrap_or_else(|_| json!({}));
    let edited = match edits {
        Some(e) if !e.trim().is_empty() => match serde_json::from_str::<Value>(e) {
            Ok(ev) => {
                merge_json(&mut payload, ev);
                true
            }
            Err(err) => return Err(format!("edits 不是合法 JSON: {err}")),
        },
        _ => false,
    };

    // 原子落库：artifact 插入 + 候选状态更新在同一事务，避免「artifact 已建但候选未标记」
    // 导致重试时重复落库。
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;

    let artifact_id = match kind.as_str() {
        "goal" => {
            let text = f_str(&payload, "text")
                .or_else(|| f_str(&payload, "title"))
                .ok_or_else(|| "goal 候选缺少 text/title".to_string())?;
            crate::context::set_goal(&tx, &text).map_err(|e| e.to_string())?
        }
        "task" => {
            let title = f_str(&payload, "title")
                .or_else(|| f_str(&payload, "text"))
                .ok_or_else(|| "task 候选缺少 title".to_string())?;
            create_task(
                &tx,
                &title,
                f_i64(&payload, "due_ms"),
                f_i64(&payload, "priority").unwrap_or(0),
                f_str(&payload, "note").as_deref(),
                Some(&capture_event_id),
                Some(intent_id),
            )
            .map_err(|e| e.to_string())?
        }
        "reminder" => {
            let text = f_str(&payload, "text")
                .or_else(|| f_str(&payload, "title"))
                .ok_or_else(|| "reminder 候选缺少 text".to_string())?;
            let remind_at = f_i64(&payload, "remind_at_ms")
                .ok_or_else(|| "reminder 候选缺少 remind_at_ms（epoch ms）".to_string())?;
            create_reminder(
                &tx,
                remind_at,
                &text,
                f_str(&payload, "recurrence").as_deref(),
                Some(&capture_event_id),
                Some(intent_id),
            )
            .map_err(|e| e.to_string())?
        }
        "note" => {
            let body = f_str(&payload, "body")
                .or_else(|| f_str(&payload, "text"))
                .unwrap_or_default();
            let tags_json = payload
                .get("tags")
                .filter(|t| t.is_array())
                .map(|t| t.to_string())
                .unwrap_or_else(|| "[]".to_string());
            create_note(
                &tx,
                f_str(&payload, "title").as_deref(),
                &body,
                &tags_json,
                Some(&capture_event_id),
                Some(intent_id),
            )
            .map_err(|e| e.to_string())?
        }
        other => return Err(format!("未知意图种类: {other}（应为 {KINDS:?}）")),
    };

    let new_status = if edited { "edited" } else { "accepted" };
    tx.execute(
        "UPDATE intent_candidates SET status = ?1, decided_at = ?2 WHERE id = ?3",
        params![new_status, now_ms(), intent_id],
    )
    .map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())?;

    Ok(artifact_id)
}

/// 忽略一条候选（回滚：不落 artifact，仅置 ignored）。
pub fn ignore_intent(conn: &Connection, intent_id: &str) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE intent_candidates SET status = 'ignored', decided_at = ?1 WHERE id = ?2",
        params![now_ms(), intent_id],
    )?;
    Ok(())
}

// ── artifact 写入（内部；仅经 accept_intent 调用）─────────────────────────────

pub fn create_task(
    conn: &Connection,
    title: &str,
    due_ms: Option<i64>,
    priority: i64,
    note: Option<&str>,
    source_event_id: Option<&str>,
    intent_id: Option<&str>,
) -> rusqlite::Result<String> {
    let id = Uuid::new_v4().to_string();
    conn.execute(
        "INSERT INTO tasks (id, created_at, source_event_id, intent_id, title, status, due_ms, priority, note)
         VALUES (?1,?2,?3,?4,?5,'todo',?6,?7,?8)",
        params![id, now_ms(), source_event_id, intent_id, title, due_ms, priority, note],
    )?;
    Ok(id)
}

pub fn create_note(
    conn: &Connection,
    title: Option<&str>,
    body: &str,
    tags_json: &str,
    source_event_id: Option<&str>,
    intent_id: Option<&str>,
) -> rusqlite::Result<String> {
    let id = Uuid::new_v4().to_string();
    conn.execute(
        "INSERT INTO notes (id, created_at, source_event_id, intent_id, title, body, tags_json, status)
         VALUES (?1,?2,?3,?4,?5,?6,?7,'active')",
        params![id, now_ms(), source_event_id, intent_id, title, body, tags_json],
    )?;
    Ok(id)
}

pub fn create_reminder(
    conn: &Connection,
    remind_at_ms: i64,
    text: &str,
    recurrence: Option<&str>,
    source_event_id: Option<&str>,
    intent_id: Option<&str>,
) -> rusqlite::Result<String> {
    let id = Uuid::new_v4().to_string();
    conn.execute(
        "INSERT INTO reminders (id, created_at, source_event_id, intent_id, remind_at_ms, text, status, recurrence)
         VALUES (?1,?2,?3,?4,?5,?6,'pending',?7)",
        params![id, now_ms(), source_event_id, intent_id, remind_at_ms, text, recurrence],
    )?;
    Ok(id)
}

// ── artifact 查询（供 query_context / today_actions）──────────────────────────

pub fn list_open_tasks(conn: &Connection) -> rusqlite::Result<Vec<Task>> {
    let mut stmt = conn.prepare(
        "SELECT id, title, status, due_ms, priority, note, created_at
         FROM tasks WHERE status IN ('todo','doing')
         ORDER BY priority DESC, created_at ASC",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok(Task {
                id: r.get(0)?,
                title: r.get(1)?,
                status: r.get(2)?,
                due_ms: r.get(3)?,
                priority: r.get(4)?,
                note: r.get(5)?,
                created_at: r.get(6)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

pub fn list_due_reminders(conn: &Connection, now_ms: i64) -> rusqlite::Result<Vec<Reminder>> {
    let mut stmt = conn.prepare(
        "SELECT id, text, remind_at_ms, status, recurrence, created_at
         FROM reminders WHERE status = 'pending' AND remind_at_ms <= ?1
         ORDER BY remind_at_ms ASC",
    )?;
    let rows = stmt
        .query_map(params![now_ms], |r| {
            Ok(Reminder {
                id: r.get(0)?,
                text: r.get(1)?,
                remind_at_ms: r.get(2)?,
                status: r.get(3)?,
                recurrence: r.get(4)?,
                created_at: r.get(5)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

pub fn update_task_status(conn: &Connection, id: &str, status: &str) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE tasks SET status = ?1 WHERE id = ?2",
        params![status, id],
    )?;
    Ok(())
}

pub fn complete_reminder(conn: &Connection, id: &str) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE reminders SET status = 'done' WHERE id = ?1",
        params![id],
    )?;
    Ok(())
}

/// 全部任务（未完成在前，再按创建时间倒序）；供 App 任务管理页。
pub fn list_tasks(conn: &Connection, limit: i64) -> rusqlite::Result<Vec<Task>> {
    let mut stmt = conn.prepare(
        "SELECT id, title, status, due_ms, priority, note, created_at
         FROM tasks
         ORDER BY CASE status WHEN 'todo' THEN 0 WHEN 'doing' THEN 0 ELSE 1 END,
                  priority DESC, created_at DESC
         LIMIT ?1",
    )?;
    let rows = stmt
        .query_map(params![limit], |r| {
            Ok(Task {
                id: r.get(0)?,
                title: r.get(1)?,
                status: r.get(2)?,
                due_ms: r.get(3)?,
                priority: r.get(4)?,
                note: r.get(5)?,
                created_at: r.get(6)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

pub fn delete_task(conn: &Connection, id: &str) -> rusqlite::Result<()> {
    conn.execute("DELETE FROM tasks WHERE id = ?1", params![id])?;
    Ok(())
}

/// 全部提醒（未取消，按时间倒序）。
pub fn list_reminders(conn: &Connection, limit: i64) -> rusqlite::Result<Vec<Reminder>> {
    let mut stmt = conn.prepare(
        "SELECT id, text, remind_at_ms, status, recurrence, created_at
         FROM reminders WHERE status != 'cancelled'
         ORDER BY remind_at_ms DESC LIMIT ?1",
    )?;
    let rows = stmt
        .query_map(params![limit], |r| {
            Ok(Reminder {
                id: r.get(0)?,
                text: r.get(1)?,
                remind_at_ms: r.get(2)?,
                status: r.get(3)?,
                recurrence: r.get(4)?,
                created_at: r.get(5)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

pub fn cancel_reminder(conn: &Connection, id: &str) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE reminders SET status = 'cancelled' WHERE id = ?1",
        params![id],
    )?;
    Ok(())
}

/// 取出到期待触发的提醒（pending 且 remind_at<=now），原子标记为 `fired` 防重复触发，返回它们。
/// 采集器/前台服务每 tick 调用 → 到点弹通知。（recurrence 复发 MVP 暂不处理。）
pub fn take_due_reminders(conn: &Connection, now_ms: i64) -> rusqlite::Result<Vec<Reminder>> {
    let tx = conn.unchecked_transaction()?;
    let mut due: Vec<Reminder> = Vec::new();
    {
        let mut stmt = tx.prepare(
            "SELECT id, text, remind_at_ms, status, recurrence, created_at
             FROM reminders WHERE status = 'pending' AND remind_at_ms <= ?1
             ORDER BY remind_at_ms ASC",
        )?;
        let rows = stmt.query_map(params![now_ms], |r| {
            Ok(Reminder {
                id: r.get(0)?,
                text: r.get(1)?,
                remind_at_ms: r.get(2)?,
                status: r.get(3)?,
                recurrence: r.get(4)?,
                created_at: r.get(5)?,
            })
        })?;
        for r in rows {
            due.push(r?);
        }
    }
    for r in &due {
        tx.execute(
            "UPDATE reminders SET status = 'fired' WHERE id = ?1",
            params![r.id],
        )?;
    }
    tx.commit()?;
    Ok(due)
}

// ── helpers ───────────────────────────────────────────────────────────────

fn f_str(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(|x| x.as_str()).map(String::from)
}

fn f_i64(v: &Value, key: &str) -> Option<i64> {
    v.get(key).and_then(|x| x.as_i64())
}

/// 用 overlay 的对象字段覆盖 base（浅合并）；非对象则整体替换。
fn merge_json(base: &mut Value, overlay: Value) {
    match (base, overlay) {
        (Value::Object(b), Value::Object(o)) => {
            for (k, v) in o {
                b.insert(k, v);
            }
        }
        (b, o) => *b = o,
    }
}
