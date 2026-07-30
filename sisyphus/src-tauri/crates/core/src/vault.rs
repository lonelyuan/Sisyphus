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
#[derive(Debug, Clone, Default)]
pub struct VaultNote {
    pub title: String,
    pub body: String,
    pub tags: Vec<String>,
    /// wikilink 目标（其它笔记标题），渲染为 `[[目标]]`。
    pub links: Vec<String>,
    /// 来源 url / 引用。
    pub sources: Vec<String>,
    /// 别名（重定向）：指向本卡的旧标题。Obsidian 用 `aliases` 解析 `[[旧标题]]`。
    pub aliases: Vec<String>,
}

/// 从 `.md` 解析回的结构（供 read_note / 测试）。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ParsedNote {
    pub title: Option<String>,
    pub tags: Vec<String>,
    pub sources: Vec<String>,
    pub aliases: Vec<String>,
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
///
/// ⚠️ **只用于 `sources/` 的原文归档文件名**，不要用它给 kb 卡片命名——见
/// [`note_filename`] 的说明。
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

/// 文件系统非法字符（Windows 取最严的一套，保证 vault 可跨平台同步）。
const ILLEGAL: &[char] = &['/', '\\', ':', '*', '?', '"', '<', '>', '|', '\n', '\r', '\t'];

/// kb 卡片的文件名：**原样用标题**，只替换文件系统非法字符。
///
/// 为什么不 slugify：Obsidian 解析 `[[X]]` 靠的是**文件名**（或 frontmatter `aliases`），
/// 与 frontmatter 的 `title` 无关。`slugify` 把 `AD CS` 变成 `ad-cs.md`，于是卡片自己
/// 渲染出的 `[[AD CS]]` 在 Obsidian 里点不开——实测该 vault 354 条 wikilink 断了 63 条（18%），
/// 其中 57 条都是这一个原因。中文标题不含空格所以没事，中英混排和多词英文标题全断。
pub fn note_filename(title: &str) -> String {
    let cleaned: String = title
        .trim()
        .chars()
        .map(|c| if ILLEGAL.contains(&c) { ' ' } else { c })
        .collect();
    // 折叠多余空白；去掉首尾的点（Windows 不允许结尾是点）。
    let collapsed = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = collapsed.trim_matches('.').trim().to_string();
    if trimmed.is_empty() {
        "未命名".to_string()
    } else {
        trimmed
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
    if !note.aliases.is_empty() {
        // Obsidian 用 aliases 解析 [[旧标题]]：合并/改名后旧链接不断。
        s.push_str(&format!("aliases: {}\n", render_inline_list(&note.aliases)));
    }
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

/// 默认相对文件名：`{title}.md`（标题原样，仅清洗非法字符）。
pub fn note_path(title: &str) -> String {
    format!("{}.md", note_filename(title))
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
    let abs = vault_dir.join(relative_path);
    // 建到文件所在子目录（relative_path 可能含分类文件夹，如 `web-security/xxx.md`）。
    if let Some(parent) = abs.parent() {
        fs::create_dir_all(parent)?;
    } else {
        fs::create_dir_all(vault_dir)?;
    }
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

/// 列出 vault 内所有 `.md` 的**相对路径**（递归；跳过 `.obsidian` / `.git` 等隐藏目录）。
///
/// 此前只读 vault 根一层——分类子目录时代它等于什么都看不到。
pub fn list_notes(vault_dir: &Path) -> io::Result<Vec<String>> {
    let mut out = Vec::new();
    if !vault_dir.exists() {
        return Ok(out);
    }
    walk(vault_dir, vault_dir, &mut out)?;
    out.sort();
    Ok(out)
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue; // .obsidian / .git / .trash
        }
        if path.is_dir() {
            walk(root, &path, out)?;
        } else if path.extension().and_then(|e| e.to_str()) == Some("md") {
            if let Ok(rel) = path.strip_prefix(root) {
                out.push(rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    Ok(())
}

/// 移动一张卡（改名 / 换文件夹）。目标已存在时报错，不覆盖。
pub fn move_note(vault_dir: &Path, from_rel: &str, to_rel: &str) -> io::Result<()> {
    let from = vault_dir.join(from_rel);
    let to = vault_dir.join(to_rel);
    if from == to {
        return Ok(());
    }
    if to.exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("目标已存在: {to_rel}"),
        ));
    }
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::rename(&from, &to)
}

// ── 版本历史：把 vault 交给 git ───────────────────────────────────────────────

/// 首次使用时初始化 git 仓库（幂等；不是 git 仓库/没装 git 都静默跳过）。
///
/// `write_knowledge_note` 是**整卡覆盖**语义，一次错误合并就可能抹掉已验证内容。
/// 交给 git 之后，风险从"不可逆"降为"可 diff 可回滚"，并顺带得到版本历史与 blame
/// ——这正是维基百科质量的支柱之一。
pub fn git_init_if_needed(vault_dir: &Path) -> bool {
    if !vault_dir.exists() {
        return false;
    }
    if vault_dir.join(".git").exists() {
        return true;
    }
    let ok = std::process::Command::new("git")
        .arg("init")
        .arg("--quiet")
        .current_dir(vault_dir)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if ok {
        let _ = fs::write(
            vault_dir.join(".gitignore"),
            ".obsidian/workspace.json\n.DS_Store\n",
        );
    }
    ok
}

/// 提交当前 vault 状态（best-effort：失败只返回 false，绝不影响写入主流程）。
pub fn git_snapshot(vault_dir: &Path, message: &str) -> bool {
    if !vault_dir.join(".git").exists() {
        return false;
    }
    let add = std::process::Command::new("git")
        .args(["add", "-A"])
        .current_dir(vault_dir)
        .status();
    if add.map(|s| !s.success()).unwrap_or(true) {
        return false;
    }
    std::process::Command::new("git")
        .args([
            "-c",
            "user.name=Sisyphus",
            "-c",
            "user.email=sisyphus@local",
            "commit",
            "--quiet",
            "--no-gpg-sign",
            "-m",
            message,
        ])
        .current_dir(vault_dir)
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

// ── 小节级编辑（补充式增长的原子操作）────────────────────────────────────────

/// 在正文里插入或替换一个 H2 小节，返回新正文。
///
/// 结晶化要求"同主题多轮对话让同一颗结晶长大（加/精一个 H2 小节）"，但此前工具面只有
/// 整卡覆盖：agent 必须先把整张卡读回来、自己拼超集再写回，读不回来就直接覆盖丢内容。
/// 这个函数把"加一节"变成一次确定性操作。
///
/// `## 关联` 小节由渲染层生成，永远排在最后，不会被这里插到中间。
pub fn upsert_section(body: &str, heading: &str, section_body: &str) -> String {
    let heading_line = format!("## {}", heading.trim());
    let mut sections: Vec<String> = Vec::new();
    let mut preamble = String::new();
    let mut current: Option<String> = None;

    for line in body.lines() {
        if line.trim_start().starts_with("## ") {
            if let Some(sec) = current.take() {
                sections.push(sec);
            }
            current = Some(format!("{line}\n"));
        } else if let Some(sec) = current.as_mut() {
            sec.push_str(line);
            sec.push('\n');
        } else {
            preamble.push_str(line);
            preamble.push('\n');
        }
    }
    if let Some(sec) = current.take() {
        sections.push(sec);
    }

    let new_section = format!("{heading_line}\n{}\n", section_body.trim());
    let mut replaced = false;
    for sec in sections.iter_mut() {
        let first = sec.lines().next().unwrap_or("").trim();
        if first == heading_line {
            *sec = new_section.clone();
            replaced = true;
            break;
        }
    }
    if !replaced {
        // 新小节插在「待确认」/「关联」之前（它们按约定收尾）。
        let tail_idx = sections.iter().position(|s| {
            let h = s.lines().next().unwrap_or("").trim();
            h == "## 待确认" || h == "## 关联"
        });
        match tail_idx {
            Some(i) => sections.insert(i, new_section),
            None => sections.push(new_section),
        }
    }

    let mut out = preamble.trim_end().to_string();
    for sec in sections {
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        out.push_str(sec.trim_end());
    }
    out.push('\n');
    out
}

/// 解析一段 markdown（frontmatter + 正文 wikilinks）。
pub fn parse_note(content: &str) -> ParsedNote {
    let lines: Vec<&str> = content.lines().collect();
    let mut title = None;
    let mut tags = Vec::new();
    let mut sources = Vec::new();
    let mut aliases = Vec::new();
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
            } else if let Some(rest) = line.strip_prefix("aliases:") {
                aliases = parse_inline_list(rest.trim());
            }
            i += 1;
        }
        body_start = if i < lines.len() { i + 1 } else { i };
    }

    // 跳过渲染层生成的 `# 标题` 行，body 只保留正文小节（与写入时对称）。
    let mut body_lines = &lines[body_start.min(lines.len())..];
    while let Some(first) = body_lines.first() {
        if first.trim().is_empty() {
            body_lines = &body_lines[1..];
        } else if first.starts_with("# ") {
            body_lines = &body_lines[1..];
            break;
        } else {
            break;
        }
    }

    let body = body_lines.join("\n").trim().to_string();
    let links = extract_wikilinks(&body);
    ParsedNote {
        title,
        tags,
        sources,
        aliases,
        body,
        links,
    }
}

