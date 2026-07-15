// 本地 SQLite 管理（rusqlite）
// schema 与 packages/protocol/SPEC.md 一致，字段命名 snake_case。

use rusqlite::{Connection, Result, params};
use crate::rule_engine::DailyGoal;

pub fn open(path: &str) -> Result<Connection> {
    let conn = Connection::open(path)?;
    conn.execute_batch(SCHEMA)?;
    Ok(conn)
}

// ── DAO 函数 ──────────────────────────────────────────────────────────────────

/// 窗口内娱乐 app 已关闭会话的总时长（ms）。
/// 防漏算：仍在前台的会话不在 DB，调用方需补入 active_entertainment_ms。
pub fn sum_entertainment_ms(conn: &Connection, user_id: &str, since_ms: i64) -> Result<i64> {
    conn.query_row(
        "SELECT COALESCE(SUM(end_time - start_time), 0)
         FROM raw_events
         WHERE user_id = ?1 AND layer = 'raw' AND type = 'app_foreground'
           AND category LIKE 'entertainment%'
           AND start_time >= ?2 AND end_time IS NOT NULL",
        params![user_id, since_ms],
        |r| r.get(0),
    )
}

/// 查询指定日期的今日目标（取最新一条 planned/started）。
pub fn get_today_goal(conn: &Connection, date: &str) -> Result<Option<DailyGoal>> {
    let mut stmt = conn.prepare(
        "SELECT id, date, raw_text, status FROM daily_goals
         WHERE date = ?1 AND status IN ('planned','started')
         ORDER BY created_at DESC LIMIT 1",
    )?;
    let mut rows = stmt.query(params![date])?;
    if let Some(row) = rows.next()? {
        Ok(Some(DailyGoal {
            id: row.get(0)?,
            date: row.get(1)?,
            raw_text: row.get(2)?,
            status: row.get(3)?,
        }))
    } else {
        Ok(None)
    }
}

/// 更新目标状态。
pub fn update_goal_status(conn: &Connection, id: &str, status: &str) -> Result<()> {
    conn.execute(
        "UPDATE daily_goals SET status = ?1 WHERE id = ?2",
        params![status, id],
    )?;
    Ok(())
}

/// 插入干预记录。
pub fn insert_intervention(
    conn: &Connection,
    id: &str,
    rule_id: &str,
    shown_at: i64,
    intensity: &str,
    message: &str,
    options_json: &str,
) -> Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO interventions
         (id, rule_id, shown_at, intensity, message, options_json)
         VALUES (?1,?2,?3,?4,?5,?6)",
        params![id, rule_id, shown_at, intensity, message, options_json],
    )?;
    Ok(())
}

/// 记录用户对干预通知的响应。
pub fn update_intervention_response(
    conn: &Connection,
    id: &str,
    action: &str,
) -> Result<()> {
    let now_ms = chrono::Utc::now().timestamp_millis();
    conn.execute(
        "UPDATE interventions SET user_response = ?1, responded_at = ?2 WHERE id = ?3",
        params![action, now_ms, id],
    )?;
    Ok(())
}

/// 冷却检查：距上次触发同一规则是否已超过 cooldown_ms。
pub fn is_cooldown_ready(
    conn: &Connection,
    rule_id: &str,
    now_ms: i64,
    cooldown_ms: i64,
) -> Result<bool> {
    let last: Option<i64> = conn.query_row(
        "SELECT MAX(shown_at) FROM interventions WHERE rule_id = ?1",
        params![rule_id],
        |r| r.get(0),
    ).or_else(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => Ok(None),
        other => Err(other),
    })?;
    Ok(last.map_or(true, |t| now_ms - t > cooldown_ms))
}

/// 今日娱乐总时长（ms），含活跃会话。
pub fn today_entertainment_ms(
    conn: &Connection,
    user_id: &str,
    date_start_ms: i64,
) -> Result<i64> {
    sum_entertainment_ms(conn, user_id, date_start_ms)
}

/// 今日干预次数。
pub fn today_intervention_count(conn: &Connection, date_start_ms: i64) -> Result<i64> {
    conn.query_row(
        "SELECT COUNT(*) FROM interventions WHERE shown_at >= ?1",
        params![date_start_ms],
        |r| r.get(0),
    )
}

/// 单设备内下一个 seq_no（单调递增，用于增量拉取与查漏）。
pub fn next_seq_no(conn: &Connection, device_id: &str) -> Result<i64> {
    let max: Option<i64> = conn
        .query_row(
            "SELECT MAX(seq_no) FROM raw_events WHERE device_id = ?1",
            params![device_id],
            |r| r.get(0),
        )
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })?;
    Ok(max.unwrap_or(0) + 1)
}

