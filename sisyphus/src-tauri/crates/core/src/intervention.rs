//! 共享干预决策：评估规则命中 → 生成不羞辱的消息 → 按 [`ResponsePolicy`] 落地。
//!
//! 三个调用方共用同一逻辑（单一来源）：
//! - App 命令 `evaluate_rules`（Android JS 触发路径，仅前台）
//! - macOS 采集器 `collector::tick`
//! - Android 前台服务经 JNI（后台刷视频也能弹，见 android_jni）
//!
//! **ResponsePolicy seam**（proactive-triggers.md §3）：
//! - `Immediate` → 写 intervention 记录，返回 [`InterventionOutput`] 供调用方**当拍派发**（保实时）。
//! - `Deferred`/`Debounce` → 入队 `scheduled_actions`，由 app ticker 到点派发，返回 `None`。
//! - `Suppress` → 不打扰，返回 `None`。

use rusqlite::Connection;
use serde::Serialize;
use uuid::Uuid;

use crate::db;
use crate::rule_engine::{Finding, ResponsePolicy, RuleContext, RuleEngine};
use crate::scheduler::{self, NewAction};

const OPTIONS_JSON: &str = r#"["start_task","take_rest","continue","abandon_today"]"#;

#[derive(Debug, Clone, Serialize)]
pub struct InterventionOutput {
    pub rule_id: String,
    pub severity: String,
    pub message: String,
    pub intervention_id: String,
    /// 派发通道：notify | pet_message。调用方按此选择通知或宠物气泡。
    pub kind: String,
}

/// 评估规则；命中后按其 [`ResponsePolicy`] 落地。`Immediate` 返回待派发的 [`InterventionOutput`]；
/// `Deferred`/`Debounce` 入队后返回 `None`；`Suppress`/未命中返回 `None`。
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
    let message = build_message(&finding, ctx);

    match finding.response.clone() {
        ResponsePolicy::Suppress => Ok(None),
        ResponsePolicy::Immediate { kind } => {
            let intervention_id = record_intervention(conn, &finding, &message, ctx.now_ms)?;
            Ok(Some(InterventionOutput {
                rule_id: finding.rule_id,
                severity: finding.severity,
                message,
                intervention_id,
                kind,
            }))
        }
        ResponsePolicy::Deferred { kind, after_ms } => {
            enqueue_action_for(conn, &finding, &message, &kind, ctx.now_ms + after_ms, None)?;
            Ok(None)
        }
        ResponsePolicy::Debounce {
            kind,
            window_ms: _,
            dedup_key,
        } => {
            enqueue_action_for(
                conn,
                &finding,
                &message,
                &kind,
                ctx.now_ms,
                Some(&dedup_key),
            )?;
            Ok(None)
        }
    }
}

/// 生成不羞辱的干预文案：规则自带 message 优先，否则用默认娱乐模板（引用真实时长与目标）。
fn build_message(finding: &Finding, ctx: &RuleContext) -> String {
    if let Some(m) = &finding.message {
        if !m.trim().is_empty() {
            return m.clone();
        }
    }
    let goal_text = ctx
        .today_goal
        .as_ref()
        .map(|g| g.raw_text.as_str())
        .unwrap_or("今日目标");
    let total_min = (ctx.active_entertainment_ms / 60_000).max(1);
    let prefix = if finding.severity == "high" { "⚠️ " } else { "" };
    format!("{prefix}你已连续刷了 {total_min} 分钟娱乐内容。\n今日目标：{goal_text}")
}

/// 写一条 intervention 记录，返回其 id。
fn record_intervention(
    conn: &Connection,
    finding: &Finding,
    message: &str,
    now_ms: i64,
) -> rusqlite::Result<String> {
    let intervention_id = Uuid::new_v4().to_string();
    db::insert_intervention(
        conn,
        &intervention_id,
        &finding.rule_id,
        now_ms,
        &finding.severity,
        message,
        OPTIONS_JSON,
    )?;
    Ok(intervention_id)
}

/// 延后 / 防打扰：把干预入队 `scheduled_actions`，由 app ticker 到点派发。
fn enqueue_action_for(
    conn: &Connection,
    finding: &Finding,
    message: &str,
    kind: &str,
    due_at_ms: i64,
    dedup_key: Option<&str>,
) -> rusqlite::Result<()> {
    let payload = serde_json::json!({ "title": "西西弗斯", "body": message }).to_string();
    let origin = finding.parent_event_ids.first().map(|s| s.as_str());
    scheduler::enqueue_action(
        conn,
        &NewAction {
            kind,
            payload_json: &payload,
            due_at_ms,
            recurrence: None,
            dedup_key,
            origin_event_id: origin,
            created_by: "rule_engine",
        },
    )
    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(e.into()))?;
    Ok(())
}
