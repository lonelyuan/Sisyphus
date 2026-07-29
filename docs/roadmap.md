# 路线图

本文档描述西西弗斯计划从个人实验项目到可推广产品的阶段性路线。

**架构总纲见 [spec/architecture.md](spec/architecture.md)**（两平面 + 双存储 + SOC 心智模型），本文件只讲阶段与验收。当前判断：不要先做"大而全的个人助理 App"，也不要先设计一套宏大的 Core 对象模型；应先给 SIEM **接上第一根真实的电线**，让感知平面与反思平面各自跑通最小闭环。

核心原则：

- 每个阶段都必须产生可验证闭环，而不是只完成孤立组件。
- 业务能力沉淀在数据层 + 引擎（Core），不写死在某个 Agent prompt、Codex skill 或 runtime session 中。
- Agent runtime 可以替换（现用 Codex/Claude Code，后期自研），数据模型、引擎、调度、反馈必须由项目自己掌控。
- **Core 是萃取出来的，不是提前设计的**：第二个采集源落地前，不新增 artifact 抽象。
- 技术选型服务长期复杂项目，早期可以用过渡方案，但要保留迁移边界，避免不可逆技术债。

---

## 进度快照（2026-07-15）

Phase 0 / 1.0 / 1.1 / 1.2 / 1.3 的核心闭环均已落地并在本机跑通（Tauri v2 + Rust `sisyphus-core` + rmcp MCP + React/Tailwind 深色 UI）。逐项状态见各阶段勾选框。

| 阶段 | 状态 | 一句话 |
|---|---|---|
| Phase 0 技术验证 | ✅（同步除外） | Tauri/SQLite/协议/`ingest_event`/MCP 全通；Supabase 同步延后至 Phase 2 |
| 1.0 Core MVP | ✅ | capture→propose_intents→accept_intent→artifact；per-type 表 + `intent_candidates` 审计 |
| 1.1 拖延干预 | ✅ 闭环 / ⏳ 真机验证 | macOS 采集器 + Android UsageStats(JNI，后台可弹) + 规则 + 通知 + 反馈；缺 outcome 观察、LLM 文案 |
| 1.2 原声笔记 | ✅ | 收件箱 → Codex 分类 → accept/edit/ignore 落 artifact；不自建 UI，走 Codex |
| 1.3 第二大脑 | ✅ 核心 | Obsidian vault(.md+`[[wikilink]]`) + `knowledge_notes` 索引 + Codex TS SDK 派发；缺关系判定/剪枝 |

**超出原 MVP 的额外落地**：感知平面 App 常驻（tray / 关窗不退 / 开机自启）、Codex 风深色 UI（今日 / 记录 / 设置 三页 + 目标·任务 CRUD）、监控名单跨端增删查改、Android 端可编译运行。

**当前主线（2026-07-27 调整）**：LifeDB 自建结构化人生看板 + Notion 普通文本双向投影。见 [spec/notion-integration.md](spec/notion-integration.md)。

> **kill-criteria 已废弃（2026-07-20 用户决定）**：原以"连续 5 天通知未改变行为→证明无效"当 kill 门。用户判断该门逻辑上无法证伪（永远可归因于"实现不够好"），既不能证伪 idea、反而拖着不敢往下做。故不再当验证门槛，改当**持续打磨的实现质量问题**。"主动提醒能改变行为"作为项目公理，实现形态（宠物/系统通知/遮罩/对话）持续迭代。

---

## 总体架构心智模型

西西弗斯从两个维度组织。

### 横向软件组件

