# Spec: API / 接口参考

本文件是西西弗斯**全部对外接口**的清单与权威参考。上位：[architecture.md](architecture.md)（§4 MCP 工具面是反思平面契约）。

## 0. 先说清楚：没有 HTTP REST API，OpenAPI 不适用

西西弗斯是**本地优先的桌面/移动 App + 本地 MCP server**，不是 HTTP 服务，因此**没有 REST endpoint，OpenAPI/Swagger 不适用**。真正的接口面有两个，都建立在同一个 `sisyphus-core`（唯一事实来源）+ 同一个 SQLite 上：

| 面 | 消费者 | 传输 | 机器可读 schema（≈OpenAPI 的等价物） |
|---|---|---|---|
| **MCP 工具**（反思平面） | Agent：Codex / Claude Code | stdio **JSON-RPC**（`rmcp`） | ✅ MCP `tools/list` 返回每个工具的 **JSON Schema**（`schemars` 自动生成）——这就是 MCP 的"OpenAPI" |
| **Tauri 命令**（App IPC） | React 前端 `invoke()` | Tauri IPC | ❌ 无标准导出；本文件 + `generate_handler![]` 是真相 |

- **想机器可读列出 MCP 工具**：启动 `sisyphus-mcp` 后发 `tools/list` 请求，返回名字 + 参数 JSON Schema。无需另写 OpenAPI。
- 两个面**能力有重叠也有专属**（见 §3 矩阵）。命名约定：MCP 参数是 Rust 结构体字段名（snake_case，如 `capture_event_id`）；Tauri 前端 `invoke` 用 camelCase，Tauri 自动转 snake_case（如 `dueMs`→`due_ms`）。

---

## 1. MCP 工具面（反思平面 · Agent 用）

实现：`sisyphus/src-tauri/crates/mcp/src/main.rs`（`rmcp` stdio）。由 Pi/Codex 作子进程拉起，打开与 App 同一个 `sisyphus.db`。

### 1.1 原声笔记闭环（capture → 意图 → artifact）

| 工具 | 参数 | 返回 | 读/写 | 作用 |
|---|---|---|---|---|
| `capture` | `text` | `captured: {id}` | 写 Event log | 零压记录一句话（目标/想法/待办/情绪） |
| `list_captures` | `unprocessed?`(默认 true) | JSON `[{event_id,text,created_at}]` | 读 | 收件箱：还没生成意图候选的 capture |
| `propose_intents` | `capture_event_id`, `candidates[]`(kind∈goal\|task\|reminder\|note) | JSON id 列表 | 读/写 | 对一条 capture 生成结构化意图候选（单事务） |
| `accept_intent` | `intent_id`, `edits?` | `accepted -> {id}` | 写 Artifact store | 接受候选，落成 goal/task/reminder/note |
| `ignore_intent` | `intent_id` | `ignored` | 写 | 忽略候选（回滚，不落 artifact） |

### 1.2 今日上下文与目标

| 工具 | 参数 | 返回 | 读/写 | 作用 |
|---|---|---|---|---|
| `query_context` | — | JSON `TodayContext` | 读 | 今日目标/娱乐时长/未完成任务/到期提醒/近期干预 |
| `today_actions` | — | JSON `[action]` | 读 | 今日最小行动（1–3 条） |
| `set_goal` | `text` | `goal set: {id}` | 写 | 设/改今日目标（同日重复=改） |

### 1.3 第二大脑（知识工程）

| 工具 | 参数 | 返回 | 读/写 | 作用 |
|---|---|---|---|---|
| `ingest_document` | `content`, `title?` | `doc_id: {id}` | 写 Event log | 收原始素材进事件流（标 material，不进意图收件箱） |
| `save_source` | `title`, `content`, `url?`, `source_type?` | JSON `{path,content_hash,updated}` | 写 `sources/` | 逐字原文归档（与 kb 物理隔离） |
| `write_knowledge_note` | `title`, `body`, `tags?`, `links?`, `sources?`, `folder?` | JSON `{id,path,content_hash,updated}` | 写 vault+索引+事件 | 写/更新一张知识卡（同标题=更新；slug 撞车自动消歧） |
| `delete_note` | `title`(或相对路径) | JSON `{path,deleted}` | 写（剪枝） | 删卡：移除 `.md`+索引剪枝+溯源（合并碎卡后清冗余） |
| `search_knowledge` | `query` | JSON `[note]` | 读 | 检索（标题/标签/路径 LIKE） |
| `list_knowledge` | — | JSON `[note]` | 读 | 列全部卡（不含已剪枝） |

