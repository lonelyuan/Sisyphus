//! Phase 1.3 第二大脑：vault markdown 读写 + knowledge_notes 索引 + 溯源事件。
//! 纯 core，无需 Codex/GUI。

use sisyphus_core::vault::VaultNote;
use sisyphus_core::{db, knowledge, vault};
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

fn sample() -> VaultNote {
    VaultNote {
        title: "AI 安全".into(),
        body: "AI 安全关注模型被攻击与滥用的风险。".into(),
        tags: vec!["security".into(), "ai".into()],
        links: vec!["AI Infra".into(), "威胁建模".into()],
        sources: vec!["https://example.com/ai-security".into()],
    }
}

#[test]
fn write_read_roundtrip() {
    let dir = temp_vault();
    let res = vault::write_note(&dir, &sample()).unwrap();
    assert_eq!(res.relative_path, "ai-安全.md");

    let parsed = vault::read_note(&dir, &res.relative_path).unwrap();
    assert_eq!(parsed.title.as_deref(), Some("AI 安全"));
    assert_eq!(parsed.tags, vec!["security", "ai"]);
    assert_eq!(parsed.sources, vec!["https://example.com/ai-security"]);
    assert!(parsed.links.contains(&"AI Infra".to_string()));
    assert!(parsed.links.contains(&"威胁建模".to_string()));
    assert!(parsed.body.contains("AI 安全关注"));
}

#[test]
fn rewrite_same_content_is_idempotent() {
    let dir = temp_vault();
    let h1 = vault::write_note(&dir, &sample()).unwrap().content_hash;
    let h2 = vault::write_note(&dir, &sample()).unwrap().content_hash;
    assert_eq!(h1, h2, "同内容重写 content_hash 应稳定");

    let mut n = sample();
    n.body = "改了正文".into();
    let h3 = vault::write_note(&dir, &n).unwrap().content_hash;
    assert_ne!(h1, h3, "改内容后 hash 应变化");
}

#[test]
fn slugify_handles_cjk_and_symbols() {
    assert_eq!(vault::slugify("AI 安全"), "ai-安全");
    assert_eq!(vault::slugify("Hello, World!"), "hello-world");
    assert_eq!(vault::slugify("   "), "note");
}

#[test]
fn write_knowledge_note_creates_md_index_and_breadcrumb() {
    let dir = temp_vault();
    let conn = temp_db();

    let out = knowledge::write_knowledge_note(&conn, &dir, "local-user", "test", None, &sample()).unwrap();
    assert!(!out.updated, "首次写应为新建");
    assert_eq!(out.path, "ai-安全.md");
    assert!(dir.join("ai-安全.md").exists(), ".md 应落到 vault");

    // 索引行
    let all = knowledge::list_knowledge(&conn).unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].title, "AI 安全");
    assert_eq!(all[0].tags, vec!["security", "ai"]);

    // 检索
    assert_eq!(knowledge::search_knowledge(&conn, "安全").unwrap().len(), 1);
    assert!(knowledge::search_knowledge(&conn, "security").unwrap().len() == 1);
    assert!(knowledge::search_knowledge(&conn, "不存在xyz").unwrap().is_empty());

    // 溯源事件进 Event log
    let cnt: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM raw_events WHERE type = 'knowledge_ingested'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(cnt, 1, "应有一条 knowledge_ingested 面包屑");

    // 再写同标题 → 更新、不新增行、hash 稳定
    let out2 =
        knowledge::write_knowledge_note(&conn, &dir, "local-user", "test", None, &sample()).unwrap();
    assert!(out2.updated, "同标题重写应为更新");
    assert_eq!(knowledge::list_knowledge(&conn).unwrap().len(), 1, "不应新增索引行");
    assert_eq!(out.content_hash, out2.content_hash);
}

#[test]
fn distinct_titles_same_slug_do_not_clobber() {
    let dir = temp_vault();
    let conn = temp_db();
    let n1 = VaultNote {
        title: "Rust: 所有权".into(),
        body: "第一张：借用检查".into(),
        tags: vec![],
        links: vec![],
        sources: vec![],
    };
    let n2 = VaultNote {
        title: "Rust 所有权".into(), // 与 n1 不同标题，但 slug 相同
        body: "第二张：生命周期".into(),
        tags: vec![],
        links: vec![],
        sources: vec![],
    };
    assert_eq!(
        vault::slugify(&n1.title),
        vault::slugify(&n2.title),
        "前提：两个标题 slug 相同"
    );

    let o1 = knowledge::write_knowledge_note(&conn, &dir, "local-user", "test", None, &n1).unwrap();
    let o2 = knowledge::write_knowledge_note(&conn, &dir, "local-user", "test", None, &n2).unwrap();
    assert!(!o1.updated);
    assert!(!o2.updated, "不同标题应新建（消歧），而非覆盖式更新");
    assert_ne!(o1.path, o2.path, "两个不同标题必须落到不同文件");

    // 两份 .md、两条索引行都在，内容各自保留
    assert!(dir.join(&o1.path).exists());
    assert!(dir.join(&o2.path).exists());
    assert!(std::fs::read_to_string(dir.join(&o1.path)).unwrap().contains("借用检查"));
    assert!(std::fs::read_to_string(dir.join(&o2.path)).unwrap().contains("生命周期"));
    assert_eq!(knowledge::list_knowledge(&conn).unwrap().len(), 2, "两条独立知识都在");

    // 再写 n1 同标题 → 更新原文件，仍 2 行
    let o1b = knowledge::write_knowledge_note(&conn, &dir, "local-user", "test", None, &n1).unwrap();
    assert!(o1b.updated);
    assert_eq!(o1b.path, o1.path);
    assert_eq!(knowledge::list_knowledge(&conn).unwrap().len(), 2);
}

#[test]
fn delete_knowledge_note_removes_md_and_prunes_index() {
    let dir = temp_vault();
    let conn = temp_db();

    // 建两张卡（模拟碎片），合并 defragment 时删其一。
    let out = knowledge::write_knowledge_note(&conn, &dir, "local-user", "test", None, &sample()).unwrap();
    assert!(dir.join(&out.path).exists());
    assert_eq!(knowledge::list_knowledge(&conn).unwrap().len(), 1);

    // 按标题删
    let del = knowledge::delete_knowledge_note(&conn, &dir, "local-user", "test", "AI 安全").unwrap();
    assert!(del.deleted, "应找到并删除");
    assert_eq!(del.path, out.path);
    assert!(!dir.join(&out.path).exists(), "vault .md 应被移除");
    assert!(
        knowledge::list_knowledge(&conn).unwrap().is_empty(),
        "剪枝后不应再列出"
    );
    assert!(
        knowledge::search_knowledge(&conn, "安全").unwrap().is_empty(),
        "剪枝后不应被检索到"
    );

    // 溯源：一条 knowledge_pruned
    let cnt: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM raw_events WHERE type = 'knowledge_pruned'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(cnt, 1, "应有一条 knowledge_pruned 面包屑");

    // 幂等：再删不存在的 → deleted=false，不报错
    let again = knowledge::delete_knowledge_note(&conn, &dir, "local-user", "test", "AI 安全").unwrap();
    assert!(!again.deleted, "已删/不存在应返回 deleted=false");
}