| 组件 | 职责 | 备注 |
|---|---|---|
| 事件与数据层 | 保存行为事件、用户输入、任务、知识、记忆、反馈 | 本地 SQLite + Supabase/Postgres 过渡；后期自建服务端 |
| 状态机层 | 管理意图、任务、提醒、干预、知识节点的合法状态转换 | 避免 loose JSON 失控 |
| Agent 基座层 | LLM 调用、工具调用、上下文构建、流式输出、多 Agent 编排 | 可先用 Codex/ChatGPT skill dogfood，后续评估 Pi runtime / 自研 runtime |
| 策略与调度层 | 今日行动选择、提醒时机、冷却、反馈学习 | 先规则，后模型 |
| 知识处理层 | 文档摄取、摘要、概念抽取、关系合并、检索、剪枝 | 不把原始材料简单堆进知识库 |
| UI 层 | 今日页、收件箱、时间轴、知识视图、设置和权限 | UI 是数据投影，不是核心状态来源 |
| 同步与权限层 | 多端同步、隐私等级、授权、导出、删除 | MVP 单用户，推广阶段必须重做多用户安全边界 |

### 纵向业务模块

| 模块 | 目标 | 核心闭环 |
|---|---|---|
| 总入口 · InputBox + 意图识别（原 1.2 原声笔记） | 无压记录 + 功能路由 | 任意输入 → `capture` → 意图识别 → 路由到下面三模块 |
| 1.1 西西弗斯计划（狭义） | 低效习惯与拖延的数字干预 | 行为采集 → 风险识别 → Agent 干预 → 用户反馈 → 近端结果 |
| LifeIndex · 人生看板 | 低摩擦记录 + 严格结构化展示 | Notion 自由文本 ↔ Agent 三方语义合并 ↔ SQLite LifeDB → App 四个重叠视图（见 [spec/notion-integration.md](spec/notion-integration.md)） |
| 1.3 第二大脑 | 和人一同进化的知识工程 | 原始材料/目标 → 学习加工 → 知识库(vault)/知识图谱 → 可读输出/教学 |

### 统一入口

用户不应理解内部模块。但要注意：**"统一"发生在数据层，不发生在管道**。系统有两条不同形状的管道，见 [spec/architecture.md](spec/architecture.md)：

```text
感知管道（实时·确定性）：
  行为事件 → ingest_event → Event log → 规则引擎 → finding → 本地干预

反思管道（人类节奏·自然语言）：
  一句话 / 语音 / 文件 / 链接
    → capture（写 Event log）
    → propose_intents / accept_intent
    → Artifact（目标 / 任务 / 提醒 / 知识 / 复盘）
    → 用户确认或安全自动执行
```

二者共享 **Event log（信封统一）**，但行为事件不产生"意图候选"、笔记不进规则引擎——不要用一条 `capture→intent` 管道强行兜住行为事件。

因此，所有业务模块共享这些核心服务（Rust 数据层 + MCP 工具面，见 [spec/architecture.md](spec/architecture.md) §3–4）：

- `capture(text | file | url | event)`：接住原始输入。
- `propose_intents(capture_id)`：生成意图候选。
- `accept_intent(intent_id)`：将候选意图转为正式对象。
- `create_artifact(...)`：创建目标、任务、记忆、知识节点、提醒等。
- `query_context(scope)`：为 Agent 构建最小必要上下文。
- `select_today_actions()`：选择今日最小行动。
- `schedule_reminder(...)`：安排固定或条件提醒。
- `record_feedback(...)`：记录用户反馈。
- `record_outcome(...)`：记录提醒后的近端结果。
- `ingest_document(...)`：处理材料并进入知识系统。

---

## Phase 0 — 技术验证阶段

**目标**：验证核心技术选型可行，建立可扩展、可迭代的工程底座，避免在长期大型复杂项目中留下不可迁移的技术债。

当前技术判断：

- 跨端 App：选择 Tauri 技术栈，复用 Rust 本地能力与 Web UI 生态。
- 本地存储：SQLite 作为端侧事实缓存、outbox 和离线规则查询基础。
- 服务端过渡方案：Supabase/Postgres 用于数据同步、事实汇聚、Realtime/command queue 和早期 pgvector/RAG 实验。
- 长期服务端：后期迁移到自建服务端，承载多用户、重计算、模型训练、复杂权限和调度。
- Agent 运行时：短期可用 Codex/ChatGPT skill、CLI、脚本进行 dogfood；中期评估 Pi agent runtime / SDK 作为可嵌入 Agent 基座；Sisyphus Core 保持运行时无关。