### 1.4 拖延干预 · 监控名单

| 工具 | 参数 | 返回 | 读/写 | 作用 |
|---|---|---|---|---|
| `list_monitored_apps` | — | JSON `[app]` | 读 | 娱乐 app 监控名单（内置+自定义） |
| `add_monitored_app` | `id`, `category` | `monitoring: {id} -> {cat}` | 写 | 纳入监控（跨端即时生效） |
| `remove_monitored_app` | `id` | `removed` | 写 | 移出监控 |

### 1.5 LifeDB / LifeIndexSync

| 工具 | 参数 | 返回 | 读/写 | 作用 |
|---|---|---|---|---|
| `list_life_items` | `include_archived?`, `dirty_only?` | JSON `[LifeItem]` | 读 | 读取结构化人生规划对象及 revision |
| `upsert_life_item` | `id?`, `expected_revision?`, kind/title/body/track/horizon/status/time/recurrence, `origin?`, `external_ref?` | `life item: {id}` | 写 LifeDB | 新建/更新；同步更新用 expected revision 防并发覆盖 |
| `archive_life_item` | `id`, `origin?` | `archived` | 写 LifeDB | 可恢复归档 |
| `link_life_items` | from/to/relation/sort_order/origin | `linked` | 写 LifeDB | 建立目标→项目→行动/日常等关系 |
| `list_life_item_edges` | — | JSON `[LifeItemEdge]` | 读 | 读取关系图 |
| `render_lifeindex_projection` | `target_id` | `LifeProjection` | 读 | 生成四视图 Markdown、上次快照、逐项 revisions |
| `complete_lifeindex_sync` | target/`remote_before_text`/snapshot/summary/`projected_revisions` | completed | 写同步审计 | 仅 Notion 成功写回后调用；保存写前备份并按逐项 revision 清 dirty |

`SISYPHUS_LIFEINDEX_ONLY=1` 时只开放本节工具；Notion 读写走独立固定单页网关，不在此 MCP 内保存 token。

---

## 2. Tauri 命令面（App IPC · 前端 `invoke()`）

实现：`sisyphus/src-tauri/src/commands.rs` + `src/lib.rs`；注册于 `lib.rs` 的 `generate_handler![]`。平台列：🖥️ 桌面 / 📱 安卓 / 双 = 两端。

### 2.1 写入与规则（感知平面）

| 命令 | 参数(Rust) | 返回 | 平台 | 作用 |
|---|---|---|---|---|
| `ingest_event` | `input: NewEvent` | `String`(event_id) | 双 | 唯一写入契约薄封装（各源写 Event log） |
| `evaluate_rules` | `ctx: RuleContextInput` | `Option<FindingOutput>` | 双 | 跑规则引擎，命中即记干预并返回 finding |
| `record_feedback` | `intervention_id`, `action` | `()` | 双 | 记录用户对干预通知的响应 |

### 2.2 目标 / 任务 / 提醒（Artifact 增删查改）

| 命令 | 参数 | 返回 | 平台 | 作用 |
|---|---|---|---|---|
| `set_goal` | `text` | `()` | 双 | 设/改今日目标 |
| `update_goal_status` | `id`, `status` | `()` | 双 | started/completed/skipped/abandoned |
| `get_today_context` | — | `TodayContext` | 双 | 今日摘要（与 MCP `query_context` 同源） |
| `list_tasks` | — | `[Task]` | 双 | 列任务（≤100） |
| `create_task` | `title`, `due_ms?`, `note?` | `String`(id) | 双 | App 直建任务（无 AI 溯源） |
| `set_task_status` | `id`, `status` | `()` | 双 | 改任务状态 |
| `delete_task` | `id` | `()` | 双 | 删任务 |
| `list_reminders` | — | `[Reminder]` | 双 | 列提醒（≤100） |
| `set_reminder_status` | `id`, `status`(done\|cancelled) | `()` | 双 | 完成/取消提醒 |

### 2.3 数据展示（记录页）

