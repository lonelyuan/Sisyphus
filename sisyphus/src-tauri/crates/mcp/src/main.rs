//! Sisyphus 反思平面 MCP server（stdio 传输）。
//!
//! 由 Codex / Claude Code 作为子进程拉起，复用 `sisyphus-core` 的逻辑，
//! 打开与 Tauri App 同一个 `sisyphus.db`（WAL + busy_timeout 支持跨进程并发）。
//! 即使桌面 App 未运行也可独立工作（见 docs/spec/architecture.md §1.2、§4）。
//!
//! ⚠️ stdout 是 MCP 协议通道，任何日志/调试输出必须走 stderr。

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rmcp::{
    handler::server::router::tool::ToolRouter,
    handler::server::wrapper::Parameters,
    model::{Implementation, ServerCapabilities, ServerInfo},
    schemars, tool, tool_handler, tool_router,
    transport::stdio,
    ServerHandler, ServiceExt,
};
use rusqlite::Connection;

// ── 工具入参 ──────────────────────────────────────────────────────────────────

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct CaptureReq {
    /// 要记录的自然语言内容（目标 / 想法 / 待办 / 情绪等）
    pub text: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetGoalReq {
    /// 今日目标文本
    pub text: String,
}

// ── Server ────────────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct SisyphusServer {
    tool_router: ToolRouter<SisyphusServer>,
    db: Arc<Mutex<Connection>>,
    user_id: String,
    device_id: String,
}

#[tool_router]
impl SisyphusServer {
    #[tool(
        description = "记录一句自然语言（目标/想法/待办/情绪）到 Sisyphus 的事件日志。零压记录入口，返回 capture_id。"
    )]
    fn capture(&self, Parameters(CaptureReq { text }): Parameters<CaptureReq>) -> String {
        let conn = self.db.lock().unwrap();
        match sisyphus_core::capture_text(&conn, &self.user_id, &self.device_id, &text) {
            Ok(id) => format!("captured: {id}"),
            Err(e) => format!("error: {e}"),
        }
    }

    #[tool(
        description = "查询今日上下文（日期、今日目标及状态、娱乐时长、干预次数、近期干预），返回 JSON。构建提醒/复盘前先调用。"
    )]
    fn query_context(&self) -> String {
        let conn = self.db.lock().unwrap();
        match sisyphus_core::context::today_context(&conn, &self.user_id) {
            Ok(ctx) => serde_json::to_string_pretty(&ctx)
                .unwrap_or_else(|e| format!("error serializing: {e}")),
            Err(e) => format!("error: {e}"),
        }
    }

    #[tool(description = "返回今日最小行动（1–3 条）。为空表示今天还没设目标，应引导用户设一个。")]
    fn today_actions(&self) -> String {
        let conn = self.db.lock().unwrap();
        match sisyphus_core::context::today_actions(&conn) {
            Ok(actions) => serde_json::to_string(&actions)
                .unwrap_or_else(|e| format!("error serializing: {e}")),
            Err(e) => format!("error: {e}"),
        }
    }

    #[tool(description = "设置/更新今日目标。同一天重复调用视为修改。返回 goal id。")]
    fn set_goal(&self, Parameters(SetGoalReq { text }): Parameters<SetGoalReq>) -> String {
        let conn = self.db.lock().unwrap();
        match sisyphus_core::context::set_goal(&conn, &text) {
            Ok(id) => format!("goal set: {id}"),
            Err(e) => format!("error: {e}"),
        }
    }
}

#[tool_handler]
impl ServerHandler for SisyphusServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            server_info: Implementation {
                name: "sisyphus-mcp".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                ..Default::default()
            },
            instructions: Some(
                "Sisyphus 反思平面。capture 记录一句话；query_context 查今日上下文；\
                 today_actions 取今日最小行动；set_goal 设今日目标。"
                    .to_string(),
            ),
            ..Default::default()
        }
    }
}

impl SisyphusServer {
    pub fn new() -> anyhow::Result<Self> {
        let path = db_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let path_str = path
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("db path not utf-8: {:?}", path))?;
        let conn = sisyphus_core::db::open(path_str)
            .map_err(|e| anyhow::anyhow!("open db {path_str}: {e}"))?;
        // 跨进程并发：与 Tauri App 同开一库，WAL + busy_timeout 处理写争用。
        conn.busy_timeout(Duration::from_secs(5))
            .map_err(|e| anyhow::anyhow!("busy_timeout: {e}"))?;
        Ok(Self {
            tool_router: Self::tool_router(),
            db: Arc::new(Mutex::new(conn)),
            user_id: "local-user".to_string(),
            device_id: "agent-mcp".to_string(),
        })
    }
}

/// 解析与 Tauri App 一致的 DB 路径：`{data_dir}/com.sisyphus/sisyphus.db`。
/// 可用环境变量 `SISYPHUS_DB` 覆盖（便于测试）。
fn db_path() -> PathBuf {
    if let Ok(p) = std::env::var("SISYPHUS_DB") {
        return PathBuf::from(p);
    }
    let base = dirs::data_dir().expect("无法定位系统 data 目录");
    base.join("com.sisyphus").join("sisyphus.db")
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let server = SisyphusServer::new()?;
    let service = server.serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
