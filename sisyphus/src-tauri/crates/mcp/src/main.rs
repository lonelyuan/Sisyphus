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
    /// 来源 url / 引用（外部原文路径 / 权威 URL）。可靠性标为「多源印证/已复现/已验证」时必填
    #[serde(default)]
    pub sources: Vec<String>,
    /// 话题领域目录，**必须以 `kb/` 开头**，如 "kb/web-security" / "kb/work-mihoyo/state"
    pub folder: String,
    /// 别名（重定向）：指向本卡的旧标题；合并碎卡后旧 [[链接]] 仍可解析
    #[serde(default)]
    pub aliases: Vec<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ReadKnowledgeNoteReq {
    /// 卡片标题（支持别名/重定向）
    pub title: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct AppendSectionReq {
    /// 要长大的那颗结晶的标题
    pub title: String,
    /// H2 小节名（同名则替换=精化，不同名则插入=增生）
    pub heading: String,
    /// 该小节的正文
    pub body: String,
    /// 追加的关联卡片标题
    #[serde(default)]
    pub links: Vec<String>,
    /// 追加的来源
    #[serde(default)]
    pub sources: Vec<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct MergeNotesReq {
    /// 要并掉的碎卡标题（会登记为目标卡别名，入链自动改写）
    pub from_titles: Vec<String>,
    /// 合并进哪颗结晶（先用 append_knowledge_section 把内容写成超集，再调本工具）
    pub into_title: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct WantedReq {
    /// 至少被引用几次才算值得补（默认 1）
    #[serde(default)]
    pub min_refs: Option<i64>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct LifeTreeReq {
    /// 只看这些 kind 的根节点，如 ["skill"] 看技能树、["goal"] 看目标分解；省略=全部
    #[serde(default)]
    pub kinds: Vec<String>,
    /// 只看某个节点的子树（传 LifeItem id）
    #[serde(default)]
    pub root_id: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SkillMapReq {
    /// 看某个历史时刻的样子（epoch ms）。省略=现在。
    #[serde(default)]
    pub at_ms: Option<i64>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct UpsertLifeAreaReq {
    /// 领域名（按名字幂等）
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    /// 是否当前重点领域（影响主线推导与今日行动选择）
    #[serde(default)]
    pub focus: Option<bool>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct LimitReq {
    #[serde(default)]
    pub limit: Option<i64>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ReviewQueueReq {
    /// 多少天没动算"停滞"（默认 7）
    #[serde(default)]
    pub idle_days: Option<i64>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SinceDaysReq {
    /// 统计最近多少天（默认 30）
    #[serde(default)]
    pub since_days: Option<i64>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SyncRunReq {
    /// life_sync_runs 的轮次 id（list_lifeindex_runs 取）
    pub run_id: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SaveSourceReq {
    /// 话题领域目录，**必须以 `kb/` 开头**——原文就地存放在它讲的那个话题旁边
    pub folder: String,
    /// 原文标题（决定文件名）
    pub title: String,
    /// 逐字原文内容（Markdown / 纯文本）
    pub content: String,
    /// 来源 URL。外部原文**必填**（否则无法溯源）；本人自撰材料请把 source_type 设为 first-party
    #[serde(default)]
    pub url: Option<String>,
    /// 素材类型：article / vuln-report / doc / paper / first-party（本人自撰）
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
    /// idea | goal | project | action | routine | skill | milestone
    /// （skill = 能力节点，用 depends_on 边表达前置；milestone = 可判定的检查点）
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
    /// 责任领域 id（list_life_areas 取）。不确定就留空，别猜。
    #[serde(default)]
    pub area_id: Option<String>,
    /// 可判定的完成条件（一句话）。goal / milestone 强烈建议填——没有它目标永远无法收敛。
    #[serde(default)]
    pub success_criteria: Option<String>,
    /// 度量目标值 / 当前值 / 单位（技能树进度由 Core 用它确定性算出）。
    #[serde(default)]
    pub target_value: Option<f64>,
    #[serde(default)]
    pub current_value: Option<f64>,
    #[serde(default)]
    pub unit: Option<String>,
    #[serde(default)]
    pub start_at_ms: Option<i64>,
    #[serde(default)]
    pub due_at_ms: Option<i64>,
    /// 审查时间（epoch ms）。idea 建议排 +7 天，到期由周回顾提出"升级/someday/归档"三选一。
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
            aliases,
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
            aliases,
        };
        let out = sisyphus_core::knowledge::write_knowledge_note(
            &conn,
            &self.vault_dir,
            &self.user_id,
            &self.device_id,
            Some(&folder),
            &note,
        )?;
        serde_json::to_string(&out).map_err(|e| format!("序列化失败: {e}"))
    }

    #[tool(
        description = "把值得原文保存的素材逐字归档，**就地存放在它讲的那个话题的文件夹里**（folder 必须 kb/ 开头）。自动标 type:source + publish:false（不外发），在图谱里是**叶子**（只被卡片/枢纽指向、自己不出链）。外部原文必须给 url；本人自撰传 source_type=\"first-party\"。⚠️ 公司 KM 这类你能直接开链接的第一方文档**不要逐字复制**——在卡片正文里引用 URL 就够，副本只会制造双份真相。返回 JSON {path,content_hash,updated}。"
    )]
    fn save_source(
        &self,
        Parameters(SaveSourceReq {
            folder,
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
            &folder,
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
        description = "读回一张知识卡片的完整内容（frontmatter + 正文小节 + 关联）。**补充式增长前必须先读回**；支持别名。返回 JSON {path,title,tags,sources,aliases,body,links}。"
    )]
    fn read_knowledge_note(
        &self,
        Parameters(ReadKnowledgeNoteReq { title }): Parameters<ReadKnowledgeNoteReq>,
    ) -> Result<String, String> {
        let conn = self.db.lock().map_err(|_| "db 锁中毒".to_string())?;
        let (path, note) =
            sisyphus_core::knowledge::read_knowledge_note(&conn, &self.vault_dir, &title)?;
        serde_json::to_string_pretty(&serde_json::json!({
            "path": path,
            "title": note.title,
            "tags": note.tags,
            "sources": note.sources,
            "aliases": note.aliases,
            "body": note.body,
            "links": note.links,
        }))
        .map_err(|e| format!("序列化失败: {e}"))
    }

    #[tool(
        description = "给已有结晶加/精**一个 H2 小节**并写回超集（结晶化的默认路径）。同名小节=精化，新名=增生；其余小节原样保留。同主题多轮对话请用它，别每次新建碎卡。返回 JSON {id,path,updated,wanted_links}。"
    )]
    fn append_knowledge_section(
        &self,
        Parameters(AppendSectionReq {
            title,
            heading,
            body,
            links,
            sources,
        }): Parameters<AppendSectionReq>,
    ) -> Result<String, String> {
        self.ensure_writable()?;
        let conn = self.db.lock().map_err(|_| "db 锁中毒".to_string())?;
        let out = sisyphus_core::knowledge::append_section(
            &conn,
            &self.vault_dir,
            &self.user_id,
            &self.device_id,
            &title,
            &heading,
            &body,
            &links,
            &sources,
        )?;
        serde_json::to_string(&out).map_err(|e| format!("序列化失败: {e}"))
    }

    #[tool(
        description = "结晶化归并：把若干碎卡并进一颗结晶。会①给目标卡登记别名（旧 [[链接]] 仍可解析）②改写其它卡里的入链 ③删掉碎卡——**不留断链**。正文超集请先用 append_knowledge_section 写好。返回 JSON {into_title,merged,rewritten_files}。"
    )]
    fn merge_knowledge_notes(
        &self,
        Parameters(MergeNotesReq {
            from_titles,
            into_title,
        }): Parameters<MergeNotesReq>,
    ) -> Result<String, String> {
        self.ensure_writable()?;
        let conn = self.db.lock().map_err(|_| "db 锁中毒".to_string())?;
        let out = sisyphus_core::knowledge::merge_notes(
            &conn,
            &self.vault_dir,
            &self.user_id,
            &self.device_id,
            &from_titles,
            &into_title,
        )?;
        serde_json::to_string(&out).map_err(|e| format!("序列化失败: {e}"))
    }

    #[tool(
        description = "知识库体检：断链/断链率、孤儿卡（入度0）、无出链、同标题或同文件名重复、缺类型或可靠性标签、无证据的高可靠性、各领域卡数与拆并建议、同前缀碎卡簇、MOC 目录漂移、散落文件、未被引用的 sources 原文。**rebalance / defragment 前先跑它**，别靠感觉扫。返回结构化 JSON。"
    )]
    fn kb_doctor(&self) -> Result<String, String> {
        let conn = self.db.lock().map_err(|_| "db 锁中毒".to_string())?;
        let report = sisyphus_core::kb_doctor::doctor(&conn, Some(&self.vault_dir))?;
        serde_json::to_string_pretty(&report).map_err(|e| format!("序列化失败: {e}"))
    }

    #[tool(
        description = "红链队列：被引用但还不存在的卡，按引用热度排序。**这是主动调研的输入**——不用等用户开口说“帮我深挖 X”，被引用最多的缺口就是最该补的。返回 JSON [{title,referenced_by,sources}]。"
    )]
    fn kb_wanted(
        &self,
        Parameters(WantedReq { min_refs }): Parameters<WantedReq>,
    ) -> Result<String, String> {
        let conn = self.db.lock().map_err(|_| "db 锁中毒".to_string())?;
        let v = sisyphus_core::kb_doctor::wanted(&conn, min_refs.unwrap_or(1).max(1) as usize)?;
        serde_json::to_string_pretty(&v).map_err(|e| format!("序列化失败: {e}"))
    }

    #[tool(
        description = "刷新所有领域枢纽（MOC）的自动目录区块：上级枢纽 + 子领域 + 卡片清单 + 原始材料清单。**写完新卡后调它**：手写目录必然漂移，而「枢纽真的连着它下面的卡片」是 Obsidian 图谱里出现树状层级的唯一机制。只替换 `<!-- kb:auto begin/end -->` 之间的内容，你写的领域叙述原样保留。返回 JSON {refreshed,cards_listed,sources_listed,skipped_missing}。"
    )]
    fn refresh_mocs(&self) -> Result<String, String> {
        self.ensure_writable()?;
        let conn = self.db.lock().map_err(|_| "db 锁中毒".to_string())?;
        let report = sisyphus_core::knowledge::refresh_mocs(&conn, &self.vault_dir)?;
        serde_json::to_string(&report).map_err(|e| format!("序列化失败: {e}"))
    }

    #[tool(
        description = "按 vault 现状重建索引与链接边（用户在 Obsidian 里手改/移动/删除过文件后调用；老库回填正文与领域也用它）。vault 的 .md 是本体，索引是可重建的投影。返回 JSON {scanned,inserted,updated,links}。"
    )]
    fn kb_reindex(&self) -> Result<String, String> {
        self.ensure_writable()?;
        let conn = self.db.lock().map_err(|_| "db 锁中毒".to_string())?;
        let report = sisyphus_core::knowledge::reindex_vault(&conn, &self.vault_dir)?;
        serde_json::to_string(&report).map_err(|e| format!("序列化失败: {e}"))
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
                area_id: req.area_id,
                success_criteria: req.success_criteria,
                target_value: req.target_value,
                current_value: req.current_value,
                unit: req.unit,
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

    #[tool(
        description = "技能树 / 目标分解树：返回根节点及其子树，含 **Core 确定性算出的进度**（叶子看状态或 current/target 度量，内部节点是子节点等权平均）、已完成叶子数、前置节点（depends_on）。kinds=[\"skill\"] 看技能树，[\"goal\"] 看目标分解，root_id 只看一根分支。进度不要自己估——用这里的数。"
    )]
    fn life_tree(
        &self,
        Parameters(LifeTreeReq { kinds, root_id }): Parameters<LifeTreeReq>,
    ) -> Result<String, String> {
        let conn = self.db.lock().map_err(|_| "db 锁中毒".to_string())?;
        if let Some(root) = root_id.as_deref().filter(|r| !r.trim().is_empty()) {
            let node = sisyphus_core::lifetree::subtree(&conn, root)?;
            return serde_json::to_string_pretty(&node).map_err(|e| format!("序列化失败: {e}"));
        }
        let refs: Vec<&str> = kinds.iter().map(|k| k.as_str()).collect();
        let forest = sisyphus_core::lifetree::forest(&conn, &refs)?;
        serde_json::to_string_pretty(&forest).map_err(|e| format!("序列化失败: {e}"))
    }

    #[tool(
        description = "技能树地图（能力全景，和 life_tree 的区别是它是**图**而不是嵌套树）：sectors=责任领域扇区（背景，永不是节点）、nodes=技能点与它们环上的里程碑刻度、edges=前置边（已把 blocks 规范化成 depends_on）、ideas=还没决定要不要变成能力的想法。每个节点带四态之一：attained 已掌握 / in_progress 在进展 / available 可解锁 / locked 锁定（blocked_by 说得出缺哪个前置）。想提「下一步学什么」时先读它——不要把 locked 的技能推荐给用户。at_ms 给历史时刻则按进度账本回放。"
    )]
    fn skill_map(
        &self,
        Parameters(SkillMapReq { at_ms }): Parameters<SkillMapReq>,
    ) -> Result<String, String> {
        let conn = self.db.lock().map_err(|_| "db 锁中毒".to_string())?;
        let map = sisyphus_core::skillmap::skill_map(&conn, at_ms)?;
        serde_json::to_string_pretty(&map).map_err(|e| format!("序列化失败: {e}"))
    }

    #[tool(
        description = "今日最小行动（确定性选择 + **每条带理由**）：逾期 → 已排在当下 → 临近截止 → 主线/重点领域下最浅的未完成事项 → 今日日常 → 候选下一步 → 待安排。规划时用它，别自己从列表里挑。返回 JSON [{item_id,title,kind,track,due_at_ms,reason}]。"
    )]    fn next_actions(
        &self,
        Parameters(LimitReq { limit }): Parameters<LimitReq>,
    ) -> Result<String, String> {
        let conn = self.db.lock().map_err(|_| "db 锁中毒".to_string())?;
        let v = sisyphus_core::lifetree::next_actions(&conn, limit.unwrap_or(3).clamp(1, 10) as usize)?;
        serde_json::to_string_pretty(&v).map_err(|e| format!("序列化失败: {e}"))
    }

    #[tool(
        description = "周回顾队列（GTD weekly review 的确定性部分）：到期该审查的想法、有子项但长期没推进的目标、完全没拆解的目标/技能、缺可判定完成条件的目标、长期滞留 inbox 的想法。每条自带该问用户的问题——让“手动维护”变成“回答几个二选一”。"
    )]
    fn review_queue(
        &self,
        Parameters(ReviewQueueReq { idle_days }): Parameters<ReviewQueueReq>,
    ) -> Result<String, String> {
        let conn = self.db.lock().map_err(|_| "db 锁中毒".to_string())?;
        let q = sisyphus_core::lifetree::review_queue(&conn, idle_days.unwrap_or(7))?;
        serde_json::to_string_pretty(&q).map_err(|e| format!("序列化失败: {e}"))
    }

    #[tool(
        description = "列出责任领域（GTD Horizon 3：无完成态，只需维持标准）。focus=true 的是当前重点，会影响主线推导与今日行动选择。写 LifeItem 时用它的 id 填 area_id。"
    )]
    fn list_life_areas(&self) -> Result<String, String> {
        let conn = self.db.lock().map_err(|_| "db 锁中毒".to_string())?;
        let v = sisyphus_core::lifedb::list_areas(&conn)?;
        serde_json::to_string_pretty(&v).map_err(|e| format!("序列化失败: {e}"))
    }

    #[tool(description = "新建/更新一个责任领域（按名字幂等），可设为当前重点。")]
    fn upsert_life_area(
        &self,
        Parameters(UpsertLifeAreaReq {
            name,
            description,
            focus,
        }): Parameters<UpsertLifeAreaReq>,
    ) -> Result<String, String> {
        self.ensure_lifeindex_writable()?;
        let conn = self.db.lock().map_err(|_| "db 锁中毒".to_string())?;
        let id = sisyphus_core::lifedb::upsert_area(&conn, &name, description.as_deref(), focus)?;
        Ok(format!("area: {id}"))
    }

    #[tool(
        description = "干预效果统计（提醒后的近端结果）：switched=转走了 / mixed=混合 / still_entertainment=还在刷 / unknown=没观测到，以及 switch_rate 转移率。这是判断“提醒到底有没有用”的唯一数据依据，复盘时引用真实数字而不是感觉。"
    )]
    fn intervention_outcomes(
        &self,
        Parameters(SinceDaysReq { since_days }): Parameters<SinceDaysReq>,
    ) -> Result<String, String> {
        let conn = self.db.lock().map_err(|_| "db 锁中毒".to_string())?;
        let since = sisyphus_core::clock::now_ms() - since_days.unwrap_or(30).max(1) * 86_400_000;
        let stats = sisyphus_core::intervention::outcome_stats(&conn, since)
            .map_err(|e| e.to_string())?;
        serde_json::to_string_pretty(&stats).map_err(|e| format!("序列化失败: {e}"))
    }

    #[tool(description = "列出 LifeItem 的全部结构关系，供 Agent 理解目标→项目→行动/日常层级。")]
    fn list_life_item_edges(&self) -> Result<String, String> {        let conn = self.db.lock().map_err(|_| "db 锁中毒".to_string())?;
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

    #[tool(
        description = "列出历史 LifeIndex 同步轮次（每轮都存了写回前全文）。整页替换是这条链路上唯一不可逆的一步，出问题时先用它找到要恢复的那一轮。返回 JSON [{id,summary,completed_at_ms,...预览}]。"
    )]
    fn list_lifeindex_runs(
        &self,
        Parameters(LimitReq { limit }): Parameters<LimitReq>,
    ) -> Result<String, String> {
        let conn = self.db.lock().map_err(|_| "db 锁中毒".to_string())?;
        let target = match sisyphus_core::lifedb::list_sync_runs(&conn, &self.notion_target(), limit.unwrap_or(10)) {
            Ok(v) => v,
            Err(e) => return Err(e),
        };
        serde_json::to_string_pretty(&target).map_err(|e| format!("序列化失败: {e}"))
    }

    #[tool(
        description = "取某一轮同步的**写回前完整文本**，用于把 Notion 页恢复成同步前的样子（LifeIndexSync 模式下把它交给 replace_lifeindex_page）。只读，不自己动 Notion。"
    )]
    fn lifeindex_rollback_text(
        &self,
        Parameters(SyncRunReq { run_id }): Parameters<SyncRunReq>,
    ) -> Result<String, String> {
        let conn = self.db.lock().map_err(|_| "db 锁中毒".to_string())?;
        sisyphus_core::lifedb::sync_run_remote_before(&conn, &run_id)
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
                "Sisyphus 反思平面。\n\
                 【意图工程】任意用户输入先 capture → list_captures 看收件箱 → propose_intents 生成候选\
                 （kind: goal|task|reminder|note|life_item|rule，你负责分类）→ accept_intent 落库 / ignore_intent 忽略。\
                 落库前复述确认；note/idea 可直接落，task/reminder 复述一句，goal/规则/监控名单必须确认。\n\
                 【今日】query_context 查上下文；next_actions 取确定性选出的最小行动（**每条带理由，别自己从列表里挑**）；set_goal 设今日目标。\n\
                 【人生看板 LifeDB】list/upsert_life_item 管 idea/goal/project/action/routine/skill/milestone；\
                 link_life_items 建关系（contains=分解，depends_on=前置）；list_life_areas 取责任领域填 area_id；\
                 life_tree 看技能树/目标分解**及 Core 算好的进度（不要自己估）**；review_queue 拿周回顾要问的问题。\
                 goal/milestone 尽量给 success_criteria，否则永远无法收敛。Notion 双向同步只走受限 LifeIndexSync 模式；\
                 出问题用 list_lifeindex_runs + lifeindex_rollback_text 找回写前全文。\n\
                 【第二大脑】写卡前先 search_knowledge（**含正文**）查重：\
                 已有结晶就 append_knowledge_section 长一个 H2 小节（默认路径）；\
                 确实是新主题 → write_knowledge_note（folder 必须以 kb/ 开头，tags 恰好一个类型 + 一个可靠性档，\
                 links 至少一条；已复现/已验证必须有 sources）。整卡覆盖前先 read_knowledge_note 读回。\
                 碎卡合并用 merge_knowledge_notes（自动留别名、改入链、不留断链），删单卡用 delete_note。\
                 整理知识库先跑 kb_doctor（断链/孤儿/重复/缺标签/超阈值目录/碎卡簇/目录漂移），别靠感觉扫；\
                 kb_wanted 是红链队列 = 主动调研的输入；用户在 Obsidian 手改过就 kb_reindex。\n\
                 【反拖延】add_monitored_app + set_goal 即启用端侧自动干预；create_detection_rule 把「帮我盯着某类行为」落成声明式规则；\
                 intervention_outcomes 看提醒后的真实转移率——复盘引用它，不要凭感觉说有效。\n\
                 语气：关心不评判、只提最小下一步、引用真实数据。"
                    .to_string(),
            ),
            ..Default::default()
        }
    }
}

impl SisyphusServer {
    /// 同步目标页 id（与宿主一致：从 notion_config.json 读；缺省空串表示未配置）。
    fn notion_target(&self) -> String {
        let dir = dirs::data_dir()
            .map(|d| d.join("com.sisyphus"))
            .unwrap_or_default();
        sisyphus_core::app_config::read_notion_config(&dir).page_id
    }

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
