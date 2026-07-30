//! Phase 1.3 第二大脑：**约束层**的端到端验证。
//!
//! 这个文件测的不是"函数能跑"，而是**图的不变量**：
//! - 写入器不制造断链（文件名 = 标题，Obsidian 点得开）
//! - 一个概念一张卡（标题唯一 + 换 folder 是移动而不是复制）
//! - 分类/类型/可靠性/关联必填，且高可靠性档必须有证据
//! - 补充式增长是原子操作（加小节不丢别的小节）
//! - 合并留重定向、改入链、不留断链
//! - 体检报告能真的发现问题（这是 rebalance/defragment 的输入）

use sisyphus_core::vault::VaultNote;
use sisyphus_core::{db, kb_doctor, knowledge, vault};
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn unique_suffix() -> String {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("{}_{}_{}", std::process::id(), nanos, n)
}

fn temp_vault() -> std::path::PathBuf {
    let p = std::env::temp_dir().join(format!("sis_vault_{}", unique_suffix()));
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn temp_db() -> rusqlite::Connection {
    let path = std::env::temp_dir().join(format!("sis_vdb_{}.db", unique_suffix()));
    let _ = std::fs::remove_file(&path);
    db::open(path.to_str().unwrap()).unwrap()
}

/// 合法卡片：一个 type + 一个可靠性 + 至少一条关联。
fn note(title: &str, body: &str, links: &[&str]) -> VaultNote {
    VaultNote {
        title: title.into(),
        body: body.into(),
        tags: vec!["web-security".into(), "theory".into(), "待确认".into()],
        links: links.iter().map(|s| s.to_string()).collect(),
        sources: vec![],
        aliases: vec![],
    }
}

fn write(
    conn: &rusqlite::Connection,
    vault_dir: &std::path::Path,
    folder: &str,
    n: &VaultNote,
) -> Result<knowledge::WriteOutcome, String> {
    knowledge::write_knowledge_note(conn, vault_dir, "u", "d", Some(folder), n)
}

// ── 断链：写入器不许自己制造 ────────────────────────────────────────────────

#[test]
fn filename_equals_title_so_wikilinks_resolve_in_obsidian() {
    // Obsidian 按**文件名**解析 [[X]]，不看 frontmatter 的 title。
    // 旧实现 slugify 把 "AD CS" 写成 ad-cs.md，于是卡片自己渲染的 [[AD CS]] 点不开。
    assert_eq!(vault::note_path("AD CS"), "AD CS.md");
    assert_eq!(vault::note_path("SQL 注入"), "SQL 注入.md");
    assert_eq!(vault::note_path("身份体系"), "身份体系.md");
    // 文件系统非法字符才替换。
    assert_eq!(vault::note_path("a/b:c"), "a b c.md");
    assert_eq!(vault::note_path("   "), "未命名.md");
}

#[test]
fn round_trip_preserves_title_tags_links_and_aliases() {
    let vault_dir = temp_vault();
    let n = VaultNote {
        title: "AD CS".into(),
        body: "> [!info] 一句话定义\n> 微软证书服务。\n\n## 原理\n略".into(),
        tags: vec!["identity".into(), "theory".into(), "待确认".into()],
        links: vec!["身份体系".into()],
        sources: vec!["https://example.com".into()],
        aliases: vec!["ADCS".into()],
    };
    let res = vault::write_note(&vault_dir, &n).unwrap();
    assert_eq!(res.relative_path, "AD CS.md");
    let parsed = vault::read_note(&vault_dir, &res.relative_path).unwrap();
    assert_eq!(parsed.title.as_deref(), Some("AD CS"));
    assert_eq!(parsed.aliases, vec!["ADCS".to_string()]);
    assert!(parsed.tags.contains(&"theory".to_string()));
    assert!(parsed.links.contains(&"身份体系".to_string()));
    // body 不再包含渲染层生成的 `# 标题` 行（写读对称，补充式增长才不会越写越多标题）。
    assert!(!parsed.body.starts_with("# "));
}

#[test]
fn list_notes_is_recursive() {
    let vault_dir = temp_vault();
    let n = note("SQL 注入", "## 原理\n略", &["输入校验"]);
    vault::write_note_at(&vault_dir, "kb/web-security/SQL 注入.md", &n).unwrap();
    std::fs::create_dir_all(vault_dir.join(".obsidian")).unwrap();
    std::fs::write(vault_dir.join(".obsidian/ignore.md"), "x").unwrap();
    let files = vault::list_notes(&vault_dir).unwrap();
    assert_eq!(files, vec!["kb/web-security/SQL 注入.md".to_string()]);
}

// ── 约束：能被拒绝的规则才是规则 ────────────────────────────────────────────

#[test]
fn rejects_folder_outside_kb() {
    let (conn, vault_dir) = (temp_db(), temp_vault());
    let n = note("SQL 注入", "## 原理", &["输入校验"]);
    let err = write(&conn, &vault_dir, "web-security", &n).unwrap_err();
    assert!(err.contains("kb/"), "{err}");
    // 根目录同样不行——"想不到放哪就丢根目录"正是上一版不成体系的病根。
    assert!(write(&conn, &vault_dir, "", &n).is_err());
}

#[test]
fn rejects_missing_type_or_reliability_tag() {
    let (conn, vault_dir) = (temp_db(), temp_vault());
    let mut n = note("SQL 注入", "## 原理", &["输入校验"]);
    n.tags = vec!["web-security".into()];
    assert!(write(&conn, &vault_dir, "kb/web-security", &n)
        .unwrap_err()
        .contains("文章类型"));

    n.tags = vec!["theory".into()];
    assert!(write(&conn, &vault_dir, "kb/web-security", &n)
        .unwrap_err()
        .contains("可靠性"));

    // 两个类型也不行（一张卡恰好一个 type）。
    n.tags = vec!["theory".into(), "news".into(), "待确认".into()];
    assert!(write(&conn, &vault_dir, "kb/web-security", &n).is_err());

    // tags 里写 `#` 前缀是常见笔误（实测 vault 里有一张）。
    n.tags = vec!["theory".into(), "#待确认".into()];
    assert!(write(&conn, &vault_dir, "kb/web-security", &n).is_err());
}

#[test]
fn high_reliability_requires_evidence() {
    let (conn, vault_dir) = (temp_db(), temp_vault());
    let mut n = note("SQL 注入", "## 原理", &["输入校验"]);
    n.tags = vec!["web-security".into(), "theory".into(), "已复现".into()];
    let err = write(&conn, &vault_dir, "kb/web-security", &n).unwrap_err();
    assert!(err.contains("sources"), "{err}");
    // 有证据就放行——"模型自身知识只配待确认"从提示变成约束。
    n.sources = vec!["sources/复现记录.md".into()];
    assert!(write(&conn, &vault_dir, "kb/web-security", &n).is_ok());
}

#[test]
fn rejects_long_title_and_missing_links() {
    let (conn, vault_dir) = (temp_db(), temp_vault());
    let long = note(
        "米哈游 YDY 计算平台能力与安全边界解析知识体系核心输出目录",
        "## 原理",
        &["输入校验"],
    );
    assert!(write(&conn, &vault_dir, "kb/web-security", &long)
        .unwrap_err()
        .contains("标题过长"));

    let lonely = note("SQL 注入", "## 原理", &[]);
    assert!(write(&conn, &vault_dir, "kb/web-security", &lonely)
        .unwrap_err()
        .contains("links"));

    // MOC（目录页）允许没有出链。
    let mut moc = note("安全基建", "领域总图", &[]);
    moc.tags = vec!["moc".into(), "待确认".into()];
    assert!(write(&conn, &vault_dir, "kb/network-infra", &moc).is_ok());
}

// ── 一个概念一张卡 ──────────────────────────────────────────────────────────

#[test]
fn same_title_in_new_folder_moves_instead_of_duplicating() {
    let (conn, vault_dir) = (temp_db(), temp_vault());
    let n = note("Certighost", "## 摘要\n初版", &["AD CS"]);
    let first = write(&conn, &vault_dir, "kb/network-infra", &n).unwrap();
    assert!(!first.updated);

    // 同一个标题、换了 folder：应当移动，而不是产生第二份（实测事故就是这样来的）。
    let n2 = note("Certighost", "## 摘要\n修订", &["AD CS"]);
    let second = write(&conn, &vault_dir, "kb/network-infra/identity-system", &n2).unwrap();
    assert!(second.updated, "同标题应视为更新");
    assert_eq!(
        second.path,
        "kb/network-infra/identity-system/Certighost.md"
    );
    assert!(!vault_dir.join(&first.path).exists(), "旧路径应已移走");

    let alive = knowledge::list_knowledge(&conn).unwrap();
    let count = alive.iter().filter(|k| k.title == "Certighost").count();
    assert_eq!(count, 1, "同标题只能有一张存活的卡");
}

#[test]
fn unresolved_links_are_reported_as_wanted() {
    let (conn, vault_dir) = (temp_db(), temp_vault());
    let n = note("SQL 注入", "## 原理\n参见 [[参数化查询]]", &["输入校验"]);
    let res = write(&conn, &vault_dir, "kb/web-security", &n).unwrap();
    // 两个目标都还不存在 → 红链（= 知识缺口 = 主动调研队列的输入）。
    assert!(res.wanted_links.contains(&"输入校验".to_string()));
    assert!(res.wanted_links.contains(&"参数化查询".to_string()));

    let wanted = kb_doctor::wanted(&conn, 1).unwrap();
    assert!(wanted.iter().any(|w| w.title == "参数化查询"));

    // 补上其中一张后，红链自动消失。
    write(
        &conn,
        &vault_dir,
        "kb/web-security",
        &note("参数化查询", "## 原理", &["SQL 注入"]),
    )
    .unwrap();
    knowledge::reindex_vault(&conn, &vault_dir).unwrap();
    let wanted2 = kb_doctor::wanted(&conn, 1).unwrap();
    assert!(!wanted2.iter().any(|w| w.title == "参数化查询"));
}

// ── 补充式增长 ──────────────────────────────────────────────────────────────

#[test]
fn upsert_section_adds_then_refines_without_losing_others() {
    let body = "> [!info] 定义\n> x\n\n## A\n第一节\n\n## 待确认\n- [ ] q";
    let grown = vault::upsert_section(body, "B", "第二节");
    assert!(grown.contains("## A"), "原小节不能丢");
    assert!(grown.contains("## B"));
    // 新小节插在「待确认」之前（约定收尾节）。
    let pos_b = grown.find("## B").unwrap();
    let pos_tail = grown.find("## 待确认").unwrap();
    assert!(pos_b < pos_tail);

    let refined = vault::upsert_section(&grown, "B", "第二节·精化");
    assert!(refined.contains("第二节·精化"));
    assert_eq!(refined.matches("## B").count(), 1, "同名小节应替换而非重复");
    assert!(refined.contains("## A"));
}

#[test]
fn append_section_is_atomic_on_a_real_card() {
    let (conn, vault_dir) = (temp_db(), temp_vault());
    write(
        &conn,
        &vault_dir,
        "kb/personal",
        &VaultNote {
            title: "雅思备考".into(),
            body: "> [!info] 定义\n> 四科 0–9 分\n\n## 考试结构\n听说读写".into(),
            tags: vec!["personal".into(), "待确认".into()],
            links: vec!["英语学习".into()],
            sources: vec![],
            aliases: vec![],
        },
    )
    .unwrap();

    // 第二轮对话：加一节，而不是另开一张「雅思分数水平对照」碎卡。
    knowledge::append_section(
        &conn,
        &vault_dir,
        "u",
        "d",
        "雅思备考",
        "分数水平对照",
        "6.5 对应…",
        &[],
        &[],
    )
    .unwrap();

    let (path, parsed) = knowledge::read_knowledge_note(&conn, &vault_dir, "雅思备考").unwrap();
    assert_eq!(path, "kb/personal/雅思备考.md");
    assert!(parsed.body.contains("## 考试结构"), "旧小节必须还在");
    assert!(parsed.body.contains("## 分数水平对照"));
    assert!(parsed.body.contains("6.5 对应"));
    // 关联小节仍由渲染层生成，且没有重复。
    let raw = std::fs::read_to_string(vault_dir.join(&path)).unwrap();
    assert_eq!(raw.matches("## 关联").count(), 1);
    assert_eq!(knowledge::list_knowledge(&conn).unwrap().len(), 1);
}

// ── 结晶化归并 ──────────────────────────────────────────────────────────────

#[test]
fn merge_notes_leaves_no_dangling_links() {
    let (conn, vault_dir) = (temp_db(), temp_vault());
    let mk = |title: &str, body: &str| VaultNote {
        title: title.into(),
        body: body.into(),
        tags: vec!["personal".into(), "待确认".into()],
        links: vec!["英语学习".into()],
        sources: vec![],
        aliases: vec![],
    };
    write(&conn, &vault_dir, "kb/personal", &mk("雅思备考", "## 结构\nx")).unwrap();
    write(&conn, &vault_dir, "kb/personal", &mk("雅思分数", "## 分数\ny")).unwrap();
    // 另一张卡引用了将被合并掉的碎卡。
    write(
        &conn,
        &vault_dir,
        "kb/personal",
        &mk("留学计划", "## 语言要求\n见 [[雅思分数]]"),
    )
    .unwrap();

    let out = knowledge::merge_notes(
        &conn,
        &vault_dir,
        "u",
        "d",
        &["雅思分数".to_string()],
        "雅思备考",
    )
    .unwrap();
    assert_eq!(out.merged, vec!["雅思分数".to_string()]);
    assert!(out.rewritten_files >= 1, "入链应被改写");

    // 引用已指向合并后的结晶。
    let plan = std::fs::read_to_string(vault_dir.join("kb/personal/留学计划.md")).unwrap();
    assert!(plan.contains("[[雅思备考]]"));
    assert!(!plan.contains("[[雅思分数]]"));
    // 目标卡登记了别名 → Obsidian 侧旧链接也仍可解析。
    let (_, target) = knowledge::read_knowledge_note(&conn, &vault_dir, "雅思备考").unwrap();
    assert!(target.aliases.contains(&"雅思分数".to_string()));
    // 别名可用于检索定位。
    assert!(knowledge::resolve_title(&conn, "雅思分数")
        .unwrap()
        .is_some());
    assert!(!vault_dir.join("kb/personal/雅思分数.md").exists());
}

// ── 检索 ────────────────────────────────────────────────────────────────────

#[test]
fn search_matches_body_not_just_title() {
    let (conn, vault_dir) = (temp_db(), temp_vault());
    write(
        &conn,
        &vault_dir,
        "kb/web-security",
        &note("SQL 注入", "## 防御\n首选参数化查询，绑定变量。", &["输入校验"]),
    )
    .unwrap();
    // "参数化查询" 只出现在正文里——旧实现只 LIKE 标题/标签/路径，查不到，于是 agent 只能新建。
    let hits = knowledge::search_knowledge(&conn, "参数化查询").unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].title, "SQL 注入");
    assert!(hits[0].excerpt.contains("参数化"));
}

