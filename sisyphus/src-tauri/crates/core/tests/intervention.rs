//! 反拖延干预的**正确性**回归测试。
//!
//! 这里锁住的是三个曾经出错、且用户能直接感受到的行为：
//! 1. 延后/防打扰策略下**不能**每拍重复入队（否则十分钟后几十条通知一起炸）；
//! 2. debounce 的 `window_ms` 真的生效（此前整个参数被丢弃）；
//! 3. 干预后的近端结果能被观察并回填（这是这个模块唯一的学习信号）。

use sisyphus_core::ingest::{ingest_event, NewEvent};
use sisyphus_core::rule_engine::config::{EntertainmentRuleConfig, RuleConfig};
use sisyphus_core::rule_engine::{DailyGoal, RuleContext, RuleEngine};
use sisyphus_core::{clock, db, intervention, rules, scheduler};

fn conn() -> rusqlite::Connection {
    db::open(":memory:").unwrap()
}

/// 显式配置内置娱乐规则的阈值，让测试不受构建 profile 影响
/// （`RuleConfig::default()` 在 debug 下是 1 分钟阈值的 debug_fast）。
fn engine_with_builtin_threshold(threshold_minutes: i64) -> RuleEngine {
    RuleEngine::new(RuleConfig {
        entertainment: EntertainmentRuleConfig {
            window_minutes: 60,
            threshold_minutes,
            cooldown_minutes: 30,
            scroll_active_threshold: 30,
            media_stable_minutes: 5,
        },
    })
}

/// 内置规则不参与的引擎（阈值设得远高于测试用的会话时长）。
fn engine() -> RuleEngine {
    engine_with_builtin_threshold(600)
}

fn ctx(now: i64, app: &str, category: &str, active_ms: i64) -> RuleContext {
    RuleContext {
        now_ms: now,
        user_id: "local-user".into(),
        device_id: "test-device".into(),
        current_app: Some(app.into()),
        current_category: Some(category.into()),
        active_entertainment_ms: active_ms,
        active_session_ms: active_ms,
        media_playing_since_ms: 0,
        recent_scroll_count: 0,
        today_goal: Some(DailyGoal {
            id: "g1".into(),
            date: clock::day_str_at(now, 0),
            raw_text: "推进西西弗斯".into(),
            status: "planned".into(),
        }),
    }
}

/// 造一条动态规则：命中娱乐类、窗口内累计 ≥1 分钟。
fn rule(conn: &rusqlite::Connection, name: &str, response: &str) -> String {
    rules::create_rule(
        conn,
        name,
        r#"{"category_prefix":"entertainment","window_minutes":30,"min_minutes_in_window":1,"requires_active_goal":true}"#,
        Some(response),
        "medium",
        30,
        "user",
        None,
    )
    .unwrap()
}

/// 写一段已闭合的娱乐会话，让规则的窗口统计能命中阈值。
fn entertainment_session(conn: &rusqlite::Connection, start: i64, minutes: i64) {
    ingest_event(
        conn,
        "local-user",
        "test-device",
        NewEvent {
            event_id: None,
            source: "desktop_agent".into(),
            layer: "raw".into(),
            event_type: "app_foreground".into(),
            time_mode: "interval".into(),
            event_time: None,
            start_time: Some(start),
            end_time: Some(start + minutes * 60_000),
            entity: Some("tv.danmaku.bili".into()),
            category: Some("entertainment.video".into()),
            payload: serde_json::json!({}),
            parent_event_ids: vec![],
            privacy_level: "L0".into(),
        },
    )
    .unwrap();
}

fn pending_count(conn: &rusqlite::Connection) -> usize {
    scheduler::list_pending(conn).unwrap().len()
}

