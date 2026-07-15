use std::sync::Mutex;
use std::path::PathBuf;
use serde::{Deserialize, Serialize};
use tauri::State;
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
    /// Obsidian 知识库目录（第二大脑 vault）。
    pub vault_dir: PathBuf,
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

    let out = sisyphus_core::intervention::evaluate_and_record(&conn, &state.rule_engine, &rule_ctx)
        .map_err(|e| e.to_string())?;

    Ok(out.map(|o| FindingOutput {
        rule_id: o.rule_id,
        severity: o.severity,
        message: o.message,
        intervention_id: o.intervention_id,
    }))
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

// ── 数据展示 + 增删查改（感知平面 App「今日/记录」页）─────────────────────────

#[tauri::command]
pub fn list_tasks(state: State<'_, AppState>) -> Result<Vec<sisyphus_core::artifacts::Task>, String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    sisyphus_core::artifacts::list_tasks(&conn, 100).map_err(|e| e.to_string())
}

/// 用户在 App 里直接建的任务（无 AI 溯源，source/intent 为空——与 Codex 意图桥区分）。
#[tauri::command]
pub fn create_task(
    state: State<'_, AppState>,
    title: String,
    due_ms: Option<i64>,
    note: Option<String>,
) -> Result<String, String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    sisyphus_core::artifacts::create_task(&conn, &title, due_ms, 0, note.as_deref(), None, None)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_task_status(
    state: State<'_, AppState>,
    id: String,
    status: String,
) -> Result<(), String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    sisyphus_core::artifacts::update_task_status(&conn, &id, &status).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_task(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    sisyphus_core::artifacts::delete_task(&conn, &id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn list_reminders(
    state: State<'_, AppState>,
) -> Result<Vec<sisyphus_core::artifacts::Reminder>, String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    sisyphus_core::artifacts::list_reminders(&conn, 100).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_reminder_status(
    state: State<'_, AppState>,
    id: String,
    status: String,
) -> Result<(), String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    match status.as_str() {
        "done" => sisyphus_core::artifacts::complete_reminder(&conn, &id),
        "cancelled" => sisyphus_core::artifacts::cancel_reminder(&conn, &id),
        other => return Err(format!("未知提醒状态: {other}")),
    }
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn list_interventions(
    state: State<'_, AppState>,
) -> Result<Vec<sisyphus_core::timeline::InterventionRow>, String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    sisyphus_core::timeline::list_interventions(&conn, 50).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn list_sessions(
    state: State<'_, AppState>,
) -> Result<Vec<sisyphus_core::timeline::SessionRow>, String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    sisyphus_core::timeline::list_recent_sessions(&conn, 60).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn list_knowledge(
    state: State<'_, AppState>,
) -> Result<Vec<sisyphus_core::knowledge::KnowledgeNote>, String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    sisyphus_core::knowledge::list_knowledge(&conn).map_err(|e| e.to_string())
}

/// 监控名单：当前被视为娱乐的 app（用户自定义 + 桌面内置 + Android 内置）。
#[tauri::command]
pub fn list_monitored_apps(
    state: State<'_, AppState>,
) -> Result<Vec<sisyphus_core::category::MonitoredApp>, String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    sisyphus_core::category::list_monitored(&conn).map_err(|e| e.to_string())
}

/// 增/改一个用户监控 app（写 monitored_apps 表，桌面+安卓即时生效）。
#[tauri::command]
pub fn add_monitored_app(
    state: State<'_, AppState>,
    id: String,
    category: String,
) -> Result<(), String> {
    let id = id.trim();
    if id.is_empty() {
        return Err("包名/bundle id 不能为空".into());
    }
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    sisyphus_core::category::add_monitored_app(&conn, id, &category).map_err(|e| e.to_string())
}

/// 删一个用户监控 app。
#[tauri::command]
pub fn remove_monitored_app(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    sisyphus_core::category::remove_monitored_app(&conn, &id).map_err(|e| e.to_string())
}

/// 返回知识库 vault 路径（供 Settings「在 Obsidian 打开」）。
#[tauri::command]
pub fn get_vault_path(state: State<'_, AppState>) -> String {
    state.vault_dir.to_string_lossy().to_string()
}

/// 第二大脑派发：调用 Codex TS SDK 的 Node 脚本，围绕 topic 深研并写入知识库。
///
/// 脚本路径由 env `SISYPHUS_KNOWLEDGE_AGENT_SCRIPT` 指定（services/knowledge-agent/index.mjs 的
/// 绝对路径，作为单个参数传入——路径含空格也安全）；node 可执行由 `SISYPHUS_NODE_BIN` 覆盖，
/// 默认 `node`。脚本运行时会拿到 `SISYPHUS_VAULT` 指向本机知识库。
/// 真机需用户已装 codex + 鉴权；未配置脚本路径则返回引导。
#[tauri::command]
pub async fn run_knowledge_agent(
    state: State<'_, AppState>,
    topic: String,
) -> Result<String, String> {
    let vault = state.vault_dir.to_string_lossy().to_string();
    let script = std::env::var("SISYPHUS_KNOWLEDGE_AGENT_SCRIPT").map_err(|_| {
        "未配置 SISYPHUS_KNOWLEDGE_AGENT_SCRIPT（应指向 services/knowledge-agent/index.mjs 的绝对路径）"
            .to_string()
    })?;
    let node = std::env::var("SISYPHUS_NODE_BIN").unwrap_or_else(|_| "node".to_string());

    tauri::async_runtime::spawn_blocking(move || {
        // script/topic 各作为单个 arg 传入，路径/主题含空格均安全；无 shell，无注入面。
        let out = std::process::Command::new(&node)
            .arg(&script)
            .arg(&topic)
            .env("SISYPHUS_VAULT", &vault)
            .output()
            .map_err(|e| format!("启动知识 agent 失败: {e}"))?;
        let stdout = String::from_utf8_lossy(&out.stdout).to_string();
        if out.status.success() {
            Ok(stdout)
        } else {
            let stderr = String::from_utf8_lossy(&out.stderr).to_string();
            Err(format!("知识 agent 退出码 {:?}\n{stderr}", out.status.code()))
        }
    })
    .await
    .map_err(|e| format!("任务 join 失败: {e}"))?
}