// ── 体检报告 ────────────────────────────────────────────────────────────────

#[test]
fn doctor_finds_broken_links_orphans_and_oversized_folders() {
    let (conn, vault_dir) = (temp_db(), temp_vault());
    // 母节点 + 一张指向不存在卡的卡
    let mut moc = note("安全基建", "## 主线\n- [[SQL 注入]]", &[]);
    moc.tags = vec!["moc".into(), "待确认".into()];
    write(&conn, &vault_dir, "kb/network-infra", &moc).unwrap();
    write(
        &conn,
        &vault_dir,
        "kb/web-security",
        &note("SQL 注入", "## 原理\n见 [[不存在的卡]]", &["安全基建"]),
    )
    .unwrap();
    // 撑爆一个领域（>12 张触发拆分建议）
    for i in 0..13 {
        let title = format!("话题{i}");
        write(
            &conn,
            &vault_dir,
            "kb/ai-redteam",
            &note(&title, "## 内容\nx", &["安全基建"]),
        )
        .unwrap();
    }
    knowledge::reindex_vault(&conn, &vault_dir).unwrap();

    let report = kb_doctor::doctor(&conn, Some(&vault_dir)).unwrap();
    assert!(report
        .broken_links
        .iter()
        .any(|b| b.to_title == "不存在的卡"));
    assert!(report.broken_ratio > 0.0);
    assert!(
        report
            .folders
            .iter()
            .any(|f| f.folder == "kb/ai-redteam" && f.advice == "split"),
        "13 张卡的领域应给出拆分建议"
    );
    // 「话题0..12」共享前缀 → 碎卡簇候选
    assert!(report
        .fragment_clusters
        .iter()
        .any(|c| c.folder == "kb/ai-redteam"));
    // 没人链接到「话题N」→ 孤儿
    assert!(report.orphans.iter().any(|o| o.title.starts_with("话题")));
    assert!(!report.summary().is_empty());
}

