//! 第二大脑知识索引（Phase 1.3）。
//!
//! `write_knowledge_note` 三合一，保证「统一发生在数据层」（architecture.md §2）：
//! 1. vault `.md`（人类可读投影，可 Obsidian 打开）——`vault::write_note`
//! 2. `knowledge_notes` 索引行（可查询真相 + 剪枝/关系的锚点）
//! 3. `knowledge_ingested` Event log 面包屑（溯源，进统一事件流）
//!
//! 摘要/概念抽取仍是 Codex（反思平面）的活；本模块只做数据结构保存。

use rusqlite::{params, Connection};
use serde::Serialize;
use std::path::Path;
use uuid::Uuid;

use crate::vault::{self, VaultNote};

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

#[derive(Debug, Serialize)]
pub struct KnowledgeNote {
    pub id: String,
    pub path: String,
    pub title: String,
    pub tags: Vec<String>,
    pub sources: Vec<String>,
    pub content_hash: String,
    pub status: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Serialize)]
pub struct WriteOutcome {
    pub id: String,
    pub path: String,
    pub content_hash: String,
    /// true = 更新了已有同名笔记；false = 新建。
    pub updated: bool,
}

/// 写一张知识卡片（vault + 索引 + 溯源事件）。返回 id / 相对路径 / 内容哈希。
///
/// 防覆盖：不同标题 slug 相同（如「Rust: 所有权」与「Rust 所有权」）时，若默认路径已被
/// **另一个标题**占用，则消歧到 `{slug}-{标题哈希}.md`，避免静默覆盖 + 丢溯源。
pub fn write_knowledge_note(
    conn: &Connection,
    vault_dir: &Path,
    user_id: &str,
    device_id: &str,
    note: &VaultNote,
) -> Result<WriteOutcome, String> {
    let now = now_ms();

    // 1. 选路径：默认 slug；若撞到不同标题则消歧。
    let mut rel = vault::note_path(&note.title);
    let occupant: Option<(String, String, i64)> = conn
        .query_row(
            "SELECT id, title, created_at FROM knowledge_notes WHERE path = ?1",
            params![rel],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .ok();
    let (id, created_at, updated) = match occupant {
        // 同标题占用 → 更新（幂等）。
        Some((id, title, ca)) if title == note.title => (id, ca, true),
        // 不同标题占用 → 消歧到带哈希后缀的新路径。
        Some(_) => {
            let base = vault::slugify(&note.title);
            let suffix = &vault::content_hash(&note.title)[..6];
            rel = format!("{base}-{suffix}.md");
            match conn
                .query_row(
                    "SELECT id, created_at FROM knowledge_notes WHERE path = ?1",
                    params![rel],
                    |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
                )
                .ok()
            {
                Some((id, ca)) => (id, ca, true),
                None => (Uuid::new_v4().to_string(), now, false),
            }
        }
        None => (Uuid::new_v4().to_string(), now, false),
    };

    // 2. 写 vault .md（人类可读投影）。
    let res = vault::write_note_at(vault_dir, &rel, note).map_err(|e| format!("写 vault 失败: {e}"))?;
    let tags_json = serde_json::to_string(&note.tags).unwrap_or_else(|_| "[]".into());
    let sources_json = serde_json::to_string(&note.sources).unwrap_or_else(|_| "[]".into());

    // 3. upsert 索引行（可查询真相），按最终 path 键。
    conn.execute(
        "INSERT INTO knowledge_notes
           (id, path, title, tags_json, sources_json, content_hash, status, created_at, updated_at)
         VALUES (?1,?2,?3,?4,?5,?6,'active',?7,?8)
         ON CONFLICT(path) DO UPDATE SET
           title=excluded.title, tags_json=excluded.tags_json, sources_json=excluded.sources_json,
           content_hash=excluded.content_hash, status='active', updated_at=excluded.updated_at",
        params![
            id,
            res.relative_path,
            note.title,
            tags_json,
            sources_json,
            res.content_hash,
            created_at,
            now
        ],
    )
    .map_err(|e| format!("写索引失败: {e}"))?;

    // 溯源面包屑：knowledge_ingested（source=agent, L1）。
    let breadcrumb = crate::ingest::NewEvent {
        event_id: None,
        source: "agent".into(),
        layer: "raw".into(),
        event_type: "knowledge_ingested".into(),
        time_mode: "point".into(),
        event_time: Some(now),
        start_time: None,
        end_time: None,
        entity: None,
        category: None,
        payload: serde_json::json!({
            "title": note.title,
            "path": res.relative_path,
            "sources": note.sources,
            "concept_count": note.links.len(),
        }),
        parent_event_ids: vec![],
        privacy_level: "L1".into(),
    };
    crate::ingest::ingest_event(conn, user_id, device_id, breadcrumb)
        .map_err(|e| format!("写溯源事件失败: {e}"))?;

    Ok(WriteOutcome {
        id,
        path: res.relative_path,
        content_hash: res.content_hash,
        updated,
    })
}

pub fn list_knowledge(conn: &Connection) -> rusqlite::Result<Vec<KnowledgeNote>> {
    let mut stmt = conn.prepare(
        "SELECT id, path, title, tags_json, sources_json, content_hash, status, created_at, updated_at
         FROM knowledge_notes WHERE status != 'pruned' ORDER BY updated_at DESC",
    )?;
    let rows = stmt
        .query_map([], row_to_note)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// 关键词检索（title / tags / path 上的 LIKE）。
pub fn search_knowledge(conn: &Connection, query: &str) -> rusqlite::Result<Vec<KnowledgeNote>> {
    let like = format!("%{query}%");
    let mut stmt = conn.prepare(
        "SELECT id, path, title, tags_json, sources_json, content_hash, status, created_at, updated_at
         FROM knowledge_notes
         WHERE status != 'pruned' AND (title LIKE ?1 OR tags_json LIKE ?1 OR path LIKE ?1)
         ORDER BY updated_at DESC",
    )?;
    let rows = stmt
        .query_map(params![like], row_to_note)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn row_to_note(r: &rusqlite::Row) -> rusqlite::Result<KnowledgeNote> {
    let tags_json: String = r.get(3)?;
    let sources_json: String = r.get(4)?;
    Ok(KnowledgeNote {
        id: r.get(0)?,
        path: r.get(1)?,
        title: r.get(2)?,
        tags: serde_json::from_str(&tags_json).unwrap_or_default(),
        sources: serde_json::from_str(&sources_json).unwrap_or_default(),
        content_hash: r.get(5)?,
        status: r.get(6)?,
        created_at: r.get(7)?,
        updated_at: r.get(8)?,
    })
}