| 命令 | 参数 | 返回 | 平台 | 作用 |
|---|---|---|---|---|
| `list_interventions` | — | `[InterventionRow]` | 双 | 干预历史（≤50） |
| `list_sessions` | — | `[SessionRow]` | 双 | 行为时间轴（≤60） |
| `list_knowledge` | — | `[KnowledgeNote]` | 双 | 知识卡列表（与 MCP 同源） |
| `list_scheduled_actions` | — | `[ScheduledAction]` | 双* | 主动计划：即将执行的排程（*安卓暂无调度→空） |

### 2.4 第二大脑 / 系统

| 命令 | 参数 | 返回 | 平台 | 作用 |
|---|---|---|---|---|
| `get_vault_path` | — | `String` | 🖥️ | 知识库路径（供「在 Obsidian 打开」） |
| `run_knowledge_agent` | `topic` | `String` | 🖥️ | 派发 Codex 深研/批量（需配 `SISYPHUS_KNOWLEDGE_AGENT_SCRIPT`） |
| `list_monitored_apps` | — | `[MonitoredApp]` | 双 | 监控名单（与 MCP 同源） |
| `add_monitored_app` | `id`, `category` | `()` | 双 | 加监控 |
| `remove_monitored_app` | `id` | `()` | 双 | 删监控 |
| `ping` | — | `String`("pong") | 双 | 连通性自检 |

### 2.5 LifeDB、同步与配置

| 命令 | 参数 | 返回 | 平台 | 作用 |
|---|---|---|---|---|
| `list_life_items` | `include_archived?` | `[LifeItem]` | 双 | 四视图同一数据源 |
| `upsert_life_item` | `input: LifeItemInput` | id | 双 | App 写入，后端强制 `origin=app` 并排队同步 |
| `archive_life_item` | `id` | `()` | 双 | 归档并排队同步 |
| `link_life_items` / `list_life_item_edges` | 关系参数 / — | `()` / edges | 双 | 编辑/读取 LifeItem 关系 |
| `get_lifeindex_sync_overview` | — | 配置状态 + projection + sync state | 双 | 显示 dirty、成功时间与错误 |
| `run_lifeindex_sync` | — | `AgentRunOutput` | 🖥️ | 立即运行受限三方语义合并 |
| `get_notion_config` | — | 无 token 的配置状态 | 双 | token 只返回 `has_token` |
| `set_notion_config` | token/page_id/sync_enabled | `()` | 双 | 保存固定页面配置并请求同步 |
| `clear_notion_config` | — | `()` | 双 | 清空连接配置，不删除 LifeDB |

### 2.6 安卓专属（前台采集 · 特殊权限）

| 命令 | 参数 | 返回 | 平台 | 作用 |
|---|---|---|---|---|
| `check_usage_permission` | — | `bool` | 📱 | 使用情况访问权限是否已授予（桌面恒 false） |
| `request_usage_permission` | — | `()` | 📱 | 跳转系统「使用情况访问」设置页 |
| `start_collector` | — | `()` | 📱 | 启动前台采集服务 |
| `stop_collector` | — | `()` | 📱 | 停止前台采集服务 |

### 2.7 事件 / 插件桥（非命令，供参考）

不是可调用命令，而是**事件监听 / 移动插件桥**：
- JS 监听 `usage`/`usage_event`（Kotlin UsagePlugin 推前台 app）、`notification`/`action_taken`（通知按钮响应）。
- JS `invoke("plugin:notification|showIntervention", …)`（安卓 Kotlin NotificationPlugin 弹干预）。

---

## 3. 能力 × 面 矩阵（谁支持哪个能力）

