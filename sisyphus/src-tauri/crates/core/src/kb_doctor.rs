//! 知识库体检（`kb_doctor`）与红链队列（`kb_wanted`）。
//!
//! 这是"约束层"的另一半：写入器负责**不制造**问题，本模块负责**发现**已经存在的问题。
//!
//! 为什么必须是确定性代码而不是让 agent 自己看：agent 无法在上下文里可靠地统计几十上百张
//! 卡的图结构（入度、断链、前缀簇、目录规模）。维基百科靠 `Special:LonelyPages` /
//! `WantedPages` / `DeadEndPages` 这类维护报告维持秩序，不是靠编辑记住方针——
//! 这里就是那几张报告。
//!
//! `rebalance` / `defragment` 例程因此从"agent 靠感觉扫一遍"变成"跑 lint → 修 top N"。

use rusqlite::{params, Connection};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

use crate::knowledge::{KB_ROOT, RELIABILITY_TAGS, TYPE_TAGS};
use crate::vault;

/// 单领域卡片数的经验阈值（**触发思考的信号，不是硬指标**）。
const FOLDER_SPLIT_AT: usize = 12;
const FOLDER_MERGE_UNDER: usize = 2;

#[derive(Debug, Clone, Serialize)]
pub struct BrokenLink {
    pub from_path: String,
    pub to_title: String,
}

/// 红链：被引用但还不存在的卡。**这是资产，不是缺陷**——它就是主动调研队列。
#[derive(Debug, Clone, Serialize)]
pub struct WantedNote {
    pub title: String,
    /// 被多少张卡引用（热度排序，先补被引用最多的）。
    pub referenced_by: usize,
    pub sources: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FolderStat {
    pub folder: String,
    pub count: usize,
    /// split（>阈值该拆）| merge（长期过少该并）| ok
    pub advice: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct NoteIssue {
    pub title: String,
    pub path: String,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct FragmentCluster {
    pub folder: String,
    /// 共享的主题前缀。
    pub prefix: String,
    pub titles: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct KbReport {
    pub total_notes: usize,
    pub total_links: usize,
    /// 断链条数 / 断链率（Obsidian 里点不开的比例）。
    pub broken_links: Vec<BrokenLink>,
    pub broken_ratio: f64,
    /// 入度为 0（没有任何卡链接到它）——孤儿卡。
    pub orphans: Vec<NoteIssue>,
    /// 没有任何出链（除 MOC 外不该出现）。
    pub dead_ends: Vec<NoteIssue>,
    /// 同标题多份 / 同文件名多份（Obsidian 链接歧义）。
    pub duplicates: Vec<NoteIssue>,
    pub missing_type: Vec<NoteIssue>,
    pub missing_reliability: Vec<NoteIssue>,
    /// 声称高可靠性却没有 sources 的卡（模型自升档）。
    pub unsupported_claims: Vec<NoteIssue>,
    pub folders: Vec<FolderStat>,
    /// 同领域内共享主题前缀的碎卡簇 → 结晶化候选。
    pub fragment_clusters: Vec<FragmentCluster>,
    /// 领域 MOC（`index.md`）没列到的卡 → 目录漂移。
    pub index_drift: Vec<NoteIssue>,
    /// `kb/` 之外的散落 .md。
    pub stray_files: Vec<String>,
    /// 没有任何卡片/枢纽指向的原始材料（= 存了但没人会看到的原文）。
    pub unreferenced_sources: Vec<NoteIssue>,
    /// 违反单向连接的原始材料（原文自己出链，会在图谱里连成一团）。
    pub linking_sources: Vec<NoteIssue>,
}

impl KbReport {
    /// 一行摘要（给 agent 回报用）。
    pub fn summary(&self) -> String {
        format!(
            "卡片 {} · 断链 {}（{:.0}%）· 孤儿 {} · 重复 {} · 缺类型 {} · 缺可靠性 {} · \
             无据高档 {} · 碎卡簇 {} · 目录漂移 {} · 散落文件 {} · 红链 {} · \
             无人引用的原文 {} · 违反单向的原文 {}",
            self.total_notes,
            self.broken_links.len(),
            self.broken_ratio * 100.0,
            self.orphans.len(),
            self.duplicates.len(),
            self.missing_type.len(),
            self.missing_reliability.len(),
            self.unsupported_claims.len(),
            self.fragment_clusters.len(),
            self.index_drift.len(),
            self.stray_files.len(),
            self.wanted_count(),
            self.unreferenced_sources.len(),
            self.linking_sources.len()
        )
    }

    fn wanted_count(&self) -> usize {
        self.broken_links
            .iter()
            .map(|b| b.to_title.as_str())
            .collect::<HashSet<_>>()
            .len()
    }
}

struct Row {
    path: String,
    folder: String,
    title: String,
    tags: Vec<String>,
    sources: Vec<String>,
    body: String,
}

fn load(conn: &Connection) -> Result<Vec<Row>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT path, folder, title, tags_json, sources_json, body
             FROM knowledge_notes WHERE status NOT IN ('pruned') ORDER BY path",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            let tags: String = r.get(3)?;
            let sources: String = r.get(4)?;
            Ok(Row {
                path: r.get(0)?,
                folder: r.get(1)?,
                title: r.get(2)?,
                tags: serde_json::from_str(&tags).unwrap_or_default(),
                sources: serde_json::from_str(&sources).unwrap_or_default(),
                body: r.get(5)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    Ok(rows)
}

fn has_tag(tags: &[String], allowed: &[&str]) -> bool {
    tags.iter()
        .any(|t| allowed.contains(&t.trim().trim_start_matches('#')))
}

fn is_moc(row: &Row) -> bool {
    row.tags.iter().any(|t| t.trim().trim_start_matches('#') == "moc")
        || row.path.ends_with("/index.md")
        || row.path == "kb/index.md"
}

/// 原始材料按 **tag** 判断，不看路径——它现在就地存放在话题夹里。
fn is_source_row(row: &Row) -> bool {
    crate::sources::is_source(&row.tags)
}

/// 领域枢纽：显式的 `hub` 角色标记（`moc` 只是类型，一个领域可以有多张 moc 卡）。
fn is_hub(row: &Row) -> bool {
    row.tags
        .iter()
        .any(|t| t.trim().trim_start_matches('#') == crate::knowledge::HUB_TAG)
}

/// 跑一次完整体检。`vault_dir` 可选：给了才检查散落文件与未引用原文。
pub fn doctor(conn: &Connection, vault_dir: Option<&Path>) -> Result<KbReport, String> {
    let rows = load(conn)?;
    let by_path: HashMap<&str, &Row> = rows.iter().map(|r| (r.path.as_str(), r)).collect();
    let titles: HashSet<&str> = rows.iter().map(|r| r.title.as_str()).collect();

    // 链接边
    let mut stmt = conn
        .prepare("SELECT from_path, to_title, resolved FROM knowledge_links")
        .map_err(|e| e.to_string())?;
    let edges = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)? != 0,
            ))
        })
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;

