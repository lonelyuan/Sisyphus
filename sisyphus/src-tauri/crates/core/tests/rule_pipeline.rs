//! 感知平面规则管道的端到端验证：给定"娱乐 + 有未完成目标 + 超阈值"应命中，否则不命中。
//! 注意：test 构建下 `RuleConfig::default()` 用 debug_fast（阈值 1min）。

use sisyphus_core::context;
use sisyphus_core::db;
use sisyphus_core::rule_engine::config::RuleConfig;
use sisyphus_core::rule_engine::{RuleContext, RuleEngine};

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

fn temp_db() -> rusqlite::Connection {
    let path = std::env::temp_dir().join(format!("sis_test_{}_{}.db", std::process::id(), now_ms()));
    let _ = std::fs::remove_file(&path);
    db::open(path.to_str().unwrap()).unwrap()
}

fn ctx_with(goal: Option<sisyphus_core::rule_engine::DailyGoal>) -> RuleContext {
    RuleContext {
        now_ms: now_ms(),
        user_id: "local-user".to_string(),
        device_id: "test".to_string(),
        current_app: Some("com.apple.TV".to_string()),
        current_category: Some("entertainment.video".to_string()),
        active_entertainment_ms: 120_000, // 2min ≥ 1min(debug) 阈值
        media_playing_since_ms: 0,
        recent_scroll_count: 0,
        today_goal: goal,
    }
}

#[test]
fn fires_when_entertainment_over_threshold_with_open_goal() {
    let conn = temp_db();
    context::set_goal(&conn, "写测试").unwrap();
    let goal = context::today_context(&conn, "local-user").unwrap().goal;
    assert!(goal.is_some(), "goal should exist after set_goal");

    let engine = RuleEngine::new(RuleConfig::default());
    let finding = engine.evaluate(&ctx_with(goal), &conn).unwrap();
    assert!(finding.is_some(), "娱乐超阈值且目标未完成，应命中");
}

#[test]
fn no_finding_without_goal() {
    let conn = temp_db();
    let engine = RuleEngine::new(RuleConfig::default());
    let finding = engine.evaluate(&ctx_with(None), &conn).unwrap();
    assert!(finding.is_none(), "无今日目标不应命中");
}

#[test]
fn no_finding_when_not_entertainment() {
    let conn = temp_db();
    context::set_goal(&conn, "写测试").unwrap();
    let goal = context::today_context(&conn, "local-user").unwrap().goal;

    let engine = RuleEngine::new(RuleConfig::default());
    let mut ctx = ctx_with(goal);
    ctx.current_category = Some("work.doc".to_string());
    let finding = engine.evaluate(&ctx, &conn).unwrap();
    assert!(finding.is_none(), "非娱乐类不应命中");
}
