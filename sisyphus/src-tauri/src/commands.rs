use std::sync::Mutex;
use serde::{Deserialize, Serialize};
use tauri::State;
use uuid::Uuid;
use chrono::Utc;
use rusqlite::Connection;
use sisyphus_core::db;
use sisyphus_core::ingest::{self, NewEvent};
use sisyphus_core::context::{self, TodayContext};
use sisyphus_core::rule_engine::{RuleContext, RuleEngine};

// ── AppState ──────────────────────────────────────────────────────────────────

pub struct AppState {
    pub conn: Mutex<Connection>,
    pub rule_engine: RuleEngine,
    pub user_id: String,
    pub device_id: String,
}

// ── 命令输入 / 输出类型 ───────────────────────────────────────────────────────

/// JS 侧传入的规则评估上下文（Kotlin/JS 组装后 invoke）
#[derive(Debug, Deserialize)]
pub struct RuleContextInput {
    pub current_app: Option<String>,
    pub current_category: Option<String>,
    pub active_entertainment_ms: i64,
    /// Layer 2：媒体播放开始时间（epoch ms），0 = 未启用
    #[serde(default)]
    pub media_playing_since_ms: i64,
    /// Layer 3：过去 10min scroll_burst 次数，0 = 未启用
    #[serde(default)]
    pub recent_scroll_count: i64,
}

#[derive(Debug, Serialize)]
pub struct FindingOutput {
    pub rule_id: String,
    pub severity: String,
    pub message: String,
    pub intervention_id: String,
}

// ── Tauri Commands ────────────────────────────────────────────────────────────

/// 唯一写入契约：所有采集源经此写入 Event log（Tauri 命令薄封装 core::ingest_event）。
#[tauri::command]
pub fn ingest_event(state: State<'_, AppState>, input: NewEvent) -> Result<String, String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    ingest::ingest_event(&conn, &state.user_id, &state.device_id, input).map_err(|e| e.to_string())
}

/// 评估规则，若命中则写干预记录并返回 Finding（含干预 ID 和消息文本）。
/// 由 JS 每 10s 调用（Android：Kotlin usage_event 触发；Desktop：setInterval）。
#[tauri::command]
pub fn evaluate_rules(
    state: State<'_, AppState>,
    ctx: RuleContextInput,
) -> Result<Option<FindingOutput>, String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    let now_ms = Utc::now().timestamp_millis();
    let today = Utc::now().format("%Y-%m-%d").to_string();

    let goal = db::get_today_goal(&conn, &today).map_err(|e| e.to_string())?;

    let rule_ctx = RuleContext {
        now_ms,
        user_id: state.user_id.clone(),
        device_id: state.device_id.clone(),
        current_app: ctx.current_app,
        current_category: ctx.current_category,
        active_entertainment_ms: ctx.active_entertainment_ms,
        media_playing_since_ms: ctx.media_playing_since_ms,
        recent_scroll_count: ctx.recent_scroll_count,
        today_goal: goal,
    };

    let finding = state
        .rule_engine
        .evaluate(&rule_ctx, &conn)
        .map_err(|e| e.to_string())?;

    if let Some(f) = finding {
        let intervention_id = Uuid::new_v4().to_string();
        let goal_text = rule_ctx
            .today_goal
            .as_ref()
            .map(|g| g.raw_text.as_str())
            .unwrap_or("今日目标");
        let total_min = (rule_ctx.active_entertainment_ms / 60_000).max(1);
        let severity_emoji = if f.severity == "high" { "⚠️ " } else { "" };
        let message = format!(
            "{}你已连续刷了 {} 分钟娱乐内容。\n今日目标：{}",
            severity_emoji, total_min, goal_text
        );
        let options = r#"["start_task","take_rest","continue","abandon_today"]"#;

        db::insert_intervention(
            &conn,
            &intervention_id,
            &f.rule_id,
            now_ms,
            &f.severity,
            &message,
            options,
        )
        .map_err(|e| e.to_string())?;

        return Ok(Some(FindingOutput {
            rule_id: f.rule_id,
            severity: f.severity,
            message,
            intervention_id,
        }));
    }

    Ok(None)
}

/// 设置今日目标。已有目标时更新文本并重置为 planned 状态。
#[tauri::command]
pub fn set_goal(state: State<'_, AppState>, text: String) -> Result<(), String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    context::set_goal(&conn, &text).map(|_| ()).map_err(|e| e.to_string())
}

/// 更新目标状态（started / completed / skipped / abandoned）。
#[tauri::command]
pub fn update_goal_status(
    state: State<'_, AppState>,
    id: String,
    status: String,
) -> Result<(), String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    db::update_goal_status(&conn, &id, &status).map_err(|e| e.to_string())
}

/// 记录用户对干预通知的响应（点击了哪个按钮）。
#[tauri::command]
pub fn record_feedback(
    state: State<'_, AppState>,
    intervention_id: String,
    action: String,
) -> Result<(), String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    context::record_feedback(&conn, &intervention_id, &action).map_err(|e| e.to_string())
}

/// 返回今日摘要数据（供 TodayScreen 展示 / 与 Agent query_context 同源）。
#[tauri::command]
pub fn get_today_context(state: State<'_, AppState>) -> Result<TodayContext, String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    context::today_context(&conn, &state.user_id).map_err(|e| e.to_string())
}
