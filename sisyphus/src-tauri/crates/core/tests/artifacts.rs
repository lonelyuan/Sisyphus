//! Phase 1.2 原声笔记：capture → propose_intents → accept_intent 桥的端到端验证。
//! 纯 core，无需 Codex/GUI。沿用 rule_pipeline.rs 的 temp_db 模式。

use serde_json::json;
use sisyphus_core::{artifacts, capture_text, context, db};
use std::sync::atomic::{AtomicU64, Ordering};

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

/// 进程内唯一的临时库路径：pid + 纳秒 + 原子计数，避免并行测试线程撞同一文件。
static COUNTER: AtomicU64 = AtomicU64::new(0);

fn temp_db() -> rusqlite::Connection {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("sis_art_{}_{}_{}.db", std::process::id(), nanos, n));
    let _ = std::fs::remove_file(&path);
    db::open(path.to_str().unwrap()).unwrap()
}

#[test]
fn capture_is_unprocessed_until_candidate_exists() {
    let conn = temp_db();
    let cap_id = capture_text(&conn, "local-user", "test", "记得周五写周报").unwrap();

    let caps = artifacts::list_captures(&conn, true, 10).unwrap();
    assert!(
        caps.iter().any(|c| c.event_id == cap_id && c.text.contains("周报")),
        "新 capture 应出现在 unprocessed 列表"
    );

    let intent_id = artifacts::insert_intent_candidate(
        &conn,
        &cap_id,
        "task",
        &json!({"title": "写周报", "priority": 1}),
        0.9,
        "agent",
    )
    .unwrap();

    let caps2 = artifacts::list_captures(&conn, true, 10).unwrap();
    assert!(
        !caps2.iter().any(|c| c.event_id == cap_id),
        "已生成候选的 capture 应被 unprocessed 过滤"
    );
    // 但 unprocessed=false 仍能看到
    let all = artifacts::list_captures(&conn, false, 10).unwrap();
    assert!(all.iter().any(|c| c.event_id == cap_id));
    let _ = intent_id;
}

