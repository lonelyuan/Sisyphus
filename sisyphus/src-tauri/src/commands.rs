use crate::agent_runtime;
use chrono::Utc;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use sisyphus_core::context::{self, TodayContext};
use sisyphus_core::db;
use sisyphus_core::ingest::{self, NewEvent};
use sisyphus_core::rule_engine::{RuleContext, RuleEngine};
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::State;

// ── AppState ──────────────────────────────────────────────────────────────────

pub struct AppState {
    pub conn: Mutex<Connection>,
    pub rule_engine: RuleEngine,
    pub user_id: String,
    pub device_id: String,
    /// Obsidian 知识库目录（第二大脑 vault）。
    pub vault_dir: PathBuf,
    /// app 数据目录（存 llm_config.json 等）。
    pub data_dir: PathBuf,
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
        active_session_ms: ctx.active_entertainment_ms,
        media_playing_since_ms: ctx.media_playing_since_ms,
        recent_scroll_count: ctx.recent_scroll_count,
        today_goal: goal,
    };

    let out =
        sisyphus_core::intervention::evaluate_and_record(&conn, &state.rule_engine, &rule_ctx)
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
    context::set_goal(&conn, &text)
        .map(|_| ())
        .map_err(|e| e.to_string())
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
pub fn list_tasks(
    state: State<'_, AppState>,
) -> Result<Vec<sisyphus_core::artifacts::Task>, String> {
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

/// 无极时间轴窗口查询。缩放级别由前端连续计算后传入，后端负责裁剪与聚合。
#[tauri::command]
pub fn query_timeline(
    state: State<'_, AppState>,
    start_ms: i64,
    end_ms: i64,
    detail: String,
    max_items: Option<i64>,
) -> Result<sisyphus_core::timeline::TimelineResponse, String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    sisyphus_core::timeline::query_timeline(
        &conn,
        start_ms,
        end_ms,
        &detail,
        max_items.unwrap_or(1_500),
    )
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

// ── 动态检测规则（Settings 规则列表：查看 / 启停 / 删）────────────────────────

#[tauri::command]
pub fn list_detection_rules(
    state: State<'_, AppState>,
) -> Result<Vec<sisyphus_core::rules::DetectionRule>, String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    sisyphus_core::rules::list_rules(&conn).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_detection_rule_enabled(
    state: State<'_, AppState>,
    id: String,
    enabled: bool,
) -> Result<(), String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    sisyphus_core::rules::set_rule_enabled(&conn, &id, enabled).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_detection_rule(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    sisyphus_core::rules::delete_rule(&conn, &id).map_err(|e| e.to_string())
}

/// 人生看板卡片（看齐 Notion 的本地投影，供「看板」页展示）。
#[tauri::command]
pub fn list_lifeindex(
    state: State<'_, AppState>,
) -> Result<Vec<sisyphus_core::lifeindex::LifeIndexCard>, String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    sisyphus_core::lifeindex::list_cards(&conn).map_err(|e| e.to_string())
}

/// 当前 Agent runtime、可执行文件与只读能力状态。
#[tauri::command]
pub async fn get_agent_runtime_status(
    state: State<'_, AppState>,
) -> Result<agent_runtime::AgentRuntimeStatus, String> {
    let data_dir = state.data_dir.clone();
    tauri::async_runtime::spawn_blocking(move || agent_runtime::status(&data_dir))
        .await
        .map_err(|e| format!("读取 Agent runtime 状态失败: {e}"))
}

/// 保存默认 runtime。`auto` 优先 Pi、不可用时回退 Codex。
#[tauri::command]
pub fn set_agent_runtime(state: State<'_, AppState>, runtime: String) -> Result<(), String> {
    agent_runtime::write_config(&state.data_dir, runtime.trim())
}

/// 主对话 / 宠物共用的只读 Agent 入口。
#[tauri::command]
pub async fn run_agent(
    state: State<'_, AppState>,
    prompt: String,
    runtime: Option<String>,
    run_id: Option<String>,
) -> Result<agent_runtime::AgentRunOutput, String> {
    if prompt.trim().is_empty() {
        return Err("消息不能为空".to_string());
    }
    let data_dir = state.data_dir.clone();
    tauri::async_runtime::spawn_blocking(move || {
        agent_runtime::run_agent(
            &data_dir,
            &prompt,
            runtime.as_deref(),
            run_id.as_deref(),
            agent_runtime::RunMode::Interactive,
        )
    })
    .await
    .map_err(|e| format!("Agent 任务 join 失败: {e}"))?
}

/// 停止主对话当前的 Pi SDK / Codex 子进程。
#[tauri::command]
pub fn cancel_agent_run(run_id: String) -> bool {
    agent_runtime::cancel_agent_run(&run_id)
}