/// 写入一条完整信封事件到 Event log（append-only，`event_id` 幂等）。
/// 这是唯一写入路径 `ingest_event` 的底层实现（见 docs/spec/architecture.md §3）。
#[allow(clippy::too_many_arguments)]
pub fn insert_behavior_event(
    conn: &Connection,
    event_id: &str,
    user_id: &str,
    device_id: &str,
    seq_no: i64,
    source: &str,
    layer: &str,
    event_type: &str,
    time_mode: &str,
    event_time: Option<i64>,
    start_time: Option<i64>,
    end_time: Option<i64>,
    entity: Option<&str>,
    category: Option<&str>,
    payload_json: &str,
    parent_event_ids_json: &str,
    privacy_level: &str,
    produced_at: i64,
) -> Result<()> {
    let now_ms = chrono::Utc::now().timestamp_millis();
    conn.execute(
        "INSERT OR IGNORE INTO raw_events
         (event_id, user_id, device_id, seq_no, source, layer, type, time_mode,
          event_time, start_time, end_time, entity, category, payload_json,
          parent_event_ids, privacy_level, produced_at, ingested_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18)",
        params![
            event_id, user_id, device_id, seq_no, source, layer, event_type, time_mode,
            event_time, start_time, end_time, entity, category, payload_json,
            parent_event_ids_json, privacy_level, produced_at, now_ms
        ],
    )?;
    Ok(())
}

/// 入 outbox 上传队列（同步是 Phase 2，现在只排队不上传）。
pub fn enqueue_outbox(
    conn: &Connection,
    event_id: &str,
    payload_json: &str,
    created_at: i64,
) -> Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO outbox (event_id, payload_json, sync_status, retry_count, created_at)
         VALUES (?1, ?2, 'pending', 0, ?3)",
        params![event_id, payload_json, created_at],
    )?;
    Ok(())
}

// ── Schema ────────────────────────────────────────────────────────────────────

const SCHEMA: &str = r#"
PRAGMA journal_mode=WAL;
PRAGMA foreign_keys=ON;

-- 事件 outbox：采集后先写这里，批量上传到 Supabase 后标记 done。
CREATE TABLE IF NOT EXISTS outbox (
    event_id     TEXT PRIMARY KEY,
    payload_json TEXT NOT NULL,
    sync_status  TEXT NOT NULL DEFAULT 'pending',  -- pending | uploading | done | failed
    retry_count  INTEGER NOT NULL DEFAULT 0,
    created_at   INTEGER NOT NULL                  -- epoch ms
);

