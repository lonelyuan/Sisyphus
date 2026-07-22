//! 原始材料库（`sources/`）：与知识图谱（`kb/`）**物理隔离**的逐字原文档案。
//!
//! 存储目的不同（architecture.md §2 的精神在知识层的延伸）：
//! - `sources/`：值得原文保存的文章/报告，逐字归档，供溯源与查阅；**不进图谱、不导出博客**。
//! - `kb/`：自己总结的、高度结构化+`[[链接]]`化的知识卡片（`knowledge.rs`）。
//!
//! kb 卡片经 frontmatter `sources:` 引用本处原文路径——这是桥，不是混合。
//! 文件名用 `{slug}-{内容哈希6}.md`：同一原文幂等、不同原文不撞名。

use rusqlite::Connection;
use serde::Serialize;
use std::fs;
use std::path::Path;

use crate::vault::{content_hash, slugify};

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

#[derive(Debug, Serialize)]
pub struct SaveSourceOutcome {
    pub path: String,
    pub content_hash: String,
    /// true = 同内容原文已存在（覆盖刷新）；false = 新归档。
    pub updated: bool,
}

/// 归档一份逐字原文到 `sources/{slug}-{hash6}.md`（frontmatter + 原文），并写一条溯源事件。
#[allow(clippy::too_many_arguments)]
pub fn save_source(
    conn: &Connection,
    vault_dir: &Path,
    user_id: &str,
    device_id: &str,
    title: &str,
    content: &str,
    url: Option<&str>,
    source_type: Option<&str>,
) -> Result<SaveSourceOutcome, String> {
    let now = now_ms();
    let chash = content_hash(content);
    let suffix = &chash[..6];
    let base = slugify(title);
    let rel = format!("sources/{base}-{suffix}.md");
    let abs = vault_dir.join(&rel);
    let existed = abs.exists();

    let stype = source_type.unwrap_or("article");
    let url_line = url.unwrap_or("");
    let safe_title = title.replace('"', "'");
    let file = format!(
        "---\ntitle: \"{safe_title}\"\nsource_type: {stype}\nurl: {url_line}\nsaved_at: {now}\nhash: {chash}\nkind: source\n---\n\n# {title}\n\n{}\n",
        content.trim()
    );

    if let Some(parent) = abs.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("建 sources 目录失败: {e}"))?;
    }
    fs::write(&abs, file.as_bytes()).map_err(|e| format!("写原文失败: {e}"))?;

    // 溯源面包屑：复用 material 语义进 Event log。
    let text = format!("[原文归档] {title}\n{rel}");
    crate::ingest::capture_material(conn, user_id, device_id, &text)
        .map_err(|e| format!("写溯源事件失败: {e}"))?;

    Ok(SaveSourceOutcome {
        path: rel,
        content_hash: chash,
        updated: existed,
    })
}
