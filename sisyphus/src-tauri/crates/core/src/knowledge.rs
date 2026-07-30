//! 第二大脑知识索引（Phase 1.3）+ **约束层**。
//!
//! `write_knowledge_note` 三合一，保证「统一发生在数据层」（architecture.md §2）：
//! 1. vault `.md`（人类可读投影，可 Obsidian 打开）——`vault::write_note`
//! 2. `knowledge_notes` 索引行（可查询真相 + 剪枝/关系的锚点）+ `knowledge_links` 边
//! 3. `knowledge_ingested` Event log 面包屑（溯源，进统一事件流）
//!
//! 摘要/概念抽取仍是反思平面 agent 的活；本模块只做数据结构保存**与规则执行**。
//!
//! # 为什么这里要有约束
//!
//! 知识库的分类学、类型生命周期、结晶化规则此前只写在 skill 的 ~470 行 prompt 里，
//! 没有一条能被拒绝、被测量。实测后果：18% 的 wikilink 在 Obsidian 里点不开、
//! 同一张卡因为 folder 少写 `kb/` 前缀而存在两份且内容分叉、5 张卡缺 type 标签、
//! 8 张缺可靠性标签、两个目录超过拆分阈值一倍。
//!
//! 维基百科的质量不靠编辑记住方针，靠"标题唯一 + 重定向 + 分类必填 + 维护报告"这些
//! 机械设施。这里就是那套设施：**能被拒绝的规则才是规则**。

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::path::Path;
use uuid::Uuid;

use crate::clock::now_ms;
use crate::vault::{self, VaultNote};

/// 话题领域必须挂在 `kb/` 下（`sources/` 是物理隔离的原始材料库，不进图谱）。
pub const KB_ROOT: &str = "kb/";

/// 文章类型（frontmatter tag）：决定生命周期与链接规则。恰好一个。
///
/// - `moc`：领域枢纽/目录页（允许无出链的反面——它就是靠出链组织领域的，但不要求入链）
/// - `source`：**原始材料**（逐字原文）。就地存放在话题夹里，图谱上是**叶子**：
///   只被卡片/MOC 指向，自己不出链。见 [`crate::sources`]。
pub const TYPE_TAGS: &[&str] = &[
    "theory",
    "news",
    "state",
    "best-practice",
    "personal",
    "moc",
    "source",
];

/// 可靠性阶梯：由低到高，只标当前档。恰好一个。
/// `已复现` / `已验证` 不能凭模型自身知识升档——写入时校验（见 [`validate`]）。
pub const RELIABILITY_TAGS: &[&str] = &[
    "待确认",
    "多源印证",
    "已复现",
    "已验证",
    "stale",
    "有反证",
];

/// 需要外部证据才能声称的档位。
const EVIDENCE_REQUIRED: &[&str] = &["多源印证", "已复现", "已验证"];

/// 标题长度上限（字符数）。标题只写检索用的主题名，上下文交给 folder/tags/正文。
const MAX_TITLE_CHARS: usize = 24;

fn in_folder(folder: &str, filename: &str) -> String {
    let f = folder.trim().trim_matches('/');
    if f.is_empty() {
        filename.to_string()
    } else {
        format!("{f}/{filename}")
    }
}

#[derive(Debug, Serialize)]
pub struct KnowledgeNote {
    pub id: String,
    pub path: String,
    pub folder: String,
    pub title: String,
    pub tags: Vec<String>,
    pub sources: Vec<String>,
    pub aliases: Vec<String>,
    pub content_hash: String,
    pub status: String,
    pub created_at: i64,
    pub updated_at: i64,
}

/// 检索命中（含正文摘要，便于 agent 判断"这颗结晶讲过没有"）。
#[derive(Debug, Serialize)]
pub struct KnowledgeHit {
    pub title: String,
    pub path: String,
    pub folder: String,
    pub tags: Vec<String>,
    /// 命中位置附近的正文片段（无命中时取开头）。
    pub excerpt: String,
    pub updated_at: i64,
}

#[derive(Debug, Serialize)]
pub struct WriteOutcome {
    pub id: String,
    pub path: String,
    pub content_hash: String,
    /// true = 更新了已有同标题笔记；false = 新建。
    pub updated: bool,
    /// 本次写入里指向"还不存在的卡"的链接（红链）= 知识缺口，进主动调研队列。
    pub wanted_links: Vec<String>,
}

// ── 校验 ─────────────────────────────────────────────────────────────────────