验收标准：

- [x] Tauri Desktop / Android 基础工程可运行，能调用 Rust command。
- [x] 本地 SQLite schema 初始化、读写、迁移机制可验证。（`CREATE TABLE IF NOT EXISTS` 增量演进）
- [x] append-only `raw_events` + outbox 模式可跑通。
- [ ] Supabase/Postgres 能接收批量事件，保持幂等写入。（**延后 Phase 2**：outbox 已排队，未接上传）
- [x] 端侧事件协议、服务端 schema、TypeScript 类型保持一致。（`SPEC.md` ↔ `events.ts`；Supabase schema 待同步时校准）
- [x] 基础隐私等级模型存在：L0/L1 默认，L2/L3 明确授权。（`privacy_level` 字段，采集只产 L0–L1）
- [x] Agent 调用与业务逻辑解耦：Agent 只能通过 MCP 调用 Sisyphus Core。

里程碑：

- 完成“本地输入 → 本地落盘 → 可选同步 → Agent 查询上下文”的最小链路。
- 明确哪些代码属于过渡适配层，哪些属于长期 Core。

---

## Phase 1 — 模块验证阶段

**目标**：按业务模块分别开发和验证关键 idea，避免一开始构建完整产品外壳。阶段结束时，每个模块都应有一个可自用的纵向闭环。

### Phase 1.0 — Sisyphus Core MVP

所有业务模块先共享一个最小 Core。

核心对象：

- `capture_items`：原始输入收件箱。
- `intent_candidates`：Agent 生成的意图候选。
- `artifacts`：目标、项目、任务、笔记、记忆、知识节点、提醒等统一对象。
- `artifact_relations`：对象间关系，支持树状 UI 和图状语义。
- `events` / `raw_events`：行为事件与系统事件。
- `reminders` / `interventions`：提醒和干预。
- `feedback_events` / `outcomes`：用户反馈和近端结果。

> **实现校准**：最终未造多态大表 `artifacts`，改为**每种对象各自建表**（`daily_goals`/`tasks`/`notes`/`reminders`/`knowledge_notes`）；capture 不单列 `capture_items`，而是 Event log 里的 `note_text` 事件；`artifact_relations` 未提前造（1.3 知识关系用 vault `[[wikilink]]`）。见 [spec/architecture.md](spec/architecture.md) §2。

验收标准：

- [x] `capture(text)` 能保存任意自然语言输入。
- [x] `propose_intents(capture_id)` 能输出结构化候选意图。（Codex 生成、MCP 持久化）
- [x] `accept_intent(intent_id)` 能创建或更新 artifact。（含 `edits` 修改、`ignore` 回滚）
- [x] `select_today_actions()` 能选出 1–3 个今日最小行动。（`today_actions`：目标 + 未完成任务）
- [x] 所有 AI 推断有来源、置信度和可回滚状态。（`intent_candidates`：capture_event_id/confidence/status）
- [x] Core 暴露 SDK 接口，供 Codex skill / 自研 UI 调用。（rmcp MCP + App 命令 + Codex TS SDK 派发）

### Phase 1.1 — 西西弗斯计划（狭义）：拖延与低效习惯干预

**目标**：验证数字干预闭环是否有效。重点关注用户行为记录和同步、习惯模型的联合建模、行为检测、干预反馈闭环，以及具有情感价值的智能体提醒。

核心闭环：

```text
跨端行为采集
  → 统一事件库
  → 规则/风险识别
  → 策略选择干预动作
  → Agent 生成提醒
  → 端侧执行
  → 用户反馈和近端结果
```

MVP 范围：

- Android Usage Stats 采集前台 app。
- Desktop 活动窗口/浏览器插件作为后续扩展。
- 娱乐/信息流规则：目标未完成 + 娱乐会话超过阈值 + 冷却满足。
- 通知干预：开始任务、合理休息、继续娱乐、放弃今日。
- 今日最小行动与行为数据联动。