/// Pi JS SDK 连接配置：provider/API 格式 + 自定义 endpoint + key + 模型。
/// 持久化在 app 数据目录，key 只在 Rust 与 SDK sidecar 之间传递。
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct LlmConfig {
    /// pi-ai provider id；openai 默认使用 Responses API。
    #[serde(default)]
    pub format: String,
    /// 自定义 API base URL（留空=用该 provider 默认）
    #[serde(default)]
    pub base_url: String,
    /// 模型名（pi-ai provider 目录内的模型 id）
    #[serde(default)]
    pub model: String,
    /// API key（仅后端保存，不随 get_llm_config 返回）
    #[serde(default)]
    pub api_key: String,
}

fn llm_config_path(state: &AppState) -> PathBuf {
    state.data_dir.join("llm_config.json")
}

fn read_llm_config(state: &AppState) -> LlmConfig {
    std::fs::read_to_string(llm_config_path(state))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// 读 LLM 配置（**不含 key**，只回 has_key 标记）。供前端构建 provider。
#[tauri::command]
pub fn get_llm_config(state: State<'_, AppState>) -> serde_json::Value {
    let c = read_llm_config(&state);
    serde_json::json!({
        "format": c.format,
        "base_url": c.base_url,
        "model": c.model,
        "has_key": !c.api_key.is_empty(),
    })
}

/// 写 LLM 配置（format/base/model/key）。api_key 传空则保留原 key（改其它项不清 key）。
#[tauri::command]
pub fn set_llm_config(
    state: State<'_, AppState>,
    format: String,
    base_url: String,
    model: String,
    api_key: String,
) -> Result<(), String> {
    let mut c = read_llm_config(&state);
    c.format = format.trim().to_string();
    c.base_url = base_url.trim().to_string();
    c.model = model.trim().to_string();
    if c.format.is_empty() {
        return Err("请选择 Provider / API 协议".to_string());
    }
    if c.model.is_empty() {
        return Err("请填写模型 ID".to_string());
    }
    if !api_key.trim().is_empty() {
        c.api_key = api_key.trim().to_string();
    }
    if c.api_key.is_empty()
        && std::env::var("SISYPHUS_LLM_API_KEY")
            .unwrap_or_default()
            .trim()
            .is_empty()
    {
        return Err("请填写 API Key".to_string());
    }
    let json = serde_json::to_string_pretty(&c).map_err(|e| e.to_string())?;
    let path = llm_config_path(&state);
    std::fs::write(&path, json).map_err(|e| format!("写 LLM 配置失败: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
            .map_err(|e| format!("收紧 LLM 配置文件权限失败: {e}"))?;
    }
    Ok(())
}

/// Notion 只读集成：官方 `@notionhq/notion-mcp-server`（NOTION_TOKEN）。
/// 机制保证只读——建议用户在 Notion 侧创建只给 "Read content" 权限的 integration token，
/// 这样即使模型误调用写工具，Notion API 侧也会拒绝（不靠提示词自觉，见 notion-integration.md §2.1）。
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct NotionConfig {
    #[serde(default)]
    pub token: String,
}

fn notion_config_path(state: &AppState) -> PathBuf {
    state.data_dir.join("notion_config.json")
}

fn read_notion_config(state: &AppState) -> NotionConfig {
    std::fs::read_to_string(notion_config_path(state))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// 读 Notion 配置（**不含 token**，只回 has_token 标记）。
#[tauri::command]
pub fn get_notion_config(state: State<'_, AppState>) -> serde_json::Value {
    let c = read_notion_config(&state);
    serde_json::json!({ "has_token": !c.token.is_empty() })
}

/// 写 Notion integration token。传空串清空配置（关闭 Notion 集成）。
#[tauri::command]
pub fn set_notion_config(state: State<'_, AppState>, token: String) -> Result<(), String> {
    let c = NotionConfig {
        token: token.trim().to_string(),
    };
    let json = serde_json::to_string_pretty(&c).map_err(|e| e.to_string())?;
    let path = notion_config_path(&state);
    std::fs::write(&path, json).map_err(|e| format!("写 Notion 配置失败: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
            .map_err(|e| format!("收紧 Notion 配置文件权限失败: {e}"))?;
    }
    Ok(())
}

/// 主动触发：即将到来的待办动作（proactive-triggers.md）。供「今日 · 主动计划」展示
/// 每日自省 / 支线梳理等排程；桌面端由调度器 ticker 播种，安卓暂无（返回空）。
#[tauri::command]
pub fn list_scheduled_actions(
    state: State<'_, AppState>,
) -> Result<Vec<sisyphus_core::scheduler::ScheduledAction>, String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    sisyphus_core::scheduler::list_pending(&conn)
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
            Err(format!(
                "知识 agent 退出码 {:?}\n{stderr}",
                out.status.code()
            ))
        }
    })
    .await
    .map_err(|e| format!("任务 join 失败: {e}"))?
}