/// 写入前校验。返回 `Err` 时**不写任何东西**，并给出可执行的修正建议。
pub fn validate(conn: &Connection, folder: &str, note: &VaultNote) -> Result<(), String> {
    let title = note.title.trim();
    if title.is_empty() {
        return Err("title 不能为空".to_string());
    }
    if title.chars().count() > MAX_TITLE_CHARS {
        return Err(format!(
            "标题过长（{} 字符 > {MAX_TITLE_CHARS}）：「{title}」。\
             标题只写未来会检索的主题规范名，公司名/类型/日期/状态/副标题交给 folder、tags、正文",
            title.chars().count()
        ));
    }
    if title.contains("[[") || title.contains('|') {
        return Err("标题不能包含 [[ 或 |".to_string());
    }

    let folder_trimmed = folder.trim().trim_matches('/');
    if !folder_trimmed.starts_with("kb/") && folder_trimmed != "kb" {
        return Err(format!(
            "folder 必须以 `{KB_ROOT}` 开头（当前 '{folder}'）。\
             `kb/` 是知识图谱，`sources/` 是隔离的原始材料库；不带前缀会在 vault 根另建一棵树，\
             同一张卡就会出现两份且内容分叉"
        ));
    }

    let types: Vec<&String> = note
        .tags
        .iter()
        .filter(|t| TYPE_TAGS.contains(&t.trim().trim_start_matches('#')))
        .collect();
    if types.len() != 1 {
        return Err(format!(
            "tags 必须恰好含一个文章类型 {TYPE_TAGS:?}，当前 {:?}",
            note.tags
        ));
    }
    if note.tags.iter().any(|t| t.starts_with('#')) {
        return Err("tags 里不要写 `#` 前缀（frontmatter 的 tags 是纯值列表）".to_string());
    }

    // 原始材料（逐字原文）是另一套规则，先分流：
    // 可靠性阶梯衡量的是"这个说法有多可信"，而原文的可信度就是它的来源本身，
    // 给逐字副本贴「待确认/已复现」没有意义——它的门槛是**必须有溯源**。
    let type_tag = types[0].trim();
    let all_links = merged_links(note);
    if type_tag == "source" {
        if !all_links.is_empty() {
            return Err(
                "原始材料（type=source）不能有出链——它在图谱里是叶子，只被卡片/MOC 单向指向。\
                 要表达关系请在**引用它的那张卡**里加链接"
                    .to_string(),
            );
        }
        if note.sources.iter().all(|s| s.trim().is_empty()) {
            return Err("原始材料必须有 sources（来源 URL 或「来源：本人确认」），否则无法溯源".to_string());
        }
        return Ok(());
    }

    let reliability: Vec<&String> = note
        .tags
        .iter()
        .filter(|t| RELIABILITY_TAGS.contains(&t.trim().trim_start_matches('#')))
        .collect();
    if reliability.len() != 1 {
        return Err(format!(
            "tags 必须恰好含一个可靠性档位 {RELIABILITY_TAGS:?}，当前 {:?}。\
             默认写「待确认」——模型自身知识只配这一档",
            note.tags
        ));
    }
    let level = reliability[0].trim().trim_start_matches('#');
    if EVIDENCE_REQUIRED.contains(&level) && note.sources.iter().all(|s| s.trim().is_empty()) {
        return Err(format!(
            "可靠性标为「{level}」必须提供 sources（外部原文路径 / 权威 URL / 复现记录）。\
             没有证据就只能是「待确认」"
        ));
    }
    // moc 类型（目录页）允许无出链（列表由 Core 生成）；其余每张卡至少要有一条有语义的关联。
    let is_moc = type_tag == "moc";
    if !is_moc && all_links.is_empty() {
        return Err(
            "至少要有 1 条 links（母结晶 / 对照 / 依赖 / 实例 / gap）。\
             说不清关系就说明它还是某颗结晶的一个小节，应该归并而不是新建"
                .to_string(),
        );
    }
    if all_links.iter().any(|l| l.trim() == title) {
        return Err("不能链接自身".to_string());
    }
    let _ = conn;
    Ok(())
}

/// links 参数 + 正文里的 wikilink，去重后的全集。
fn merged_links(note: &VaultNote) -> Vec<String> {
    let mut all: Vec<String> = note
        .links
        .iter()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();
    for l in vault::extract_wikilinks(&note.body) {
        let l = l.trim().to_string();
        if !l.is_empty() && !all.contains(&l) {
            all.push(l);
        }
    }
    all
}

// ── 写入 ─────────────────────────────────────────────────────────────────────

