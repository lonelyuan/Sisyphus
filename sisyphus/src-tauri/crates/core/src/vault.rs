//! Obsidian 兼容的 Markdown 知识库（vault）管理（Phase 1.3 第二大脑）。
//!
//! 纯 `std::fs`，**vault 路径由调用方传入**（app/mcp 解析 `SISYPHUS_VAULT` 或 data_dir），
//! `dirs` 不进 core（依赖卫生铁律）。知识本体是 `.md`（人类可读投影，可直接 Obsidian 打开）；
//! 可查询真相是 `knowledge_notes` 索引行（db.rs）；溯源是 Event log 的 `knowledge_ingested` 事件。
//!
//! 无时间戳写入 → 同内容重写产出字节一致的文件 → `content_hash` 稳定（幂等）。

use std::fs;
use std::io;
use std::path::Path;

/// 一张待写入的知识卡片。
#[derive(Debug, Clone)]
pub struct VaultNote {
    pub title: String,
    pub body: String,
    pub tags: Vec<String>,
    /// wikilink 目标（其它笔记标题），渲染为 `[[目标]]`。
    pub links: Vec<String>,
    /// 来源 url / 引用。
    pub sources: Vec<String>,
}

/// 从 `.md` 解析回的结构（供 read_note / 测试）。
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedNote {
    pub title: Option<String>,
    pub tags: Vec<String>,
    pub sources: Vec<String>,
    pub body: String,
    /// 正文中出现的 wikilink 目标。
    pub links: Vec<String>,
}

/// 写入结果。
#[derive(Debug, Clone)]
pub struct WriteResult {
    pub relative_path: String,
    pub abs_path: String,
    pub content_hash: String,
}

/// 把标题转成文件名安全的 slug（保留 CJK；空白/非法字符→单个 `-`；ascii 转小写）。
pub fn slugify(title: &str) -> String {
    let mut out = String::new();
    let mut prev_dash = false;
    for c in title.trim().chars() {
        if c.is_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            prev_dash = false;
        } else if c == '-' || c == '_' {
            out.push(c);
            prev_dash = false;
        } else if !prev_dash {
            out.push('-');
            prev_dash = true;
        }
    }
    let s = out.trim_matches('-').to_string();
    if s.is_empty() {
        "note".to_string()
    } else {
        s
    }
}

/// 稳定内容哈希（std DefaultHasher，无新 crate）；用于幂等判重。
pub fn content_hash(s: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    format!("{:016x}", h.finish())
}

/// 渲染成确定性的 Obsidian markdown（无时间戳，保证同输入同输出）。
pub fn render_note(note: &VaultNote) -> String {
    let safe_title = note.title.replace('"', "'");
    let mut s = String::new();
    s.push_str("---\n");
    s.push_str(&format!("title: \"{safe_title}\"\n"));
    s.push_str(&format!("tags: {}\n", render_inline_list(&note.tags)));
    s.push_str(&format!("sources: {}\n", render_inline_list(&note.sources)));
    s.push_str("---\n\n");
    s.push_str(&format!("# {}\n\n", note.title));
    s.push_str(note.body.trim());
    s.push('\n');
    if !note.links.is_empty() {
        s.push_str("\n## 关联\n");
        for l in &note.links {
            s.push_str(&format!("- [[{l}]]\n"));
        }
    }
    s
}

/// 默认相对文件名：`{slug(title)}.md`。
pub fn note_path(title: &str) -> String {
    format!("{}.md", slugify(title))
}

/// 写一张知识卡片到 vault，使用默认 slug 路径。返回相对路径 / 绝对路径 / 内容哈希。
pub fn write_note(vault_dir: &Path, note: &VaultNote) -> io::Result<WriteResult> {
    write_note_at(vault_dir, &note_path(&note.title), note)
}

/// 写到 vault 内**指定**相对路径（调用方决定路径，用于消歧防覆盖）。
pub fn write_note_at(
    vault_dir: &Path,
    relative_path: &str,
    note: &VaultNote,
) -> io::Result<WriteResult> {
    fs::create_dir_all(vault_dir)?;
    let abs = vault_dir.join(relative_path);
    let content = render_note(note);
    fs::write(&abs, &content)?;
    Ok(WriteResult {
        relative_path: relative_path.to_string(),
        abs_path: abs.to_string_lossy().into_owned(),
        content_hash: content_hash(&content),
    })
}

/// 读并解析 vault 内的一张笔记（相对路径）。
pub fn read_note(vault_dir: &Path, relative_path: &str) -> io::Result<ParsedNote> {
    let content = fs::read_to_string(vault_dir.join(relative_path))?;
    Ok(parse_note(&content))
}

/// 列出 vault 内所有 `.md` 的相对文件名。
pub fn list_notes(vault_dir: &Path) -> io::Result<Vec<String>> {
    let mut out = Vec::new();
    if !vault_dir.exists() {
        return Ok(out);
    }
    for entry in fs::read_dir(vault_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("md") {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                out.push(name.to_string());
            }
        }
    }
    out.sort();
    Ok(out)
}

/// 解析一段 markdown（frontmatter + 正文 wikilinks）。
pub fn parse_note(content: &str) -> ParsedNote {
    let lines: Vec<&str> = content.lines().collect();
    let mut title = None;
    let mut tags = Vec::new();
    let mut sources = Vec::new();
    let mut body_start = 0;

    if lines.first() == Some(&"---") {
        let mut i = 1;
        while i < lines.len() && lines[i] != "---" {
            let line = lines[i];
            if let Some(rest) = line.strip_prefix("title:") {
                title = Some(unquote(rest.trim()));
            } else if let Some(rest) = line.strip_prefix("tags:") {
                tags = parse_inline_list(rest.trim());
            } else if let Some(rest) = line.strip_prefix("sources:") {
                sources = parse_inline_list(rest.trim());
            }
            i += 1;
        }
        body_start = if i < lines.len() { i + 1 } else { i };
    }

    let body = lines[body_start..].join("\n").trim().to_string();
    let links = extract_wikilinks(&body);
    ParsedNote {
        title,
        tags,
        sources,
        body,
        links,
    }
}

/// 提取正文里的 `[[目标]]` / `[[目标|别名]]`（取目标部分）。
pub fn extract_wikilinks(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = body;
    while let Some(start) = rest.find("[[") {
        let after = &rest[start + 2..];
        if let Some(end) = after.find("]]") {
            let inner = &after[..end];
            let target = inner.split('|').next().unwrap_or(inner).trim();
            if !target.is_empty() {
                out.push(target.to_string());
            }
            rest = &after[end + 2..];
        } else {
            break;
        }
    }
    out
}

// ── helpers ─────────────────────────────────────────────────────────────────

fn render_inline_list(items: &[String]) -> String {
    let inner = items
        .iter()
        .map(|s| s.replace(',', " ").replace(']', ""))
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{inner}]")
}

fn parse_inline_list(s: &str) -> Vec<String> {
    let s = s.trim();
    let s = s.strip_prefix('[').unwrap_or(s);
    let s = s.strip_suffix(']').unwrap_or(s);
    s.split(',')
        .map(|x| unquote(x.trim()))
        .filter(|x| !x.is_empty())
        .collect()
}

fn unquote(s: &str) -> String {
    let s = s.trim();
    let s = s.strip_prefix('"').unwrap_or(s);
    let s = s.strip_suffix('"').unwrap_or(s);
    s.to_string()
}
