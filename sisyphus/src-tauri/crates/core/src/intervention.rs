//! 共享干预决策：评估规则命中 → 生成不羞辱的消息 → 写 interventions 表 → 返回结果。
//!
//! 三个调用方共用同一逻辑（单一来源）：
//! - App 命令 `evaluate_rules`（Android JS 触发路径，仅前台）
//! - macOS 采集器 `collector::tick`
//! - Android 前台服务经 JNI（后台刷视频也能弹，见 android_jni）

use rusqlite::Connection;
use serde::Serialize;
use uuid::Uuid;

use crate::db;
use crate::rule_engine::{RuleContext, RuleEngine};

const OPTIONS_JSON: &str = r#"["start_task","take_rest","continue","abandon_today"]"#;

#[derive(Debug, Clone, Serialize)]
pub struct InterventionOutput {
    pub rule_id: String,
    pub severity: String,
    pub message: String,
    pub intervention_id: String,
}

/// 评估规则；命中则生成消息 + 写入一条 intervention，返回它。未命中返回 None。
/// 消息语气：具体（引用真实时长与目标）、不羞辱、不说教（见 docs/spec/agent.md）。
pub fn evaluate_and_record(
    conn: &Connection,
    engine: &RuleEngine,
    ctx: &RuleContext,
) -> rusqlite::Result<Option<InterventionOutput>> {
    let finding = match engine.evaluate(ctx, conn)? {
        Some(f) => f,
        None => return Ok(None),
    };

    let goal_text = ctx
        .today_goal
        .as_ref()
        .map(|g| g.raw_text.as_str())
        .unwrap_or("今日目标");
    let total_min = (ctx.active_entertainment_ms / 60_000).max(1);
    let prefix = if finding.severity == "high" { "⚠️ " } else { "" };
    let message = format!("{prefix}你已连续刷了 {total_min} 分钟娱乐内容。\n今日目标：{goal_text}");

    let intervention_id = Uuid::new_v4().to_string();
    db::insert_intervention(
        conn,
        &intervention_id,
        &finding.rule_id,
        ctx.now_ms,
        &finding.severity,
        &message,
        OPTIONS_JSON,
    )?;

    Ok(Some(InterventionOutput {
        rule_id: finding.rule_id,
        severity: finding.severity,
        message,
        intervention_id,
    }))
}