/// 写一张知识卡片（校验 → vault + 索引 + 链接边 + 溯源事件 + git 快照）。
///
/// **幂等键是 title，不是 path**：同一个标题只能有一张存活的卡。folder 变了就把文件
/// 移过去，而不是产生第二份（此前按 path 幂等，导致 `kb/…/certighost.md` 与
/// `…/certighost.md` 两份并存、内容分叉）。
pub fn write_knowledge_note(
    conn: &Connection,
    vault_dir: &Path,
    user_id: &str,
    device_id: &str,
    folder: Option<&str>,
    note: &VaultNote,
) -> Result<WriteOutcome, String> {
    let folder = folder.unwrap_or("").trim().trim_matches('/').to_string();
    validate(conn, &folder, note)?;
    let now = now_ms();
    let title = note.title.trim().to_string();
    let target_rel = in_folder(&folder, &vault::note_path(&title));

    // 按标题找存活的卡（含别名指向）。
    let existing: Option<(String, String, i64)> = conn
        .query_row(
            "SELECT id, path, created_at FROM knowledge_notes
             WHERE title = ?1 AND status NOT IN ('pruned','duplicate')",
            params![title],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?;

    let (id, created_at, updated, old_path) = match existing {
        Some((id, path, created)) => (id, created, true, Some(path)),
        None => {
            // 路径被**别的标题**占用（不同标题清洗后同名）→ 消歧，不覆盖。
            let occupied: Option<String> = conn
                .query_row(
                    "SELECT title FROM knowledge_notes WHERE path = ?1
                     AND status NOT IN ('pruned','duplicate')",
                    params![target_rel],
                    |r| r.get(0),
                )
                .optional()
                .map_err(|e| e.to_string())?;
            if occupied.is_some_and(|t| t != title) {
                let suffix = &vault::content_hash(&title)[..6];
                let alt = in_folder(
                    &folder,
                    &format!("{}-{suffix}.md", vault::note_filename(&title)),
                );
                return write_at(
                    conn, vault_dir, user_id, device_id, &folder, note, &alt, None, now, false,
                    Uuid::new_v4().to_string(), now,
                );
            }
            (Uuid::new_v4().to_string(), now, false, None)
        }
    };

    write_at(
        conn,
        vault_dir,
        user_id,
        device_id,
        &folder,
        note,
        &target_rel,
        old_path.as_deref(),
        now,
        updated,
        id,
        created_at,
    )
}

#[allow(clippy::too_many_arguments)]
fn write_at(
    conn: &Connection,
    vault_dir: &Path,
    user_id: &str,
    device_id: &str,
    folder: &str,
    note: &VaultNote,
    rel: &str,
    old_path: Option<&str>,
    now: i64,
    updated: bool,
    id: String,
    created_at: i64,
) -> Result<WriteOutcome, String> {
    // folder 变了 → 移动旧文件（Obsidian 里 wikilink 按文件名解析，移动不断链）。
    if let Some(old) = old_path {
        if old != rel {
            let _ = vault::move_note(vault_dir, old, rel);
            conn.execute(
                "DELETE FROM knowledge_links WHERE from_path = ?1",
                params![old],
            )
            .map_err(|e| e.to_string())?;
            // 索引行跟着搬（同一行改 path），否则下面的 upsert 会插出第二行、
            // 撞上 title 唯一索引——那正是我们要消灭的"同标题两份"。
            conn.execute(
                "UPDATE knowledge_notes SET path = ?2 WHERE id = ?1",
                params![id, rel],
            )
            .map_err(|e| format!("移动索引行失败: {e}"))?;
        }
    }

    let res =
        vault::write_note_at(vault_dir, rel, note).map_err(|e| format!("写 vault 失败: {e}"))?;
    let tags_json = serde_json::to_string(&note.tags).unwrap_or_else(|_| "[]".into());
    let sources_json = serde_json::to_string(&note.sources).unwrap_or_else(|_| "[]".into());
    let aliases_json = serde_json::to_string(&note.aliases).unwrap_or_else(|_| "[]".into());

    conn.execute(
        "INSERT INTO knowledge_notes
           (id, path, folder, title, body, tags_json, sources_json, aliases_json,
            content_hash, status, created_at, updated_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,'active',?10,?11)
         ON CONFLICT(path) DO UPDATE SET
           folder=excluded.folder, title=excluded.title, body=excluded.body,
           tags_json=excluded.tags_json, sources_json=excluded.sources_json,
           aliases_json=excluded.aliases_json, content_hash=excluded.content_hash,
           status='active', updated_at=excluded.updated_at",
        params![
            id,
            res.relative_path,
            folder,
            note.title.trim(),
            note.body,
            tags_json,
            sources_json,
            aliases_json,
            res.content_hash,
            created_at,
            now
        ],
    )
    .map_err(|e| format!("写索引失败: {e}"))?;

    let wanted = rebuild_links(conn, &res.relative_path, &merged_links(note))?;

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
            "folder": folder,
            "sources": note.sources,
            "concept_count": note.links.len(),
            "wanted_links": wanted,
        }),
        parent_event_ids: vec![],
        privacy_level: "L1".into(),
    };
    crate::ingest::ingest_event(conn, user_id, device_id, breadcrumb)
        .map_err(|e| format!("写溯源事件失败: {e}"))?;

    vault::git_snapshot(
        vault_dir,
        &format!(
            "{} {}",
            if updated { "更新" } else { "新建" },
            note.title.trim()
        ),
    );

    Ok(WriteOutcome {
        id,
        path: res.relative_path,
        content_hash: res.content_hash,
        updated,
        wanted_links: wanted,
    })
}

