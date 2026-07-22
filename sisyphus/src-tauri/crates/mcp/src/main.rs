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

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ListCapturesReq {
    /// 只列还没生成意图候选的 capture（默认 true，即「收件箱」视图）
    #[serde(default)]
    pub unprocessed: Option<bool>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct IntentCandidateReq {
    /// 意图种类：goal | task | reminder | note
    pub kind: String,
    /// 候选内容对象。task:{title, due_ms?, priority?, note?}；reminder:{text, remind_at_ms(epoch ms), recurrence?}；note:{title?, body, tags?}；goal:{text}
    pub proposed: serde_json::Value,
    /// 置信度 0–1（可选）
    #[serde(default)]
    pub confidence: Option<f64>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ProposeIntentsReq {
    /// 目标 capture 的 event_id（来自 capture 返回或 list_captures）
    pub capture_event_id: String,
    /// Codex 分类后生成的一个或多个意图候选
    pub candidates: Vec<IntentCandidateReq>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct AcceptIntentReq {
    /// 意图候选 id（来自 propose_intents 返回）
    pub intent_id: String,
    /// 可选：覆盖候选字段的 JSON 对象（用户在对话里就地修改）
    #[serde(default)]
    pub edits: Option<serde_json::Value>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct IntentIdReq {
    /// 意图候选 id
    pub intent_id: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct IngestDocumentReq {
    /// 原始素材：文本内容，或 URL / 文件路径引用（由你后续加工）
    pub content: String,
    /// 素材标题（可选）
    #[serde(default)]
    pub title: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct WriteKnowledgeNoteReq {
    /// 知识卡片标题（同时决定 vault 文件名，同名视为更新）
    pub title: String,
    /// 卡片正文（Markdown；你加工出的摘要/概念，5 行以内摘要 + 要点）
    pub body: String,
    /// 标签
    #[serde(default)]
    pub tags: Vec<String>,
    /// 关联的其它知识卡片标题（渲染为 Obsidian [[wikilink]]）
    #[serde(default)]
    pub links: Vec<String>,
    /// 来源 url / 引用
    #[serde(default)]
    pub sources: Vec<String>,
    /// 可选分类子目录（话题领域），如 "web-security" 或 "work-mihoyo/state"；省略=vault 根。卡片落到 {folder}/{标题}.md
    #[serde(default)]
    pub folder: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SaveSourceReq {
    /// 原文标题（决定文件名）
    pub title: String,
    /// 逐字原文内容（Markdown / 纯文本）
    pub content: String,
    /// 来源 URL（可选）
    #[serde(default)]
    pub url: Option<String>,
    /// 素材类型（可选），如 article / vuln-report / doc / paper
    #[serde(default)]
    pub source_type: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SearchKnowledgeReq {
    /// 关键词（匹配标题 / 标签 / 路径）
    pub query: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct DeleteNoteReq {
    /// 要删除的卡片**标题**（优先精确匹配）或**相对路径**（如 kb/personal/x.md）
    pub title: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct AddMonitoredReq {
    /// app 的 bundle id（macOS）或包名（Android），如 com.ss.android.ugc.aweme
    pub id: String,
    /// 分类：entertainment.video | entertainment.game | entertainment.social | entertainment.news
    pub category: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct RemoveMonitoredReq {
    /// 要移除的 bundle id / 包名
    pub id: String,
}

// ── Server ────────────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct SisyphusServer {
    tool_router: ToolRouter<SisyphusServer>,
    db: Arc<Mutex<Connection>>,
    vault_dir: PathBuf,
    user_id: String,
    device_id: String,
}

#[tool_router]
impl SisyphusServer {
    #[tool(
        description = "记录一句自然语言（目标/想法/待办/情绪）到 Sisyphus 的事件日志。零压记录入口，返回 capture_id。"
    )]
    fn capture(&self, Parameters(CaptureReq { text }): Parameters<CaptureReq>) -> Result<String, String> {
        let conn = self.db.lock().map_err(|_| "db 锁中毒".to_string())?;
        let id = sisyphus_core::capture_text(&conn, &self.user_id, &self.device_id, &text)
            .map_err(|e| e.to_string())?;
        Ok(format!("captured: {id}"))
    }

    #[tool(
        description = "查询今日上下文（日期、今日目标及状态、娱乐时长、干预次数、未完成任务、到期提醒、近期干预），返回 JSON。构建提醒/复盘前先调用。"
    )]
    fn query_context(&self) -> Result<String, String> {
        let conn = self.db.lock().map_err(|_| "db 锁中毒".to_string())?;
        let ctx = sisyphus_core::context::today_context(&conn, &self.user_id)
            .map_err(|e| e.to_string())?;
        serde_json::to_string_pretty(&ctx).map_err(|e| format!("序列化失败: {e}"))
    }

    #[tool(description = "返回今日最小行动（1–3 条）。为空表示今天还没设目标，应引导用户设一个。")]
    fn today_actions(&self) -> Result<String, String> {
        let conn = self.db.lock().map_err(|_| "db 锁中毒".to_string())?;
        let actions = sisyphus_core::context::today_actions(&conn).map_err(|e| e.to_string())?;
        serde_json::to_string(&actions).map_err(|e| format!("序列化失败: {e}"))
    }

    #[tool(description = "设置/更新今日目标。同一天重复调用视为修改。返回 goal id。")]
    fn set_goal(&self, Parameters(SetGoalReq { text }): Parameters<SetGoalReq>) -> Result<String, String> {
        let conn = self.db.lock().map_err(|_| "db 锁中毒".to_string())?;
        let id = sisyphus_core::context::set_goal(&conn, &text).map_err(|e| e.to_string())?;
        Ok(format!("goal set: {id}"))
    }

    #[tool(
        description = "列出最近的 capture（原声记录）。unprocessed=true（默认）只列还没生成意图候选的，即待处理「收件箱」（不含 ingest_document 收下的素材）。返回 JSON 数组 [{event_id,text,created_at}]。"
    )]
    fn list_captures(
        &self,
        Parameters(ListCapturesReq { unprocessed }): Parameters<ListCapturesReq>,
    ) -> Result<String, String> {
        let conn = self.db.lock().map_err(|_| "db 锁中毒".to_string())?;
        let unproc = unprocessed.unwrap_or(true);
        let v = sisyphus_core::artifacts::list_captures(&conn, unproc, 20).map_err(|e| e.to_string())?;
        serde_json::to_string_pretty(&v).map_err(|e| format!("序列化失败: {e}"))
    }

    #[tool(
        description = "对一条 capture 持久化你（Codex）分类生成的意图候选（单事务，全成或全不成；校验 capture 存在）。不落 artifact，等 accept_intent 才落库。kind ∈ goal|task|reminder|note。只提最小候选，不要生成任务海。返回候选 id 列表。"
    )]
    fn propose_intents(
        &self,
        Parameters(ProposeIntentsReq {
            capture_event_id,
            candidates,
        }): Parameters<ProposeIntentsReq>,
    ) -> Result<String, String> {
        let conn = self.db.lock().map_err(|_| "db 锁中毒".to_string())?;
        let batch: Vec<(String, serde_json::Value, f64)> = candidates
            .into_iter()
            .map(|c| (c.kind, c.proposed, c.confidence.unwrap_or(0.0)))
            .collect();
        let ids = sisyphus_core::artifacts::insert_intent_candidates(
            &conn,
            &capture_event_id,
            &batch,
            "agent",
        )?;
        serde_json::to_string(&ids).map_err(|e| format!("序列化失败: {e}"))
    }

    #[tool(
        description = "接受一条意图候选，原子落成对应 artifact（goal→今日目标 / task / reminder / note）。edits 可选，用 JSON 对象覆盖候选字段。返回 artifact id。"
    )]
    fn accept_intent(
        &self,
        Parameters(AcceptIntentReq { intent_id, edits }): Parameters<AcceptIntentReq>,
    ) -> Result<String, String> {
        let conn = self.db.lock().map_err(|_| "db 锁中毒".to_string())?;
        let edits_s = edits.map(|v| v.to_string());
        let id = sisyphus_core::artifacts::accept_intent(&conn, &intent_id, edits_s.as_deref())?;
        Ok(format!("accepted -> {id}"))
    }

    #[tool(description = "忽略一条意图候选（回滚，不落 artifact，置为 ignored）。")]
    fn ignore_intent(
        &self,
        Parameters(IntentIdReq { intent_id }): Parameters<IntentIdReq>,
    ) -> Result<String, String> {
        let conn = self.db.lock().map_err(|_| "db 锁中毒".to_string())?;
        sisyphus_core::artifacts::ignore_intent(&conn, &intent_id).map_err(|e| e.to_string())?;
        Ok("ignored".to_string())
    }

    // ── 第二大脑（Phase 1.3）─────────────────────────────────────────────────

    #[tool(
        description = "收下一份原始素材（文本/URL/文件引用）到事件日志（标记为 material，不进意图收件箱），返回 doc_id。之后你负责阅读加工，再用 write_knowledge_note 保存概念卡片。"
    )]
    fn ingest_document(
        &self,
        Parameters(IngestDocumentReq { content, title }): Parameters<IngestDocumentReq>,
    ) -> Result<String, String> {
        let conn = self.db.lock().map_err(|_| "db 锁中毒".to_string())?;
        let text = match &title {
            Some(t) if !t.is_empty() => format!("[素材] {t}\n{content}"),
            _ => format!("[素材] {content}"),
        };
        let id = sisyphus_core::ingest::capture_material(&conn, &self.user_id, &self.device_id, &text)
            .map_err(|e| e.to_string())?;
        Ok(format!("doc_id: {id}"))
    }

    #[tool(
        description = "把一张加工好的知识卡片写入 Obsidian 知识库：生成 .md（frontmatter+正文+[[wikilink]]）+ 索引 + 溯源事件。folder 指定话题分类子目录（如 web-security、work-mihoyo/state），省略则落 vault 根。同标题视为更新；不同标题若 slug 撞车则自动消歧防覆盖。返回 JSON {id,path,content_hash,updated}。"
    )]
    fn write_knowledge_note(
        &self,
        Parameters(WriteKnowledgeNoteReq {
            title,
            body,
            tags,
            links,
            sources,
            folder,
        }): Parameters<WriteKnowledgeNoteReq>,
    ) -> Result<String, String> {
        let conn = self.db.lock().map_err(|_| "db 锁中毒".to_string())?;
        let note = sisyphus_core::vault::VaultNote {
            title,
            body,
            tags,
            links,
            sources,
        };
        let out = sisyphus_core::knowledge::write_knowledge_note(
            &conn,
            &self.vault_dir,
            &self.user_id,
            &self.device_id,
            folder.as_deref(),
            &note,
        )?;
        serde_json::to_string(&out).map_err(|e| format!("序列化失败: {e}"))
    }

    #[tool(
        description = "把值得原文保存的素材逐字归档到原始材料库 sources/（与知识图谱 kb/ 物理隔离，不进图谱、不导出博客）。返回 JSON {path,content_hash,updated}。加工总结请另用 write_knowledge_note 写 kb 卡片，并在其 sources 里引用本原文路径。"
    )]
    fn save_source(
        &self,
        Parameters(SaveSourceReq {
            title,
            content,
            url,
            source_type,
        }): Parameters<SaveSourceReq>,
    ) -> Result<String, String> {
        let conn = self.db.lock().map_err(|_| "db 锁中毒".to_string())?;
        let out = sisyphus_core::sources::save_source(
            &conn,
            &self.vault_dir,
            &self.user_id,
            &self.device_id,
            &title,
            &content,
            url.as_deref(),
            source_type.as_deref(),
        )?;
        serde_json::to_string(&out).map_err(|e| format!("序列化失败: {e}"))
    }

    #[tool(description = "检索知识库（匹配标题/标签/路径），返回 JSON 数组。")]
    fn search_knowledge(
        &self,
        Parameters(SearchKnowledgeReq { query }): Parameters<SearchKnowledgeReq>,
    ) -> Result<String, String> {
        let conn = self.db.lock().map_err(|_| "db 锁中毒".to_string())?;
        let v = sisyphus_core::knowledge::search_knowledge(&conn, &query).map_err(|e| e.to_string())?;
        serde_json::to_string_pretty(&v).map_err(|e| format!("序列化失败: {e}"))
    }

    #[tool(description = "列出知识库所有卡片（不含已剪枝），返回 JSON 数组。")]
    fn list_knowledge(&self) -> Result<String, String> {
        let conn = self.db.lock().map_err(|_| "db 锁中毒".to_string())?;
        let v = sisyphus_core::knowledge::list_knowledge(&conn).map_err(|e| e.to_string())?;
        serde_json::to_string_pretty(&v).map_err(|e| format!("序列化失败: {e}"))
    }

    #[tool(
        description = "从知识库删除一张卡片：移除 vault .md + 索引剪枝 + 溯源事件。**用于 defragment**——把多张碎卡合并进一颗结晶（用 write_knowledge_note 写合并后的卡）后，删掉冗余的旧碎卡；也用于 rebalance 的「并」。传标题（优先精确匹配）或相对路径。⚠️ 指向被删卡的 [[wikilink]] 需你另行 write_knowledge_note 改写到合并后的卡。幂等（不存在返回 deleted=false）。返回 JSON {path,deleted}。"
    )]
    fn delete_note(
        &self,
        Parameters(DeleteNoteReq { title }): Parameters<DeleteNoteReq>,
    ) -> Result<String, String> {
        let conn = self.db.lock().map_err(|_| "db 锁中毒".to_string())?;
        let out = sisyphus_core::knowledge::delete_knowledge_note(
            &conn,
            &self.vault_dir,
            &self.user_id,
            &self.device_id,
            &title,
        )?;
        serde_json::to_string(&out).map_err(|e| format!("序列化失败: {e}"))
    }

    // ── 西西弗斯计划：监控名单（拖延干预）─────────────────────────────────────

    #[tool(
        description = "列出被视为娱乐/摸鱼的 app 监控名单（内置 + 用户自定义），返回 JSON。启动/调整拖延干预前先看它。"
    )]
    fn list_monitored_apps(&self) -> Result<String, String> {
        let conn = self.db.lock().map_err(|_| "db 锁中毒".to_string())?;
        let v = sisyphus_core::category::list_monitored(&conn).map_err(|e| e.to_string())?;
        serde_json::to_string_pretty(&v).map_err(|e| format!("序列化失败: {e}"))
    }

    #[tool(
        description = "把一个 app 纳入监控（启动西西弗斯计划盯它）：当用户设了目标又超时刷它时会弹干预。id=bundle id/包名，category=entertainment.video|game|social|news。桌面+安卓即时生效。"
    )]
    fn add_monitored_app(
        &self,
        Parameters(AddMonitoredReq { id, category }): Parameters<AddMonitoredReq>,
    ) -> Result<String, String> {
        let id = id.trim();
        if id.is_empty() {
            return Err("id 不能为空".into());
        }
        let conn = self.db.lock().map_err(|_| "db 锁中毒".to_string())?;
        sisyphus_core::category::add_monitored_app(&conn, id, &category).map_err(|e| e.to_string())?;
        Ok(format!("monitoring: {id} -> {category}"))
    }

    #[tool(description = "从监控名单移除一个 app。")]
    fn remove_monitored_app(
        &self,
        Parameters(RemoveMonitoredReq { id }): Parameters<RemoveMonitoredReq>,
    ) -> Result<String, String> {
        let conn = self.db.lock().map_err(|_| "db 锁中毒".to_string())?;
        sisyphus_core::category::remove_monitored_app(&conn, &id).map_err(|e| e.to_string())?;
        Ok("removed".to_string())
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
                "Sisyphus 反思平面。原声笔记闭环：capture 记录一句话 → list_captures 看收件箱 → \
                 propose_intents 对一条 capture 生成意图候选（你负责分类为 goal/task/reminder/note）→ \
                 accept_intent 落成 artifact（或 ignore_intent 忽略）。query_context 查今日上下文（目标/娱乐时长/未完成任务/到期提醒）；\
                 today_actions 取今日最小行动；set_goal 设今日目标。\
                 第二大脑：ingest_document 收素材 → 你阅读加工 → write_knowledge_note 写概念卡片到 Obsidian 知识库（[[wikilink]] 关联）；search_knowledge/list_knowledge 检索；delete_note 删卡（合并碎卡后清冗余）。\
                 西西弗斯计划（拖延干预）：add_monitored_app 把某娱乐 app 纳入监控 + set_goal 设目标，用户超时刷它时端侧自动弹干预；list_monitored_apps 查名单。\
                 语气：关心不评判、只提最小下一步、引用真实数据。"
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
            vault_dir: vault_path(),
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

/// 知识库 vault 路径：`{data_dir}/com.sisyphus/vault`，可用 `SISYPHUS_VAULT` 覆盖。
/// 这是 Obsidian 可直接打开的知识库目录（.md 文件）。
fn vault_path() -> PathBuf {
    if let Ok(p) = std::env::var("SISYPHUS_VAULT") {
        return PathBuf::from(p);
    }
    let base = dirs::data_dir().expect("无法定位系统 data 目录");
    base.join("com.sisyphus").join("vault")
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let server = SisyphusServer::new()?;
    let service = server.serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