    let mut broken = Vec::new();
    let mut in_degree: HashMap<String, usize> = HashMap::new();
    let mut out_degree: HashMap<String, usize> = HashMap::new();
    let mut total_links = 0usize;
    let paths: HashSet<&str> = rows.iter().map(|r| r.path.as_str()).collect();
    for (from, to, resolved) in &edges {
        // drawio/excalidraw 嵌入不是知识链接。
        if to.starts_with("drawio:") || to.starts_with("excalidraw:") {
            continue;
        }
        total_links += 1;
        *out_degree.entry(from.clone()).or_default() += 1;
        // 断链判定必须与写入侧的 `knowledge::resolve_title` 一致，否则同一条链接
        // 在"写入时已解析"和"体检报告说它断了"之间自相矛盾。
        // 三种解析方式：标题、路径式引用（领域落地页都叫 index.md）、以及写入时算好的 resolved。
        let as_path = format!("{}.md", to.trim_start_matches("./"));
        let ok = *resolved || titles.contains(to.as_str()) || paths.contains(as_path.as_str());
        if ok {
            // 入度按解析到的目标累加（路径式引用记到该文件的标题上）。
            let key = if titles.contains(to.as_str()) {
                to.clone()
            } else {
                by_path
                    .get(as_path.as_str())
                    .map(|r| r.title.clone())
                    .unwrap_or_else(|| to.clone())
            };
            *in_degree.entry(key).or_default() += 1;
        } else {
            broken.push(BrokenLink {
                from_path: from.clone(),
                to_title: to.clone(),
            });
        }
    }

    let issue = |r: &Row, detail: &str| NoteIssue {
        title: r.title.clone(),
        path: r.path.clone(),
        detail: detail.to_string(),
    };