-- 本地原始事件缓存（采集后写入，用于规则引擎查询）。
-- 字段与 SPEC.md §1 信封保持一致。
CREATE TABLE IF NOT EXISTS raw_events (
    event_id         TEXT PRIMARY KEY,
    schema_version   TEXT NOT NULL DEFAULT '1.0',
    user_id          TEXT NOT NULL,
    device_id        TEXT NOT NULL,
    seq_no           INTEGER NOT NULL,
    source           TEXT NOT NULL,
    layer            TEXT NOT NULL,
    type             TEXT NOT NULL,
    time_mode        TEXT NOT NULL,
    event_time       INTEGER,   -- epoch ms，point
    start_time       INTEGER,   -- epoch ms，interval
    end_time         INTEGER,
    entity           TEXT,
    category         TEXT,
    payload_json     TEXT NOT NULL DEFAULT '{}',
    parent_event_ids TEXT NOT NULL DEFAULT '[]',
    privacy_level    TEXT NOT NULL DEFAULT 'L0',
    produced_at      INTEGER NOT NULL,
    ingested_at      INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_raw_user_layer_time
    ON raw_events (user_id, layer, COALESCE(start_time, event_time) DESC);
CREATE INDEX IF NOT EXISTS idx_raw_user_category
    ON raw_events (user_id, category) WHERE category IS NOT NULL;
CREATE UNIQUE INDEX IF NOT EXISTS idx_raw_device_seq
    ON raw_events (device_id, seq_no);

-- 今日目标。
CREATE TABLE IF NOT EXISTS daily_goals (
    id         TEXT PRIMARY KEY,
    date       TEXT NOT NULL,   -- "2026-06-29"
    raw_text   TEXT NOT NULL,
    status     TEXT NOT NULL DEFAULT 'planned',  -- planned|started|completed|skipped|abandoned
    created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_goal_date ON daily_goals (date);

-- 干预记录。
CREATE TABLE IF NOT EXISTS interventions (
    id              TEXT PRIMARY KEY,
    trigger_event_id TEXT,
    rule_id         TEXT NOT NULL,
    shown_at        INTEGER NOT NULL,
    intensity       TEXT NOT NULL,
    message         TEXT NOT NULL,
    options_json    TEXT NOT NULL,
    user_response   TEXT,
    responded_at    INTEGER,
    outcome         TEXT
);

-- ── 反思平面 Artifact store（Phase 1.2：原声笔记）────────────────────────────
-- 铁律：每种有状态对象各自建表，禁止多态大表。intent_candidates 是 capture→artifact
-- 的「桥/暂存」（非 artifact 本身），承载来源/置信度/可回滚审计。

-- 意图候选：Codex 对一条 capture 生成的结构化候选（Core 不做推断，只持久化）。
CREATE TABLE IF NOT EXISTS intent_candidates (
    id                TEXT PRIMARY KEY,
    capture_event_id  TEXT NOT NULL,               -- → raw_events.event_id（note_text）
    kind              TEXT NOT NULL,               -- goal | task | reminder | note
    proposed_json     TEXT NOT NULL,               -- 候选内容（title/body/due 等）
    confidence        REAL NOT NULL DEFAULT 0,
    source            TEXT NOT NULL DEFAULT 'agent',
    status            TEXT NOT NULL DEFAULT 'proposed', -- proposed|accepted|edited|ignored
    created_at        INTEGER NOT NULL,
    decided_at        INTEGER
);
CREATE INDEX IF NOT EXISTS idx_intent_capture ON intent_candidates (capture_event_id);
CREATE INDEX IF NOT EXISTS idx_intent_status  ON intent_candidates (status);

-- 任务。
CREATE TABLE IF NOT EXISTS tasks (
    id              TEXT PRIMARY KEY,
    created_at      INTEGER NOT NULL,
    source_event_id TEXT,                          -- 源 capture 事件（溯源便利）
    intent_id       TEXT,                          -- → intent_candidates.id（回滚锚点）
    title           TEXT NOT NULL,
    status          TEXT NOT NULL DEFAULT 'todo',  -- todo|doing|done|dropped
    due_ms          INTEGER,
    priority        INTEGER NOT NULL DEFAULT 0,
    note            TEXT
);
CREATE INDEX IF NOT EXISTS idx_tasks_status ON tasks (status);

-- 笔记 / 素材 / 偏好（打 tag 区分）。
CREATE TABLE IF NOT EXISTS notes (
    id              TEXT PRIMARY KEY,
    created_at      INTEGER NOT NULL,
    source_event_id TEXT,
    intent_id       TEXT,
    title           TEXT,
    body            TEXT NOT NULL DEFAULT '',
    tags_json       TEXT NOT NULL DEFAULT '[]',
    status          TEXT NOT NULL DEFAULT 'active' -- active|archived
);

-- 提醒（MVP 被动：query_context 暴露到期项，主动触发留待采集器后续接线）。
CREATE TABLE IF NOT EXISTS reminders (
    id              TEXT PRIMARY KEY,
    created_at      INTEGER NOT NULL,
    source_event_id TEXT,
    intent_id       TEXT,
    remind_at_ms    INTEGER NOT NULL,
    text            TEXT NOT NULL,
    status          TEXT NOT NULL DEFAULT 'pending', -- pending|done|cancelled
    recurrence      TEXT
);
CREATE INDEX IF NOT EXISTS idx_reminders_due ON reminders (status, remind_at_ms);

-- ── 第二大脑知识索引（Phase 1.3）─────────────────────────────────────────────
-- 可读知识本体是 Obsidian vault 的 .md（投影）；本表是可查询真相 + 溯源指针。
CREATE TABLE IF NOT EXISTS knowledge_notes (
    id           TEXT PRIMARY KEY,
    path         TEXT NOT NULL UNIQUE,          -- vault 内相对路径，如 "ai-security.md"
    title        TEXT NOT NULL,
    tags_json    TEXT NOT NULL DEFAULT '[]',
    sources_json TEXT NOT NULL DEFAULT '[]',
    content_hash TEXT NOT NULL,
    status       TEXT NOT NULL DEFAULT 'active', -- active|stale|duplicate|pruned
    created_at   INTEGER NOT NULL,
    updated_at   INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_knowledge_status ON knowledge_notes (status);

-- 用户可编辑的监控名单（哪些 app 算娱乐）。分类判定：此表 > 内置白名单。
-- 桌面采集器与 Android JNI 都读它，故加/删一个 app 跨端立即生效、无需重编。
CREATE TABLE IF NOT EXISTS monitored_apps (
    bundle_id  TEXT PRIMARY KEY,          -- macOS bundle id 或 Android 包名
    category   TEXT NOT NULL,             -- entertainment.video|game|social|news
    created_at INTEGER NOT NULL
);
"#;