验收标准：

- [x] 手机端刷 B 站/抖音超过阈值时，能基于今日目标触发提醒。（Android UsageStats→JNI→规则→Kotlin 通知，后台可弹；⏳ 真机连续验证中）
- [x] 用户点击反馈后写入本地数据库和 outbox。（通知按钮→`record_feedback`；事件入 outbox）
- [ ] 系统能观察提醒后 10/30/60 分钟近端结果。（**未做**：`interventions.outcome` 列已留，回填逻辑待写）
- [~] Agent 能生成不羞辱、不说教、引用实际上下文的提醒文案。（现为**确定性模板**：引用真实时长+目标、不羞辱；LLM 生成待做）
- [~] 规则和策略分离：规则只识别机会，策略决定是否提醒和如何提醒。（规则识别✓；策略层仅冷却，contextual bandit 属 Phase 2.1）

### Phase 1.2 — 原声笔记：无压记录的个人助手

**目标**：验证“零压力记录 + 意图取代 TODO”的产品假设。用户可以输入任意时间、琐碎、非结构化的内容，系统从中提取意图，并自动整理为日程、习惯、提醒、素材、知识或今日行动。

核心闭环：

```text
散乱输入
  → 本地 Capture 待处理队列（不是 Notion Inbox）
  → 意图提取
  → 候选对象
  → 用户确认/安全自动落盘
  → 今日行动或未来提醒
```

典型输入：

- “我想学吉他。”
- “最近想增强社交能力，避免孤独。”
- “周末提醒我约朋友吃饭。”
- “这篇 AI Infra 文章之后要看。”
- “我今天什么都不想干，只想刷视频。”

验收标准：

- [x] 输入一句自然语言后，系统能判断是目标、任务、提醒、材料、偏好、情绪还是反馈。（Codex 分类为 goal/task/reminder/note；情绪→打标 note，材料→知识库，反馈→事件）
- [x] 系统只提出最小下一步，不生成任务海。（`SKILL.md` 明确「只提最小候选」）
- [x] 用户可一键接受、修改、忽略。（`accept_intent` / `accept_intent(edits)` / `ignore_intent`）
- [x] 今日页只展示 1–3 个最小行动。（`today_actions` 上限 3）
- [x] 已接受意图能被提醒、复盘和后续对话引用。（`query_context` 含未完成任务 + 到期提醒）

### Phase 1.3 — 第二大脑：和人一同进化的知识工程

**目标**：验证“材料不是堆放，而是学习加工和内化”的知识系统。给定目标领域或原始材料引用，系统可使用 deep research 等工具自主扩展资料，并将可验证知识总结为知识库/知识图谱，保持人类高度可读。

核心闭环：

```text
目标领域 / 原始材料
  → 来源保存
  → 摘要和概念抽取
  → 关联已有知识节点
  → 待确认关系
  → 可读知识卡片 / 博客式输出 / 图谱视图
  → 定期剪枝和更新
```

MVP 范围：

- 原始材料保存来源和 hash。
- 文档 chunking、摘要、概念提取。
- `knowledge_node` 与 `artifact_relations` 构成图谱底座。
- 使用 Markdown/博客式长文作为人类可读输出。
- 低置信度合并进入待确认队列，不污染知识库。

验收标准：

- [x] 输入一篇文章或链接，系统能生成 5 行以内摘要和 3–10 个概念节点。（Codex + `write_knowledge_note`；已真机写出「游泳知识库」5 张卡片）
- [~] 系统能判断材料与已有节点的关系。（关系靠 Codex 写 `[[wikilink]]`；无自动关系判定/合并/共享根节点推断）
- [~] 用户能纠正分类和关系。（`.md` 可直接在 Obsidian 编辑；App 内知识列表暂只读）
- [x] 知识节点能被后续对话、今日行动、学习计划引用。（`search_knowledge` / `list_knowledge` + query）
- [ ] 系统能标记过期、重复、低价值节点，进入剪枝候选。（**未做**：`status` 列已留 stale/duplicate/pruned，判定逻辑待写）

