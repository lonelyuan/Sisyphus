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
///
/// **无论走哪条策略，命中当拍都要写一条 `rule_fires`**——它是冷却与 debounce 的唯一依据。
/// 之前只有 `Immediate` 会留痕（`interventions`），于是延后/防打扰策略下冷却永远 ready，
/// 采集器每 5–15s 就重新入队一条：一条 10 分钟延迟的规则会攒出几十条通知同时炸出来。
pub fn evaluate_and_record(
    conn: &Connection,
    engine: &RuleEngine,
    ctx: &RuleContext,
) -> rusqlite::Result<Option<InterventionOutput>> {
    let finding = match engine.evaluate(ctx, conn)? {
        Some(f) => f,
        None => return Ok(None),
    };

    match finding.response.clone() {
        ResponsePolicy::Suppress => {
            // 记痕：抑制也是一次响应，冷却照样推进（否则每拍都重算同一条规则）。
            db::record_rule_fire(conn, &finding.rule_id, ctx.now_ms, "suppress", None)?;
            Ok(None)
        }
        ResponsePolicy::Immediate { kind } => {
            let message = build_message(&finding, ctx);
            let intervention_id = record_intervention(conn, &finding, &message, ctx.now_ms)?;
            db::record_rule_fire(conn, &finding.rule_id, ctx.now_ms, "immediate", None)?;
            schedule_outcome_check(conn, &intervention_id, ctx.now_ms);
            Ok(Some(InterventionOutput {
                rule_id: finding.rule_id,
                severity: finding.severity,
                message,
                intervention_id,
                kind,
            }))
        }
        ResponsePolicy::Deferred { kind, after_ms } => {
            // 延后派发时文案要重算（"你已连续刷了 X 分钟"在 10 分钟后早就不成立），
            // 所以队列里带上 rule_id 与阈值，由执行器投递前生成最终文案。
            enqueue_action_for(
                conn,
                &finding,
                &kind,
                ctx.now_ms + after_ms.max(0),
                None,
                ctx,
            )?;
            db::record_rule_fire(conn, &finding.rule_id, ctx.now_ms, "deferred", None)?;
            Ok(None)
        }
        ResponsePolicy::Debounce {
            kind,
            window_ms,
            dedup_key,
        } => {
            // window_ms 此前被整个丢掉（`window_ms: _`），去重只靠"队列里还有 pending"，
            // 一旦派发完就又能入队 → 退化成每 30 秒响一次。现在窗口真的生效。
            if db::debounced_recently(conn, &dedup_key, ctx.now_ms, window_ms)? {
                return Ok(None);
            }
            enqueue_action_for(conn, &finding, &kind, ctx.now_ms, Some(&dedup_key), ctx)?;
            db::record_rule_fire(
                conn,
                &finding.rule_id,
                ctx.now_ms,
                "debounce",
                Some(&dedup_key),
            )?;
            Ok(None)
        }
    }
}