#[test]
fn accept_task_promotes_and_links_back() {
    let conn = temp_db();
    let cap_id = capture_text(&conn, "local-user", "test", "记得周五写周报").unwrap();
    let intent_id = artifacts::insert_intent_candidate(
        &conn,
        &cap_id,
        "task",
        &json!({"title": "写周报", "priority": 1}),
        0.9,
        "agent",
    )
    .unwrap();

    let task_id = artifacts::accept_intent(&conn, &intent_id, None).unwrap();

    let open = artifacts::list_open_tasks(&conn).unwrap();
    let t = open.iter().find(|t| t.id == task_id).expect("task 应已创建");
    assert_eq!(t.title, "写周报");
    assert_eq!(t.status, "todo");
    assert_eq!(t.priority, 1);

    // 候选转 accepted
    let accepted = artifacts::list_intent_candidates(&conn, Some("accepted")).unwrap();
    assert!(accepted.iter().any(|c| c.id == intent_id));

    // 溯源链：task.intent_id / source_event_id 回填
    let (linked_intent, linked_src): (String, String) = conn
        .query_row(
            "SELECT intent_id, source_event_id FROM tasks WHERE id = ?1",
            [&task_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(linked_intent, intent_id);
    assert_eq!(linked_src, cap_id);
}

#[test]
fn accept_with_edits_marks_edited() {
    let conn = temp_db();
    let cap_id = capture_text(&conn, "local-user", "test", "学吉他").unwrap();
    let intent_id = artifacts::insert_intent_candidate(
        &conn,
        &cap_id,
        "task",
        &json!({"title": "学吉他"}),
        0.5,
        "agent",
    )
    .unwrap();

    let task_id = artifacts::accept_intent(
        &conn,
        &intent_id,
        Some(r#"{"title":"每天练吉他 15 分钟","priority":2}"#),
    )
    .unwrap();

    let open = artifacts::list_open_tasks(&conn).unwrap();
    let t = open.iter().find(|t| t.id == task_id).unwrap();
    assert_eq!(t.title, "每天练吉他 15 分钟");
    assert_eq!(t.priority, 2);

    let status: String = conn
        .query_row(
            "SELECT status FROM intent_candidates WHERE id = ?1",
            [&intent_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(status, "edited");
}

#[test]
fn ignore_intent_is_rollback() {
    let conn = temp_db();
    let cap_id = capture_text(&conn, "local-user", "test", "随便记一句").unwrap();
    let intent_id = artifacts::insert_intent_candidate(
        &conn,
        &cap_id,
        "note",
        &json!({"body": "随便记一句"}),
        0.3,
        "agent",
    )
    .unwrap();

    artifacts::ignore_intent(&conn, &intent_id).unwrap();
    let status: String = conn
        .query_row(
            "SELECT status FROM intent_candidates WHERE id = ?1",
            [&intent_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(status, "ignored");

    // 已忽略的候选不能再接受
    assert!(artifacts::accept_intent(&conn, &intent_id, None).is_err());

    // 无 note 落库
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM notes", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn accept_goal_sets_today_goal() {
    let conn = temp_db();
    let cap = capture_text(&conn, "local-user", "test", "今天写完论文第三章").unwrap();
    let iid = artifacts::insert_intent_candidate(
        &conn,
        &cap,
        "goal",
        &json!({"text": "写完论文第三章"}),
        0.9,
        "agent",
    )
    .unwrap();
    artifacts::accept_intent(&conn, &iid, None).unwrap();

    let ctx = context::today_context(&conn, "local-user").unwrap();
    let goal = ctx.goal.expect("今日目标应已设置");
    assert_eq!(goal.raw_text, "写完论文第三章");
}

#[test]
fn material_capture_not_in_inbox() {
    let conn = temp_db();
    let cap = capture_text(&conn, "local-user", "test", "普通记录").unwrap();
    let mat =
        sisyphus_core::ingest::capture_material(&conn, "local-user", "test", "一篇文章素材").unwrap();
    let inbox = artifacts::list_captures(&conn, true, 20).unwrap();
    assert!(inbox.iter().any(|c| c.event_id == cap), "普通 capture 应在收件箱");
    assert!(
        !inbox.iter().any(|c| c.event_id == mat),
        "material 素材不应进意图三分收件箱"
    );
}

#[test]
fn propose_batch_rejects_unknown_capture_and_bad_kind() {
    let conn = temp_db();
    // 悬空 capture_event_id → 拒绝（防止悬空溯源）
    let bad_cap = artifacts::insert_intent_candidates(
        &conn,
        "does-not-exist",
        &[("task".into(), json!({"title": "x"}), 0.5)],
        "agent",
    );
    assert!(bad_cap.is_err(), "不存在的 capture 应被拒绝");

    // 合法 capture + 非法 kind → 拒绝，且不留半成品
    let cap = capture_text(&conn, "local-user", "test", "记一句").unwrap();
    let bad_kind = artifacts::insert_intent_candidates(
        &conn,
        &cap,
        &[("bogus".into(), json!({"x": 1}), 0.5)],
        "agent",
    );
    assert!(bad_kind.is_err(), "非法 kind 应被拒绝");
    let n: i64 = conn
        .query_row("SELECT COUNT(*) FROM intent_candidates", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 0, "校验失败不应留下任何候选");
}

#[test]
fn due_reminders_fire_once() {
    let conn = temp_db();
    let cap = capture_text(&conn, "local-user", "test", "提醒").unwrap();
    let past = now_ms() - 60_000;
    let future = now_ms() + 3_600_000;
    let ip = artifacts::insert_intent_candidate(&conn, &cap, "reminder", &json!({"text":"吃药","remind_at_ms":past}), 0.9, "agent").unwrap();
    artifacts::accept_intent(&conn, &ip, None).unwrap();
    let cf = capture_text(&conn, "local-user", "test", "开会").unwrap();
    let ifu = artifacts::insert_intent_candidate(&conn, &cf, "reminder", &json!({"text":"开会","remind_at_ms":future}), 0.9, "agent").unwrap();
    artifacts::accept_intent(&conn, &ifu, None).unwrap();

    let fired = artifacts::take_due_reminders(&conn, now_ms()).unwrap();
    assert_eq!(fired.len(), 1, "只触发已到期那条");
    assert_eq!(fired[0].text, "吃药");
    assert!(
        artifacts::take_due_reminders(&conn, now_ms()).unwrap().is_empty(),
        "已 fired 不重复触发"
    );
}

#[test]
fn context_surfaces_open_tasks_and_due_reminders() {
    let conn = temp_db();

    // 已到期提醒
    let cap1 = capture_text(&conn, "local-user", "test", "提醒我喝水").unwrap();
    let past = now_ms() - 60_000;
    let i1 = artifacts::insert_intent_candidate(
        &conn,
        &cap1,
        "reminder",
        &json!({"text": "喝水", "remind_at_ms": past}),
        0.8,
        "agent",
    )
    .unwrap();
    artifacts::accept_intent(&conn, &i1, None).unwrap();

    // 未来提醒（不应到期）
    let cap_f = capture_text(&conn, "local-user", "test", "明天提醒开会").unwrap();
    let future = now_ms() + 3_600_000;
    let i_f = artifacts::insert_intent_candidate(
        &conn,
        &cap_f,
        "reminder",
        &json!({"text": "开会", "remind_at_ms": future}),
        0.8,
        "agent",
    )
    .unwrap();
    artifacts::accept_intent(&conn, &i_f, None).unwrap();

    // 未完成任务
    let cap2 = capture_text(&conn, "local-user", "test", "写测试").unwrap();
    let i2 = artifacts::insert_intent_candidate(
        &conn,
        &cap2,
        "task",
        &json!({"title": "写测试"}),
        0.9,
        "agent",
    )
    .unwrap();
    artifacts::accept_intent(&conn, &i2, None).unwrap();

    let ctx = context::today_context(&conn, "local-user").unwrap();
    assert_eq!(ctx.due_reminders.len(), 1, "只有 1 个已到期提醒");
    assert_eq!(ctx.due_reminders[0].text, "喝水");
    assert!(ctx.open_tasks.iter().any(|t| t.title == "写测试"));

    let actions = context::today_actions(&conn).unwrap();
    assert!(actions.iter().any(|a| a == "写测试"), "today_actions 应合并未完成任务");
    assert!(actions.len() <= 3);
}