#[test]
fn doctor_flags_stray_files_and_index_drift() {
    let (conn, vault_dir) = (temp_db(), temp_vault());
    // vault 根散落文件（不在 kb/ 也不在 sources/）
    std::fs::write(vault_dir.join("随手记.md"), "# 随手记\n").unwrap();
    // 领域 MOC 没列到新卡 → 目录漂移
    std::fs::create_dir_all(vault_dir.join("kb/web-security")).unwrap();
    // 领域枢纽：hub 是角色标记（文件名随意，这里沿用历史的 index.md 验证兼容）。
    std::fs::write(
        vault_dir.join("kb/web-security/index.md"),
        "---\ntitle: \"目录\"\ntags: [moc, hub]\n---\n\n# 目录\n\n## 卡片\n- [[SQL 注入]]\n",
    )
    .unwrap();
    std::fs::write(
        vault_dir.join("kb/web-security/SQL 注入.md"),
        "---\ntitle: \"SQL 注入\"\ntags: [theory, 待确认]\n---\n\n# SQL 注入\n\n## 原理\nx\n",
    )
    .unwrap();
    std::fs::write(
        vault_dir.join("kb/web-security/输入校验.md"),
        "---\ntitle: \"输入校验\"\ntags: [theory, 待确认]\n---\n\n# 输入校验\n\n## 原理\n见 [[SQL 注入]]\n",
    )
    .unwrap();

    let report0 = knowledge::reindex_vault(&conn, &vault_dir).unwrap();
    assert_eq!(report0.scanned, 3, "只扫 kb/ 下的卡");

    let report = kb_doctor::doctor(&conn, Some(&vault_dir)).unwrap();
    assert!(report.stray_files.contains(&"随手记.md".to_string()));
    assert!(
        report.index_drift.iter().any(|d| d.title == "输入校验"),
        "MOC 未列到的卡应报目录漂移"
    );
}