/// 重建一张卡的出链边；返回其中**未解析**的目标（红链）。
fn rebuild_links(
    conn: &Connection,
    from_path: &str,
    links: &[String],
) -> Result<Vec<String>, String> {
    conn.execute(
        "DELETE FROM knowledge_links WHERE from_path = ?1",
        params![from_path],
    )
    .map_err(|e| e.to_string())?;
    let now = now_ms();
    let mut wanted = Vec::new();
    for target in links {
        let resolved = resolve_title(conn, target)?.is_some();
        if !resolved {
            wanted.push(target.clone());
        }
        conn.execute(
            "INSERT OR REPLACE INTO knowledge_links (from_path, to_title, resolved, updated_at)
             VALUES (?1,?2,?3,?4)",
            params![from_path, target, resolved as i64, now],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(wanted)
}

/// 标题（或别名，或路径式引用）→ 存活卡片路径。
///
/// 路径式（`[[kb/network-infra/index]]`）也要能解析：领域落地页都叫 `index.md`，标题无法唯一，
/// Obsidian 侧本来就是按路径链接它们的。
pub fn resolve_title(conn: &Connection, title: &str) -> Result<Option<String>, String> {
    let t = title.trim();
    if t.contains('/') {
        let as_path = format!("{}.md", t.trim_start_matches("./"));
        if let Some(path) = conn
            .query_row(
                "SELECT path FROM knowledge_notes
                 WHERE path = ?1 AND status NOT IN ('pruned','duplicate')",
                params![as_path],
                |r| r.get::<_, String>(0),
            )
            .optional()
            .map_err(|e| e.to_string())?
        {
            return Ok(Some(path));
        }
    }
    if let Some(path) = conn
        .query_row(
            "SELECT path FROM knowledge_notes
             WHERE title = ?1 AND status NOT IN ('pruned','duplicate')",
            params![t],
            |r| r.get::<_, String>(0),
        )
        .optional()
        .map_err(|e| e.to_string())?
    {
        return Ok(Some(path));
    }
    // 别名重定向（合并/改名后旧链接仍可解析）。
    let like = format!("%\"{t}\"%");
    conn.query_row(
        "SELECT path FROM knowledge_notes
         WHERE aliases_json LIKE ?1 AND status NOT IN ('pruned','duplicate') LIMIT 1",
        params![like],
        |r| r.get::<_, String>(0),
    )
    .optional()
    .map_err(|e| e.to_string())
}

// ── 读回 / 小节增生 ─────────────────────────────────────────────────────────

/// 读回一张卡的完整内容（标题解析，支持别名）。
///
/// 补充式增长要求"更新前先读回现有卡"，但工具面此前**没有读卡工具**——
/// agent 只能靠文件系统读，在没有文件工具的基座上就是直接整卡覆盖、静默丢内容。
pub fn read_knowledge_note(
    conn: &Connection,
    vault_dir: &Path,
    title: &str,
) -> Result<(String, vault::ParsedNote), String> {
    let path = resolve_title(conn, title)?
        .ok_or_else(|| format!("知识库里没有「{title}」（先 search_knowledge 确认主题名）"))?;
    let parsed = vault::read_note(vault_dir, &path).map_err(|e| format!("读 vault 失败: {e}"))?;
    Ok((path, parsed))
}

/// 给已有结晶加/精**一个 H2 小节**并写回超集（结晶化的默认路径，原子操作）。
///
/// 读回现有卡 → 只替换/插入指定小节 → 其余内容原样保留 → 整卡写回。
/// 这样"同主题多轮对话让同一颗结晶长大"不再依赖 agent 自己拼超集。
#[allow(clippy::too_many_arguments)]
pub fn append_section(
    conn: &Connection,
    vault_dir: &Path,
    user_id: &str,
    device_id: &str,
    title: &str,
    heading: &str,
    section_body: &str,
    extra_links: &[String],
    extra_sources: &[String],
) -> Result<WriteOutcome, String> {
    let (path, parsed) = read_knowledge_note(conn, vault_dir, title)?;
    let folder = path.rsplit_once('/').map(|(d, _)| d.to_string()).unwrap_or_default();

    let mut links = parsed.links.clone();
    // `## 关联` 是渲染层生成的，parsed.links 已含它；再并入新增关联。
    for l in extra_links {
        let l = l.trim().to_string();
        if !l.is_empty() && !links.contains(&l) {
            links.push(l);
        }
    }
    let mut sources = parsed.sources.clone();
    for s in extra_sources {
        let s = s.trim().to_string();
        if !s.is_empty() && !sources.contains(&s) {
            sources.push(s);
        }
    }
    // 正文里去掉 `## 关联` 小节（写回时由渲染层重新生成，避免重复）。
    let body_without_links = strip_section(&parsed.body, "关联");
    let new_body = vault::upsert_section(&body_without_links, heading, section_body);

    let note = VaultNote {
        title: parsed.title.unwrap_or_else(|| title.to_string()),
        body: new_body,
        tags: parsed.tags,
        links,
        sources,
        aliases: parsed.aliases,
    };
    write_knowledge_note(conn, vault_dir, user_id, device_id, Some(&folder), &note)
}

fn strip_section(body: &str, heading: &str) -> String {
    let target = format!("## {}", heading.trim());
    let mut out = String::new();
    let mut skipping = false;
    for line in body.lines() {
        if line.trim_start().starts_with("## ") {
            skipping = line.trim() == target;
        }
        if !skipping {
            out.push_str(line);
            out.push('\n');
        }
    }
    out.trim_end().to_string()
}

// ── 删除 / 合并 ─────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct DeleteOutcome {
    pub path: String,
    /// true = 找到并删除；false = 本就不存在（幂等）。
    pub deleted: bool,
}

/// 删除一张知识卡片：移除 vault `.md` + 索引软删（status='pruned'）+ 溯源事件。
///
/// ⚠️ 只删指定的那张卡，**不会**改别处的 wikilink（会留断链）。
/// 合并碎卡请用 [`merge_notes`]——它会写别名重定向并改写入链，不留断链。
pub fn delete_knowledge_note(
    conn: &Connection,
    vault_dir: &Path,
    user_id: &str,
    device_id: &str,
    key: &str,
) -> Result<DeleteOutcome, String> {
    let row: Option<(String, String)> = conn
        .query_row(
            "SELECT id, path FROM knowledge_notes WHERE title = ?1 AND status != 'pruned'",
            params![key],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .or_else(|_| {
            conn.query_row(
                "SELECT id, path FROM knowledge_notes WHERE path = ?1 AND status != 'pruned'",
                params![key],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
        })
        .ok();

    let (id, rel) = match row {
        Some(v) => v,
        None => {
            return Ok(DeleteOutcome {
                path: key.to_string(),
                deleted: false,
            })
        }
    };

    let abs = vault_dir.join(&rel);
    if abs.exists() {
        std::fs::remove_file(&abs).map_err(|e| format!("删 vault 文件失败: {e}"))?;
    }

    let now = now_ms();
    conn.execute(
        "UPDATE knowledge_notes SET status='pruned', updated_at=?2 WHERE id=?1",
        params![id, now],
    )
    .map_err(|e| format!("剪枝索引失败: {e}"))?;
    conn.execute(
        "DELETE FROM knowledge_links WHERE from_path = ?1",
        params![rel],
    )
    .map_err(|e| e.to_string())?;

    let breadcrumb = crate::ingest::NewEvent {
        event_id: None,
        source: "agent".into(),
        layer: "raw".into(),
        event_type: "knowledge_pruned".into(),
        time_mode: "point".into(),
        event_time: Some(now),
        start_time: None,
        end_time: None,
        entity: None,
        category: None,
        payload: serde_json::json!({ "path": rel, "key": key }),
        parent_event_ids: vec![],
        privacy_level: "L1".into(),
    };
    crate::ingest::ingest_event(conn, user_id, device_id, breadcrumb)
        .map_err(|e| format!("写溯源事件失败: {e}"))?;
    vault::git_snapshot(vault_dir, &format!("删除 {key}"));

    Ok(DeleteOutcome {
        path: rel,
        deleted: true,
    })
}

#[derive(Debug, Serialize)]
pub struct MergeOutcome {
    pub into_title: String,
    pub merged: Vec<String>,
    /// 改写了入链的文件数。
    pub rewritten_files: usize,
}

/// 结晶化归并（defragment 的原子操作）：把若干碎卡并进一颗结晶。
///
/// 顺序保证不留断链：
/// 1. 目标卡登记别名（旧标题 → 本卡），Obsidian 侧 `[[旧标题]]` 立即仍可解析；
/// 2. 改写其它卡正文里的 `[[旧标题]]` → `[[新标题]]`（索引 + vault 文件同时改）；
/// 3. 删掉碎卡。
///
/// 正文内容的合并（写超集）仍由 agent 先用 `append_section` 完成——机器不判断语义。
pub fn merge_notes(
    conn: &Connection,
    vault_dir: &Path,
    user_id: &str,
    device_id: &str,
    from_titles: &[String],
    into_title: &str,
) -> Result<MergeOutcome, String> {
    let (target_path, target) = read_knowledge_note(conn, vault_dir, into_title)?;
    let target_title = target.title.clone().unwrap_or_else(|| into_title.to_string());

    // 1. 别名
    let mut aliases = target.aliases.clone();
    for t in from_titles {
        let t = t.trim().to_string();
        if t.is_empty() || t == target_title || aliases.contains(&t) {
            continue;
        }
        aliases.push(t);
    }
    let folder = target_path
        .rsplit_once('/')
        .map(|(d, _)| d.to_string())
        .unwrap_or_default();
    let note = VaultNote {
        title: target_title.clone(),
        body: strip_section(&target.body, "关联"),
        tags: target.tags.clone(),
        links: target.links.clone(),
        sources: target.sources.clone(),
        aliases,
    };
    write_knowledge_note(conn, vault_dir, user_id, device_id, Some(&folder), &note)?;

    // 2. 改写入链
    let mut rewritten = 0usize;
    for rel in vault::list_notes(vault_dir).map_err(|e| e.to_string())? {
        if rel == target_path {
            continue;
        }
        let abs = vault_dir.join(&rel);
        let Ok(content) = std::fs::read_to_string(&abs) else {
            continue;
        };
        let mut updated = content.clone();
        for old in from_titles {
            let old = old.trim();
            if old.is_empty() || old == target_title {
                continue;
            }
            updated = updated
                .replace(&format!("[[{old}]]"), &format!("[[{target_title}]]"))
                .replace(&format!("[[{old}|"), &format!("[[{target_title}|"));
        }
        if updated != content {
            std::fs::write(&abs, &updated).map_err(|e| e.to_string())?;
            let parsed = vault::parse_note(&updated);
            conn.execute(
                "UPDATE knowledge_notes SET body=?2, content_hash=?3, updated_at=?4 WHERE path=?1",
                params![rel, parsed.body, vault::content_hash(&updated), now_ms()],
            )
            .map_err(|e| e.to_string())?;
            rebuild_links(conn, &rel, &parsed.links)?;
            rewritten += 1;
        }
    }

    // 3. 删碎卡
    let mut merged = Vec::new();
    for t in from_titles {
        if t.trim() == target_title {
            continue;
        }
        if delete_knowledge_note(conn, vault_dir, user_id, device_id, t)?.deleted {
            merged.push(t.clone());
        }
    }
    vault::git_snapshot(
        vault_dir,
        &format!("结晶化：{} → {target_title}", merged.join("、")),
    );

    Ok(MergeOutcome {
        into_title: target_title,
        merged,
        rewritten_files: rewritten,
    })
}

// ── 检索 ─────────────────────────────────────────────────────────────────────

pub fn list_knowledge(conn: &Connection) -> rusqlite::Result<Vec<KnowledgeNote>> {
    let mut stmt = conn.prepare(
        "SELECT id, path, folder, title, tags_json, sources_json, aliases_json,
                content_hash, status, created_at, updated_at
         FROM knowledge_notes WHERE status != 'pruned' ORDER BY folder, title",
    )?;
    let rows = stmt
        .query_map([], row_to_note)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// 关键词检索。**含正文**——这是查重能不能生效的关键：
/// 此前只 LIKE 标题/标签/路径，一个概念只要不在标题里就查不出来，于是 agent 只能新建。
/// （FTS5 是后续优化；个人库规模下 LIKE 已经够用，且不引入新依赖。）
pub fn search_knowledge(conn: &Connection, query: &str) -> rusqlite::Result<Vec<KnowledgeHit>> {
    let q = query.trim();
    let like = format!("%{q}%");
    let mut stmt = conn.prepare(
        "SELECT title, path, folder, tags_json, body, updated_at
         FROM knowledge_notes
         WHERE status != 'pruned'
           AND (title LIKE ?1 OR tags_json LIKE ?1 OR path LIKE ?1
                OR body LIKE ?1 OR aliases_json LIKE ?1)
         ORDER BY
           CASE WHEN title = ?2 THEN 0
                WHEN title LIKE ?1 THEN 1
                WHEN aliases_json LIKE ?1 THEN 2
                ELSE 3 END,
           updated_at DESC
         LIMIT 30",
    )?;
    let rows = stmt
        .query_map(params![like, q], |r| {
            let tags_json: String = r.get(3)?;
            let body: String = r.get(4)?;
            Ok(KnowledgeHit {
                title: r.get(0)?,
                path: r.get(1)?,
                folder: r.get(2)?,
                tags: serde_json::from_str(&tags_json).unwrap_or_default(),
                excerpt: excerpt(&body, q),
                updated_at: r.get(5)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn excerpt(body: &str, query: &str) -> String {
    let flat = body.replace('\n', " ");
    let chars: Vec<char> = flat.chars().collect();
    let start = if query.is_empty() {
        0
    } else {
        flat.find(query)
            .map(|byte_idx| flat[..byte_idx].chars().count().saturating_sub(40))
            .unwrap_or(0)
    };
    let end = (start + 160).min(chars.len());
    let mut s: String = chars[start.min(chars.len())..end].iter().collect();
    if end < chars.len() {
        s.push('…');
    }
    s.trim().to_string()
}

fn row_to_note(r: &rusqlite::Row) -> rusqlite::Result<KnowledgeNote> {
    let tags_json: String = r.get(4)?;
    let sources_json: String = r.get(5)?;
    let aliases_json: String = r.get(6)?;
    Ok(KnowledgeNote {
        id: r.get(0)?,
        path: r.get(1)?,
        folder: r.get(2)?,
        title: r.get(3)?,
        tags: serde_json::from_str(&tags_json).unwrap_or_default(),
        sources: serde_json::from_str(&sources_json).unwrap_or_default(),
        aliases: serde_json::from_str(&aliases_json).unwrap_or_default(),
        content_hash: r.get(7)?,
        status: r.get(8)?,
        created_at: r.get(9)?,
        updated_at: r.get(10)?,
    })
}

// ── 重建索引（把 vault 现状同步进索引）──────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct ReindexReport {
    pub scanned: usize,
    pub inserted: usize,
    pub updated: usize,
    pub links: usize,
}

/// 扫描 vault，按文件重建 `knowledge_notes` / `knowledge_links`。
///
/// 用途：① 老库的 `body`/`folder`/链接边是空的，需要回填；② 用户在 Obsidian 里手改/移动/删除
/// 过文件后让索引追上；③ 索引本来就是"可重建的投影"，vault 的 `.md` 才是本体。
///
/// **顺序很重要**：先剪枝（path 已不存在的行），再插入。否则文件被改名时，旧行还占着
/// `title` 的唯一索引，新行插不进去（真实库上验证过这个坑）。改名的卡会复用旧行的
/// `id` 与 `created_at`，保住历史。
pub fn reindex_vault(conn: &Connection, vault_dir: &Path) -> Result<ReindexReport, String> {
    let files = vault::list_notes(vault_dir).map_err(|e| e.to_string())?;
    let mut report = ReindexReport {
        scanned: 0,
        inserted: 0,
        updated: 0,
        links: 0,
    };
    let now = now_ms();

    // 1. 剪枝：索引里存在但 vault 里已消失的行（用户删了文件，或文件被改名）。
    let indexed: Vec<(String, String)> = {
        let mut stmt = conn
            .prepare("SELECT path, title FROM knowledge_notes WHERE status != 'pruned'")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .map_err(|e| e.to_string())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| e.to_string())?;
        rows
    };
    for (path, _) in &indexed {
        if !files.contains(path) {
            conn.execute(
                "UPDATE knowledge_notes SET status='pruned', updated_at=?2 WHERE path=?1",
                params![path, now],
            )
            .map_err(|e| e.to_string())?;
            conn.execute(
                "DELETE FROM knowledge_links WHERE from_path = ?1",
                params![path],
            )
            .map_err(|e| e.to_string())?;
        }
    }

    // 2. 逐文件 upsert。
    for rel in &files {
        if !rel.starts_with(KB_ROOT) {
            continue; // sources/ 与 vault 根的散文件不进图谱索引
        }
        report.scanned += 1;
        let content = match std::fs::read_to_string(vault_dir.join(rel)) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let parsed = vault::parse_note(&content);
        let stem = rel
            .rsplit('/')
            .next()
            .and_then(|f| f.strip_suffix(".md"))
            .unwrap_or(rel)
            .to_string();
        let folder = rel
            .rsplit_once('/')
            .map(|(d, _)| d.to_string())
            .unwrap_or_default();
        // Obsidian 按**文件名**解析链接，所以文件名通常就是权威标题。
        // 例外：领域落地页一律叫 `index.md`（Quartz 的栏目首页约定），文件名会互相撞车 ——
        // 这类卡用 frontmatter 的 title，链接侧走路径式 `[[kb/xxx/index]]`（见 resolve_title）。
        let title = if stem == "index" {
            parsed
                .title
                .clone()
                .filter(|t| !t.trim().is_empty())
                .unwrap_or_else(|| {
                    let leaf = folder.rsplit('/').next().unwrap_or("kb");
                    format!("{leaf} 目录")
                })
        } else {
            stem
        };

        // 同 path → 更新；否则找同标题的已剪枝行（= 改名）复用 id/created_at；否则新建。
        let existing: Option<(String, i64)> = conn
            .query_row(
                "SELECT id, created_at FROM knowledge_notes WHERE path = ?1",
                params![rel],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        let renamed: Option<(String, i64)> = if existing.is_none() {
            conn.query_row(
                "SELECT id, created_at FROM knowledge_notes
                 WHERE title = ?1 AND status = 'pruned' ORDER BY updated_at DESC LIMIT 1",
                params![title],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(|e| e.to_string())?
        } else {
            None
        };
        let (id, created_at) = match existing.or(renamed) {
            Some((id, created)) => {
                report.updated += 1;
                (id, created)
            }
            None => {
                report.inserted += 1;
                (Uuid::new_v4().to_string(), now)
            }
        };
        // 复用旧行时先把它的 path 指到新位置，避免插出第二行撞 title 唯一索引。
        conn.execute(
            "UPDATE knowledge_notes SET path=?2, status='active' WHERE id=?1",
            params![id, rel],
        )
        .map_err(|e| e.to_string())?;

        conn.execute(
            "INSERT INTO knowledge_notes
               (id,path,folder,title,body,tags_json,sources_json,aliases_json,
                content_hash,status,created_at,updated_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,'active',?10,?11)
             ON CONFLICT(path) DO UPDATE SET
               folder=excluded.folder,title=excluded.title,body=excluded.body,
               tags_json=excluded.tags_json,sources_json=excluded.sources_json,
               aliases_json=excluded.aliases_json,content_hash=excluded.content_hash,
               status='active',updated_at=excluded.updated_at",
            params![
                id,
                rel,
                folder,
                title,
                parsed.body,
                serde_json::to_string(&parsed.tags).unwrap_or_else(|_| "[]".into()),
                serde_json::to_string(&parsed.sources).unwrap_or_else(|_| "[]".into()),
                serde_json::to_string(&parsed.aliases).unwrap_or_else(|_| "[]".into()),
                vault::content_hash(&content),
                created_at,
                now
            ],
        )
        .map_err(|e| format!("重建索引失败 {rel}: {e}"))?;
    }

    // 3. 链接边最后重建（此时全部标题都已在索引里，resolved 判定才准）。
    for rel in &files {
        if !rel.starts_with(KB_ROOT) {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(vault_dir.join(rel)) else {
            continue;
        };
        let links = vault::extract_wikilinks(&content);
        report.links += links.len();
        rebuild_links(conn, rel, &links)?;
    }
    Ok(report)
}

// ── 领域枢纽（MOC）：让图谱显出层级 ─────────────────────────────────────────

/// Core 生成区块的边界标记。标记之外的内容（领域叙述）由人/agent 自由编写，永不被覆盖。
pub const MOC_BEGIN: &str = "<!-- kb:auto begin -->";
pub const MOC_END: &str = "<!-- kb:auto end -->";

#[derive(Debug, Serialize)]
pub struct MocRefreshReport {
    pub refreshed: Vec<String>,
    pub cards_listed: usize,
    pub sources_listed: usize,
    pub skipped_missing: Vec<String>,
}

/// 领域枢纽的**角色标记**。一个领域恰好一个 `hub`。
///
/// 为什么不能靠 `moc` 标签猜：`moc` 是**类型**（目录/地图性质的卡），一个领域里可以有多张
/// ——比如 `network-infra` 既有领域枢纽「网络与基础设施」，又有概念地图「安全基建」。
/// 早先按"路径最短的 moc"猜，结果把自动目录写进了「安全基建」，真正的枢纽反而空着。
/// 枢纽是**结构角色**，必须显式声明。
pub const HUB_TAG: &str = "hub";

/// 一个领域的枢纽笔记（`(path, title)`）。
///
/// **枢纽不叫 `index.md`**：Obsidian 图谱的节点标签用的是**文件名**，九个领域都叫 `index`
/// 就会在图上出现九个无法区分的「index」节点——这正是"图谱里全是占位符"的观感来源。
/// 枢纽用领域自己的名字（`网络与基础设施`、`Web 安全`…），图谱一眼就能读出层级。
///
/// 0 个或多于 1 个都返回 `None`（调用方报告，不猜）。
fn hub_of(conn: &Connection, folder: &str) -> Result<Option<(String, String)>, String> {
    let like = format!("%\"{HUB_TAG}\"%");
    let mut stmt = conn
        .prepare(
            "SELECT path, title FROM knowledge_notes
             WHERE folder = ?1 AND status NOT IN ('pruned','duplicate') AND tags_json LIKE ?2",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![folder, like], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    match rows.len() {
        1 => Ok(Some(rows.into_iter().next().unwrap())),
        _ => Ok(None),
    }
}

/// 重新生成所有领域枢纽的自动区块：**卡片清单 + 子领域 + 上级枢纽 + 原始材料**。
///
/// 为什么必须由 Core 生成：手写的目录必然漂移（实测该 vault 的 MOC 漏掉了 24 张卡）。
/// 而"MOC 真的连着它下面的卡片"正是图谱里出现树状层级的**唯一**机制——
/// 漂移的目录 = 图谱里一堆互不相连的孤点。
///
/// 同时它天然实现了原始材料的**单向连接**：MOC → 原文，原文自己不出链。
pub fn refresh_mocs(conn: &Connection, vault_dir: &Path) -> Result<MocRefreshReport, String> {
    let notes = list_knowledge(conn).map_err(|e| e.to_string())?;
    let mut report = MocRefreshReport {
        refreshed: Vec::new(),
        cards_listed: 0,
        sources_listed: 0,
        skipped_missing: Vec::new(),
    };

    // 领域集合（含没有卡片但有子领域的中间层）。
    let mut folders: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for n in &notes {
        let mut f = n.folder.clone();
        while !f.is_empty() {
            folders.insert(f.clone());
            match f.rsplit_once('/') {
                Some((parent, _)) if parent.starts_with("kb") => f = parent.to_string(),
                _ => break,
            }
        }
    }

    for folder in &folders {
        let Some((moc_path, moc_title)) = hub_of(conn, folder)? else {
            report.skipped_missing.push(folder.clone());
            continue;
        };
        // 本领域直属的卡片（排除枢纽自己），分成知识卡与原始材料两组。
        let mut cards: Vec<&KnowledgeNote> = Vec::new();
        let mut srcs: Vec<&KnowledgeNote> = Vec::new();
        for n in &notes {
            if &n.folder != folder || n.path == moc_path {
                continue;
            }
            if crate::sources::is_source(&n.tags) {
                srcs.push(n);
            } else {
                cards.push(n);
            }
        }
        cards.sort_by(|a, b| a.title.cmp(&b.title));
        srcs.sort_by(|a, b| a.title.cmp(&b.title));

        // 直接子领域的枢纽（脊椎向下）。
        let mut children: Vec<(String, String)> = Vec::new();
        for f in &folders {
            let is_direct_child = f
                .strip_prefix(&format!("{folder}/"))
                .is_some_and(|rest| !rest.contains('/'));
            if is_direct_child {
                if let Some((_, t)) = hub_of(conn, f)? {
                    children.push((f.clone(), t));
                }
            }
        }
        children.sort();
        // 上级枢纽（脊椎向上）。
        let parent_moc = match folder.rsplit_once('/') {
            Some((parent, _)) if parent.starts_with("kb") => hub_of(conn, parent)?.map(|(_, t)| t),
            _ => None,
        };

        let mut block = String::new();
        if let Some(parent) = &parent_moc {
            block.push_str(&format!("上级：[[{parent}]]\n\n"));
        }
        if !children.is_empty() {
            block.push_str("### 子领域\n");
            for (_, t) in &children {
                block.push_str(&format!("- [[{t}]]\n"));
            }
            block.push('\n');
        }
        block.push_str(&format!("### 卡片（{}）\n", cards.len()));
        if cards.is_empty() {
            block.push_str("- 暂无\n");
        }
        for c in &cards {
            report.cards_listed += 1;
            block.push_str(&format!("- [[{}]]\n", c.title));
        }
        if !srcs.is_empty() {
            block.push_str(&format!("\n### 原始材料（{}）\n", srcs.len()));
            block.push_str("> 逐字原文，不外发；图谱里是叶子节点（只被指向）。\n");
            for s in &srcs {
                report.sources_listed += 1;
                block.push_str(&format!("- [[{}]]\n", s.title));
            }
        }

        // 写回：只替换 marker 之间的内容，marker 外的领域叙述原样保留。
        let abs = vault_dir.join(&moc_path);
        let existing = std::fs::read_to_string(&abs).unwrap_or_default();
        let updated = splice_auto_block(&existing, &block);
        if updated != existing {
            std::fs::write(&abs, &updated).map_err(|e| format!("写枢纽失败 {moc_path}: {e}"))?;
            let parsed = vault::parse_note(&updated);
            conn.execute(
                "UPDATE knowledge_notes SET body=?2, content_hash=?3, updated_at=?4 WHERE path=?1",
                params![moc_path, parsed.body, vault::content_hash(&updated), now_ms()],
            )
            .map_err(|e| e.to_string())?;
            rebuild_links(conn, &moc_path, &parsed.links)?;
            report.refreshed.push(moc_title);
        }
    }
    vault::git_snapshot(vault_dir, "刷新领域枢纽目录");
    Ok(report)
}

/// 把自动区块塞进（或替换掉）文件里的 marker 之间；没有 marker 时追加到末尾。
fn splice_auto_block(content: &str, block: &str) -> String {
    let body = block.trim_end();
    match (content.find(MOC_BEGIN), content.find(MOC_END)) {
        (Some(b), Some(e)) if e > b => {
            let mut out = String::new();
            out.push_str(&content[..b]);
            out.push_str(MOC_BEGIN);
            out.push('\n');
            out.push_str(body);
            out.push('\n');
            out.push_str(&content[e..]);
            out
        }
        _ => {
            let mut out = content.trim_end().to_string();
            out.push_str("\n\n## 目录\n");
            out.push_str(MOC_BEGIN);
            out.push('\n');
            out.push_str(body);
            out.push('\n');
            out.push_str(MOC_END);
            out.push('\n');
            out
        }
    }
}