| 能力 | MCP（Agent） | App（前端） | 说明 |
|---|---|---|---|
| 记录一句话 capture | ✅ `capture` | ✅ `ingest_event` | 都写 Event log |
| capture→意图→artifact 收件箱 | ✅ 全套 | ❌ | 反思平面专属（Agent 做分类） |
| 设今日目标 | ✅ `set_goal` | ✅ `set_goal` | 同源 |
| 今日上下文 | ✅ `query_context` | ✅ `get_today_context` | 同一 core 函数 |
| 今日最小行动 | ✅ `today_actions` | ❌ | Agent 规划用 |
| 任务/提醒 增删改 | ⚠️ 仅经 `accept_intent` 间接建 | ✅ 直接 CRUD | App 是手动 CRUD 主场 |
| 目标状态流转 | ❌ | ✅ `update_goal_status` | App 专属 |
| 跑规则引擎 / 干预 | ❌ | ✅ `evaluate_rules` | 确定性引擎，Agent 不逐拍驱动 |
| 干预历史 / 行为时间轴 | ❌ | ✅ `list_interventions`/`list_sessions` | App 展示 |
| 知识：查 | ✅ `search_knowledge`/`list_knowledge` | ✅ `list_knowledge` | 列表两面都有；搜索仅 MCP |
| 知识：写/删/归档原文 | ✅ `write_knowledge_note`/`save_source`/`delete_note`/`ingest_document` | ❌ | Agent 加工专属 |
| 知识：派发深研/批量 | ❌ | ✅ `run_knowledge_agent` | App 拉起 Codex |
| 监控名单 增删查 | ✅ | ✅ | 两面同源 |
| 主动计划（调度队列） | ❌ | ✅ `list_scheduled_actions` | 暂只读；写入在 core/app 内部 |
| LifeItem 增删改查 / 关系 | ✅ | ✅ | 两面同源；App 写强制标记 local_dirty |
| Notion 双向语义同步 | ✅ `LifeIndexSync` 白名单 | ✅ 手动触发/看状态 | 网关固定唯一 page ID；普通模式不可写 Notion |
| 系统/权限/采集控制 | ❌ | ✅ | App/端侧专属 |

**规律**：MCP 面偏"**理解 + 加工 + 沉淀**"（capture 闭环、知识工程、上下文），App 面偏"**展示 + 手动 CRUD + 端侧控制**"（任务/提醒 CRUD、规则引擎、权限、采集、主动计划）。两面共享 core 与同一 DB，能力重叠处（目标/上下文/知识列表/监控名单）保证同源不分叉。

---

## 4. 维护约定

- **新增 MCP 工具**：在 `mcp/src/main.rs` 加 `#[tool]` + Req 结构体 → 更新本文件 §1 与 §3。
- **新增 App 命令**：在 `commands.rs`（或 `lib.rs`）加 `#[tauri::command]` + 注册进 `generate_handler![]` → 更新 §2 与 §3。
- 二者都应经 `sisyphus-core` 落库（守唯一事实来源）；`tokio`/`rmcp`/进程/通知只在 mcp 或 app 层，**绝不进 core**（安卓构建铁律）。
- 本文件为手工维护的真相；MCP 侧可随时用 `tools/list` 交叉核对参数 schema。

---

## 2026-07-30 变更（工具面）

**新增（只读）**：`read_knowledge_note` · `kb_doctor` · `kb_wanted` · `life_tree` · `next_actions` ·
`review_queue` · `list_life_areas` · `intervention_outcomes` · `list_lifeindex_runs` · `lifeindex_rollback_text`

**新增（写）**：`append_knowledge_section`（结晶化的默认写入路径）· `merge_knowledge_notes` ·
`kb_reindex` · `upsert_life_area`

**契约变更**：
- `write_knowledge_note.folder` 由可选变为**必填且必须以 `kb/` 开头**；新增 `aliases`；
  写入前校验 tags（恰好一个类型 + 一个可靠性档）、`links`（≥1，MOC 豁免）、高可靠性档必须有 `sources`。
  幂等键由 `path` 改为 **`title`**。
- `search_knowledge` 检索**含正文**，返回 `{title,path,folder,tags,excerpt,updated_at}`。
- `query_context` 的 `open_tasks` → **`open_items`**（LifeItem 全字段），新增 `next_actions`；
  `recent_interventions` 增加 `outcome`；`date` 是本地逻辑日。
- `propose_intents` 的 `kind` 增加 **`life_item`** 与 **`rule`**。
- `upsert_life_item` 的 `kind` 增加 `skill` / `milestone`；新增 `area_id` / `success_criteria` /
  `target_value` / `current_value` / `unit`。
- `query_timeline` 返回新增 `bucket` / `bands` / `plans`，`events[]` 增加 `level`（显著性等级）。

**Tauri 命令**同步新增：`kb_doctor` / `kb_wanted` / `kb_reindex` / `life_tree` / `next_actions` /
`review_queue` / `list_life_areas` / `upsert_life_area` / `intervention_outcomes` / `rebuild_rollups` /
`set_day_boundary_hour` / `get_day_boundary_hour` / `list_lifeindex_runs`。