#[test]
fn reindex_picks_up_manual_obsidian_edits_and_deletions() {
    let (conn, vault_dir) = (temp_db(), temp_vault());
    write(
        &conn,
        &vault_dir,
        "kb/web-security",
        &note("SQL 注入", "## 原理\n初版", &["输入校验"]),
    )
    .unwrap();
    // 用户在 Obsidian 里手改正文
    let path = vault_dir.join("kb/web-security/SQL 注入.md");
    let content = std::fs::read_to_string(&path).unwrap();
    std::fs::write(&path, content.replace("初版", "用户手改过的内容")).unwrap();
    knowledge::reindex_vault(&conn, &vault_dir).unwrap();
    let hits = knowledge::search_knowledge(&conn, "用户手改过的内容").unwrap();
    assert_eq!(hits.len(), 1, "索引应追上 vault（.md 才是本体）");

    // 用户在 Obsidian 里删了文件 → 索引剪枝
    std::fs::remove_file(&path).unwrap();
    knowledge::reindex_vault(&conn, &vault_dir).unwrap();
    assert!(knowledge::list_knowledge(&conn).unwrap().is_empty());
}

#[test]
fn delete_note_prunes_index_and_leaves_breadcrumb() {
    let (conn, vault_dir) = (temp_db(), temp_vault());
    let res = write(
        &conn,
        &vault_dir,
        "kb/web-security",
        &note("SQL 注入", "## 原理", &["输入校验"]),
    )
    .unwrap();
    let out = knowledge::delete_knowledge_note(&conn, &vault_dir, "u", "d", "SQL 注入").unwrap();
    assert!(out.deleted);
    assert!(!vault_dir.join(&res.path).exists());
    assert!(knowledge::list_knowledge(&conn).unwrap().is_empty());
    let n: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM raw_events WHERE type='knowledge_pruned'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 1);
    // 幂等
    assert!(
        !knowledge::delete_knowledge_note(&conn, &vault_dir, "u", "d", "SQL 注入")
            .unwrap()
            .deleted
    );
}

