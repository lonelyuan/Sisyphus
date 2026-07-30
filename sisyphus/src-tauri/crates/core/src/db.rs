// 本地 SQLite 管理（rusqlite）
// schema 与 packages/protocol/SPEC.md 一致，字段命名 snake_case。

use crate::rule_engine::DailyGoal;
use rusqlite::{params, Connection, Result};
use std::time::Duration;

/// 跨进程并发（App/collector/scheduler/mcp 同开一库）：WAL + busy_timeout 处理写争用，
/// 而不是让并发写直接抛 SQLITE_BUSY 给调用方。
///
/// 打开顺序：`SCHEMA`（最新定义，新库一次建好）→ [`crate::migrations::run`]（把老库补齐）。
///
/// 两条必须守的规则：
/// 1. **给已有表加列/改约束一律走 migrations**——`CREATE TABLE IF NOT EXISTS` 对已存在的表是空操作。
/// 2. **SCHEMA 里不许对"迁移才加的列"建索引**。`execute_batch` 遇到失败语句会**中止整批**，
///    于是它后面的所有建表语句都不会执行——老库会缺一堆表，而错误信息只提到那个索引。
///    索引跟着加列的那条迁移一起建（已用真实库验证过这个坑）。
pub fn open(path: &str) -> Result<Connection> {
    let conn = Connection::open(path)?;
    conn.busy_timeout(Duration::from_secs(5))?;
    conn.execute_batch(SCHEMA)?;
    crate::migrations::run(&conn)?;
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

/// 窗口内匹配指定分类 / app 的已闭合前台会话总时长（ms）。供动态规则通用统计：
/// 分类维度 = (category LIKE prefix%) OR (category IN categories)；再 AND (entity IN apps)。
/// 三者皆空则不加分类/应用过滤（统计全部前台）。防漏算的进行中会话由调用方另行补入。
pub fn sum_foreground_ms(
    conn: &Connection,
    user_id: &str,
    since_ms: i64,
    category_prefix: Option<&str>,
    categories: &[String],
    apps: &[String],
) -> Result<i64> {
    use rusqlite::types::Value;
    let mut sql = String::from(
        "SELECT COALESCE(SUM(end_time - start_time), 0) FROM raw_events \
         WHERE user_id = ? AND layer = 'raw' AND type = 'app_foreground' \
           AND start_time >= ? AND end_time IS NOT NULL",
    );
    let mut binds: Vec<Value> = vec![Value::Text(user_id.to_string()), Value::Integer(since_ms)];

    let mut cat_clauses: Vec<String> = Vec::new();
    if let Some(prefix) = category_prefix.filter(|p| !p.is_empty()) {
        cat_clauses.push("category LIKE ?".to_string());
        binds.push(Value::Text(format!("{prefix}%")));
    }
    if !categories.is_empty() {
        let ph = vec!["?"; categories.len()].join(",");
        cat_clauses.push(format!("category IN ({ph})"));
        binds.extend(categories.iter().map(|c| Value::Text(c.clone())));
    }
    if !cat_clauses.is_empty() {
        sql.push_str(&format!(" AND ({})", cat_clauses.join(" OR ")));
    }
    if !apps.is_empty() {
        let ph = vec!["?"; apps.len()].join(",");
        sql.push_str(&format!(" AND entity IN ({ph})"));
        binds.extend(apps.iter().map(|a| Value::Text(a.clone())));
    }

    conn.query_row(&sql, rusqlite::params_from_iter(binds), |r| r.get(0))
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
pub fn update_intervention_response(conn: &Connection, id: &str, action: &str) -> Result<()> {
    let now_ms = chrono::Utc::now().timestamp_millis();
    conn.execute(
        "UPDATE interventions SET user_response = ?1, responded_at = ?2 WHERE id = ?3",
        params![action, now_ms, id],
    )?;
    Ok(())
}

/// 冷却检查：距上次**响应**同一规则是否已超过 cooldown_ms。
///
/// ⚠️ 读的是 `rule_fires`（"这条规则产生过响应"）而**不是** `interventions`（"通知真的弹了"）。
/// 二者不同：`Deferred` / `Debounce` 策略在命中时只入队、并不立刻弹通知，若冷却按
/// `interventions.shown_at` 判断，则冷却永远 ready，采集器每一拍（5–15s）都会重新入队一条
/// —— 十分钟延迟的规则会攒出几十条通知同时炸出来。响应事实必须在**命中当拍**落库。
pub fn is_cooldown_ready(
    conn: &Connection,
    rule_id: &str,
    now_ms: i64,
    cooldown_ms: i64,
) -> Result<bool> {
    let last: Option<i64> = conn
        .query_row(
            "SELECT MAX(fired_at_ms) FROM rule_fires WHERE rule_id = ?1",
            params![rule_id],
            |r| r.get(0),
        )
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })?;
    Ok(last.map_or(true, |t| now_ms - t > cooldown_ms))
}

