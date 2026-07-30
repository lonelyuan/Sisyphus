//! 原始材料（`type: source`）：**就地存放在话题文件夹**里的逐字原文。
//!
//! # 从"物理隔离"改为"逻辑隔离"（2026-07-30 决策）
//!
//! 旧设计把原文全塞进 vault 根的 `sources/` 一个平铺目录。结果是实测到的三个问题：
//! 1. **没人会去看**——它和话题树完全脱节，找一份原文要先知道它叫什么；
//! 2. **混进了大量不该逐字复制的东西**（公司 KM 的目录快照、能直接开 URL 的单篇文档），
//!    这本来就违反"第一方成品只链接、不复制"的规则；
//! 3. 目录一大坨之后，"重要原文"和"顺手存的"混在一起，隔离反而降低了信息密度。
//!
//! 新设计：**原文放在它讲的那个话题的文件夹里**（`kb/network-infra/xxx.md`），用
//! frontmatter 与 tag 标注它是原始材料，隔离靠**元信息**而不是靠目录：
//!
//! | 关注点 | 旧（路径隔离） | 新（元信息隔离） |
//! |---|---|---|
//! | 不污染知识图谱 | 不进 `kb/` | **单向连接**：卡片/MOC → 原文，原文自己不出链（图上是叶子） |
//! | 不导出博客 | 按路径排除 `sources/` | frontmatter `publish: false` + `source` tag（Quartz 按此排除）|
//! | 能找到 | 得记住文件名 | 就在话题夹里，和卡片相邻 |
//! | 质量 | 无门槛 | 必须给 `url`/来源；`kb_doctor` 报"没人引用的原文" |
//!
//! ⚠️ **接博客导出时必须改用 tag/frontmatter 排除**，不能再依赖 `sources/` 路径——
//! 原文现在住在 `kb/` 里面（见 `skills/knowledge-engine/references/blog-export.md`）。

use rusqlite::{params, Connection};
use serde::Serialize;
use std::path::Path;

use crate::clock::now_ms;
use crate::knowledge::KB_ROOT;
use crate::vault::{self, content_hash};

/// 原始材料的类型标签（与卡片的 `theory`/`news`/… 并列，见 `knowledge::TYPE_TAGS`）。
pub const SOURCE_TAG: &str = "source";

#[derive(Debug, Serialize)]
pub struct SaveSourceOutcome {
    pub path: String,
    pub content_hash: String,
    /// true = 同路径原文已存在（覆盖刷新）；false = 新归档。
    pub updated: bool,
}

/// 归档一份逐字原文到**话题文件夹**，并登记进索引（图谱里作为叶子节点）。
///
/// `folder` 必须以 `kb/` 开头——原文和它讲的那个话题放在一起。
/// `url` 是来源凭据：没有来源的"原文"无法溯源，直接拒绝（这是质量门槛，不是形式主义）。
/// 例外是用户本人口述/自撰的第一方材料，用 `source_type="first-party"` 声明，此时可无 url。
#[allow(clippy::too_many_arguments)]
pub fn save_source(
    conn: &Connection,
    vault_dir: &Path,
    user_id: &str,
    device_id: &str,
    folder: &str,
    title: &str,
    content: &str,
    url: Option<&str>,
    source_type: Option<&str>,
) -> Result<SaveSourceOutcome, String> {
    let folder = folder.trim().trim_matches('/');
    if !folder.starts_with(KB_ROOT) && folder != "kb" {
        return Err(format!(
            "folder 必须以 `{KB_ROOT}` 开头（当前 '{folder}'）。\
             原始材料现在**就地存放**在它讲的那个话题的文件夹里，不再有独立的 sources/ 目录"
        ));
    }
    let title = title.trim();
    if title.is_empty() {
        return Err("原文标题不能为空".to_string());
    }
    let stype = source_type.unwrap_or("article").trim().to_string();
    let is_first_party = stype == "first-party";
    let url_value = url.map(|u| u.trim()).filter(|u| !u.is_empty());
    if url_value.is_none() && !is_first_party {
        return Err(
            "外部原文必须提供 url（否则无法溯源，也无法判断该不该逐字复制）。\
             本人自撰/口述的第一方材料请传 source_type=\"first-party\""
                .to_string(),
        );
    }
    // 公司 KM 这类"用户本人持续维护的第一方成品"不该逐字镜像——引用链接即可，
    // 否则就会出现双份真相，且副本永远追不上原文。
    if let Some(u) = url_value {
        if stype == "km-space-index" {
            return Err(format!(
                "目录/索引类快照不值得逐字归档（{u}）。直接在相关卡片正文里引用这个链接"
            ));
        }
    }

    let now = now_ms();
    let chash = content_hash(content);
    let rel = format!("{folder}/{}", vault::note_path(title));
    let abs = vault_dir.join(&rel);
    let existed = abs.exists();

    let file = render_source(title, &stype, url_value, now, &chash, content);
    if let Some(parent) = abs.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("建目录失败: {e}"))?;
    }
    std::fs::write(&abs, file.as_bytes()).map_err(|e| format!("写原文失败: {e}"))?;

    // 登记进索引：图谱与体检都要看得到它（否则又变成"没人知道存在"的死角）。
    index_source(conn, &rel, folder, title, &stype, url_value, content, &chash, now)?;

    let text = format!("[原文归档] {title}\n{rel}");
    crate::ingest::capture_material(conn, user_id, device_id, &text)
        .map_err(|e| format!("写溯源事件失败: {e}"))?;
    vault::git_snapshot(vault_dir, &format!("归档原文 {title}"));

    Ok(SaveSourceOutcome {
        path: rel,
        content_hash: chash,
        updated: existed,
    })
}