    let mut orphans = Vec::new();
    let mut dead_ends = Vec::new();
    let mut missing_type = Vec::new();
    let mut missing_reliability = Vec::new();
    let mut unsupported = Vec::new();
    let mut unreferenced_sources = Vec::new();
    let mut linking_sources = Vec::new();
    for r in &rows {
        // 原始材料是另一套规则：图谱里应当是**叶子**（有入链、无出链）。
        if is_source_row(r) {
            if in_degree.get(&r.title).copied().unwrap_or(0) == 0 {
                unreferenced_sources.push(issue(
                    r,
                    "没有任何卡片或枢纽指向它——存了但没人会看到。要么从相关卡片引用，要么删掉",
                ));
            }
            if out_degree.get(&r.path).copied().unwrap_or(0) > 0 {
                linking_sources.push(issue(
                    r,
                    "原始材料不该有出链（单向连接）——关系写在引用它的那张卡里",
                ));
            }
            continue;
        }
        if is_moc(r) {
            continue;
        }
        if in_degree.get(&r.title).copied().unwrap_or(0) == 0 {
            orphans.push(issue(r, "没有任何卡链接到它——先接进母结晶或对应 MOC"));
        }
        if out_degree.get(&r.path).copied().unwrap_or(0) == 0 {
            dead_ends.push(issue(r, "没有任何出链——至少接一条有语义的关联"));
        }
        if !has_tag(&r.tags, TYPE_TAGS) {
            missing_type.push(issue(r, &format!("缺文章类型标签 {TYPE_TAGS:?}")));
        }
        if !has_tag(&r.tags, RELIABILITY_TAGS) {
            missing_reliability.push(issue(r, "缺可靠性档位（默认「待确认」）"));
        }
        let high = r.tags.iter().any(|t| {
            matches!(
                t.trim().trim_start_matches('#'),
                "多源印证" | "已复现" | "已验证"
            )
        });
        if high && r.sources.iter().all(|s| s.trim().is_empty()) {
            unsupported.push(issue(
                r,
                "声称高可靠性但没有 sources——没有证据只能是「待确认」",
            ));
        }
    }

    // 重复：同标题多份 + 同文件名多份（Obsidian 按文件名解析 → 歧义）
    let mut duplicates = Vec::new();
    let mut title_seen: HashMap<&str, Vec<&Row>> = HashMap::new();
    let mut base_seen: HashMap<String, Vec<&Row>> = HashMap::new();
    for r in &rows {
        title_seen.entry(r.title.as_str()).or_default().push(r);
        let base = r
            .path
            .rsplit('/')
            .next()
            .unwrap_or(&r.path)
            .to_ascii_lowercase();
        base_seen.entry(base).or_default().push(r);
    }
    for (title, group) in &title_seen {
        if group.len() > 1 {
            for r in group {
                duplicates.push(issue(
                    r,
                    &format!("标题「{title}」有 {} 份，需合并（merge_notes）", group.len()),
                ));
            }
        }
    }
    for (base, group) in &base_seen {
        if group.len() > 1 && base != "index.md" {
            for r in group {
                duplicates.push(issue(
                    r,
                    &format!("文件名 `{base}` 有 {} 份，Obsidian 链接会歧义", group.len()),
                ));
            }
        }
    }

    // 领域规模
    let mut folder_count: BTreeMap<&str, usize> = BTreeMap::new();
    for r in &rows {
        if is_moc(r) || is_source_row(r) {
            continue; // 枢纽与原始材料不计入"该拆该并"的卡片数
        }
        *folder_count.entry(r.folder.as_str()).or_default() += 1;
    }
    let folders: Vec<FolderStat> = folder_count
        .iter()
        .map(|(folder, count)| FolderStat {
            folder: (*folder).to_string(),
            count: *count,
            advice: if *count > FOLDER_SPLIT_AT {
                "split".to_string()
            } else if *count < FOLDER_MERGE_UNDER {
                "merge".to_string()
            } else {
                "ok".to_string()
            },
        })
        .collect();

    // 碎卡簇：同领域内共享 >=2 字前缀的标题
    let mut clusters = Vec::new();
    let mut by_folder: BTreeMap<&str, Vec<&Row>> = BTreeMap::new();
    for r in &rows {
        if is_moc(r) || is_source_row(r) {
            continue;
        }
        by_folder.entry(r.folder.as_str()).or_default().push(r);
    }
    for (folder, group) in &by_folder {
        let mut prefix_map: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for r in group {
            let chars: Vec<char> = r.title.chars().collect();
            if chars.len() < 3 {
                continue;
            }
            let prefix: String = chars[..2].iter().collect();
            prefix_map.entry(prefix).or_default().push(r.title.clone());
        }
        for (prefix, titles) in prefix_map {
            if titles.len() >= 2 {
                clusters.push(FragmentCluster {
                    folder: (*folder).to_string(),
                    prefix,
                    titles,
                });
            }
        }
    }

