//! 人生看板 LifeIndex（[docs/spec/notion-integration.md](../../../../../docs/spec/notion-integration.md)）。
//!
//! 看板内容看齐用户的 Notion：每次由智能体**只读参考 Notion + 本地上下文**，再把结构化卡片
//! 写进这张本地表。它是**可重建的只读投影**（architecture.md §2.4）——Notion 仍是唯一真相源，
//! 智能体绝不回写 Notion。纯 rusqlite，安卓可编。

use rusqlite::{params, Connection};
use serde::Serialize;
use uuid::Uuid;

/// 看板一张卡片。按 (section,title) 幂等 upsert，便于每次刷新覆盖同一张。
#[derive(Debug, Clone, Serialize)]
pub struct LifeIndexCard {
    pub id: String,
    pub section: String,
    pub title: String,
    pub body: String,
    pub source_ref: Option<String>,
    pub source_updated_at: Option<i64>,
    pub observed_at: i64,
    pub sort_order: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

/// 幂等写入一张卡片：同 (section,title) 存在则更新正文/来源/观测时间，否则插入。返回卡片 id。
pub fn upsert_card(
    conn: &Connection,
    section: &str,
    title: &str,
    body: &str,
    source_ref: Option<&str>,
    source_updated_at: Option<i64>,
    sort_order: i64,
) -> Result<String, String> {
    let section = section.trim();
    let title = title.trim();
    if section.is_empty() || title.is_empty() {
        return Err("section / title 不能为空".to_string());
    }
    let now = chrono::Utc::now().timestamp_millis();
    let existing: Option<String> = conn
        .query_row(
            "SELECT id FROM lifeindex_cards WHERE section=?1 AND title=?2",
            params![section, title],
            |r| r.get(0),
        )
        .ok();
    if let Some(id) = existing {
        conn.execute(
            "UPDATE lifeindex_cards
             SET body=?2, source_ref=?3, source_updated_at=?4, observed_at=?5,
                 sort_order=?6, status='active', updated_at=?5
             WHERE id=?1",
            params![id, body, source_ref, source_updated_at, now, sort_order],
        )
        .map_err(|e| format!("更新看板卡片失败: {e}"))?;
        Ok(id)
    } else {
        let id = Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO lifeindex_cards
               (id,section,title,body,source_ref,source_updated_at,observed_at,status,sort_order,created_at,updated_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,'active',?8,?7,?7)",
            params![id, section, title, body, source_ref, source_updated_at, now, sort_order],
        )
        .map_err(|e| format!("写入看板卡片失败: {e}"))?;
        Ok(id)
    }
}

/// 列出全部活跃卡片，按分区 + sort_order 排序（供看板视图）。
pub fn list_cards(conn: &Connection) -> rusqlite::Result<Vec<LifeIndexCard>> {
    let mut stmt = conn.prepare(
        "SELECT id,section,title,body,source_ref,source_updated_at,observed_at,sort_order,created_at,updated_at
         FROM lifeindex_cards WHERE status='active'
         ORDER BY section ASC, sort_order ASC, updated_at DESC",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok(LifeIndexCard {
                id: r.get(0)?,
                section: r.get(1)?,
                title: r.get(2)?,
                body: r.get(3)?,
                source_ref: r.get(4)?,
                source_updated_at: r.get(5)?,
                observed_at: r.get(6)?,
                sort_order: r.get(7)?,
                created_at: r.get(8)?,
                updated_at: r.get(9)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// 删除一张卡片。
pub fn delete_card(conn: &Connection, id: &str) -> rusqlite::Result<()> {
    conn.execute("DELETE FROM lifeindex_cards WHERE id=?1", params![id])?;
    Ok(())
}

/// 归档某分区里 observed_at 早于本轮的卡片（刷新时的 mark-and-sweep，清掉 Notion 已删项）。
pub fn archive_stale_in_section(
    conn: &Connection,
    section: &str,
    keep_since_ms: i64,
) -> rusqlite::Result<usize> {
    let n = conn.execute(
        "UPDATE lifeindex_cards SET status='archived'
         WHERE section=?1 AND status='active' AND observed_at < ?2",
        params![section, keep_since_ms],
    )?;
    Ok(n)
}