#[test]
fn rewrite_same_content_is_idempotent() {
    let vault_dir = temp_vault();
    let n = note("幂等", "## 内容\n同样的输入", &["母节点"]);
    let a = vault::write_note(&vault_dir, &n).unwrap();
    let b = vault::write_note(&vault_dir, &n).unwrap();
    assert_eq!(a.content_hash, b.content_hash);
}

// ── 原始材料：就地存放 + 元信息隔离 + 单向连接 ──────────────────────────────

#[test]
fn source_lives_in_topic_folder_and_is_not_published() {
    let (conn, vault_dir) = (temp_db(), temp_vault());
    let out = sisyphus_core::sources::save_source(
        &conn,
        &vault_dir,
        "u",
        "d",
        "kb/network-infra",
        "OWASP SQLi 备忘单",
        "原文正文……",
        Some("https://owasp.org/x"),
        Some("article"),
    )
    .unwrap();
    // 就在话题夹里，不再有独立的 sources/ 目录。
    assert_eq!(out.path, "kb/network-infra/OWASP SQLi 备忘单.md");
    let raw = std::fs::read_to_string(vault_dir.join(&out.path)).unwrap();
    assert!(raw.contains("tags: [source, 原始材料]"), "类型标签就是隔离标记");
    assert!(raw.contains("publish: false"), "逐字原文不外发，靠 frontmatter 排除");
    assert!(raw.contains("url: https://owasp.org/x"));
    // 图谱里是叶子：不生成 `## 关联`。
    assert!(!raw.contains("## 关联"));
    // 已登记进索引（否则又变成"没人知道存在"的死角）。
    assert!(knowledge::list_knowledge(&conn)
        .unwrap()
        .iter()
        .any(|n| n.title == "OWASP SQLi 备忘单" && n.folder == "kb/network-infra"));
}