/// 干预弹出后排两次近端结果观察（10 / 30 分钟）：到点看用户在干什么，回填
/// `interventions.outcome`。这是 1.1 唯一的学习信号——没有它，阈值只能靠感觉调，
/// 后续的可学习策略也没有 label 可学。
fn schedule_outcome_check(conn: &Connection, intervention_id: &str, now_ms: i64) {
    for minutes in [10_i64, 30] {
        let payload = serde_json::json!({
            "intervention_id": intervention_id,
            "after_minutes": minutes,
        })
        .to_string();
        let _ = scheduler::enqueue_action(
            conn,
            &NewAction {
                kind: "observe_outcome",
                payload_json: &payload,
                due_at_ms: now_ms + minutes * 60_000,
                recurrence: None,
                dedup_key: Some(&format!("outcome-{intervention_id}-{minutes}")),
                origin_event_id: None,
                created_by: "rule_engine",
            },
        );
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

// ── 近端结果观察（proximal outcome）──────────────────────────────────────────

/// 归一化的近端结果标签。
pub const OUTCOME_UNKNOWN: &str = "unknown";
pub const OUTCOME_STILL: &str = "still_entertainment";
pub const OUTCOME_MIXED: &str = "mixed";
pub const OUTCOME_SWITCHED: &str = "switched";

/// 观察一次干预的近端结果：干预弹出后 `after_minutes` 分钟里，用户实际在干什么。
///
/// 这是**唯一**能把"提醒到底有没有用"从感觉变成数据的东西：
/// 娱乐占比 ≥60% → 还在刷；≤20% → 转走了；中间 → 混合；没有观测 → unknown（不编）。
/// 只回填一次（`outcome IS NULL` 时才写），所以 10 分钟那次先落，30 分钟那次不会覆盖它。
pub fn observe_outcome(
    conn: &Connection,
    intervention_id: &str,
    after_minutes: i64,
) -> rusqlite::Result<Option<String>> {
    let shown_at: Option<i64> = conn
        .query_row(
            "SELECT shown_at FROM interventions WHERE id=?1 AND outcome IS NULL",
            rusqlite::params![intervention_id],
            |r| r.get(0),
        )
        .ok();
    let Some(shown_at) = shown_at else {
        return Ok(None); // 不存在或已回填
    };
    let until = shown_at + after_minutes.max(1) * 60_000;
    let (entertainment_ms, observed_ms) = db::category_split_between(conn, shown_at, until)?;

    let (outcome, detail) = if observed_ms <= 0 {
        (OUTCOME_UNKNOWN, "窗口内无前台观测".to_string())
    } else {
        let share = entertainment_ms as f64 / observed_ms as f64;
        let detail = format!(
            "娱乐 {:.1}min / 观测 {:.1}min",
            entertainment_ms as f64 / 60_000.0,
            observed_ms as f64 / 60_000.0
        );
        if share >= 0.6 {
            (OUTCOME_STILL, detail)
        } else if share <= 0.2 {
            (OUTCOME_SWITCHED, detail)
        } else {
            (OUTCOME_MIXED, detail)
        }
    };
    db::record_intervention_outcome(conn, intervention_id, outcome, Some(&detail), until)?;
    Ok(Some(outcome.to_string()))
}

/// 干预效果统计（"提醒后转移率"）：给 App 一张能看的表，也给后续策略层当 baseline。
#[derive(Debug, Clone, Serialize)]
pub struct OutcomeStats {
    pub total: i64,
    pub switched: i64,
    pub mixed: i64,
    pub still_entertainment: i64,
    pub unknown: i64,
    /// switched / (switched + mixed + still)，无有效样本时为 None（不伪装成 0）。
    pub switch_rate: Option<f64>,
}

pub fn outcome_stats(conn: &Connection, since_ms: i64) -> rusqlite::Result<OutcomeStats> {
    let mut stmt = conn.prepare(
        "SELECT COALESCE(outcome,'pending'), COUNT(*) FROM interventions
         WHERE shown_at >= ?1 GROUP BY 1",
    )?;
    let mut stats = OutcomeStats {
        total: 0,
        switched: 0,
        mixed: 0,
        still_entertainment: 0,
        unknown: 0,
        switch_rate: None,
    };
    let rows = stmt.query_map(rusqlite::params![since_ms], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
    })?;
    for row in rows {
        let (label, count) = row?;
        stats.total += count;
        match label.as_str() {
            OUTCOME_SWITCHED => stats.switched = count,
            OUTCOME_MIXED => stats.mixed = count,
            OUTCOME_STILL => stats.still_entertainment = count,
            OUTCOME_UNKNOWN => stats.unknown = count,
            _ => {}
        }
    }
    let effective = stats.switched + stats.mixed + stats.still_entertainment;
    if effective > 0 {
        stats.switch_rate = Some(stats.switched as f64 / effective as f64);
    }
    Ok(stats)
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
///
/// payload 里带上 `rule_id` / `rule_name` / `severity` 与检测当时的快照，但**不写死文案**
/// 的瞬时数字部分：执行器投递前会用当下数据重算（见 `body_hint`）。
fn enqueue_action_for(
    conn: &Connection,
    finding: &Finding,
    kind: &str,
    due_at_ms: i64,
    dedup_key: Option<&str>,
    ctx: &RuleContext,
) -> rusqlite::Result<()> {
    let goal_text = ctx.today_goal.as_ref().map(|g| g.raw_text.clone());
    let payload = serde_json::json!({
        "title": "西西弗斯",
        "rule_id": finding.rule_id,
        "severity": finding.severity,
        "goal": goal_text,
        "detected_at_ms": ctx.now_ms,
        "context": finding.context_snapshot,
        // 兜底文案：执行器若不想重算，也有一条能用的（不含"此刻已刷 N 分钟"这类会过期的数字）。
        "body": deferred_body(finding, ctx),
    })
    .to_string();
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

/// 延后投递用的文案：只引用**不会过期**的信息（目标、规则名），不写"已连续刷 N 分钟"。
fn deferred_body(finding: &Finding, ctx: &RuleContext) -> String {
    let rule_name = finding
        .context_snapshot
        .get("rule_name")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let head = if rule_name.is_empty() {
        "刚才那段娱乐时间有点长了。".to_string()
    } else {
        format!("「{rule_name}」提醒你一下。")
    };
    match ctx.today_goal.as_ref() {
        Some(g) => format!("{head}\n今日目标：{}", g.raw_text),
        None => head,
    }
}