---

## Phase 2 — Feature 完善阶段

**目标**：在坚实的 Core、状态机、数据层和模块闭环上，开发预期特色功能。此阶段才开始追求“产品形状”。

### Phase 2.1 — 西西弗斯计划：跨端同步与联合建模

- [ ] 行为采集扩展到 Desktop、浏览器、更多 Android 信号。
- [ ] 支持更多 IoT / 可穿戴设备，例如睡眠、运动、久坐、心率等 proxy。
- [ ] 多端事件聚合为跨端 session。
- [ ] 从固定规则升级为可学习策略：风险模型、contextual bandit、离线策略评估。
- [ ] 使用更复杂 AI 模型学习用户行为，输出更精准、更高成功率的干预措施。
- [ ] 建立策略回放和评估系统，避免提醒疲劳和误报伤害用户体验。

### Phase 2.2 — 原声笔记：进化为数字员工

- [ ] 从“记录和提醒”升级为“处理现实工作”。
- [ ] 接入邮件、日历、文档、浏览器、文件系统等用户授权视野。
- [ ] 支持把自然语言意图转为多步骤工作流。
- [ ] 支持主动跟进：等待结果、检查状态、提醒用户决策。
- [ ] 支持 computer use / browser use，但必须受权限、确认和审计约束。
- [ ] 建立长期用户模型：偏好、工作节奏、社交关系、常用流程。

### Phase 2.3 — 第二大脑：自主学习、自主探索、自主验证

- [ ] 支持围绕一个领域自动制定学习路线。
- [ ] 接入 deep research、论文/网页/视频/书籍等多源材料。
- [ ] 自动生成知识图谱、学习地图、博客式综述和问答卡片。
- [ ] 对关键知识进行来源追踪、交叉验证和置信度评估。
- [ ] 周期性更新知识节点，淘汰过时内容。
- [ ] 不只保存知识，还要能“教会用户”：生成课程、练习、测验和复盘。

---

## Phase 3 — 推广阶段

**目标**：在个人自用验证充分后，考虑将项目推广为创业项目，面向多用户提供服务。

关键任务：

- [ ] 从单用户本地优先架构迁移到多用户服务架构。
- [ ] 自建服务端替换 Supabase 过渡能力，或将 Supabase 限定为明确边界内的基础设施。
- [ ] 重新设计认证、授权、RLS/租户隔离、密钥管理和审计。
- [ ] 建立数据导出、删除、隐私授权、模型调用透明度和合规策略。
- [ ] 明确商业定位：个人效率工具、AI 助理、知识管理、行为干预，还是垂直人群解决方案。
- [ ] 建立可观测系统：事件处理延迟、提醒效果、用户留存、干预厌烦率、知识复用率。
- [ ] 设计付费模型和成本控制：LLM 调用、向量检索、后台任务、存储、同步、推理成本。

阶段性判断标准：

- 个人自用场景连续数周稳定产生价值。
- 核心闭环能被非开发者用户理解和使用。
- 数据安全边界清楚。
- Agent 自动行为有审计、撤销和用户控制。
- 产品不是 Notion/Todo/Calendar 的简单替代，而是能证明“低摩擦意图系统”带来新增价值。

---

## 近期推荐下一步

三条轨道——A 反思平面、B 感知平面（macOS 采集器）、跨端安卓（UsageStats→JNI 后台闭环）——都已跑通。当前主线是把 **LifeDB 作为人生规划事实层**，让 App 严格展示、Notion 自由编辑，并由受限 Agent 自动双向合并。

### 当前冲刺：以可用性为目标推进（分批，逐批验证 + 更新本节）

目标：让 **Pi / Codex 两个基座都能从 app 内真正驱动**西西弗斯全部功能（反拖延含动态规则、第二大脑、人生看板），并富化时间轴。计划见 `.claude/plans/`。