/// 记一条"规则已响应"事实（冷却与去重的唯一依据）。
/// `policy` = immediate | deferred | debounce | suppress；`dedup_key` 供 debounce 窗口判断。
pub fn record_rule_fire(
    conn: &Connection,
    rule_id: &str,
    fired_at_ms: i64,
    policy: &str,
    dedup_key: Option<&str>,
) -> Result<()> {
    conn.execute(
        "INSERT INTO rule_fires (rule_id, fired_at_ms, policy, dedup_key)
         VALUES (?1,?2,?3,?4)",
        params![rule_id, fired_at_ms, policy, dedup_key],
    )?;
    Ok(())
}

/// debounce 窗口：同 `dedup_key` 在 `window_ms` 内是否已响应过。
pub fn debounced_recently(
    conn: &Connection,
    dedup_key: &str,
    now_ms: i64,
    window_ms: i64,
) -> Result<bool> {
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM rule_fires WHERE dedup_key = ?1 AND fired_at_ms > ?2",
        params![dedup_key, now_ms - window_ms.max(0)],
        |r| r.get(0),
    )?;
    Ok(n > 0)
}

/// 回填近端结果（干预后 N 分钟用户在干什么）。`outcome` 是归一化标签，`detail` 存实际分类/app。
pub fn record_intervention_outcome(
    conn: &Connection,
    id: &str,
    outcome: &str,
    detail: Option<&str>,
    observed_at_ms: i64,
) -> Result<()> {
    conn.execute(
        "UPDATE interventions SET outcome=?2, outcome_detail=?3, outcome_at_ms=?4
         WHERE id=?1 AND outcome IS NULL",
        params![id, outcome, detail, observed_at_ms],
    )?;
    Ok(())
}

