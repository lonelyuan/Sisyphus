//! 主动触发调度器：待办动作队列的纯数据逻辑（core，无副作用）。

use sisyphus_core::db;
use sisyphus_core::scheduler::{self, NewAction};
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn temp_db() -> rusqlite::Connection {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("sis_sched_{}_{}_{}.db", std::process::id(), nanos, n));
    let _ = std::fs::remove_file(&path);
    db::open(path.to_str().unwrap()).unwrap()
}

const T0: i64 = 1_784_000_000_000; // 固定基准，避免依赖 wall clock

#[test]
fn immediate_action_is_due_and_marked_fired() {
    let conn = temp_db();
    let id = scheduler::enqueue_action(
        &conn,
        &NewAction {
            kind: "notify",
            payload_json: r#"{"title":"hi"}"#,
            due_at_ms: T0,
            recurrence: None,
            dedup_key: None,
            origin_event_id: None,
            created_by: "manual",
        },
    )
    .unwrap()
    .unwrap();

    let due = scheduler::due_actions(&conn, T0).unwrap();
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].id, id);
    assert_eq!(due[0].kind, "notify");

    // 取出即置 fired：再查不再返回（防重复执行）
    assert!(scheduler::due_actions(&conn, T0 + 1000).unwrap().is_empty());
    assert!(scheduler::list_pending(&conn).unwrap().is_empty());

    scheduler::mark_done(&conn, &id).unwrap();
}

#[test]
fn deferred_action_not_due_until_time() {
    let conn = temp_db();
    scheduler::enqueue_action(
        &conn,
        &NewAction {
            kind: "notify",
            payload_json: "{}",
            due_at_ms: T0 + 60_000, // 1 分钟后
            recurrence: None,
            dedup_key: None,
            origin_event_id: None,
            created_by: "rule_engine",
        },
    )
    .unwrap();

    assert!(scheduler::due_actions(&conn, T0).unwrap().is_empty(), "未到点不应触发");
    assert_eq!(
        scheduler::due_actions(&conn, T0 + 60_000).unwrap().len(),
        1,
        "到点应触发"
    );
}

#[test]
fn dedup_key_prevents_duplicate_pending() {
    let conn = temp_db();
    let a = NewAction {
        kind: "agent_run",
        payload_json: "{}",
        due_at_ms: T0,
        recurrence: None,
        dedup_key: Some("daily-kb-introspect"),
        origin_event_id: None,
        created_by: "scheduler",
    };
    assert!(scheduler::enqueue_action(&conn, &a).unwrap().is_some(), "首次入队");
    assert!(
        scheduler::enqueue_action(&conn, &a).unwrap().is_none(),
        "同 dedup_key 已有 pending → 跳过"
    );
    assert_eq!(scheduler::list_pending(&conn).unwrap().len(), 1);
}

#[test]
fn recurring_reschedules_to_next_day() {
    let conn = temp_db();
    // 每日 09:00 的周期动作
    scheduler::enqueue_action(
        &conn,
        &NewAction {
            kind: "agent_run",
            payload_json: r#"{"skill":"knowledge-engine"}"#,
            due_at_ms: T0,
            recurrence: Some("daily@09:00"),
            dedup_key: Some("daily-kb-introspect"),
            origin_event_id: None,
            created_by: "scheduler",
        },
    )
    .unwrap();

    let due = scheduler::due_actions(&conn, T0).unwrap();
    assert_eq!(due.len(), 1);

    // 触发后排下一次：应是严格晚于 now 的 09:00，且 dedup_key 延续
    let next_id = scheduler::reschedule(&conn, &due[0], T0).unwrap();
    assert!(next_id.is_some(), "周期动作应排出下一次");
    let pending = scheduler::list_pending(&conn).unwrap();
    assert_eq!(pending.len(), 1);
    assert!(pending[0].due_at_ms > T0, "下一次应晚于当前");
    assert_eq!(pending[0].recurrence.as_deref(), Some("daily@09:00"));
    assert_eq!(pending[0].dedup_key.as_deref(), Some("daily-kb-introspect"));
}

#[test]
fn next_due_parses_daily_and_is_strictly_after() {
    // 解析 daily@HH:MM，返回严格晚于 after 的时刻
    let n = scheduler::next_due("daily@09:00", T0).expect("应解析");
    assert!(n > T0);
    // 非法格式返回 None
    assert!(scheduler::next_due("weekly@09:00", T0).is_none());
    assert!(scheduler::next_due("daily@99:99", T0).is_none());
}
