//! 可替换的只读 Agent runtime。
//!
//! 主对话、桌面宠物和主动调度都走这一层，避免各自维护一套 Pi/Codex 能力。
//! Agent 可以读取 Sisyphus / 外部信息源并推理，但不能修改用户内容或本地文件。

use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use serde::{Deserialize, Serialize};

const CONFIG_FILE: &str = "agent_runtime.json";
const LLM_CONFIG_FILE: &str = "llm_config.json";

#[derive(Default)]
struct ActiveRuns {
    pids: HashMap<String, u32>,
    cancelled: HashSet<String>,
}

static ACTIVE_RUNS: OnceLock<Mutex<ActiveRuns>> = OnceLock::new();

fn active_runs() -> &'static Mutex<ActiveRuns> {
    ACTIVE_RUNS.get_or_init(|| Mutex::new(ActiveRuns::default()))
}

struct ActiveRunGuard(Option<String>);

impl ActiveRunGuard {
    fn new(run_id: Option<&str>) -> Result<Self, String> {
        let id = match run_id.map(str::trim).filter(|id| !id.is_empty()) {
            Some(id)
                if id.len() <= 80
                    && id.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '-') =>
            {
                Some(id.to_string())
            }
            Some(_) => return Err("非法的 Agent run id".to_string()),
            None => None,
        };
        Ok(Self(id))
    }

    fn id(&self) -> Option<&str> {
        self.0.as_deref()
    }
}

impl Drop for ActiveRunGuard {
    fn drop(&mut self) {
        let Some(id) = self.0.as_deref() else {
            return;
        };
        if let Ok(mut runs) = active_runs().lock() {
            runs.pids.remove(id);
            runs.cancelled.remove(id);
        }
    }
}

const READ_ONLY_PREAMBLE: &str = r#"你是西西弗斯个人助手。请遵循 sisyphus skill，并保持严格只读：
- 可以读取西西弗斯本地状态、已授权的 Notion 只读信息源和网页；
- 不得创建、编辑、删除 Notion 内容，不维护 Inbox/NOW/“下一项”；
- 不得修改本地文件、数据库、任务、目标或知识库；
- 给用户的建议最多聚焦一件当下值得做的事，依据不足时明确说不知道。
"#;

const INTERACTIVE_PREAMBLE: &str = r#"你是西西弗斯个人助手。请遵循 sisyphus skill。当前是用户主动发起的交互会话：
- 你可以调用西西弗斯本地工具去真正落地用户意图：capture 记录、set_goal 设目标、
  add_monitored_app 纳入监控、create_detection_rule 建检测规则、propose_intents/accept_intent
  落任务/提醒、write_knowledge_note 写知识库、upsert_lifeindex_card 更新看板等；
- 落库前用一句话向用户复述确认（认可 / 修改 / 忽略），只提最小下一步，不生成任务海；
- 外部信息源（如 Notion）始终只读：只读取参考，绝不创建 / 编辑 / 删除用户的 Notion 内容；
- 语气关心不评判、具体（引用真实时长 / 目标 / 数据）、不羞辱不说教。
"#;

const LIFEINDEX_PREAMBLE: &str = r#"你是西西弗斯的看板维护助手（LifeIndex 刷新任务）。目标：让本地人生看板看齐用户的 Notion。
- 只读参考：读取用户已授权的 Notion（长期目标 / 短期 Todo / 研究问题 / 个人发展等）与本地 query_context；
- 唯一允许的写操作：用 upsert_lifeindex_card 把提炼出的卡片写进本地看板（section+title 幂等），
  source_ref 填 Notion 溯源；必要时 delete_lifeindex_card 清理已失效的卡片；
- 严禁修改 Notion，严禁调用其它本地写工具（set_goal / 任务 / 规则 / 知识库都不要动）；
- 卡片正文简洁：每张 3 行内，聚焦"是什么 + 当前状态"。完成后简述本次更新了哪些分区。
"#;

