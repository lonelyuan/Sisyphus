//! Schema 迁移（`PRAGMA user_version` 驱动）。
//!
//! **为什么必须有**：此前 schema 只靠 `CREATE TABLE IF NOT EXISTS` 演进——建新表可以，
//! **给已存在的表加列不行**。任何 `ALTER` 需求都会让已装机的库在查询时报 `no such column`。
//!
//! 约定：
//! - [`SCHEMA`](crate::db::SCHEMA) 始终是**最新**定义，新库一次建好；
//! - 本模块负责把**老库**补齐到同一形状，因此每条迁移都必须**幂等**
//!   （新库跑到它时该改动通常已由 SCHEMA 完成，迁移要能识别并跳过）；
//! - 数据类一次性导入也放这里（而不是 SCHEMA），避免每次 `open()` 重跑。

use rusqlite::{Connection, Result};

/// 当前目标版本。加迁移时 +1 并在 [`apply`] 里加分支。
pub const TARGET_VERSION: i64 = 6;

pub fn run(conn: &Connection) -> Result<()> {
    let mut version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    // 老库 user_version=0 且表已存在；新库刚由 SCHEMA 建好，也是 0。两者都从 1 开始跑，
    // 靠每条迁移自身的幂等性区分。
    while version < TARGET_VERSION {
        let next = version + 1;
        apply(conn, next)?;
        conn.pragma_update(None, "user_version", next)?;
        version = next;
    }
    Ok(())
}

fn apply(conn: &Connection, version: i64) -> Result<()> {
    match version {
        1 => v1_add_columns(conn),
        2 => v2_widen_life_item_kinds(conn),
        3 => v3_import_legacy_into_lifedb(conn),
        4 => v4_knowledge_title_uniqueness(conn),
        5 => v5_seed_default_areas(conn),
        6 => v6_seed_progress_ledger(conn),
        _ => Ok(()),
    }
}

// ── helpers ──────────────────────────────────────────────────────────────────

fn table_exists(conn: &Connection, table: &str) -> Result<bool> {
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
        [table],
        |r| r.get(0),
    )?;
    Ok(n > 0)
}

fn has_column(conn: &Connection, table: &str, column: &str) -> Result<bool> {
    if !table_exists(conn, table)? {
        return Ok(false);
    }
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let name: String = row.get(1)?;
        if name == column {
            return Ok(true);
        }
    }
    Ok(false)
}

/// 幂等加列：已存在则跳过。`decl` 形如 `"TEXT"` / `"REAL"` / `"TEXT NOT NULL DEFAULT ''"`。
fn add_column(conn: &Connection, table: &str, column: &str, decl: &str) -> Result<()> {
    if !table_exists(conn, table)? || has_column(conn, table, column)? {
        return Ok(());
    }
    conn.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {column} {decl};"))
}

fn table_ddl(conn: &Connection, table: &str) -> Result<String> {
    conn.query_row(
        "SELECT COALESCE(sql,'') FROM sqlite_master WHERE type='table' AND name=?1",
        [table],
        |r| r.get(0),
    )
    .or(Ok(String::new()))
}

// ── v1：补列 ─────────────────────────────────────────────────────────────────

/// 老库补上 SCHEMA 新增的列。**新库这些列已在，全部跳过。**
fn v1_add_columns(conn: &Connection) -> Result<()> {
    // 近端结果观察（1.1 的学习信号）：什么时候看的、看到了什么。
    add_column(conn, "interventions", "outcome_at_ms", "INTEGER")?;
    add_column(conn, "interventions", "outcome_detail", "TEXT")?;

    // LifeDB 心智系统底座：责任领域 + 可判定完成条件 + 度量（技能树进度靠它算）。
    add_column(conn, "life_items", "area_id", "TEXT")?;
    add_column(conn, "life_items", "success_criteria", "TEXT")?;
    add_column(conn, "life_items", "target_value", "REAL")?;
    add_column(conn, "life_items", "current_value", "REAL")?;
    add_column(conn, "life_items", "unit", "TEXT")?;

    // 知识库：正文进索引（查重要能搜正文，不然只能按标题查）+ 别名（重定向）+ 领域。
    add_column(conn, "knowledge_notes", "body", "TEXT NOT NULL DEFAULT ''")?;
    add_column(
        conn,
        "knowledge_notes",
        "aliases_json",
        "TEXT NOT NULL DEFAULT '[]'",
    )?;
    add_column(conn, "knowledge_notes", "folder", "TEXT NOT NULL DEFAULT ''")?;

    // 依赖上面这些列的索引必须在这里建，不能放 SCHEMA：SCHEMA 是一个 execute_batch，
    // 对老库来说 `CREATE INDEX ... (folder)` 会失败并**中止整批**，导致其后所有建表语句都不执行。
    if has_column(conn, "knowledge_notes", "folder")? {
        conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_knowledge_folder ON knowledge_notes (folder);",
        )?;
    }
    if has_column(conn, "life_items", "area_id")? {
        conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_life_items_area ON life_items (area_id);",
        )?;
    }
    Ok(())
}