#[test]
fn source_requires_provenance_and_rejects_directory_snapshots() {
    let (conn, vault_dir) = (temp_db(), temp_vault());
    let save = |folder: &str, title: &str, url: Option<&str>, st: Option<&str>| {
        sisyphus_core::sources::save_source(
            &conn, &vault_dir, "u", "d", folder, title, "正文", url, st,
        )
    };
    // 外部原文没有 url → 无法溯源，拒绝。
    assert!(save("kb/personal", "来路不明", None, Some("article"))
        .unwrap_err()
        .contains("url"));
    // 本人自撰的第一方材料可以无 url。
    assert!(save("kb/personal", "我的调研笔记", None, Some("first-party")).is_ok());
    // 目录/索引类快照不该逐字复制（第一方 KM 只链接不复制）。
    assert!(save(
        "kb/personal",
        "KM 空间目录快照",
        Some("https://km.example.com/space/1"),
        Some("km-space-index")
    )
    .unwrap_err()
    .contains("目录"));
    // folder 必须在 kb/ 下。
    assert!(save("sources", "跑偏了", Some("https://x"), None).is_err());
}

#[test]
fn source_cards_must_not_link_out() {
    let (conn, vault_dir) = (temp_db(), temp_vault());
    let mut n = note("某原文", "## 正文\nx", &["SQL 注入"]);
    n.tags = vec!["source".into(), "原始材料".into()];
    n.sources = vec!["https://x".into()];
    let err = write(&conn, &vault_dir, "kb/web-security", &n).unwrap_err();
    assert!(err.contains("单向"), "{err}");

    // 无出链、有来源 → 通过。
    let mut ok = note("某原文", "## 正文\nx", &[]);
    ok.tags = vec!["source".into(), "原始材料".into()];
    ok.sources = vec!["https://x".into()];
    assert!(write(&conn, &vault_dir, "kb/web-security", &ok).is_ok());
}