#[test]
fn deferred_policy_enqueues_once_not_every_tick() {
    let conn = conn();
    let engine = engine();
    let now = clock::now_ms();
    entertainment_session(&conn, now - 10 * 60_000, 5);
    rule(
        &conn,
        "延后提醒",
        r#"{"policy":"deferred","kind":"notify","after_ms":600000}"#,
    );

    // 模拟采集器连续 20 拍（每 15s 一拍）——冷却 30 分钟内只应入队一次。
    for i in 0..20 {
        let c = ctx(now + i * 15_000, "tv.danmaku.bili", "entertainment.video", 300_000);
        let out = intervention::evaluate_and_record(&conn, &engine, &c).unwrap();
        assert!(out.is_none(), "延后策略不应当拍派发");
    }
    assert_eq!(
        pending_count(&conn),
        1,
        "延后策略必须只入队一条；每拍重复入队会导致十分钟后几十条通知同时炸出来"
    );

    // 冷却过后允许再来一次。
    let later = now + 31 * 60_000;
    entertainment_session(&conn, later - 5 * 60_000, 5);
    intervention::evaluate_and_record(
        &conn,
        &engine,
        &ctx(later, "tv.danmaku.bili", "entertainment.video", 300_000),
    )
    .unwrap();
    assert_eq!(pending_count(&conn), 2, "冷却过后应能再次提醒");
}