// ── v2：life_items.kind 扩 skill / milestone ──────────────────────────────────

/// `kind` 的 CHECK 约束无法 ALTER，只能重建表。**新库 SCHEMA 已是宽约束 → 跳过。**
///
/// 扩的两种 kind 是技能树的底座：
/// - `skill`：一个能力节点（用 `depends_on` 边表达前置，`contains` 边挂里程碑）。
/// - `milestone`：可判定的检查点（目标自动拆解的产物，也是无极时间线的抽象层标记）。
fn v2_widen_life_item_kinds(conn: &Connection) -> Result<()> {
    if !table_exists(conn, "life_items")? {
        return Ok(());
    }
    if table_ddl(conn, "life_items")?.contains("'skill'") {
        return Ok(()); // 已是新定义（新库或已迁移）
    }
    // 重建：FK 关掉 → 建新表 → 搬数据 → 换名 → 重建索引。
    conn.pragma_update(None, "foreign_keys", "OFF")?;
    let result = conn.execute_batch(
        r#"
BEGIN;
CREATE TABLE life_items_migrating (
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
INSERT INTO life_items_migrating
    (id,kind,title,body,track,horizon,status,area_id,success_criteria,target_value,current_value,
     unit,start_at_ms,due_at_ms,review_at_ms,recurrence,source_event_id,intent_id,sync_status,
     revision,created_at,updated_at,archived_at)
SELECT id,kind,title,body,track,horizon,status,area_id,success_criteria,target_value,current_value,
       unit,start_at_ms,due_at_ms,review_at_ms,recurrence,source_event_id,intent_id,sync_status,
       revision,created_at,updated_at,archived_at
FROM life_items;
DROP TABLE life_items;
ALTER TABLE life_items_migrating RENAME TO life_items;
CREATE INDEX IF NOT EXISTS idx_life_items_view
    ON life_items (status, kind, track, horizon, updated_at DESC);
CREATE INDEX IF NOT EXISTS idx_life_items_sync
    ON life_items (sync_status, updated_at DESC);
CREATE INDEX IF NOT EXISTS idx_life_items_area ON life_items (area_id);
CREATE INDEX IF NOT EXISTS idx_life_items_span
    ON life_items (COALESCE(start_at_ms, created_at), COALESCE(due_at_ms, start_at_ms, created_at));
COMMIT;
"#,
    );
    conn.pragma_update(None, "foreign_keys", "ON")?;
    result
}

// ── v3：旧表一次性导入 LifeDB ────────────────────────────────────────────────

/// 把 `tasks` / `lifeindex_cards` 导入 `life_items`。
/// 之前这两条 INSERT 写在 SCHEMA 里、**每次 open 都跑**；现在只跑一次。
/// 导入后 `accept_intent` 直接写 life_items，`tasks` 不再接收新行。
fn v3_import_legacy_into_lifedb(conn: &Connection) -> Result<()> {
    if table_exists(conn, "tasks")? {
        conn.execute_batch(
            r#"
INSERT OR IGNORE INTO life_items
    (id,kind,title,body,track,horizon,status,due_at_ms,source_event_id,intent_id,
     sync_status,revision,created_at,updated_at,archived_at)
SELECT id,'action',title,COALESCE(note,''),'undecided',
       CASE WHEN due_ms IS NULL THEN 'unscheduled' ELSE 'next' END,
       CASE status WHEN 'todo' THEN 'inbox' WHEN 'doing' THEN 'active'
                   WHEN 'done' THEN 'done' ELSE 'archived' END,
       due_ms,source_event_id,intent_id,'local_dirty',1,created_at,created_at,
       CASE WHEN status='dropped' THEN created_at ELSE NULL END
FROM tasks;
"#,
        )?;
    }
    if table_exists(conn, "lifeindex_cards")? {
        conn.execute_batch(
            r#"
INSERT OR IGNORE INTO life_items
    (id,kind,title,body,track,horizon,status,sync_status,revision,created_at,updated_at,archived_at)
SELECT id,
       CASE WHEN section LIKE '%日常%' THEN 'routine'
            WHEN section LIKE '%目标%' THEN 'goal'
            WHEN section LIKE '%事项%' OR section LIKE '%焦点%' THEN 'action'
            ELSE 'idea' END,
       title,body,
       CASE WHEN section LIKE '%主线%' THEN 'main'
            WHEN section LIKE '%支线%' THEN 'side' ELSE 'undecided' END,
       'unscheduled',
       CASE status WHEN 'archived' THEN 'archived' ELSE 'active' END,
       'local_dirty',1,created_at,updated_at,
       CASE WHEN status='archived' THEN updated_at ELSE NULL END
FROM lifeindex_cards;
"#,
        )?;
    }
    Ok(())
}

// ── v4：知识卡标题唯一 ───────────────────────────────────────────────────────

/// 标题唯一是知识图谱的地基（维基百科靠"一个概念一个条目名 + 重定向"维持秩序）。
/// 此前幂等键是 `path`，同标题落在不同 folder 就是两张卡、两份真相、Obsidian 里链接歧义
/// （实测已发生：`certighost` 在 `kb/…` 和 `…`（漏 kb 前缀）各一份且内容分叉）。
///
/// 做法**非破坏**：把重复标题里较旧的标记 `status='duplicate'`（文件保留、待人工合并），
/// 再对存活行建**部分唯一索引**。
fn v4_knowledge_title_uniqueness(conn: &Connection) -> Result<()> {
    if !table_exists(conn, "knowledge_notes")? {
        return Ok(());
    }
    conn.execute_batch(
        r#"
UPDATE knowledge_notes SET status='duplicate'
WHERE status NOT IN ('pruned','duplicate')
  AND id NOT IN (
      SELECT id FROM (
          SELECT id, ROW_NUMBER() OVER (
              PARTITION BY title ORDER BY updated_at DESC, LENGTH(path) ASC
          ) AS rn
          FROM knowledge_notes WHERE status NOT IN ('pruned','duplicate')
      ) WHERE rn = 1
  );
CREATE UNIQUE INDEX IF NOT EXISTS idx_knowledge_title_unique
    ON knowledge_notes (title) WHERE status NOT IN ('pruned','duplicate');
"#,
    )
}

// ── v5：默认责任领域 ─────────────────────────────────────────────────────────

/// 播种 GTD Horizon 3（责任领域）的初始集合。**无完成态**，只需维持标准。
/// 有了 area，`track`（主线/支线）可以由"该领域当前是否重点"推导，而不是每条手标。
fn v5_seed_default_areas(conn: &Connection) -> Result<()> {
    if !table_exists(conn, "life_areas")? {
        return Ok(());
    }
    let existing: i64 = conn.query_row("SELECT COUNT(*) FROM life_areas", [], |r| r.get(0))?;
    if existing > 0 {
        return Ok(());
    }
    let now = crate::clock::now_ms();
    let seeds = [
        ("职业", "工作产出、职级、专业影响力", 0_i64),
        ("能力", "长期可迁移技能：技术、表达、语言", 1),
        ("健康", "睡眠、运动、体检指标", 2),
        ("关系", "家人、朋友、伴侣", 3),
        ("财务", "收入、储蓄、支出结构", 4),
        ("兴趣", "让自己回血、愿意长期投入的事", 5),
    ];
    for (name, desc, order) in seeds {
        conn.execute(
            "INSERT OR IGNORE INTO life_areas (id,name,description,sort_order,focus,created_at,updated_at)
             VALUES (?1,?2,?3,?4,0,?5,?5)",
            rusqlite::params![uuid::Uuid::new_v4().to_string(), name, desc, order, now],
        )?;
    }
    Ok(())
}

// ── v6：进度账本（技能树时间播放的数据源）──────────────────────────────────────

/// 老库补上 `life_item_progress`，并做一次**近似**回填。
///
/// 老库没有真历史，所以只能给每个 item 两个锚点：`created_at` 的初始态与 `updated_at` 的当前态，
/// `origin='backfill'` 明确标注它是近似的——不假装拥有从未记录过的中间过程。
/// 从这次迁移之后，`lifedb::upsert_item` / `archive_item` 会写下真实的每一次变更。
fn v6_seed_progress_ledger(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        r#"
CREATE TABLE IF NOT EXISTS life_item_progress (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    item_id       TEXT NOT NULL,
    at_ms         INTEGER NOT NULL,
    status        TEXT,
    current_value REAL,
    target_value  REAL,
    revision      INTEGER NOT NULL DEFAULT 0,
    origin        TEXT NOT NULL,
    FOREIGN KEY (item_id) REFERENCES life_items(id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_life_progress_item ON life_item_progress (item_id, at_ms);
CREATE INDEX IF NOT EXISTS idx_life_progress_time ON life_item_progress (at_ms);
"#,
    )?;
    if !table_exists(conn, "life_items")? {
        return Ok(());
    }
    // 幂等：已回填过就不再来一遍（用户此后的真实变更不会被重复的锚点污染）。
    let seeded: i64 = conn.query_row(
        "SELECT COUNT(*) FROM life_item_progress WHERE origin='backfill'",
        [],
        |r| r.get(0),
    )?;
    if seeded > 0 {
        return Ok(());
    }
    let metrics = has_column(conn, "life_items", "current_value")?;
    let current = if metrics { "current_value" } else { "NULL" };
    let target = if metrics { "target_value" } else { "NULL" };
    // 出生锚点：inbox 是 status 的默认值，也是任何 item 的起点。
    conn.execute_batch(&format!(
        r#"
INSERT INTO life_item_progress (item_id,at_ms,status,current_value,target_value,revision,origin)
SELECT id, created_at, 'inbox', NULL, NULL, 0, 'backfill' FROM life_items;
INSERT INTO life_item_progress (item_id,at_ms,status,current_value,target_value,revision,origin)
SELECT id, MAX(updated_at, created_at + 1), status, {current}, {target}, revision, 'backfill'
FROM life_items;
"#
    ))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 老库形状（缺列 + 窄 kind 约束 + 有数据）→ 迁移 → 列齐、数据完好、新 kind 可写。
    #[test]
    fn migrates_legacy_shape_without_data_loss() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            r#"
CREATE TABLE life_items (
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL CHECK (kind IN ('idea','goal','project','action','routine')),
    title TEXT NOT NULL,
    body TEXT NOT NULL DEFAULT '',
    track TEXT NOT NULL DEFAULT 'undecided',
    horizon TEXT NOT NULL DEFAULT 'unscheduled',
    status TEXT NOT NULL DEFAULT 'inbox',
    start_at_ms INTEGER, due_at_ms INTEGER, review_at_ms INTEGER, recurrence TEXT,
    source_event_id TEXT, intent_id TEXT,
    sync_status TEXT NOT NULL DEFAULT 'local_dirty',
    revision INTEGER NOT NULL DEFAULT 1,
    created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, archived_at INTEGER
);
INSERT INTO life_items (id,kind,title,created_at,updated_at)
VALUES ('a','goal','写完评审',1,1);
CREATE TABLE interventions (id TEXT PRIMARY KEY, rule_id TEXT NOT NULL, shown_at INTEGER NOT NULL,
    intensity TEXT NOT NULL, message TEXT NOT NULL, options_json TEXT NOT NULL, outcome TEXT);
CREATE TABLE knowledge_notes (id TEXT PRIMARY KEY, path TEXT NOT NULL UNIQUE, title TEXT NOT NULL,
    tags_json TEXT NOT NULL DEFAULT '[]', sources_json TEXT NOT NULL DEFAULT '[]',
    content_hash TEXT NOT NULL, status TEXT NOT NULL DEFAULT 'active',
    created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
INSERT INTO knowledge_notes (id,path,title,content_hash,created_at,updated_at)
VALUES ('n1','kb/a/certighost.md','Certighost','h',1,1),
       ('n2','a/certighost.md','Certighost','h',2,2);
CREATE TABLE life_areas (id TEXT PRIMARY KEY, name TEXT NOT NULL UNIQUE, description TEXT NOT NULL DEFAULT '',
    sort_order INTEGER NOT NULL DEFAULT 0, focus INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
"#,
        )
        .unwrap();

        run(&conn).unwrap();

        // 列补齐
        assert!(has_column(&conn, "life_items", "area_id").unwrap());
        assert!(has_column(&conn, "life_items", "target_value").unwrap());
        assert!(has_column(&conn, "interventions", "outcome_at_ms").unwrap());
        assert!(has_column(&conn, "knowledge_notes", "body").unwrap());
        // 数据完好
        let title: String = conn
            .query_row("SELECT title FROM life_items WHERE id='a'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(title, "写完评审");
        // 新 kind 可写
        conn.execute(
            "INSERT INTO life_items (id,kind,title,created_at,updated_at) VALUES ('s','skill','Rust',1,1)",
            [],
        )
        .unwrap();
        // 重复标题只留一张存活
        let alive: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM knowledge_notes WHERE status NOT IN ('pruned','duplicate')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(alive, 1);
        // 默认领域已播种
        let areas: i64 = conn
            .query_row("SELECT COUNT(*) FROM life_areas", [], |r| r.get(0))
            .unwrap();
        assert!(areas >= 6);
        // 进度账本已建表并回填（老库没有真历史，两个锚点标为 backfill）
        let anchors: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM life_item_progress WHERE item_id='a' AND origin='backfill'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(anchors, 2);
        // 版本推进 + 重复运行幂等
        let v: i64 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v, TARGET_VERSION);
        run(&conn).unwrap();
        v6_seed_progress_ledger(&conn).unwrap();
        let anchors_again: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM life_item_progress WHERE origin='backfill'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            anchors_again, 2,
            "重跑不重复回填（'s' 是迁移后插的，本来就没有锚点）"
        );
    }
}