- **批次 A ✅ 已落地（本次）** — 地基：
  - 解除 in-app 智能体的硬编码只读：`agent_runtime::RunMode{Interactive,Proactive}`；主对话/宠物=可写（可 `set_goal`/`add_monitored_app`/建规则/写知识），定时/规则触发=严格只读。外部源（Notion）恒只读。
  - 修编译期路径依赖：`agent_runtime::init_paths` 用运行期 `resource_dir()` 注入 skills/pi-runner/mcp 路径；release 不再烘焙 `CARGO_MANIFEST_DIR`（退回 `current_exe`）；`tauri.conf.json` 打包 `skills/`、`scripts/`。
  - 修调度器阻塞：`agent_run` 移到 worker 线程，不再卡住 30s ticker 里的 `notify`。
  - 立 `ResponsePolicy` seam（core `rule_engine`：Immediate/Deferred/Debounce/Suppress）：命中即时→notify，延后/防打扰→入队 `scheduled_actions`；补 `pet_message` 派发分支（emit `pet-message`，`Pet.tsx` 已监听）。
- **批次 B ✅ 已落地（本次）** — Pi 基座接 MCP 工具面：`scripts/pi-agent-runtime.mjs` spawn `sisyphus-mcp`（stdio），`listTools` 后用 `Type.Unsafe(inputSchema)` 逐个包成 Pi `customTool`；系统提示按 `SISYPHUS_READ_ONLY` 切交互/只读。新增依赖 `@modelcontextprotocol/sdk`、`@sinclair/typebox`。桥已冒烟验证（连上→17 工具→set_goal 写→query_context 读回）。Pi 与 Codex 现共用同一工具契约。
- **批次 C ✅ 已落地（本次）** — 动态规则引擎（“一句话建规则”）：`detection_rules` 表 + 声明式 `core::rules::DynamicRule`（category_prefix/category_in/app_in + window/threshold + requires_active_goal + time_of_day，跨午夜）；`RuleEngine::evaluate` 每次热加载启用规则。MCP 工具 `create/list/set_enabled/delete_detection_rule`（可写门禁）+ 同名 Tauri 命令 + Settings 规则列表（查看/启停/删）+ skill `references/rules.md`。core 单测 3/3、MCP 建/列/只读拒写均已冒烟验证。
- **批次 D ✅ 已落地（本次）** — 人生看板 LifeIndex：`lifeindex_cards` 表（(section,title) 幂等 upsert，可重建投影）+ MCP `upsert_lifeindex_card`/`list_lifeindex`/`delete_lifeindex_card` + Tauri `list_lifeindex` + 「看板」tab（分区卡片、Notion 溯源链接）。MCP 写门禁细化为三档 `write_scope`（只读 / 仅看板 / 全写）；新增 `RunMode::LifeIndex` + 每日 8:30 `agent_run(mode=lifeindex_refresh)` job：agent 只读参考 Notion + 本地上下文后仅写本地看板，绝不回写 Notion。已冒烟验证（仅看板可写、set_goal 被拒、list 正常）。
- **批次 E ✅ 已落地（本次）** — 时间轴富化：`query_timeline` 新增 artifact 里程碑图层（目标/任务/提醒/知识卡片/规则创建，点事件）+ note_text 重标为 capture 图层；`TimelineScreen` 按 kind 配色、里程碑画独立小圆标记、详情面板显中文类型标签。所有尺度都展示稀疏里程碑。
- **批次 F ✅ 已落地（本次）** — 清理死代码（删 `TodayScreen`/`RecordsScreen` + `list_sessions`/`list_recent_sessions`/`SessionRow`）；回写 spec（rule-engine 动态规则、proactive-triggers §4/§7 状态、architecture §9、local-storage 表清单、agent 运行模式）；更新项目记忆（proactive_triggers / pi_agent_inapp）。
- **批次 G ✅ 已落地（同日追加）** — 排查"智能体没权限"发现两个根因并修复：① `pi-agent-runtime.mjs` 的 `DefaultResourceLoader` 会自动加载项目/全局 `.pi/extensions/pi-permission-system`（给交互终端 UI 写的扩展），在我们的 headless 子进程里对每次工具调用返回"需要审批但无 UI"而拒绝——加 `noExtensions: true` 跳过；② SDK 文档说 `noTools:"builtin"` 会保留 custom tools，但实现里未提供 `tools` 允许名单时不会激活它们——改为显式 `tools: sisyphusTools.map(t=>t.name)`。顺带接入 Notion 只读集成：官方 `@notionhq/notion-mcp-server`，Pi 侧多开一个 MCP client 合并工具面、Codex 侧用 `-c mcp_servers.notion.*` 注入，token 存 `notion_config.json`（0600）+ Settings 新卡片；只读边界由 Notion 侧 integration 权限（仅 "Read content"）机制保证。`AgentScreen` 会话删除失效（`window.confirm` 在 Tauri webview 里常返回 false）一并修复。详见 `docs/spec/notion-integration.md` §8"现状"小节。
- **批次 H ✅ 已落地（2026-07-27）** — LifeDB / LifeItem / LifeIndex：SQLite 新增统一人生规划对象、关系、外部引用和三方合并快照；旧 tasks/LifeIndex 卡片幂等迁入。MCP/Tauri 增加 LifeItem CRUD、关系、投影和同步完成 API；Pi/Codex 新增 `LifeIndexSync` 白名单模式。Notion 改为固定单页受限网关，只暴露整页 Markdown 读/替换；每日 8:30 入站、本地修改后即时出站、App 手动同步。看板 UI 重构为事项/日常/主线/支线四个重叠视图 + 待整理，并可完整编辑字段。详见 `docs/spec/notion-integration.md`。