#[test]
fn doctor_flags_unreferenced_and_outlinking_sources() {
    let (conn, vault_dir) = (temp_db(), temp_vault());
    sisyphus_core::sources::save_source(
        &conn, &vault_dir, "u", "d", "kb/personal", "没人引用的原文", "正文",
        Some("https://x"), Some("article"),
    )
    .unwrap();
    let report = kb_doctor::doctor(&conn, Some(&vault_dir)).unwrap();
    assert!(
        report
            .unreferenced_sources
            .iter()
            .any(|i| i.title == "没人引用的原文"),
        "存了但没人指向它 → 必须报出来"
    );
    // 原始材料不该被算进"孤儿卡"或"无出链"（它们是另一套规则）。
    assert!(!report.orphans.iter().any(|i| i.title == "没人引用的原文"));
    assert!(!report.dead_ends.iter().any(|i| i.title == "没人引用的原文"));
}

// ── 领域枢纽：图谱层级 ──────────────────────────────────────────────────────

#[test]
fn refresh_mocs_builds_spine_and_lists_cards_deterministically() {
    let (conn, vault_dir) = (temp_db(), temp_vault());
    let moc = |title: &str| VaultNote {
        title: title.into(),
        body: "本领域范围的一句话说明。".into(),
        // hub = 领域枢纽的**角色**标记；moc 只是类型（一个领域可以有多张 moc 卡）。
        tags: vec!["moc".into(), "hub".into(), "待确认".into()],
        links: vec![],
        sources: vec![],
        aliases: vec![],
    };
    // 父领域枢纽 + 子领域枢纽（**不叫 index**，图谱节点标签才有意义）
    write(&conn, &vault_dir, "kb/network-infra", &moc("网络与基础设施")).unwrap();
    write(
        &conn,
        &vault_dir,
        "kb/network-infra/identity-system",
        &moc("身份体系地图"),
    )
    .unwrap();
    // 子领域下两张卡 + 一份原始材料
    write(
        &conn,
        &vault_dir,
        "kb/network-infra/identity-system",
        &note("Kerberos", "## 原理\nx", &["身份体系地图"]),
    )
    .unwrap();
    write(
        &conn,
        &vault_dir,
        "kb/network-infra/identity-system",
        &note("LDAP", "## 原理\nx", &["身份体系地图"]),
    )
    .unwrap();
    sisyphus_core::sources::save_source(
        &conn,
        &vault_dir,
        "u",
        "d",
        "kb/network-infra/identity-system",
        "AD CS 白皮书",
        "原文",
        Some("https://x"),
        Some("article"),
    )
    .unwrap();

    let report = knowledge::refresh_mocs(&conn, &vault_dir).unwrap();
    assert!(report.cards_listed >= 2);
    assert_eq!(report.sources_listed, 1);

    let child = std::fs::read_to_string(
        vault_dir.join("kb/network-infra/identity-system/身份体系地图.md"),
    )
    .unwrap();
    // 脊椎：子枢纽指向上级枢纽 —— 这才让图谱出现树干
    assert!(child.contains("上级：[[网络与基础设施]]"));
    // 卡片清单齐全（手写目录必然漂移，这里由 Core 生成）
    assert!(child.contains("[[Kerberos]]") && child.contains("[[LDAP]]"));
    // 原始材料单独一节：MOC → 原文的单向连接
    assert!(child.contains("### 原始材料（1）") && child.contains("[[AD CS 白皮书]]"));
    // 领域叙述（marker 之外）原样保留
    assert!(child.contains("本领域范围的一句话说明。"));

    let parent = std::fs::read_to_string(vault_dir.join("kb/network-infra/网络与基础设施.md")).unwrap();
    assert!(parent.contains("### 子领域") && parent.contains("[[身份体系地图]]"));

    // 幂等：再跑一次不产生变化
    let again = knowledge::refresh_mocs(&conn, &vault_dir).unwrap();
    assert!(again.refreshed.is_empty(), "无变化时不该重写文件");

    // 目录漂移归零（此前该 vault 有 24 条）
    let doctor = kb_doctor::doctor(&conn, Some(&vault_dir)).unwrap();
    assert!(doctor.index_drift.is_empty(), "{:?}", doctor.index_drift);
}