/// 渲染原始材料的 `.md`。
///
/// 三个关键点：
/// - `tags` 含 `source`：类型标签就是隔离标记（不再靠目录）；
/// - `publish: false`：博客导出按它排除——原文是他人版权内容，不外发；
/// - **不生成 `## 关联`**：原文在图谱里是叶子，只被指向、不指向别人（单向连接）。
pub fn render_source(
    title: &str,
    source_type: &str,
    url: Option<&str>,
    saved_at: i64,
    hash: &str,
    content: &str,
) -> String {
    let safe_title = title.replace('"', "'");
    let mut s = String::new();
    s.push_str("---\n");
    s.push_str(&format!("title: \"{safe_title}\"\n"));
    s.push_str(&format!("tags: [{SOURCE_TAG}, 原始材料]\n"));
    s.push_str(&format!("source_type: {source_type}\n"));
    if let Some(u) = url {
        s.push_str(&format!("url: {u}\n"));
    }
    s.push_str(&format!("saved_at: {saved_at}\n"));
    s.push_str(&format!("hash: {hash}\n"));
    // 逐字原文是他人内容：默认不进博客导出。
    s.push_str("publish: false\n");
    s.push_str("---\n\n");
    s.push_str(&format!("# {title}\n\n"));
    s.push_str(content.trim());
    s.push('\n');
    s
}

/// 把原文登记进 `knowledge_notes`（`tags` 含 `source`，无出链边）。
#[allow(clippy::too_many_arguments)]
fn index_source(
    conn: &Connection,
    rel: &str,
    folder: &str,
    title: &str,
    source_type: &str,
    url: Option<&str>,
    content: &str,
    chash: &str,
    now: i64,
) -> Result<(), String> {
    let tags = serde_json::json!([SOURCE_TAG, "原始材料"]).to_string();
    let sources = match url {
        Some(u) => serde_json::json!([u]).to_string(),
        None => serde_json::json!(["来源：本人确认"]).to_string(),
    };
    let id = uuid::Uuid::new_v4().to_string();
    let body = format!("<!-- source_type: {source_type} -->\n{}", content.trim());
    conn.execute(
        "INSERT INTO knowledge_notes
           (id,path,folder,title,body,tags_json,sources_json,aliases_json,
            content_hash,status,created_at,updated_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,'[]',?8,'active',?9,?9)
         ON CONFLICT(path) DO UPDATE SET
           folder=excluded.folder,title=excluded.title,body=excluded.body,
           tags_json=excluded.tags_json,sources_json=excluded.sources_json,
           content_hash=excluded.content_hash,status='active',updated_at=excluded.updated_at",
        params![id, rel, folder, title, body, tags, sources, chash, now],
    )
    .map_err(|e| format!("登记原文索引失败: {e}"))?;
    Ok(())
}

/// 这张卡是原始材料吗（按 tags 判断，不看路径）。
pub fn is_source(tags: &[String]) -> bool {
    tags.iter()
        .any(|t| t.trim().trim_start_matches('#') == SOURCE_TAG)
}