### 主线：LifeDB + Notion 文本投影闭环（已完成首版）

心智模型见 [spec/notion-integration.md](spec/notion-integration.md)：**LifeDB 是事实源，App 是严格视图，Notion 是自由文本交互层，Agent 是语义编译器。**

1. **已完成：LifeDB 数据模型**：五种 kind + track + horizon + 状态/时间/循环 + 邻接关系。
2. **已完成：受限双向同步**：三方快照、乐观 revision、固定单页网关、失败审计和并发保护。
3. **已完成：App 四视图**：事项/日常按形态，主线/支线按意义，允许同一项重叠展示。
4. **下一步：同步实测与恢复 UI**：用真实 LifeIndex 页面覆盖首次导入、并发编辑、删除冲突、Notion 限流和子页面保护；为 conflict 提供显式人工合并入口。

### 让闭环真正有用的小补丁（按性价比排序，都不需新架构）

1. **近端结果 outcome 观察**（1.1 缺口）：干预/提醒后 10 / 30 min 回看前台在干嘛，回填 `interventions.outcome`。把"感觉有没有用"变成"数据说话"。采集器 tick 里加一步即可。
2. **浏览器插件**（桌面最大信号缺口）：`packages/browser-extension` 已有骨架。桌面真正的刷视频在浏览器内、原生 app 白名单抓不到。接一个 tab/URL → `ingest_event(url_visit)` 的最小插件。
3. **LLM 生成干预文案**：现为 Rust 确定性模板。让反思平面（Codex）在冷却窗口预生成几条个性化文案缓存，命中时取用——引擎实时触发 + Agent 措辞温度。

### 之后（Phase 2，明确延后）

Supabase 同步 / 跨端联合分析、可学习策略（contextual bandit / 离线策略评估）、知识关系判定与剪枝、自研 Agent 基座、多用户安全边界。

### 对拖延症开发者的推进纪律（不变）

- 里程碑按"天"记，每步当天能自用、拿到多巴胺。
- 每个里程碑限时，超预估 2 倍还没通就砍需求，不许加东西。
- 明确**延后不碰**：Supabase 同步、可学习策略、自研基座、多用户——全部等主线闭环稳定再说。