#[test]
fn refresh_mocs_preserves_hand_written_narrative_outside_markers() {
    let (conn, vault_dir) = (temp_db(), temp_vault());
    let mut m = VaultNote {
        title: "Web 安全".into(),
        body: "## 领域范围\n应用层攻防。\n\n## 目录\n<!-- kb:auto begin -->\n旧的过时列表\n<!-- kb:auto end -->\n\n## 后续可拆\n- SSRF".into(),
        tags: vec!["moc".into(), "hub".into(), "待确认".into()],
        links: vec![],
        sources: vec![],
        aliases: vec![],
    };
    write(&conn, &vault_dir, "kb/web-security", &m).unwrap();
    m.title = "Web 安全".into();
    write(
        &conn,
        &vault_dir,
        "kb/web-security",
        &note("SQL 注入", "## 原理\nx", &["Web 安全"]),
    )
    .unwrap();

    knowledge::refresh_mocs(&conn, &vault_dir).unwrap();
    let out = std::fs::read_to_string(vault_dir.join("kb/web-security/Web 安全.md")).unwrap();
    assert!(out.contains("## 领域范围") && out.contains("应用层攻防。"));
    assert!(out.contains("## 后续可拆") && out.contains("- SSRF"));
    assert!(out.contains("[[SQL 注入]]"));
    assert!(!out.contains("旧的过时列表"), "自动区块内的旧内容应被替换");
}

#[test]
fn wikilink_extraction_skips_code_spans_and_fences() {
    // 文档里讲语法时会写 `[[链接]]`——那是示意，不是真链接。
    let body = "说明：用 `[[链接]]` 关联卡片。\n\n```md\n[[代码块里的示例]]\n```\n\n真链接：[[SQL 注入]] 与 [[输入校验|校验]]。";
    let links = vault::extract_wikilinks(body);
    assert_eq!(links, vec!["SQL 注入".to_string(), "输入校验".to_string()]);
    assert!(!links.iter().any(|l| l == "链接"), "行内代码里的示意写法不算链接");
    assert!(!links.iter().any(|l| l.contains("代码块")), "围栏代码块内不算链接");
}

#[test]
fn ambiguous_hub_is_reported_not_guessed() {
    let (conn, vault_dir) = (temp_db(), temp_vault());
    let hub = |title: &str, tags: Vec<String>| VaultNote {
        title: title.into(),
        body: "说明".into(),
        tags,
        links: vec![],
        sources: vec![],
        aliases: vec![],
    };
    // 一个领域里两张 moc 卡：领域枢纽 + 概念地图。只有带 hub 的那张是枢纽。
    write(
        &conn,
        &vault_dir,
        "kb/network-infra",
        &hub("网络与基础设施", vec!["moc".into(), "hub".into(), "待确认".into()]),
    )
    .unwrap();
    write(
        &conn,
        &vault_dir,
        "kb/network-infra",
        &hub("安全基建", vec!["moc".into(), "待确认".into()]),
    )
    .unwrap();
    write(
        &conn,
        &vault_dir,
        "kb/network-infra",
        &note("VPC", "## 原理\nx", &["网络与基础设施"]),
    )
    .unwrap();

    knowledge::refresh_mocs(&conn, &vault_dir).unwrap();
    // 自动目录必须写进真正的枢纽，而不是概念地图。
    let real = std::fs::read_to_string(vault_dir.join("kb/network-infra/网络与基础设施.md")).unwrap();
    assert!(real.contains("[[VPC]]"), "枢纽应列出本领域卡片");
    let concept_map = std::fs::read_to_string(vault_dir.join("kb/network-infra/安全基建.md")).unwrap();
    assert!(
        !concept_map.contains("kb:auto"),
        "非枢纽的 moc 卡不该被写入自动区块"
    );

    // 领域没有唯一枢纽时，体检要报出来（图谱里这一支会没有树干），而不是随便猜一张。
    let (conn2, vault2) = (temp_db(), temp_vault());
    write(
        &conn2,
        &vault2,
        "kb/web-security",
        &note("SQL 注入", "## 原理\nx", &["输入校验"]),
    )
    .unwrap();
    knowledge::reindex_vault(&conn2, &vault2).unwrap();
    let report = kb_doctor::doctor(&conn2, Some(&vault2)).unwrap();
    assert!(
        report.index_drift.iter().any(|i| i.detail.contains("hub")),
        "缺枢纽的领域必须被报告：{:?}",
        report.index_drift
    );
}