/// Agent 运行模式。决定只读门禁与系统前言。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunMode {
    /// 用户主动发起（主对话 / 宠物）：可写本地 artifact，外部源仍只读。
    Interactive,
    /// 定时 / 规则触发的主动任务：严格只读，只产出一条建议。
    Proactive,
    /// 看板刷新：只读参考 Notion + 本地上下文，仅允许写本地看板卡片（不动其它本地状态、不写 Notion）。
    LifeIndex,
}

impl RunMode {
    fn read_only(self) -> bool {
        matches!(self, RunMode::Proactive)
    }

    fn lifeindex_only(self) -> bool {
        matches!(self, RunMode::LifeIndex)
    }

    fn preamble(self) -> &'static str {
        match self {
            RunMode::Interactive => INTERACTIVE_PREAMBLE,
            RunMode::Proactive => READ_ONLY_PREAMBLE,
            RunMode::LifeIndex => LIFEINDEX_PREAMBLE,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRuntimeConfig {
    /// auto | pi | codex
    #[serde(default = "default_runtime")]
    pub runtime: String,
}

fn default_runtime() -> String {
    "auto".to_string()
}

impl Default for AgentRuntimeConfig {
    fn default() -> Self {
        Self {
            runtime: default_runtime(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeInfo {
    pub id: String,
    pub available: bool,
    /// 对应 runtime 已安装且有可实际运行的认证 / 模型。
    pub ready: bool,
    pub path: Option<String>,
    pub version: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AgentRuntimeStatus {
    pub configured: String,
    pub resolved: Option<String>,
    pub pi: RuntimeInfo,
    pub codex: RuntimeInfo,
    pub skill_path: String,
    pub extensions: Vec<String>,
    pub read_only: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct AgentRunOutput {
    pub runtime: String,
    pub text: String,
    pub elapsed_ms: u128,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct PiLlmConfig {
    #[serde(default)]
    format: String,
    #[serde(default)]
    base_url: String,
    #[serde(default)]
    model: String,
    #[serde(default)]
    api_key: String,
}

impl PiLlmConfig {
    fn api_key(&self) -> String {
        if self.api_key.trim().is_empty() {
            std::env::var("SISYPHUS_LLM_API_KEY").unwrap_or_default()
        } else {
            self.api_key.clone()
        }
    }

    fn ready(&self) -> bool {
        !self.format.trim().is_empty()
            && !self.model.trim().is_empty()
            && !self.api_key().trim().is_empty()
    }
}

pub fn read_config(data_dir: &Path) -> AgentRuntimeConfig {
    std::fs::read_to_string(data_dir.join(CONFIG_FILE))
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

pub fn write_config(data_dir: &Path, runtime: &str) -> Result<(), String> {
    if !matches!(runtime, "auto" | "pi" | "codex") {
        return Err(format!("未知 Agent runtime: {runtime}"));
    }
    let cfg = AgentRuntimeConfig {
        runtime: runtime.to_string(),
    };
    let raw = serde_json::to_string_pretty(&cfg).map_err(|e| e.to_string())?;
    std::fs::write(data_dir.join(CONFIG_FILE), raw)
        .map_err(|e| format!("保存 Agent runtime 配置失败: {e}"))
}

pub fn status(data_dir: &Path) -> AgentRuntimeStatus {
    let cfg = read_config(data_dir);
    let node_path = find_executable("SISYPHUS_NODE_BIN", "node");
    let pi_runner = pi_sdk_runner();
    let codex_path = find_executable("SISYPHUS_CODEX_BIN", "codex");
    let pi = pi_sdk_runtime_info(data_dir, node_path, &pi_runner);
    let codex = runtime_info("codex", codex_path);
    let resolved = resolve_runtime(&cfg.runtime, &pi, &codex).ok();
    AgentRuntimeStatus {
        configured: cfg.runtime,
        resolved,
        pi,
        codex,
        skill_path: skill_dir().to_string_lossy().to_string(),
        extensions: vec![
            "pi-mcp-adapter".to_string(),
            "pi-web-access".to_string(),
            "@gotgenes/pi-permission-system".to_string(),
        ],
        read_only: true,
    }
}

pub fn run_agent(
    data_dir: &Path,
    prompt: &str,
    preferred: Option<&str>,
    run_id: Option<&str>,
    mode: RunMode,
) -> Result<AgentRunOutput, String> {
    let run_guard = ActiveRunGuard::new(run_id)?;
    if is_cancelled(run_guard.id()) {
        return Err("Agent 请求已停止".to_string());
    }
    let cfg = read_config(data_dir);
    let node_path = find_executable("SISYPHUS_NODE_BIN", "node");
    let pi_runner = pi_sdk_runner();
    let codex_path = find_executable("SISYPHUS_CODEX_BIN", "codex");
    let pi = pi_sdk_runtime_info(data_dir, node_path.clone(), &pi_runner);
    let codex = runtime_info("codex", codex_path.clone());
    let requested = preferred.filter(|v| *v != "auto").unwrap_or(&cfg.runtime);
    let candidates = runtime_candidates(requested, &pi, &codex)?;
    let full_prompt = format!("{}\n用户请求：\n{}", mode.preamble(), prompt.trim());
    let started = Instant::now();
    let mut errors = Vec::new();

    for (index, runtime) in candidates.iter().enumerate() {
        match run_with_runtime(
            data_dir,
            runtime,
            node_path.as_deref(),
            &pi_runner,
            codex_path.as_deref(),
            &full_prompt,
            run_guard.id(),
            mode,
        ) {
            Ok(text) => {
                if is_cancelled(run_guard.id()) {
                    return Err("Agent 请求已停止".to_string());
                }
                return Ok(AgentRunOutput {
                    runtime: runtime.clone(),
                    text,
                    elapsed_ms: started.elapsed().as_millis(),
                });
            }
            Err(error) => {
                if is_cancelled(run_guard.id()) {
                    return Err("Agent 请求已停止".to_string());
                }
                errors.push(error.clone());
                if index + 1 < candidates.len() {
                    log::warn!("Agent runtime {runtime} 失败，auto 将回退下一个 runtime：{error}");
                }
            }
        }
    }

    if errors.len() == 1 {
        Err(errors.remove(0))
    } else {
        Err(format!(
            "所有可用 Agent runtime 均失败：\n- {}",
            errors.join("\n- ")
        ))
    }
}

fn run_with_runtime(
    data_dir: &Path,
    runtime: &str,
    node_path: Option<&Path>,
    pi_runner: &Path,
    codex_path: Option<&Path>,
    full_prompt: &str,
    run_id: Option<&str>,
    mode: RunMode,
) -> Result<String, String> {
    let out = match runtime {
        "pi" => run_pi_sdk(data_dir, node_path, pi_runner, full_prompt, run_id, mode)?,
        "codex" => {
            let bin = codex_path.ok_or_else(|| "未找到 Codex CLI".to_string())?;
            let mut command = Command::new(bin);
            command.current_dir(project_root()).arg("exec");
            // Codex 用户配置里可能还有能写外部系统的 MCP。App runtime 只保留 sisyphus
            // 本地 server，其余逐个禁用；外部内容源（如 Notion）永远只读。
            for server in codex_config_mcp_servers() {
                if server != "sisyphus" {
                    command.arg("-c").arg(disable_mcp_override(&server));
                }
            }
            command.args([
                "--disable",
                "plugins",
                "--disable",
                "remote_plugin",
                "--disable",
                "plugin_sharing",
            ]);
            // Notion 只读集成：官方 notion-mcp-server（npx），只在配置了 token 时接入。
            // 读工具，两种模式都放行（只读边界由 Notion 侧的 integration 权限机制保证，
            // 见 notion-integration.md §2.1：建议 token 只给 "Read content" 权限）。
            if let Some(token) = read_notion_token(data_dir) {
                command
                    .arg("-c")
                    .arg("mcp_servers.notion.command=\"npx\"")
                    .arg("-c")
                    .arg("mcp_servers.notion.args=[\"-y\",\"@notionhq/notion-mcp-server\"]")
                    .arg("-c")
                    .arg(format!(
                        "mcp_servers.notion.env.NOTION_TOKEN=\"{}\"",
                        escape_toml_string(&token)
                    ));
            }
            // 主动模式：给 sisyphus MCP 注入 SISYPHUS_READ_ONLY 硬门禁；
            // 看板刷新模式：注入 SISYPHUS_LIFEINDEX_ONLY（仅看板可写）；
            // 交互模式：不注入，用户可经确认后驱动本地写工具（set_goal/建规则/写知识等）。
            if mode.read_only() {
                command
                    .arg("-c")
                    .arg("mcp_servers.sisyphus.env.SISYPHUS_READ_ONLY=\"1\"");
            } else if mode.lifeindex_only() {
                command
                    .arg("-c")
                    .arg("mcp_servers.sisyphus.env.SISYPHUS_LIFEINDEX_ONLY=\"1\"");
            }
            command
                .args([
                    "-c",
                    "model_reasoning_effort=\"low\"",
                    "--sandbox",
                    "read-only",
                    "--ephemeral",
                    "--skip-git-repo-check",
                    "--color",
                    "never",
                    "-C",
                ])
                .arg(project_root())
                .arg(format!("$sisyphus\n\n{full_prompt}"));
            run_process(&mut command, None, run_id, "Codex")?
        }
        _ => unreachable!(),
    };

    let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
    if !out.status.success() {
        let detail = if stderr.is_empty() { stdout } else { stderr };
        return Err(format!(
            "{runtime} 运行失败（{:?}）：{detail}",
            out.status.code()
        ));
    }
    if stdout.is_empty() {
        return Err(format!(
            "{runtime} 未返回文本{}",
            if stderr.is_empty() {
                ""
            } else {
                "（详情见应用日志）"
            }
        ));
    }
    Ok(stdout)
}

fn runtime_candidates(
    requested: &str,
    pi: &RuntimeInfo,
    codex: &RuntimeInfo,
) -> Result<Vec<String>, String> {
    match requested {
        "auto" | "" => {
            let candidates = [pi, codex]
                .into_iter()
                .filter(|runtime| runtime.ready)
                .map(|runtime| runtime.id.clone())
                .collect::<Vec<_>>();
            if candidates.is_empty() {
                Err("没有可用的 Pi JS SDK 或 Codex runtime".to_string())
            } else {
                Ok(candidates)
            }
        }
        _ => resolve_runtime(requested, pi, codex).map(|runtime| vec![runtime]),
    }
}

fn resolve_runtime(
    requested: &str,
    pi: &RuntimeInfo,
    codex: &RuntimeInfo,
) -> Result<String, String> {
    match requested {
        "pi" if pi.ready => Ok("pi".to_string()),
        "codex" if codex.ready => Ok("codex".to_string()),
        "pi" if pi.available => {
            Err("Pi JS SDK 已安装，但尚未在设置页完整配置 Provider、Model 和 API Key".to_string())
        }
        "pi" => Err("已选择 Pi，但 Pi JS SDK runner 或 Node.js 不可用".to_string()),
        "codex" => Err("已选择 Codex，但 Codex CLI 当前不可用".to_string()),
        "auto" | "" => {
            if pi.ready {
                Ok("pi".to_string())
            } else if codex.ready {
                Ok("codex".to_string())
            } else {
                Err("没有可用的 Pi JS SDK 或 Codex runtime".to_string())
            }
        }
        other => Err(format!("未知 Agent runtime: {other}")),
    }
}

fn runtime_info(id: &str, path: Option<PathBuf>) -> RuntimeInfo {
    let version = path.as_ref().and_then(|bin| {
        Command::new(bin)
            .arg("--version")
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .filter(|v| !v.is_empty())
    });
    let ready = path.is_some();
    RuntimeInfo {
        id: id.to_string(),
        available: path.is_some(),
        ready,
        path: path.map(|p| p.to_string_lossy().to_string()),
        version,
    }
}

fn read_pi_llm_config(data_dir: &Path) -> PiLlmConfig {
    std::fs::read_to_string(data_dir.join(LLM_CONFIG_FILE))
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

const NOTION_CONFIG_FILE: &str = "notion_config.json";

#[derive(Debug, Default, Deserialize)]
struct NotionConfigFile {
    #[serde(default)]
    token: String,
}

/// 读用户在设置页保存的 Notion integration token（非空才 Some）。
fn read_notion_token(data_dir: &Path) -> Option<String> {
    let cfg: NotionConfigFile = std::fs::read_to_string(data_dir.join(NOTION_CONFIG_FILE))
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default();
    let token = cfg.token.trim().to_string();
    (!token.is_empty()).then_some(token)
}

/// 转义 codex `-c key="value"` 里的双引号 TOML 字符串值。
fn escape_toml_string(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

fn pi_sdk_runtime_info(data_dir: &Path, node_path: Option<PathBuf>, runner: &Path) -> RuntimeInfo {
    let sdk_package = project_root()
        .join("sisyphus")
        .join("node_modules")
        .join("@earendil-works")
        .join("pi-coding-agent")
        .join("package.json");
    let sdk_version = std::fs::read_to_string(&sdk_package)
        .ok()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
        .and_then(|value| value.get("version")?.as_str().map(str::to_string));
    let available = node_path.is_some() && runner.is_file() && sdk_package.is_file();
    RuntimeInfo {
        id: "pi".to_string(),
        available,
        ready: available && read_pi_llm_config(data_dir).ready(),
        path: available.then(|| runner.to_string_lossy().to_string()),
        version: sdk_version.map(|version| format!("Pi JS SDK {version}")),
    }
}

fn run_pi_sdk(
    data_dir: &Path,
    node_path: Option<&Path>,
    runner: &Path,
    full_prompt: &str,
    run_id: Option<&str>,
    mode: RunMode,
) -> Result<std::process::Output, String> {
    let node = node_path.ok_or_else(|| "未找到 Node.js，无法运行 Pi JS SDK".to_string())?;
    if !runner.is_file() {
        return Err(format!("Pi JS SDK runner 不存在：{}", runner.display()));
    }
    let config = read_pi_llm_config(data_dir);
    if !config.ready() {
        return Err("Pi JS SDK 未配置：请在设置页填写 Provider、Model 和 API Key".to_string());
    }

    let mut command = Command::new(node);
    command
        .current_dir(project_root())
        .arg(runner)
        .env("SISYPHUS_PROJECT_DIR", project_root())
        .env("SISYPHUS_PI_AGENT_DIR", data_dir.join("pi-sdk"))
        .env("SISYPHUS_SKILL_DIR", skill_dir())
        .env("SISYPHUS_PI_FORMAT", config.format.trim())
        .env("SISYPHUS_PI_BASE_URL", config.base_url.trim())
        .env("SISYPHUS_PI_MODEL", config.model.trim())
        .env("SISYPHUS_PI_API_KEY", config.api_key())
        // 批次 B：Pi 运行脚本 spawn 同一个 sisyphus-mcp，拿到与 Codex 相同的工具面。
        // 让它开同一库/同一 vault，并按模式决定只读门禁（外部内容源永远只读）。
        .env("SISYPHUS_MCP_BIN", mcp_bin())
        .env("SISYPHUS_DB", db_file(data_dir))
        .env("SISYPHUS_VAULT", vault_dir(data_dir))
        .env(
            "SISYPHUS_READ_ONLY",
            if mode.read_only() { "1" } else { "0" },
        )
        .env(
            "SISYPHUS_LIFEINDEX_ONLY",
            if mode.lifeindex_only() { "1" } else { "0" },
        )
        // Notion 只读集成：runner 侧会另起一个 npx notion-mcp-server 客户端合并进工具面。
        .env(
            "SISYPHUS_NOTION_TOKEN",
            read_notion_token(data_dir).unwrap_or_default(),
        )
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    run_process(&mut command, Some(full_prompt), run_id, "Pi JS SDK")
}

fn run_process(
    command: &mut Command,
    input: Option<&str>,
    run_id: Option<&str>,
    label: &str,
) -> Result<std::process::Output, String> {
    command
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|e| format!("启动 {label} 失败: {e}"))?;
    if let Some(id) = run_id {
        if let Ok(mut runs) = active_runs().lock() {
            runs.pids.insert(id.to_string(), child.id());
        }
        if is_cancelled(Some(id)) {
            terminate_pid(child.id());
        }
    }

    let write_result = if let Some(input) = input {
        child
            .stdin
            .take()
            .ok_or_else(|| format!("无法写入 {label} prompt"))?
            .write_all(input.as_bytes())
    } else {
        Ok(())
    };
    let output = child
        .wait_with_output()
        .map_err(|e| format!("等待 {label} 失败: {e}"))?;
    if let Some(id) = run_id {
        if let Ok(mut runs) = active_runs().lock() {
            runs.pids.remove(id);
        }
    }
    if let Err(error) = write_result {
        if !is_cancelled(run_id) {
            return Err(format!("写入 {label} prompt 失败: {error}"));
        }
    }
    Ok(output)
}

fn is_cancelled(run_id: Option<&str>) -> bool {
    let Some(id) = run_id else {
        return false;
    };
    active_runs()
        .lock()
        .map(|runs| runs.cancelled.contains(id))
        .unwrap_or(false)
}

pub fn cancel_agent_run(run_id: &str) -> bool {
    let id = run_id.trim();
    if id.is_empty() || id.len() > 80 {
        return false;
    }
    let pid = active_runs().lock().ok().and_then(|mut runs| {
        runs.cancelled.insert(id.to_string());
        runs.pids.get(id).copied()
    });
    if let Some(pid) = pid {
        terminate_pid(pid);
    }
    true
}

#[cfg(unix)]
fn terminate_pid(pid: u32) {
    // SAFETY: pid 来自当前宿主刚启动且仍在 registry 中的子进程。
    unsafe {
        libc::kill(pid as libc::pid_t, libc::SIGTERM);
    }
}

#[cfg(windows)]
fn terminate_pid(pid: u32) {
    let _ = Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .status();
}

#[cfg(not(any(unix, windows)))]
fn terminate_pid(_pid: u32) {}

fn find_executable(env_name: &str, name: &str) -> Option<PathBuf> {
    if let Ok(value) = std::env::var(env_name) {
        let path = PathBuf::from(value);
        if path.is_file() {
            return Some(path);
        }
    }
    for base in ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"] {
        let path = Path::new(base).join(name);
        if path.is_file() {
            return Some(path);
        }
    }
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|dir| dir.join(name))
            .find(|path| path.is_file())
    })
}

fn pi_sdk_runner() -> PathBuf {
    if let Ok(path) = std::env::var("SISYPHUS_PI_SDK_RUNNER") {
        return PathBuf::from(path);
    }
    project_root()
        .join("sisyphus")
        .join("scripts")
        .join("pi-agent-runtime.mjs")
}

/// 读取用户级 / 项目级 config.toml 以及项目 `.mcp.json` 中配置的 MCP 名称。
/// 外部系统连接器都逐个禁用，只留下 Sisyphus 自己的只读 server。
fn codex_config_mcp_servers() -> Vec<String> {
    let codex_home = std::env::var("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|_| std::env::var("HOME").map(|home| PathBuf::from(home).join(".codex")));
    let paths = [
        codex_home.ok().map(|dir| dir.join("config.toml")),
        Some(project_root().join(".codex").join("config.toml")),
    ];
    let mut servers = paths
        .into_iter()
        .flatten()
        .filter_map(|path| std::fs::read_to_string(path).ok())
        .flat_map(|raw| parse_mcp_servers(&raw))
        .collect::<Vec<_>>();
    if let Ok(raw) = std::fs::read_to_string(project_root().join(".mcp.json")) {
        servers.extend(parse_mcp_json_servers(&raw));
    }
    servers.sort();
    servers.dedup();
    servers
}

fn parse_mcp_json_servers(raw: &str) -> Vec<String> {
    serde_json::from_str::<serde_json::Value>(raw)
        .ok()
        .and_then(|value| value.get("mcpServers")?.as_object().cloned())
        .into_iter()
        .flatten()
        .map(|(name, _)| name)
        .filter(|name| {
            name.chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_'))
        })
        .collect()
}

fn parse_mcp_servers(raw: &str) -> Vec<String> {
    raw.lines()
        .map(str::trim)
        .filter_map(|line| line.strip_prefix("[mcp_servers.")?.strip_suffix(']'))
        .filter(|name| {
            !name.contains('.')
                && name
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_'))
        })
        .map(str::to_string)
        .collect()
}

fn disable_mcp_override(server: &str) -> String {
    format!("mcp_servers.{server}.enabled=false")
}

/// 应用启动时由 lib.rs 调用：把运行期解析出的资源路径写进进程环境，供下面各 `*_dir/*_bin`
/// 读取。**这修掉了打包/移动仓库后仍指向编译期 `CARGO_MANIFEST_DIR` 的 bug**——release 构建
/// 不再依赖编译期路径，而由宿主在启动时用 `resource_dir()` / sidecar 位置显式注入。
/// 仅设置尚未被外部显式覆盖的变量（dev 里 `SISYPHUS_*` 若已设则尊重）。
pub fn init_paths(resource_dir: Option<&Path>, mcp_bin_path: Option<&Path>) {
    fn set_if_unset(key: &str, value: &Path) {
        let has = std::env::var(key).map(|v| !v.trim().is_empty()).unwrap_or(false);
        if !has {
            std::env::set_var(key, value);
        }
    }
    // 只在 release 从 resource_dir 注入。dev（debug）一律走 project_root() 的 CARGO_MANIFEST_DIR
    // 回退（= 仓库根，node_modules/scripts/skills 都在那），避免被 `tauri dev` 拷到 target/debug 的
    // 部分资源（无 node_modules）误导——那正是"已选择 Pi 但 runner/Node 不可用"的根因。
    #[cfg(not(debug_assertions))]
    if let Some(rd) = resource_dir {
        set_if_unset("SISYPHUS_PROJECT_DIR", rd);
        let skill = rd.join("skills").join("sisyphus");
        if skill.is_dir() {
            set_if_unset("SISYPHUS_SKILL_DIR", &skill);
        }
        let runner = rd.join("scripts").join("pi-agent-runtime.mjs");
        if runner.is_file() {
            set_if_unset("SISYPHUS_PI_SDK_RUNNER", &runner);
        }
    }
    #[cfg(debug_assertions)]
    let _ = resource_dir;

    if let Some(mcp) = mcp_bin_path {
        set_if_unset("SISYPHUS_MCP_BIN", mcp);
    }
}

pub fn project_root() -> PathBuf {
    if let Ok(path) = std::env::var("SISYPHUS_PROJECT_DIR") {
        if !path.trim().is_empty() {
            return PathBuf::from(path);
        }
    }
    // dev（debug）便利：从源码树推出仓库根。release 不烘焙编译期路径——
    // 若 init_paths 未注入 env，则退回到可执行文件所在目录（打包后稳定）。
    #[cfg(debug_assertions)]
    {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")))
    }
    #[cfg(not(debug_assertions))]
    {
        std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(Path::to_path_buf))
            .unwrap_or_default()
    }
}

fn skill_dir() -> PathBuf {
    if let Ok(path) = std::env::var("SISYPHUS_SKILL_DIR") {
        if !path.trim().is_empty() {
            return PathBuf::from(path);
        }
    }
    project_root().join("skills").join("sisyphus")
}

/// `sisyphus-mcp` 二进制路径：env 优先 → PATH / 常见前缀 → dev 的 cargo target → 兜底裸名。
fn mcp_bin() -> PathBuf {
    if let Some(p) = find_executable("SISYPHUS_MCP_BIN", "sisyphus-mcp") {
        return p;
    }
    #[cfg(debug_assertions)]
    {
        let dev = project_root()
            .join("sisyphus")
            .join("src-tauri")
            .join("target")
            .join("debug")
            .join("sisyphus-mcp");
        if dev.is_file() {
            return dev;
        }
    }
    PathBuf::from("sisyphus-mcp")
}

/// 与 Tauri App 一致的本地库路径：`{app_data_dir}/sisyphus.db`。
fn db_file(data_dir: &Path) -> PathBuf {
    data_dir.join("sisyphus.db")
}

/// 知识库 vault：`SISYPHUS_VAULT` 覆盖，否则 `{app_data_dir}/vault`（与 lib.rs 一致）。
fn vault_dir(data_dir: &Path) -> PathBuf {
    std::env::var("SISYPHUS_VAULT")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| data_dir.join("vault"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runtime(id: &str, available: bool, ready: bool) -> RuntimeInfo {
        RuntimeInfo {
            id: id.to_string(),
            available,
            ready,
            path: None,
            version: None,
        }
    }

    #[test]
    fn auto_tries_pi_then_codex() {
        let pi = runtime("pi", true, true);
        let codex = runtime("codex", true, true);

        assert_eq!(
            runtime_candidates("auto", &pi, &codex).unwrap(),
            vec!["pi", "codex"]
        );
    }

    #[test]
    fn explicit_runtime_does_not_add_fallback() {
        let pi = runtime("pi", true, true);
        let codex = runtime("codex", true, true);

        assert_eq!(runtime_candidates("pi", &pi, &codex).unwrap(), vec!["pi"]);
        assert_eq!(
            runtime_candidates("codex", &pi, &codex).unwrap(),
            vec!["codex"]
        );
    }

    #[test]
    fn codex_mcp_override_uses_unquoted_dotted_key() {
        assert_eq!(
            disable_mcp_override("node_repl"),
            "mcp_servers.node_repl.enabled=false"
        );
        assert_eq!(
            disable_mcp_override("mihoyo-soc-mcp"),
            "mcp_servers.mihoyo-soc-mcp.enabled=false"
        );
    }

    #[test]
    fn parses_only_top_level_mcp_server_tables() {
        let raw = r#"
[mcp_servers.sisyphus]
command = "sisyphus-mcp"

[mcp_servers.sisyphus.env]
SISYPHUS_READ_ONLY = "1"

[mcp_servers.mihoyo-soc-mcp]
url = "https://example.invalid"
"#;

        assert_eq!(parse_mcp_servers(raw), vec!["sisyphus", "mihoyo-soc-mcp"]);
    }

    #[test]
    fn parses_project_mcp_json_servers() {
        let raw = r#"{
          "mcpServers": {
            "sisyphus": { "command": "sisyphus-mcp" },
            "supabase": { "type": "http", "url": "https://example.invalid" },
            "invalid.name": {}
          }
        }"#;

        assert_eq!(parse_mcp_json_servers(raw), vec!["sisyphus", "supabase"]);
    }
}
