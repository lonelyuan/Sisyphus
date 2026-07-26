//! 动态检测规则端到端：建一条规则 → RuleEngine 热加载 → 命中/不命中判定。

use sisyphus_core::context;
use sisyphus_core::db;
use sisyphus_core::rule_engine::config::RuleConfig;
use sisyphus_core::rule_engine::{RuleContext, RuleEngine};
use sisyphus_core::rules;
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn temp_db() -> rusqlite::Connection {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path =
        std::env::temp_dir().join(format!("sis_rules_{}_{}_{}.db", std::process::id(), nanos, n));
    let _ = std::fs::remove_file(&path);
    db::open(path.to_str().unwrap()).unwrap()
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

fn ctx(category: &str, app: &str, active_min: i64, goal: bool, conn: &rusqlite::Connection) -> RuleContext {
    if goal {
        context::set_goal(conn, "写测试").unwrap();
    }
    let today_goal = context::today_context(conn, "local-user").unwrap().goal;
    RuleContext {
        now_ms: now_ms(),
        user_id: "local-user".to_string(),
        device_id: "test".to_string(),
        current_app: Some(app.to_string()),
        current_category: Some(category.to_string()),
        active_entertainment_ms: 0,
        active_session_ms: active_min * 60_000,
        media_playing_since_ms: 0,
        recent_scroll_count: 0,
        today_goal,
    }
}

#[test]
fn dynamic_rule_fires_on_matching_category_over_threshold() {
    let conn = temp_db();
    rules::create_rule(
        &conn,
        "少打游戏",
        r#"{"category_prefix":"entertainment.game","min_minutes_in_window":15}"#,
        None,
        "medium",
        30,
        "agent",
        None,
    )
    .unwrap();

    let engine = RuleEngine::new(RuleConfig::default());
    // 命中：游戏分类 + 20min ≥ 15min + 有目标。
    let hit = engine
        .evaluate(&ctx("entertainment.game", "com.game", 20, true, &conn), &conn)
        .unwrap();
    assert!(hit.is_some(), "游戏超阈值应命中动态规则");
    assert!(hit.unwrap().message.unwrap().contains("少打游戏"));

    // 不命中：分类不匹配。
    let miss = engine
        .evaluate(&ctx("productivity.ide", "com.ide", 60, true, &conn), &conn)
        .unwrap();
    assert!(miss.is_none(), "非目标分类不应命中");
}

#[test]
fn disabled_rule_does_not_fire() {
    let conn = temp_db();
    let id = rules::create_rule(
        &conn,
        "夜间刷视频",
        r#"{"category_prefix":"entertainment","min_minutes_in_window":5,"requires_active_goal":false}"#,
        None,
        "medium",
        0,
        "user",
        None,
    )
    .unwrap();
    rules::set_rule_enabled(&conn, &id, false).unwrap();

    let engine = RuleEngine::new(RuleConfig::default());
    let miss = engine
        .evaluate(&ctx("entertainment.video", "com.v", 30, false, &conn), &conn)
        .unwrap();
    assert!(miss.is_none(), "停用的规则不应命中");
}

#[test]
fn create_rule_rejects_scopeless_trigger() {
    let conn = temp_db();
    let err = rules::create_rule(
        &conn,
        "空规则",
        r#"{"min_minutes_in_window":10}"#,
        None,
        "medium",
        30,
        "agent",
        None,
    );
    assert!(err.is_err(), "无 category/app 作用域应被拒绝");
}