    // 目录漂移：领域枢纽没列到的卡。枢纽按 **moc tag** 找（不再假设文件名是 index.md）。
    // 正常情况下这一项应该恒为空——枢纽的目录区块由 `knowledge::refresh_mocs` 确定性生成；
    // 非空说明有人手改了自动区块，或新卡写入后没刷新枢纽。
    let mut index_drift = Vec::new();
    let mut hub_count: HashMap<&str, usize> = HashMap::new();
    for r in rows.iter().filter(|r| is_hub(r)) {
        *hub_count.entry(r.folder.as_str()).or_default() += 1;
    }
    let mocs: HashMap<&str, &Row> = rows
        .iter()
        .filter(|r| is_hub(r) && hub_count.get(r.folder.as_str()) == Some(&1))
        .map(|r| (r.folder.as_str(), r))
        .collect();
    for (folder, group) in &by_folder {
        let Some(moc_row) = mocs.get(folder) else {
            // 没有唯一枢纽的领域：图谱里这一支不会有树干，必须报出来。
            let n = hub_count.get(*folder).copied().unwrap_or(0);
            if let Some(any) = group.first() {
                index_drift.push(NoteIssue {
                    title: (*folder).to_string(),
                    path: (*folder).to_string(),
                    detail: format!(
                        "该领域有 {n} 个 hub 枢纽（应恰好 1 个）——图谱里这一支没有树干。\
                         给领域落地页加 `hub` 标签，多余的去掉",
                    ),
                });
                let _ = any;
            }
            continue;
        };
        let listed: HashSet<String> = vault::extract_wikilinks(&moc_row.body)
            .into_iter()
            .collect();
        for r in group {
            if !listed.contains(&r.title) {
                index_drift.push(issue(
                    r,
                    &format!("领域枢纽「{}」没列到它——跑 refresh_mocs", moc_row.title),
                ));
            }
        }
    }

    // 文件系统层面的检查
    let mut stray_files = Vec::new();
    if let Some(dir) = vault_dir {
        if let Ok(files) = vault::list_notes(dir) {
            for rel in files {
                // 原始材料现在就地存放在 `kb/` 里，靠 tag 隔离；
                // 因此 `kb/` 之外的 `.md` 一律是散落文件（含遗留的 sources/ 目录）。
                if !rel.starts_with(KB_ROOT) {
                    stray_files.push(rel);
                }
            }
        }
    }

    let broken_ratio = if total_links == 0 {
        0.0
    } else {
        broken.len() as f64 / total_links as f64
    };

    Ok(KbReport {
        total_notes: rows.len(),
        total_links,
        broken_links: broken,
        broken_ratio,
        orphans,
        dead_ends,
        duplicates,
        missing_type,
        missing_reliability,
        unsupported_claims: unsupported,
        folders,
        fragment_clusters: clusters,
        index_drift,
        stray_files,
        unreferenced_sources,
        linking_sources,
    })
}

/// 红链队列：被引用但还不存在的主题，按引用热度排序。
///
/// 三个场景由此闭环：① 沉淀时留红链 → ③ 按热度自动深研 → ① 补齐。
/// 场景③ 不再需要用户开口说"帮我深挖 X"。
pub fn wanted(conn: &Connection, min_refs: usize) -> Result<Vec<WantedNote>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT to_title, COUNT(*) AS n, GROUP_CONCAT(from_path, '|')
             FROM knowledge_links
             WHERE resolved = 0 AND to_title NOT LIKE 'drawio:%' AND to_title NOT LIKE 'excalidraw:%'
             GROUP BY to_title ORDER BY n DESC, to_title ASC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            let sources: String = r.get(2).unwrap_or_default();
            Ok(WantedNote {
                title: r.get(0)?,
                referenced_by: r.get::<_, i64>(1)? as usize,
                sources: sources.split('|').map(|s| s.to_string()).collect(),
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    Ok(rows
        .into_iter()
        .filter(|w| w.referenced_by >= min_refs.max(1))
        .collect())
}

/// 把断链目标改写成新标题（迁移旧 slug 链接、批量改名时用）。
/// 返回改写的文件数。会同步更新索引与链接边。
pub fn rewrite_link_targets(
    conn: &Connection,
    vault_dir: &Path,
    mapping: &[(String, String)],
) -> Result<usize, String> {
    let mut changed = 0usize;
    for rel in vault::list_notes(vault_dir).map_err(|e| e.to_string())? {
        let abs = vault_dir.join(&rel);
        let Ok(content) = std::fs::read_to_string(&abs) else {
            continue;
        };
        let mut updated = content.clone();
        for (from, to) in mapping {
            if from == to {
                continue;
            }
            updated = updated
                .replace(&format!("[[{from}]]"), &format!("[[{to}]]"))
                .replace(&format!("[[{from}|"), &format!("[[{to}|"));
        }
        if updated != content {
            std::fs::write(&abs, &updated).map_err(|e| e.to_string())?;
            let parsed = vault::parse_note(&updated);
            conn.execute(
                "UPDATE knowledge_notes SET body=?2, content_hash=?3, updated_at=?4 WHERE path=?1",
                params![
                    rel,
                    parsed.body,
                    vault::content_hash(&updated),
                    crate::clock::now_ms()
                ],
            )
            .map_err(|e| e.to_string())?;
            changed += 1;
        }
    }
    Ok(changed)
}