/// 今日娱乐总时长（ms），含活跃会话。
pub fn today_entertainment_ms(conn: &Connection, user_id: &str, date_start_ms: i64) -> Result<i64> {
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

/// 窗口 `[from, to)` 内的（娱乐时长, 全部前台观测时长），会话与窗口取交集。
/// 近端结果观察用它算"提醒后这段时间里娱乐占比多少"。
pub fn category_split_between(conn: &Connection, from_ms: i64, to_ms: i64) -> Result<(i64, i64)> {
    conn.query_row(
        "SELECT
           COALESCE(SUM(CASE WHEN category LIKE 'entertainment%'
                             THEN MAX(0, MIN(end_time, ?2) - MAX(start_time, ?1)) ELSE 0 END), 0),
           COALESCE(SUM(MAX(0, MIN(end_time, ?2) - MAX(start_time, ?1))), 0)
         FROM raw_events
         WHERE layer='raw' AND type='app_foreground'
           AND start_time IS NOT NULL AND end_time IS NOT NULL
           AND start_time < ?2 AND end_time > ?1",
        params![from_ms, to_ms],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
}

/// 单设备内下一个 seq_no（单调递增，用于增量拉取与查漏）。
///
/// **不能用 `MAX(seq_no)+1`**：`raw_events` 上有 `UNIQUE(device_id, seq_no)`，而写入是
/// `INSERT OR IGNORE` —— 两个同 device_id 的写入者并发算出同一个 seq_no 时，后一条会被
/// 静默吞掉，函数却照样返回一个库里并不存在的 event_id（Pi 与 Codex 拉起的 MCP 都用
/// `agent-mcp`，主对话与定时任务重叠时就会撞）。改用计数器表单语句原子自增。
pub fn next_seq_no(conn: &Connection, device_id: &str) -> Result<i64> {
    conn.query_row(
        "INSERT INTO device_seq (device_id, next_seq)
         VALUES (?1, (SELECT COALESCE(MAX(seq_no), 0) + 1 FROM raw_events WHERE device_id = ?1))
         ON CONFLICT(device_id) DO UPDATE SET next_seq = next_seq + 1
         RETURNING next_seq",
        params![device_id],
        |r| r.get(0),
    )
}

/// 写入一条完整信封事件到 Event log（append-only，`event_id` 幂等）。
/// 这是唯一写入路径 `ingest_event` 的底层实现（见 docs/spec/architecture.md §3）。
///
/// `INSERT OR IGNORE` 只应吞掉"同 event_id 重复提交"这一种情况。若没插进去且 event_id
/// 也不在库里，说明撞的是别的唯一约束（如 seq_no）——那是**静默丢事件**，必须报错而不是假装成功。
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
    let changed = conn.execute(
        "INSERT OR IGNORE INTO raw_events
         (event_id, user_id, device_id, seq_no, source, layer, type, time_mode,
          event_time, start_time, end_time, entity, category, payload_json,
          parent_event_ids, privacy_level, produced_at, ingested_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18)",
        params![
            event_id,
            user_id,
            device_id,
            seq_no,
            source,
            layer,
            event_type,
            time_mode,
            event_time,
            start_time,
            end_time,
            entity,
            category,
            payload_json,
            parent_event_ids_json,
            privacy_level,
            produced_at,
            now_ms
        ],
    )?;
    if changed == 0 {
        let exists: i64 = conn.query_row(
            "SELECT COUNT(*) FROM raw_events WHERE event_id = ?1",
            params![event_id],
            |r| r.get(0),
        )?;
        if exists == 0 {
            return Err(rusqlite::Error::StatementChangedRows(0));
        }
    }
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

-- Core 行为开关的本地 KV（换日点、rollup 保留窗口…）。凭据/模型配置不进这里。
CREATE TABLE IF NOT EXISTS settings (
    key        TEXT PRIMARY KEY,
    value      TEXT NOT NULL,
    updated_at INTEGER NOT NULL
);

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
CREATE INDEX IF NOT EXISTS idx_raw_timeline_time
    ON raw_events (COALESCE(start_time, event_time, produced_at));
CREATE UNIQUE INDEX IF NOT EXISTS idx_raw_device_seq
    ON raw_events (device_id, seq_no);
CREATE INDEX IF NOT EXISTS idx_raw_type_time
    ON raw_events (type, COALESCE(start_time, event_time, produced_at));
CREATE INDEX IF NOT EXISTS idx_raw_ingested ON raw_events (ingested_at);

-- 每设备 seq_no 计数器：单语句原子自增，避免并发 MAX+1 撞 UNIQUE 导致静默丢事件。
CREATE TABLE IF NOT EXISTS device_seq (
    device_id TEXT PRIMARY KEY,
    next_seq  INTEGER NOT NULL
);

-- 今日目标。
CREATE TABLE IF NOT EXISTS daily_goals (
    id         TEXT PRIMARY KEY,
    date       TEXT NOT NULL,   -- "2026-06-29"
    raw_text   TEXT NOT NULL,
    status     TEXT NOT NULL DEFAULT 'planned',  -- planned|started|completed|skipped|abandoned
    created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_goal_date ON daily_goals (date);

-- 干预记录。outcome_* 是近端结果观察（干预后 N 分钟用户在干什么）——1.1 唯一的学习信号。
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
    outcome         TEXT,
    outcome_detail  TEXT,
    outcome_at_ms   INTEGER
);
CREATE INDEX IF NOT EXISTS idx_interventions_shown ON interventions (shown_at);

-- 规则响应事实（append-only）：命中当拍就写，**与"通知有没有真的弹"无关**。
-- 冷却与 debounce 窗口只看这张表；否则 Deferred/Debounce 策略下冷却永远 ready，
-- 采集器每拍都会重复入队（十分钟延迟 → 几十条通知同时炸）。
CREATE TABLE IF NOT EXISTS rule_fires (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    rule_id     TEXT NOT NULL,
    fired_at_ms INTEGER NOT NULL,
    policy      TEXT NOT NULL,     -- immediate | deferred | debounce | suppress
    dedup_key   TEXT
);
CREATE INDEX IF NOT EXISTS idx_rule_fires_rule ON rule_fires (rule_id, fired_at_ms DESC);
CREATE INDEX IF NOT EXISTS idx_rule_fires_dedup ON rule_fires (dedup_key, fired_at_ms DESC);


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
-- `body` 进索引是为了**查重能搜正文**——只按标题查重是碎片化的机械原因。
-- `title` 存活行唯一（部分唯一索引）：一个概念一张卡，其余走 aliases 重定向。
CREATE TABLE IF NOT EXISTS knowledge_notes (
    id           TEXT PRIMARY KEY,
    path         TEXT NOT NULL UNIQUE,          -- vault 内相对路径，如 "kb/web-security/sql-注入.md"
    folder       TEXT NOT NULL DEFAULT '',      -- 话题领域（= 博客栏目），一律以 kb/ 开头
    title        TEXT NOT NULL,
    body         TEXT NOT NULL DEFAULT '',      -- 正文（供正文查重；vault 的 .md 才是本体）
    tags_json    TEXT NOT NULL DEFAULT '[]',
    sources_json TEXT NOT NULL DEFAULT '[]',
    aliases_json TEXT NOT NULL DEFAULT '[]',    -- 重定向：旧标题 → 本卡
    content_hash TEXT NOT NULL,
    status       TEXT NOT NULL DEFAULT 'active', -- active|stale|duplicate|pruned
    created_at   INTEGER NOT NULL,
    updated_at   INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_knowledge_status ON knowledge_notes (status);
-- idx_knowledge_folder / idx_knowledge_title_unique 在 migrations 里建（依赖迁移才加的列）。

-- 卡片间的 wikilink 边（从正文解析出来，写入时重建）。
-- 未解析的目标就是**红链**：被引用但还不存在 = 知识缺口 = 主动调研队列的输入。
CREATE TABLE IF NOT EXISTS knowledge_links (
    from_path   TEXT NOT NULL,
    to_title    TEXT NOT NULL,
    resolved    INTEGER NOT NULL DEFAULT 0,
    updated_at  INTEGER NOT NULL,
    PRIMARY KEY (from_path, to_title)
);
CREATE INDEX IF NOT EXISTS idx_knowledge_links_to ON knowledge_links (to_title, resolved);


-- 用户可编辑的监控名单（哪些 app 算娱乐）。分类判定：此表 > 内置白名单。
-- 桌面采集器与 Android JNI 都读它，故加/删一个 app 跨端立即生效、无需重编。
CREATE TABLE IF NOT EXISTS monitored_apps (
    bundle_id  TEXT PRIMARY KEY,          -- macOS bundle id 或 Android 包名
    category   TEXT NOT NULL,             -- entertainment.video|game|social|news
    created_at INTEGER NOT NULL
);

-- ── 主动触发：待办动作队列（proactive-triggers.md §2）───────────────────────
-- 统一"到点要做的动作"：时间触发的周期任务、规则引擎的立即/延后响应、agent 排程都塞这里。
-- due_at_ms=now 即"立即"；=now+Δ 即"延后"。core 只管队列增删查（纯数据、安卓可编）；
-- 副作用（弹通知/宠物展示/拉 codex）由 app 层按 kind 派发；外部内容源只读。
CREATE TABLE IF NOT EXISTS scheduled_actions (
    id              TEXT PRIMARY KEY,
    kind            TEXT NOT NULL,               -- notify|pet_message|agent_run|…
    payload_json    TEXT NOT NULL DEFAULT '{}',
    due_at_ms       INTEGER NOT NULL,
    recurrence      TEXT,                        -- NULL=一次性；如 "daily@09:00"
    status          TEXT NOT NULL DEFAULT 'pending', -- pending|fired|done|failed|cancelled
    dedup_key       TEXT,                        -- 同 key 已有 pending 则不重复入队（防打扰）
    origin_event_id TEXT,                        -- 溯源：触发它的 finding/事件
    created_by      TEXT NOT NULL,               -- rule_engine|scheduler|agent|manual
    created_at_ms   INTEGER NOT NULL,
    fired_at_ms     INTEGER
);
CREATE INDEX IF NOT EXISTS idx_sched_due ON scheduled_actions (status, due_at_ms);

-- ── 动态检测规则（rule-engine.md）──────────────────────────────────────────────
-- 用户/智能体用一句话建的检测规则：声明式 trigger_json（泛化 EntertainmentSessionRule）
-- + response_json（ResponsePolicy）。RuleEngine 每次评估热加载 enabled 行，无需重编。
CREATE TABLE IF NOT EXISTS detection_rules (
    id                TEXT PRIMARY KEY,
    name              TEXT NOT NULL,
    enabled           INTEGER NOT NULL DEFAULT 1,
    trigger_json      TEXT NOT NULL,               -- 声明式触发条件（见 core::rules::RuleTrigger）
    response_json     TEXT NOT NULL DEFAULT '{"policy":"immediate","kind":"notify"}',
    severity          TEXT NOT NULL DEFAULT 'medium', -- medium | high
    cooldown_minutes  INTEGER NOT NULL DEFAULT 30,
    created_by        TEXT NOT NULL DEFAULT 'agent',  -- agent | user
    origin_capture_id TEXT,                         -- 溯源：哪条 capture 催生的
    created_at        INTEGER NOT NULL,
    updated_at        INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_detection_rules_enabled ON detection_rules (enabled);

-- ── 旧版人生看板兼容表 ───────────────────────────────────────────────────────
-- 新实现以 life_items 为事实源；本表只保留已安装版本兼容并在下方幂等迁入 LifeDB。
CREATE TABLE IF NOT EXISTS lifeindex_cards (
    id                TEXT PRIMARY KEY,
    section           TEXT NOT NULL,              -- 分区，如 今日焦点 / 长期目标 / 研究问题
    title             TEXT NOT NULL,
    body              TEXT NOT NULL DEFAULT '',
    source_ref        TEXT,                       -- Notion page id / url 溯源
    source_updated_at INTEGER,                    -- 外部源更新时间
    observed_at       INTEGER NOT NULL,           -- 本轮刷新观测时间（mark-and-sweep 用）
    status            TEXT NOT NULL DEFAULT 'active', -- active | archived
    sort_order        INTEGER NOT NULL DEFAULT 0,
    created_at        INTEGER NOT NULL,
    updated_at        INTEGER NOT NULL,
    UNIQUE(section, title)
);
CREATE INDEX IF NOT EXISTS idx_lifeindex_status ON lifeindex_cards (status, section);

-- ── LifeDB：LifeIndex 的结构化事实来源 ───────────────────────────────────────
-- LifeItem 是人生规划领域内的统一对象，不是兜住所有业务的多态 artifact：
-- note / reminder / intervention 等仍保留独立表。Notion 只通过受控同步器交换表面文本。
--
-- 责任领域（GTD Horizon 3）：无完成态，只需维持标准。有了它，track（主线/支线）可由
-- "该领域当前是否重点"推导，而不是每条手标——这是"手动维护不丝滑"的主要来源。
CREATE TABLE IF NOT EXISTS life_areas (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL UNIQUE,
    description TEXT NOT NULL DEFAULT '',
    sort_order  INTEGER NOT NULL DEFAULT 0,
    focus       INTEGER NOT NULL DEFAULT 0,   -- 1 = 当前重点领域（→ track=main 的推导依据）
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL
);

-- kind 里 skill / milestone 是**技能树的底座**：
--   skill      一个能力节点；前置关系用 depends_on 边，层级/等级用 contains 边挂 milestone
--   milestone  可判定的检查点（目标自动拆解的产物），也是无极时间线上的抽象层标记
-- target_value/current_value/unit + success_criteria 让"完成"可判定，进度由 Core 确定性算出。
CREATE TABLE IF NOT EXISTS life_items (
    id              TEXT PRIMARY KEY,
    kind            TEXT NOT NULL CHECK (kind IN ('idea','goal','project','action','routine','skill','milestone')),
    title           TEXT NOT NULL,
    body            TEXT NOT NULL DEFAULT '',
    track           TEXT NOT NULL DEFAULT 'undecided'
                    CHECK (track IN ('main','side','neutral','undecided')),
    horizon         TEXT NOT NULL DEFAULT 'unscheduled'
                    CHECK (horizon IN ('now','next','later','someday','unscheduled')),
    status          TEXT NOT NULL DEFAULT 'inbox'
                    CHECK (status IN ('inbox','active','waiting','done','archived')),
    area_id         TEXT,
    success_criteria TEXT,
    target_value    REAL,
    current_value   REAL,
    unit            TEXT,
    start_at_ms     INTEGER,
    due_at_ms       INTEGER,
    review_at_ms    INTEGER,
    recurrence      TEXT,
    source_event_id TEXT,
    intent_id       TEXT,
    sync_status     TEXT NOT NULL DEFAULT 'local_dirty'
                    CHECK (sync_status IN ('clean','local_dirty','notion_dirty','conflict')),
    revision        INTEGER NOT NULL DEFAULT 1,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL,
    archived_at     INTEGER
);
CREATE INDEX IF NOT EXISTS idx_life_items_view
    ON life_items (status, kind, track, horizon, updated_at DESC);
CREATE INDEX IF NOT EXISTS idx_life_items_sync
    ON life_items (sync_status, updated_at DESC);
-- idx_life_items_area 在 migrations 里建（area_id 是迁移才加的列）。
CREATE INDEX IF NOT EXISTS idx_life_items_span
    ON life_items (COALESCE(start_at_ms, created_at), COALESCE(due_at_ms, start_at_ms, created_at));
CREATE INDEX IF NOT EXISTS idx_life_items_review ON life_items (review_at_ms) WHERE review_at_ms IS NOT NULL;


-- 邻接表就是 LifeDB 的图结构；SQLite recursive CTE 足够查询目标→项目→行动/日常。
CREATE TABLE IF NOT EXISTS life_item_edges (
    from_item_id TEXT NOT NULL,
    to_item_id   TEXT NOT NULL,
    relation     TEXT NOT NULL
                 CHECK (relation IN ('contains','supports','depends_on','blocks','derived_from','related')),
    sort_order   INTEGER NOT NULL DEFAULT 0,
    created_at   INTEGER NOT NULL,
    PRIMARY KEY (from_item_id, to_item_id, relation),
    FOREIGN KEY (from_item_id) REFERENCES life_items(id) ON DELETE CASCADE,
    FOREIGN KEY (to_item_id) REFERENCES life_items(id) ON DELETE CASCADE,
    CHECK (from_item_id != to_item_id)
);
CREATE INDEX IF NOT EXISTS idx_life_edges_to ON life_item_edges (to_item_id, relation);

-- 外部引用与本体解耦；同一个 LifeItem 可来自 capture、Notion 页面或其它适配器。
CREATE TABLE IF NOT EXISTS life_item_external_refs (
    item_id                TEXT NOT NULL,
    provider               TEXT NOT NULL,
    external_id            TEXT NOT NULL,
    external_url           TEXT,
    external_updated_at_ms INTEGER,
    content_hash           TEXT,
    last_pushed_revision   INTEGER,
    observed_at_ms         INTEGER NOT NULL,
    PRIMARY KEY (provider, external_id),
    FOREIGN KEY (item_id) REFERENCES life_items(id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_life_refs_item ON life_item_external_refs (item_id, provider);

-- 三方合并基线：上次成功投影文本 + 同步状态。Agent 负责理解差异，Core 负责审计与门禁。
CREATE TABLE IF NOT EXISTS life_sync_state (
    provider           TEXT NOT NULL,
    target_id          TEXT NOT NULL,
    last_snapshot_text TEXT NOT NULL DEFAULT '',
    last_summary       TEXT NOT NULL DEFAULT '',
    last_success_at_ms INTEGER,
    last_attempt_at_ms INTEGER,
    last_error         TEXT,
    PRIMARY KEY (provider, target_id)
);

-- 每次成功写回前后的完整文本审计。首次同步也保留用户原始 Notion 页面，便于恢复。
CREATE TABLE IF NOT EXISTS life_sync_runs (
    id                   TEXT PRIMARY KEY,
    provider             TEXT NOT NULL,
    target_id            TEXT NOT NULL,
    remote_before_text   TEXT NOT NULL,
    final_snapshot_text  TEXT NOT NULL,
    summary              TEXT NOT NULL DEFAULT '',
    completed_at_ms      INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_life_sync_runs_target
    ON life_sync_runs (provider, target_id, completed_at_ms DESC);

-- ── 无极时间线：预聚合桶 ─────────────────────────────────────────────────────
-- 为什么必须预聚合：年尺度下按 raw_events 现算 GROUP BY strftime 是全表扫描且用不上索引。
-- 时间线的代价必须是 O(可见桶数) 而不是 O(事件数)，缩放才可能"无极"。
-- 桶按**逻辑日**（本地时区 + 换日点）切；周/月桶由日桶再聚合。
CREATE TABLE IF NOT EXISTS time_rollups (
    bucket_kind     TEXT NOT NULL,               -- day | week | month
    bucket_start_ms INTEGER NOT NULL,
    dimension       TEXT NOT NULL,               -- category | app
    key             TEXT NOT NULL,               -- 'entertainment.video' / bundle id / '(unknown)'
    duration_ms     INTEGER NOT NULL DEFAULT 0,
    event_count     INTEGER NOT NULL DEFAULT 0,
    updated_at_ms   INTEGER NOT NULL,
    PRIMARY KEY (bucket_kind, bucket_start_ms, dimension, key)
);
CREATE INDEX IF NOT EXISTS idx_rollups_scan
    ON time_rollups (bucket_kind, dimension, bucket_start_ms);

-- 增量重建水位：只重算水位之后新事件所触碰到的桶（重算是幂等的删+插）。
CREATE TABLE IF NOT EXISTS rollup_state (
    scope         TEXT PRIMARY KEY,              -- 'behavior'
    watermark_ms  INTEGER NOT NULL,              -- 已处理到的 ingested_at
    updated_at_ms INTEGER NOT NULL
);
"#;

