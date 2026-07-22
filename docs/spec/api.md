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

实现：`sisyphus/src-tauri/crates/mcp/src/main.rs`（`rmcp` stdio）。由 Codex/Claude 作子进程拉起，打开与 App 同一个 `sisyphus.db`。共 **17 个工具**。

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

---

## 2. Tauri 命令面（App IPC · 前端 `invoke()`）

实现：`sisyphus/src-tauri/src/commands.rs` + `src/lib.rs`；注册于 `lib.rs` 的 `generate_handler![]`。共 **26 个命令**。平台列：🖥️ 桌面 / 📱 安卓 / 双 = 两端。

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

### 2.5 安卓专属（前台采集 · 特殊权限）

| 命令 | 参数 | 返回 | 平台 | 作用 |
|---|---|---|---|---|
| `check_usage_permission` | — | `bool` | 📱 | 使用情况访问权限是否已授予（桌面恒 false） |
| `request_usage_permission` | — | `()` | 📱 | 跳转系统「使用情况访问」设置页 |
| `start_collector` | — | `()` | 📱 | 启动前台采集服务 |
| `stop_collector` | — | `()` | 📱 | 停止前台采集服务 |

### 2.6 事件 / 插件桥（非命令，供参考）

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
| 系统/权限/采集控制 | ❌ | ✅ | App/端侧专属 |

**规律**：MCP 面偏"**理解 + 加工 + 沉淀**"（capture 闭环、知识工程、上下文），App 面偏"**展示 + 手动 CRUD + 端侧控制**"（任务/提醒 CRUD、规则引擎、权限、采集、主动计划）。两面共享 core 与同一 DB，能力重叠处（目标/上下文/知识列表/监控名单）保证同源不分叉。

---

## 4. 维护约定

- **新增 MCP 工具**：在 `mcp/src/main.rs` 加 `#[tool]` + Req 结构体 → 更新本文件 §1 与 §3。
- **新增 App 命令**：在 `commands.rs`（或 `lib.rs`）加 `#[tauri::command]` + 注册进 `generate_handler![]` → 更新 §2 与 §3。
- 二者都应经 `sisyphus-core` 落库（守唯一事实来源）；`tokio`/`rmcp`/进程/通知只在 mcp 或 app 层，**绝不进 core**（安卓构建铁律）。
- 本文件为手工维护的真相；MCP 侧可随时用 `tools/list` 交叉核对参数 schema。