/// 提取正文里的 `[[目标]]` / `[[目标|别名]]`（取目标部分）。
///
/// **跳过代码区**：围栏代码块（```）与行内代码（`` ` ``）里的 `[[…]]` 是**示意写法**，
/// 不是真链接。文档里写 `` `[[链接]]` `` 讲解语法很常见，把它当成真链接会在体检报告里
/// 冒出一个永远补不上的假红链（实测该 vault 的「知识库地图」就中过这一枪）。
pub fn extract_wikilinks(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut in_fence = false;
    for line in body.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        extract_line_links(line, &mut out);
    }
    out
}

/// 单行提取，按反引号切段：奇数段（反引号之间）是行内代码，跳过。
fn extract_line_links(line: &str, out: &mut Vec<String>) {
    for (i, segment) in line.split('`').enumerate() {
        if i % 2 == 1 {
            continue; // 行内代码
        }
        let mut rest = segment;
        while let Some(start) = rest.find("[[") {
            let after = &rest[start + 2..];
            match after.find("]]") {
                Some(end) => {
                    let inner = &after[..end];
                    let target = inner.split('|').next().unwrap_or(inner).trim();
                    if !target.is_empty() {
                        out.push(target.to_string());
                    }
                    rest = &after[end + 2..];
                }
                None => break,
            }
        }
    }
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