#[test]
fn debounce_window_is_honoured() {
    let conn = conn();
    let engine = engine();
    let now = clock::now_ms();
    entertainment_session(&conn, now - 10 * 60_000, 5);
    // 冷却设 0，只靠 debounce 窗口拦——这样才测得出 window_ms 是否真的生效。
    rules::create_rule(
        &conn,
        "防打扰",
        r#"{"category_prefix":"entertainment","window_minutes":30,"min_minutes_in_window":1}"#,
        Some(r#"{"policy":"debounce","kind":"notify","window_ms":3600000,"dedup_key":"ent-scroll"}"#),
        "medium",
        0,
        "user",
        None,
    )
    .unwrap();

    intervention::evaluate_and_record(
        &conn,
        &engine,
        &ctx(now, "tv.danmaku.bili", "entertainment.video", 300_000),
    )
    .unwrap();
    assert_eq!(pending_count(&conn), 1);

    // 把队列里那条标记为已派发：此后"队列里还有 pending"这道闸门失效，
    // 只剩 window_ms 能拦住它——旧实现在这一步会每 30 秒响一次。
    for a in scheduler::due_actions(&conn, now + 1).unwrap() {
        scheduler::mark_done(&conn, &a.id).unwrap();
    }
    entertainment_session(&conn, now + 60_000, 5);
    intervention::evaluate_and_record(
        &conn,
        &engine,
        &ctx(now + 120_000, "tv.danmaku.bili", "entertainment.video", 300_000),
    )
    .unwrap();
    assert_eq!(pending_count(&conn), 0, "窗口内不应再次入队");

    // 窗口外可以再提醒。
    entertainment_session(&conn, now + 61 * 60_000, 5);
    intervention::evaluate_and_record(
        &conn,
        &engine,
        &ctx(
            now + 62 * 60_000,
            "tv.danmaku.bili",
            "entertainment.video",
            300_000,
        ),
    )
    .unwrap();
    assert_eq!(pending_count(&conn), 1, "窗口外应恢复提醒");
}

#[test]
fn suppress_policy_advances_cooldown_and_stays_silent() {
    let conn = conn();
    let engine = engine();
    let now = clock::now_ms();
    entertainment_session(&conn, now - 10 * 60_000, 5);
    let id = rule(&conn, "夜间免打扰", r#"{"policy":"suppress"}"#);

    let out = intervention::evaluate_and_record(
        &conn,
        &engine,
        &ctx(now, "tv.danmaku.bili", "entertainment.video", 300_000),
    )
    .unwrap();
    assert!(out.is_none());
    assert_eq!(pending_count(&conn), 0);
    // 抑制也算一次响应：冷却推进，避免每拍重算同一条规则。
    assert!(!db::is_cooldown_ready(&conn, &id, now + 1000, 30 * 60_000).unwrap());
}

#[test]
fn immediate_policy_records_intervention_and_schedules_outcome_checks() {
    let conn = conn();
    let engine = engine();
    let now = clock::now_ms();
    entertainment_session(&conn, now - 10 * 60_000, 5);
    rule(&conn, "立即提醒", r#"{"policy":"immediate","kind":"notify"}"#);

    let out = intervention::evaluate_and_record(
        &conn,
        &engine,
        &ctx(now, "tv.danmaku.bili", "entertainment.video", 300_000),
    )
    .unwrap()
    .expect("立即策略应当拍派发");
    assert!(out.message.contains("推进西西弗斯"), "文案要引用真实目标");

    // 干预弹出后应排两次近端结果观察（10 / 30 分钟）。
    let pending = scheduler::list_pending(&conn).unwrap();
    let checks: Vec<_> = pending
        .iter()
        .filter(|a| a.kind == "observe_outcome")
        .collect();
    assert_eq!(checks.len(), 2, "应排 10 分钟与 30 分钟两次观察");
    assert!(checks.iter().any(|a| a.due_at_ms == now + 10 * 60_000));
    assert!(checks.iter().any(|a| a.due_at_ms == now + 30 * 60_000));
}

#[test]
fn observe_outcome_classifies_still_scrolling_vs_switched() {
    let conn = conn();
    let now = clock::now_ms();

    // 情形一：提醒后 10 分钟基本还在刷 → still_entertainment
    db::insert_intervention(&conn, "i-still", "r1", now, "medium", "msg", "[]").unwrap();
    entertainment_session(&conn, now, 9);
    let outcome = intervention::observe_outcome(&conn, "i-still", 10)
        .unwrap()
        .unwrap();
    assert_eq!(outcome, intervention::OUTCOME_STILL);

    // 情形二：提醒后转去工作 → switched
    db::insert_intervention(&conn, "i-switch", "r1", now, "medium", "msg", "[]").unwrap();
    ingest_event(
        &conn,
        "local-user",
        "test-device",
        NewEvent {
            event_id: None,
            source: "desktop_agent".into(),
            layer: "raw".into(),
            event_type: "app_foreground".into(),
            time_mode: "interval".into(),
            event_time: None,
            start_time: Some(now + 10 * 60_000),
            end_time: Some(now + 19 * 60_000),
            entity: Some("com.apple.Terminal".into()),
            category: Some("work".into()),
            payload: serde_json::json!({}),
            parent_event_ids: vec![],
            privacy_level: "L0".into(),
        },
    )
    .unwrap();
    // 观察窗口取 [shown_at, shown_at+20min)，其中娱乐 0、工作 9 分钟。
    db::insert_intervention(&conn, "i-switch2", "r1", now + 10 * 60_000, "medium", "m", "[]")
        .unwrap();
    let outcome2 = intervention::observe_outcome(&conn, "i-switch2", 10)
        .unwrap()
        .unwrap();
    assert_eq!(outcome2, intervention::OUTCOME_SWITCHED);

    // 情形三：没有任何观测 → unknown（不编）
    db::insert_intervention(
        &conn,
        "i-unknown",
        "r1",
        now + 10 * 86_400_000,
        "medium",
        "m",
        "[]",
    )
    .unwrap();
    assert_eq!(
        intervention::observe_outcome(&conn, "i-unknown", 10)
            .unwrap()
            .unwrap(),
        intervention::OUTCOME_UNKNOWN
    );

    // 只回填一次：30 分钟那次不会覆盖 10 分钟的结论。
    assert!(intervention::observe_outcome(&conn, "i-still", 30)
        .unwrap()
        .is_none());

    let stats = intervention::outcome_stats(&conn, now - 1000).unwrap();
    assert!(stats.total >= 3);
    assert!(stats.switch_rate.is_some(), "有有效样本时应给出转移率");
}

#[test]
fn user_rule_wins_over_builtin_when_more_severe() {
    let conn = conn();
    // 内置规则阈值 30min：35min 会命中但只是 medium；用户规则 high 应胜出。
    let engine = engine_with_builtin_threshold(30);
    let now = clock::now_ms();
    entertainment_session(&conn, now - 40 * 60_000, 35);
    // 用户建的高危规则应优先于内置通用娱乐规则被处理（此前"第一个命中即返回"
    // 且内置永远排在前面，用户精心建的规则可能永远不触发）。
    rules::create_rule(
        &conn,
        "夜间游戏",
        r#"{"category_prefix":"entertainment","window_minutes":60,"min_minutes_in_window":5}"#,
        Some(r#"{"policy":"immediate","kind":"notify"}"#),
        "high",
        30,
        "user",
        None,
    )
    .unwrap();

    let out = intervention::evaluate_and_record(
        &conn,
        &engine,
        &ctx(now, "tv.danmaku.bili", "entertainment.video", 35 * 60_000),
    )
    .unwrap()
    .unwrap();
    assert_eq!(out.severity, "high", "应挑 severity 更高的那条");
}
