//! Sisyphus 反思平面 MCP server（stdio 传输）。
//!
//! 由 Codex / Claude Code 作为子进程拉起，复用 `sisyphus-core` 的逻辑，
//! 打开与 Tauri App 同一个 `sisyphus.db`（WAL + busy_timeout 支持跨进程并发）。
//! 即使桌面 App 未运行也可独立工作（见 docs/spec/architecture.md §1.2、§4）。
//!
//! ⚠️ stdout 是 MCP 协议通道，任何日志/调试输出必须走 stderr。

use std::collections::BTreeMap;
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

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct CreateDetectionRuleReq {
    /// 规则名（展示用），如 "少打游戏" / "夜间刷视频提醒"
    pub name: String,
    /// 声明式触发条件对象。字段：category_prefix?（分类前缀，如 "entertainment.game"）、
    /// category_in?（精确分类数组）、app_in?（包名/bundle id 数组）、window_minutes?（统计窗口，默认 30）、
    /// min_minutes_in_window（阈值分钟，必填正数）、requires_active_goal?（默认 true）、
    /// time_of_day?（{from:"HH:MM",to:"HH:MM"} 本地时段，支持跨午夜）。至少要有一个 category/app 作用域。
    pub trigger: serde_json::Value,
    /// 可选响应策略对象。默认 {"policy":"immediate","kind":"notify"}。
    /// 可选：{"policy":"immediate","kind":"pet_message"} / {"policy":"deferred","after_ms":600000} /
    /// {"policy":"debounce","window_ms":2700000,"dedup_key":"..."} / {"policy":"suppress"}。
    #[serde(default)]
    pub response: Option<serde_json::Value>,
    /// medium | high（默认 medium）。
    #[serde(default)]
    pub severity: Option<String>,
    /// 冷却分钟数（同规则两次提醒最小间隔），默认 30。
    #[serde(default)]
    pub cooldown_minutes: Option<i64>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetRuleEnabledReq {
    /// 规则 id
    pub id: String,
    /// true=启用，false=停用
    pub enabled: bool,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct RuleIdReq {
    /// 规则 id
    pub id: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct UpsertLifeIndexReq {
    /// 分区，如 "今日焦点" / "长期目标" / "研究问题" / "个人发展"
    pub section: String,
    /// 卡片标题（同 section+title 视为同一张卡，幂等更新）
    pub title: String,
    /// 卡片正文（Markdown / 纯文本）
    pub body: String,
    /// 来源溯源：Notion page id 或 URL（可选）
    #[serde(default)]
    pub source_ref: Option<String>,
    /// 外部源更新时间 epoch ms（可选）
    #[serde(default)]
    pub source_updated_at: Option<i64>,
    /// 分区内排序（越小越靠前），默认 0
    #[serde(default)]
    pub sort_order: Option<i64>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct LifeIndexIdReq {
    /// 卡片 id
    pub id: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct LifeExternalRefReq {
    /// 外部适配器：当前通常为 notion
    pub provider: String,
    /// 外部 page / block 的稳定 id
    pub external_id: String,
    #[serde(default)]
    pub external_url: Option<String>,
    #[serde(default)]
    pub external_updated_at_ms: Option<i64>,
    #[serde(default)]
    pub content_hash: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct UpsertLifeItemReq {
    /// 已有 LifeItem id；新建时留空。同步 Notion 时优先传 external_ref 做幂等匹配。
    #[serde(default)]
    pub id: Option<String>,
    /// 更新已有项时建议回传 list_life_items 返回的 revision；不一致会拒绝覆盖并要求重新合并。
    #[serde(default)]
    pub expected_revision: Option<i64>,
    /// idea | goal | project | action | routine
    pub kind: String,
    pub title: String,
    #[serde(default)]
    pub body: String,
    /// main | side | neutral | undecided
    #[serde(default)]
    pub track: Option<String>,
    /// now | next | later | someday | unscheduled
    #[serde(default)]
    pub horizon: Option<String>,
    /// inbox | active | waiting | done | archived
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub start_at_ms: Option<i64>,
    #[serde(default)]
    pub due_at_ms: Option<i64>,
    #[serde(default)]
    pub review_at_ms: Option<i64>,
    /// RFC 5545 RRULE 或人类可读周期；未知则留空，不猜。
    #[serde(default)]
    pub recurrence: Option<String>,
    /// app | agent | notion | import。同步 Notion 文本进入本地时必须传 notion，避免回环。
    #[serde(default)]
    pub origin: Option<String>,
    #[serde(default)]
    pub external_ref: Option<LifeExternalRefReq>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ListLifeItemsReq {
    #[serde(default)]
    pub include_archived: Option<bool>,
    /// true 时只返回尚未投影到 Notion 的本地变更。
    #[serde(default)]
    pub dirty_only: Option<bool>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ArchiveLifeItemReq {
    pub id: String,
    #[serde(default)]
    pub expected_revision: Option<i64>,
    /// app | agent | notion
    #[serde(default)]
    pub origin: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct LinkLifeItemsReq {
    pub from_item_id: String,
    pub to_item_id: String,
    /// contains | supports | depends_on | blocks | derived_from | related
    pub relation: String,
    #[serde(default)]
    pub sort_order: Option<i64>,
    #[serde(default)]
    pub origin: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct LifeProjectionReq {
    /// Notion 投影目标 page id；用于取三方合并基线。
    pub target_id: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct CompleteLifeSyncReq {
    pub target_id: String,
    /// 本轮 read_lifeindex_page 得到的写回前完整文本，用于本地恢复审计。
    pub remote_before_text: String,
    /// 成功写回 Notion 的最终页面文本。
    pub snapshot_text: String,
    /// 本轮从 Notion 吸收/向 Notion 发布的简短摘要。
    #[serde(default)]
    pub summary: String,
    /// render_lifeindex_projection 返回的 projected_revisions，必须原样回传。
    pub projected_revisions: BTreeMap<String, i64>,
}

// ── Server ────────────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct SisyphusServer {
    tool_router: ToolRouter<SisyphusServer>,
    db: Arc<Mutex<Connection>>,
    vault_dir: PathBuf,
    user_id: String,
    device_id: String,
    /// 完全只读：所有写工具禁用（主动推荐模式）。
    read_only: bool,
    /// 仅 LifeDB 可写：除 LifeItem/同步工具外的写工具禁用（LifeIndexSync 模式）。
    lifeindex_only: bool,
}

#[tool_router]
impl SisyphusServer {
    #[tool(
        description = "记录一句自然语言（目标/想法/待办/情绪）到 Sisyphus 的事件日志。零压记录入口，返回 capture_id。"
    )]
    fn capture(
        &self,
        Parameters(CaptureReq { text }): Parameters<CaptureReq>,
    ) -> Result<String, String> {
        self.ensure_writable()?;
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
    fn set_goal(
        &self,
        Parameters(SetGoalReq { text }): Parameters<SetGoalReq>,
    ) -> Result<String, String> {
        self.ensure_writable()?;
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
        let v = sisyphus_core::artifacts::list_captures(&conn, unproc, 20)
            .map_err(|e| e.to_string())?;
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
        self.ensure_writable()?;
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
        self.ensure_writable()?;
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
        self.ensure_writable()?;
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
        self.ensure_writable()?;
        let conn = self.db.lock().map_err(|_| "db 锁中毒".to_string())?;
        let text = match &title {
            Some(t) if !t.is_empty() => format!("[素材] {t}\n{content}"),
            _ => format!("[素材] {content}"),
        };
        let id =
            sisyphus_core::ingest::capture_material(&conn, &self.user_id, &self.device_id, &text)
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
        self.ensure_writable()?;
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
        self.ensure_writable()?;
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
        let v =
            sisyphus_core::knowledge::search_knowledge(&conn, &query).map_err(|e| e.to_string())?;
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
        self.ensure_writable()?;
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
        self.ensure_writable()?;
        let id = id.trim();
        if id.is_empty() {
            return Err("id 不能为空".into());
        }
        let conn = self.db.lock().map_err(|_| "db 锁中毒".to_string())?;
        sisyphus_core::category::add_monitored_app(&conn, id, &category)
            .map_err(|e| e.to_string())?;
        Ok(format!("monitoring: {id} -> {category}"))
    }

    #[tool(description = "从监控名单移除一个 app。")]
    fn remove_monitored_app(
        &self,
        Parameters(RemoveMonitoredReq { id }): Parameters<RemoveMonitoredReq>,
    ) -> Result<String, String> {
        self.ensure_writable()?;
        let conn = self.db.lock().map_err(|_| "db 锁中毒".to_string())?;
        sisyphus_core::category::remove_monitored_app(&conn, &id).map_err(|e| e.to_string())?;
        Ok("removed".to_string())
    }

    // ── 动态检测规则（西西弗斯计划：一句话建规则）─────────────────────────────

    #[tool(
        description = "列出所有检测规则（含启用状态、trigger/response、冷却），返回 JSON。改/建规则前先看它，避免重复。"
    )]
    fn list_detection_rules(&self) -> Result<String, String> {
        let conn = self.db.lock().map_err(|_| "db 锁中毒".to_string())?;
        let v = sisyphus_core::rules::list_rules(&conn).map_err(|e| e.to_string())?;
        serde_json::to_string_pretty(&v).map_err(|e| format!("序列化失败: {e}"))
    }

    #[tool(
        description = "把用户口述的“什么情况提醒我”落成一条检测规则：声明式 trigger（category_prefix/category_in/app_in + window_minutes + min_minutes_in_window + requires_active_goal + 可选 time_of_day）+ 可选 response 策略。命中后端侧自动干预（通知/宠物气泡）。建前先 list_detection_rules 看有无重复；trigger 至少含一个 category/app 作用域。返回规则 id。"
    )]
    fn create_detection_rule(
        &self,
        Parameters(CreateDetectionRuleReq {
            name,
            trigger,
            response,
            severity,
            cooldown_minutes,
        }): Parameters<CreateDetectionRuleReq>,
    ) -> Result<String, String> {
        self.ensure_writable()?;
        let conn = self.db.lock().map_err(|_| "db 锁中毒".to_string())?;
        let trigger_s = trigger.to_string();
        let response_s = response.map(|v| v.to_string());
        let id = sisyphus_core::rules::create_rule(
            &conn,
            &name,
            &trigger_s,
            response_s.as_deref(),
            severity.as_deref().unwrap_or("medium"),
            cooldown_minutes.unwrap_or(30),
            "agent",
            None,
        )?;
        Ok(format!("rule created: {id}"))
    }

    #[tool(description = "启用 / 停用一条检测规则。")]
    fn set_detection_rule_enabled(
        &self,
        Parameters(SetRuleEnabledReq { id, enabled }): Parameters<SetRuleEnabledReq>,
    ) -> Result<String, String> {
        self.ensure_writable()?;
        let conn = self.db.lock().map_err(|_| "db 锁中毒".to_string())?;
        sisyphus_core::rules::set_rule_enabled(&conn, &id, enabled).map_err(|e| e.to_string())?;
        Ok(if enabled { "enabled" } else { "disabled" }.to_string())
    }

    #[tool(description = "删除一条检测规则。")]
    fn delete_detection_rule(
        &self,
        Parameters(RuleIdReq { id }): Parameters<RuleIdReq>,
    ) -> Result<String, String> {
        self.ensure_writable()?;
        let conn = self.db.lock().map_err(|_| "db 锁中毒".to_string())?;
        sisyphus_core::rules::delete_rule(&conn, &id).map_err(|e| e.to_string())?;
        Ok("deleted".to_string())
    }

    // ── LifeDB / LifeItem / LifeIndex ──────────────────────────────────────────

    #[tool(
        description = "列出 LifeDB 中高度结构化的 LifeItem。四个看板只是过滤视图：action=事项，routine=日常，track=main/side=主线/支线。同步前先读取，按稳定 id 更新，禁止凭标题生成重复项。"
    )]
    fn list_life_items(
        &self,
        Parameters(ListLifeItemsReq {
            include_archived,
            dirty_only,
        }): Parameters<ListLifeItemsReq>,
    ) -> Result<String, String> {
        let conn = self.db.lock().map_err(|_| "db 锁中毒".to_string())?;
        let v = if dirty_only.unwrap_or(false) {
            sisyphus_core::lifedb::list_dirty_items(&conn)?
        } else {
            sisyphus_core::lifedb::list_items(&conn, include_archived.unwrap_or(false))?
        };
        serde_json::to_string_pretty(&v).map_err(|e| format!("序列化失败: {e}"))
    }

    #[tool(
        description = "创建或更新一个 LifeItem。Notion→本地同步时 origin 必须为 notion，并优先用已有 id；App/对话产生的改动用 app/agent，会自动标记待出站同步。未知时间、轨道和优先级保持默认，不要猜。"
    )]
    fn upsert_life_item(
        &self,
        Parameters(req): Parameters<UpsertLifeItemReq>,
    ) -> Result<String, String> {
        self.ensure_lifeindex_writable()?;
        let conn = self.db.lock().map_err(|_| "db 锁中毒".to_string())?;
        let external_ref = req
            .external_ref
            .map(|ext| sisyphus_core::lifedb::ExternalRefInput {
                provider: ext.provider,
                external_id: ext.external_id,
                external_url: ext.external_url,
                external_updated_at_ms: ext.external_updated_at_ms,
                content_hash: ext.content_hash,
            });
        let id = sisyphus_core::lifedb::upsert_item(
            &conn,
            sisyphus_core::lifedb::LifeItemInput {
                id: req.id,
                expected_revision: req.expected_revision,
                kind: req.kind,
                title: req.title,
                body: req.body,
                track: req.track.unwrap_or_else(|| "undecided".to_string()),
                horizon: req.horizon.unwrap_or_else(|| "unscheduled".to_string()),
                status: req.status.unwrap_or_else(|| "inbox".to_string()),
                start_at_ms: req.start_at_ms,
                due_at_ms: req.due_at_ms,
                review_at_ms: req.review_at_ms,
                recurrence: req.recurrence,
                source_event_id: None,
                intent_id: None,
                origin: req.origin.unwrap_or_else(|| "agent".to_string()),
                external_ref,
            },
        )?;
        Ok(format!("life item: {id}"))
    }

    #[tool(
        description = "归档一个 LifeItem。仅当用户明确删除/归档，或 Notion 同步确认对应文本被删除时调用。"
    )]
    fn archive_life_item(
        &self,
        Parameters(ArchiveLifeItemReq {
            id,
            expected_revision,
            origin,
        }): Parameters<ArchiveLifeItemReq>,
    ) -> Result<String, String> {
        self.ensure_lifeindex_writable()?;
        let conn = self.db.lock().map_err(|_| "db 锁中毒".to_string())?;
        sisyphus_core::lifedb::archive_item(
            &conn,
            &id,
            origin.as_deref().unwrap_or("agent"),
            expected_revision,
        )?;
        Ok("archived".to_string())
    }

    #[tool(
        description = "建立 LifeItem 关系。contains 表示目标/项目包含子项目、事项或日常；其它关系用于支持、依赖、阻塞、派生和弱关联。"
    )]
    fn link_life_items(
        &self,
        Parameters(req): Parameters<LinkLifeItemsReq>,
    ) -> Result<String, String> {
        self.ensure_lifeindex_writable()?;
        let conn = self.db.lock().map_err(|_| "db 锁中毒".to_string())?;
        sisyphus_core::lifedb::link_items(
            &conn,
            &req.from_item_id,
            &req.to_item_id,
            &req.relation,
            req.sort_order.unwrap_or(0),
            req.origin.as_deref().unwrap_or("agent"),
        )?;
        Ok("linked".to_string())
    }

    #[tool(description = "列出 LifeItem 的全部结构关系，供 Agent 理解目标→项目→行动/日常层级。")]
    fn list_life_item_edges(&self) -> Result<String, String> {
        let conn = self.db.lock().map_err(|_| "db 锁中毒".to_string())?;
        let v = sisyphus_core::lifedb::list_edges(&conn)?;
        serde_json::to_string_pretty(&v).map_err(|e| format!("序列化失败: {e}"))
    }

    #[tool(
        description = "生成 Notion 普通页面应展示的最终 LifeIndex Markdown，同时返回上次成功同步快照。同步 Agent 用 remote 当前文本 + last_snapshot_text + 本地 LifeItem 做三方语义合并。"
    )]
    fn render_lifeindex_projection(
        &self,
        Parameters(LifeProjectionReq { target_id }): Parameters<LifeProjectionReq>,
    ) -> Result<String, String> {
        let conn = self.db.lock().map_err(|_| "db 锁中毒".to_string())?;
        let v = sisyphus_core::lifedb::render_projection(&conn, &target_id)?;
        serde_json::to_string_pretty(&v).map_err(|e| format!("序列化失败: {e}"))
    }

    #[tool(
        description = "仅在 Notion 页面成功写回后调用：保存三方合并基线，并把本地脏 LifeItem 标为 clean。写回失败时绝对不要调用。"
    )]
    fn complete_lifeindex_sync(
        &self,
        Parameters(CompleteLifeSyncReq {
            target_id,
            remote_before_text,
            snapshot_text,
            summary,
            projected_revisions,
        }): Parameters<CompleteLifeSyncReq>,
    ) -> Result<String, String> {
        self.ensure_lifeindex_writable()?;
        let conn = self.db.lock().map_err(|_| "db 锁中毒".to_string())?;
        sisyphus_core::lifedb::complete_sync(
            &conn,
            &target_id,
            &remote_before_text,
            &snapshot_text,
            &summary,
            &projected_revisions,
        )?;
        Ok("lifeindex sync complete".to_string())
    }

    // 旧卡片工具暂时保留，供已安装 skill 兼容；新实现统一使用上面的 LifeDB 工具。

    #[tool(
        description = "列出人生看板全部卡片（按分区排序），返回 JSON。刷新看板前先看它，避免重复。"
    )]
    fn list_lifeindex(&self) -> Result<String, String> {
        let conn = self.db.lock().map_err(|_| "db 锁中毒".to_string())?;
        let v = sisyphus_core::lifeindex::list_cards(&conn).map_err(|e| e.to_string())?;
        serde_json::to_string_pretty(&v).map_err(|e| format!("序列化失败: {e}"))
    }

    #[tool(
        description = "写/更新人生看板的一张卡片（按 section+title 幂等）。用法：只读参考用户 Notion + query_context 后，把「长期目标/今日焦点/研究问题/个人发展」等提炼成卡片写到本地看板。source_ref 填 Notion 溯源。绝不回写 Notion。返回卡片 id。"
    )]
    fn upsert_lifeindex_card(
        &self,
        Parameters(UpsertLifeIndexReq {
            section,
            title,
            body,
            source_ref,
            source_updated_at,
            sort_order,
        }): Parameters<UpsertLifeIndexReq>,
    ) -> Result<String, String> {
        self.ensure_lifeindex_writable()?;
        let conn = self.db.lock().map_err(|_| "db 锁中毒".to_string())?;
        let id = sisyphus_core::lifeindex::upsert_card(
            &conn,
            &section,
            &title,
            &body,
            source_ref.as_deref(),
            source_updated_at,
            sort_order.unwrap_or(0),
        )?;
        Ok(format!("lifeindex card: {id}"))
    }

    #[tool(description = "删除人生看板的一张卡片。")]
    fn delete_lifeindex_card(
        &self,
        Parameters(LifeIndexIdReq { id }): Parameters<LifeIndexIdReq>,
    ) -> Result<String, String> {
        self.ensure_lifeindex_writable()?;
        let conn = self.db.lock().map_err(|_| "db 锁中毒".to_string())?;
        sisyphus_core::lifeindex::delete_card(&conn, &id).map_err(|e| e.to_string())?;
        Ok("deleted".to_string())
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
                 想“盯住某类行为/新场景”时用 create_detection_rule 把用户口述落成检测规则（声明式 trigger + response），list/set_enabled/delete_detection_rule 管理。\
                 LifeIndex：LifeDB 是事实源；list/upsert_life_item 管理 idea/goal/project/action/routine，link_life_items 建关系；Notion 双向同步必须走受限 LifeIndexSync，三方合并后成功写回才 complete_lifeindex_sync。\
                 语气：关心不评判、只提最小下一步、引用真实数据。"
                    .to_string(),
            ),
            ..Default::default()
        }
    }
}

impl SisyphusServer {
    fn ensure_writable(&self) -> Result<(), String> {
        if self.read_only || self.lifeindex_only {
            Err("当前 Sisyphus MCP 以只读模式运行；智能体不能修改用户内容或本地状态".to_string())
        } else {
            Ok(())
        }
    }

    /// 看板写门禁：完全只读时禁用；lifeindex_only 或完全可写时放行。
    fn ensure_lifeindex_writable(&self) -> Result<(), String> {
        if self.read_only {
            Err("当前 Sisyphus MCP 以只读模式运行；不能更新看板".to_string())
        } else {
            Ok(())
        }
    }

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
        let env_flag = |k: &str| {
            std::env::var(k)
                .map(|v| matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes"))
                .unwrap_or(false)
        };
        Ok(Self {
            tool_router: Self::tool_router(),
            db: Arc::new(Mutex::new(conn)),
            vault_dir: vault_path(),
            user_id: "local-user".to_string(),
            device_id: "agent-mcp".to_string(),
            read_only: env_flag("SISYPHUS_READ_ONLY"),
            lifeindex_only: env_flag("SISYPHUS_LIFEINDEX_ONLY"),
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
