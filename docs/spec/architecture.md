# Spec: 总体架构

本文件是西西弗斯的**架构权威文档**。其他 spec（protocol / local-storage / rule-engine / agent / sync / android-collection / notion-integration / proactive-triggers / api）都是本文件某一部分的展开。改动架构时先改本文件。

---

## 0. 一句话心智模型：西西弗斯是一个 SOC

把系统类比为企业安全运营中心（SOC）：

| SOC | 西西弗斯 |
|---|---|
| 各类 sensor（HIDS / XDR / NDR） | **采集源**：Android 使用情况、桌面前台窗口、浏览器插件、手动输入、Agent |
| 归一化事件格式（ECS / OCSF） | `BehaviorEvent` 统一信封（见 [protocol.md](protocol.md)） |
| 事件存储 | **Event log**（append-only） |
| 关联 / 检测规则引擎 | **习惯引擎**（[rule-engine.md](rule-engine.md)） |
| SOAR 响应 | **干预**（通知 / 对话 / 遮罩） |
| 统一控制台 | 一个 Tauri App（一个图标） |

由此得到本项目最重要的两个结论：

1. **业务模块（拖延干预 / 原声笔记 / 第二大脑）不是三个 App**，而是"**采集源 + 规则包 + 视图**"三种配方，跑在同一套 Core 上。
2. **Core 不是 `capture/intent/artifact` 那套对象模型**——那只是"手动输入"这一个源的私有数据。Core 是 SIEM：信封 + 事件存储 + 引擎 + 响应 + 唯一接入契约。

---

## 0.5 产品组件视图：一个总入口 + 三大模块

§0 的 SOC / 两平面是**技术地基**；面向用户，系统组织为**一个总入口 + 三大模块**，全部跑在同一 Core 上（见 §1–§4）：

- **InputBox（无压力记录）—— 总入口**：兼容跨端输入源（Codex、Notion，后续加西西弗斯 APP 本地输入），把任意一句话原样接住（`capture`）。经**意图识别智能体**（现为 Codex/Claude runtime，后期自建 Agent 底座）做**功能路由**，调对应模块。
- **模块一 · 狭义西西弗斯（习惯培养 / 对抗拖延）**：采集端 → 行为日志库 → 习惯引擎（规则 / ML）→ 提醒干预端。项目最早的闭环，即感知平面。
- **模块二 · LifeIndex（人生看板）**：SQLite LifeDB 保存 idea/goal/project/action/routine 及关系；App 提供事项/日常/主线/支线四个严格重叠视图；Notion 是用户可自由编辑的普通文本输入/投影。隔离的 LifeIndexSync Agent 做三方语义合并。见 [notion-integration.md](notion-integration.md)。
- **模块三 · 第二大脑（知识库 / 知识图谱）**：以 Markdown vault 为存储介质，配调研 / 验证 / 梳理智能体构建个人知识库；后续可派生知识图谱（图数据库）。

> InputBox / 意图识别 / 三模块是**产品组件视图**，不改变 §2 的双存储与 §3 的 `ingest_event` 唯一契约：InputBox = `capture`，意图识别 = `propose_intents`，模块产物落 Artifact store 或 vault。**被动采集不经 InputBox**，直接进狭义西西弗斯（感知平面）。

---

## 1. 两个平面

系统沿两个平面组织，二者在同一数据层相遇。**不要把它们混成一个东西**——这是过去"Core 越设计越大、推不动"的根因。

```
      [ Mac 采集器 ]   [ Android 采集器 ]   [ 浏览器插件 ]     ← 感知平面（常驻·确定性·实时·本地）
              \             |               /
               \            |              /
                 ingest_event(BehaviorEvent)                  ← 唯一写入契约
                            ↓
          ┌───────────────────────────────────────┐
          │  Event log   (append-only, 信封+payload) │          ← 统一
          │  Artifact store (goals/tasks/notes/…)   │          ← 不统一，分表
          └───────────────────────────────────────┘
                 ↑ 本地引擎直接读              ↑ MCP 工具读写
        [ 规则引擎 → finding → 通知 ]      [ Codex / Claude Code ]   ← 反思平面（人类节奏·自然语言）
         实时本地干预（拖延模块）          capture / query / today / ingest
                                          后期整体替换为自研 Agent 基座
```

### 1.1 感知 / 响应平面（Sensing / Response）

- **性质**：常驻、确定性、实时、本地优先。
- **载体**：Tauri App（Mac / Android），**必须自建**，无法用现成 Agent 基座替代。
- **职责**：各端采集 → `ingest_event` 写入 → 规则引擎每 ~10s 评估 → 命中即在**本机**弹干预。
- **为什么不能交给 Agent**：LLM 从架构上不适合实时监控每一个动作（见 background.md）；每 10s 调一次 LLM 判断"在不在摸鱼"既慢又贵。实时判定是确定性引擎的活，Agent 只在人类节奏下介入。

### 1.2 反思平面（Reflective）

- **性质**：人类节奏（对话、每日任务、复盘），自然语言入口。
- **载体（现在）**：Codex / Claude Code，通过 **MCP 工具** 读写数据层；用它们现成的对话 UI、工具调用循环、定时任务，**不自建 Agent 基座**。
- **载体（后期）**：迁移到自研 Agent 基座。因为 Agent 只能经 MCP / CLI 碰数据，**换基座时只替换这一个交互前端，Core 与 MCP 能力契约不变**。
- **职责**：原声笔记（capture→意图→artifact）、第二大脑（知识加工）、拖延模块的对话与复盘，以及定时 / 行为触发后基于多源上下文生成一次性主动建议。
- **边界**：Agent **读取**引擎产出（findings、today-context），**不逐拍驱动**引擎。引擎自己常驻自转，Agent 查它的结果。**引擎是常驻的确定性后端，Agent 是按人类节奏介入的交互前端。**

---

## 2. 数据层：两个存储，不是一个

行为事件和文本笔记"怎么统一"的答案：**共享信封、差异入 payload 的那部分进 Event log；有状态、可变的对象进 Artifact store。二者分开。**

### 2.1 Event log（统一）

- append-only，不可变，`event_id` 幂等主键。
- 所有"**发生过的事**"：行为事件、原始文本 capture、finding、decision、intervention、outcome、feedback。
- 统一靠**信封**（见 [protocol.md](protocol.md) §1），异构字段全部关进 `payload`——这正是 SIEM 的 ECS/OCSF 做法。**共享的不是只有时间戳，是整个信封。**
- 例：
  - 行为事件 `source=desktop_agent, layer=raw, type=window_active, entity=com.apple.Safari, category=entertainment.video`
  - 文本输入 `source=manual, layer=raw, type=note_text, payload={text:"我想学吉他"}`

### 2.2 Artifact store（不统一，分表）

- 可变、有生命周期、有 status 的对象：`goals`、`tasks`、`notes`、`interventions`、`knowledge_nodes` 等，**各自建表**。
- **禁止**造一张多态大表 `artifacts` 用 type 字段兜住目标+任务+笔记+知识节点——那是过度抽象，是"Core 推不动"的病根。
- LifeDB 只统一人生规划域，并用 `life_item_edges` 承载目标→项目→行动/日常等关系；其它 artifact 不塞进 LifeItem。

### 2.3 一条铁律

> **在第二个采集源真正落地之前，不准新增任何 Core 抽象 / artifact 表。**
> Core 是被后来的第二个垂直"逼"出来、**萃取**出来的，不是提前设计出来的。

一条 capture 事件（进 Event log）经"意图提取"这一步，产出 / 更新某张 Artifact 表——这是两存储之间唯一的桥。

### 2.4 LifeDB 与外部文本投影

LifeDB 是人生规划域的本地事实层。Notion 不进入 Event log，也不承载结构约束；它是同一 LifeIndex 的用户可编辑 Markdown 投影。

- Core 确定性维护 LifeItem 枚举、关系、revision、dirty 状态和成功快照。
- Agent 对比 Notion 当前文本、上次成功快照和 LifeDB 当前行，负责模糊文本到结构字段的语义编译。
- `life_item_external_refs` 与 `life_sync_state` 保留来源和审计；凭据不进 SQLite。
- 受限网关把 Notion 读写固定到用户配置的唯一页面；普通主动任务只读。

完整的所有权、并发和降级语义见 [notion-integration.md](notion-integration.md)。

---

## 3. 唯一写入契约：`ingest_event`

所有采集源——桌面采集、Android 插件、浏览器插件、手动输入、Agent——最终都走同一条写入路径：

```rust
// Rust 侧统一入口（感知平面各源、反思平面 capture 都调它）
fn ingest_event(event: BehaviorEvent) -> Result<()>;
//   1. 校验信封与 privacy_level
//   2. append 写入 Event log（INSERT OR IGNORE，幂等）
//   3. 写 outbox（供后期同步）
//   4.（可选）就地触发引擎评估
```

- 守住这一个契约 → "一个图标、可插拔的统一 SOC"自然成立。
- 守不住 → 得到三个互不相通的数据孤岛。
- 新增一个采集端 = 新增一个 `ingest_event` 的调用方，**不改数据层**。

---

## 4. MCP 工具面（反思平面契约）

App 侧把数据层能力以 **MCP 工具** 暴露给 Agent（先挂 Codex / Claude Code）。这层是 Agent 与 Core 之间唯一的接口，换基座时保持不变：

| MCP 工具 | 语义 | 读/写 |
|---|---|---|
| `capture(text \| url \| file)` | 接住任意自然语言 / 材料，写一条 `manual` capture 事件 | 写 Event log |
| `query_context(scope)` | 为 Agent 构建最小必要上下文（今日目标、时长、近期干预） | 读 |
| `today_actions()` | 返回 1–3 个今日最小行动 | 读 |
| `propose_intents(capture_id)` | 对一条 capture 生成结构化意图候选 | 读/写 |
| `accept_intent(intent_id)` | 把候选意图落成 / 更新一个 artifact | 写 Artifact store |
| `record_feedback(...)` | 记录用户反馈 | 写 Event log |
| `ingest_document(...)` | 材料摄取（第二大脑） | 写 |

> MCP server 是数据层之上的**薄适配器**，和 Tauri App 读写**同一个 SQLite**。反思平面所有能力都建立在这张表上。

### 4.1 实现（已落地）

反思平面 MCP server 用 **Rust（`rmcp` 官方 SDK，stdio 传输）** 实现，直接链接 `sisyphus-core`，
**零逻辑重复、无 CLI 中间层**（Rust 是核心后端；既有常驻后端下 CLI 是多余的第三者）。

- 位置：`sisyphus/src-tauri/crates/mcp`（bin `sisyphus-mcp`）。
- 由 Codex/Claude Code 作为子进程 stdio 拉起；打开与 App 同一个 `sisyphus.db`（WAL + `busy_timeout` 支持跨进程并发），**App 未运行也能工作**。
- 已实现工具：`capture` / `query_context` / `today_actions` / `set_goal`（MVP，未提前造 intent/artifact 工具）。
- **交付物,不接进本开发仓库**:用户侧把 `sisyphus-mcp` 注册进自己的 Codex/Claude,并配合 skill `skills/sisyphus/`(内含安装与每日规划/复盘例程 + 定时任务模板)。本 dev 仓库的 `.codex/config.toml` 只保留开发用的 supabase MCP。

---

## 5. 跨端与同步：本地优先，同步延后

拖延干预天然跨端（工作在电脑、摸鱼在手机），但**"跨端采集"与"实时同步"是两件事，别焊在一起**：

- **干预本地触发**：在哪台设备产生的行为，就在哪台设备弹提醒。每台设备跑自己的**本地 Event log + 本地引擎 + 本地通知**，闭环不需要任何同步。
- **同步只服务"联合分析"**：跨端关联（"电脑变空闲 + 手机开始刷"）是 **Phase 2** 的离线批量分析，**不在任何 MVP 闭环的关键路径上**。
- **零成本留门**：只要两端吐同一信封、同一 `user_id`、可比的 `category`（协议已满足），未来的联合建模就随时能做。

同步细节见 [sync.md](sync.md)（标注为 Phase 2）。

---

## 6. 组件与目录映射

保持当前 monorepo，**不拆成多个可部署 App**（那是 Phase 3 推广阶段的事）。顶层按**消费者/构建方式**划分，而不是"是不是交付物"（app、扩展、skill 都是交付物）：

| 顶层 | 消费者 | 内容 |
|---|---|---|
| `sisyphus/` | 终端用户（桌面/安卓）| Tauri 客户端 = Rust workspace（app + core + mcp）+ Web UI |
| `packages/` | **代码**（被 import / 装载）| 共享库与可分发件：`protocol`(npm 类型包)、`browser-extension` |
| `services/` | 部署运行 | 服务端：`ingest`(Supabase) |
| `skills/` | **Agent 运行时**（Codex/Claude）| 交付给用户装进 LLM 的技能包：`sisyphus` |
| `docs/` | 人 | 规格与路线图 |

> skill 单列顶层是合理的：它的消费者是 agent 运行时、格式是 SKILL.md 包，和 `packages/`(给代码)、`services/`(部署)、`sisyphus/`(客户端)都不同；且反思平面后续会有更多 skill。
> MCP server(`sisyphus-mcp`)虽也是反思平面交付物,但因需与 `sisyphus-core` 共享 Rust workspace(path 依赖)而住在 `sisyphus/src-tauri/crates/mcp`,以编译产物形式交付。

`sisyphus/` 内部（Rust workspace）：

| 目录 | 角色 | 平面 |
|---|---|---|
| `src-tauri/`（workspace 根 + App crate `sisyphus_lib`）| Tauri 桌面/安卓外壳 + 前台采集器，链接 core | 感知 |
| `src-tauri/crates/core/`（`sisyphus-core` rlib）| **本地事件与系统运行状态的事实来源**：db + `ingest_event` + 查询 + 规则引擎 + 分类 | 数据 + 引擎 |
| `src-tauri/crates/mcp/`（`sisyphus-mcp` bin）| rmcp stdio server，链接 core，反思平面接口 | 反思 |
| `src/` | React 前端（今日页等） | UI |

**Rust workspace 约束**（务必遵守）：App crate 留在 `src-tauri`、`[lib] name="sisyphus_lib"` 不改（安卓 `System.loadLibrary("sisyphus_lib")` 绑定它）；core 保持普通 rlib；**tokio/rmcp 等只放 mcp crate，绝不进 core**，否则拖垮安卓构建。

Android 采集源以 Kotlin Tauri 插件形式接入 `sisyphus/`，见 [android-collection.md](android-collection.md)。（`_deprecated/` 旧 Kotlin 方案与本机 agent 配置 `.claude/`/`.mcp.json`/`.agents/` 均已 gitignore，不入库。）

---

## 7. 现状与技术选型（2026-07）

- 已验证：Tauri v2 能在 Mac 上编译桌面端与 Android 包。
- 数据层/引擎：`BehaviorEvent` 信封、Event log(`raw_events`)/`outbox`/`daily_goals`/`interventions`、`Rule` trait、`EntertainmentSessionRule` 均已抽入 `sisyphus-core`。
- 写入契约：`ingest_event` + `capture_text` 已落地（core），App 命令与 MCP 共用。
- 反思平面:`sisyphus-mcp`(rmcp stdio)已实现并端到端冒烟通过(capture/query_context/today_actions/set_goal 真实读写同库);作为交付物,用户侧接入,配套 skill `skills/sisyphus/` 已建。
- 感知平面(B 轨第一根真实电线):macOS 前台采集器已落地(`sisyphus/src-tauri/src/collector.rs`,后台线程 osascript 轮询→分类→`ingest_event` 写 `app_foreground`→规则引擎→桌面通知 via tauri-plugin-notification)。规则管道有集成测试(`crates/core/tests/rule_pipeline.rs`,3 通过);osascript 前台探测已验证可用。
- **待真机验证**:`npm run tauri dev` 跑起来后,设目标 + 停留在娱乐类 app 一分钟(debug 阈值),确认弹真通知——即验证核心假设。桌面分类白名单在 `crates/core/src/category.rs`(看 collector 打到 stderr 的 bundle id 自行增补)。
- 技术栈：Tauri v2（Rust + React/TS）、rusqlite（本地）、rmcp（MCP server）、tauri-plugin-notification（桌面通知）、Codex/Claude Code（反思平面基座，现阶段）、Supabase/Postgres（同步，Phase 2）。

开发路径见 [../roadmap.md](../roadmap.md)「近期推荐下一步」。

---

## 8. 开发计划与可拓展边界（敏捷 · 复用 · 留迁移门）

总原则：**敏捷开发、充分复用现成工具，但在架构上保留后续换成自研的可拓展性。**

1. **智能体基座可换**：现阶段用 Codex / Claude Code 作基座，智能体能力一律沉淀为 **skill / MCP / CLI / SDK**；通用智能体亦作前期主入口。后期整体替换为自研 Agent 底座——**替换的只是交互前端，Core 经 MCP 暴露的能力契约不变**（见 §4）。
2. **自建存储 + 生态投影并存**：SQLite LifeDB 保存结构化人生规划；Notion 保留用户熟悉的自由文本交互，由受限 Agent 双向合并，但不依赖 Notion 数据库能力。知识库用 **Obsidian Markdown vault**，后续可派生知识图谱（图数据库）。
3. **先验证，再"真正用起来"**：先验证前期架构可行性，通过后再追求日常可用性与产品形状。

**长期规划暂缓（明确克制，验证期不碰）：**

| 模块 | 暂缓项 |
|---|---|
| 狭义西西弗斯 | IoT / 可穿戴端采集、跨端同步、RL 跨端学习 |
| LifeIndex | 无极时间线、技能树、跨用户服务端同步 |
| 第二大脑 | 知识图谱（图数据库）派生 |

---

## 9. 主动触发：调度器 + 动作队列 + Agent 派发

系统不止"被动响应输入"，还要**主动**：每日定时自省知识库、下班时挑一件支线提醒、规则引擎检出行为特征后择时干预。这些统一到**一条"待办动作队列"**，不是散落的 cron。

**核心抽象**：一条 `scheduled_actions` 队列，多个生产者塞"在时刻 T 做动作 A"，一个常驻循环到点派发。`T=now` 即"立即"，`T=now+Δ` 即"延后"——**同一条路径**。这满足关键可拓展性：规则引擎检出特征后，响应策略既可"立即提醒"也可"延后提醒"，只是 `due_at` 不同；每日定时任务只是带 `recurrence` 的一条。

**分层守铁律**：
- **`sisyphus-core::scheduler`**：队列纯数据逻辑（enqueue / due_actions / mark_fired / reschedule），纯 rusqlite、无副作用，安卓可编。
- **app（感知平面常驻）**：ticker 到点取 `due_actions` → 按 `kind` 执行**平台相关副作用**（`notify` 弹通知 / `pet_message` 展示宠物气泡 / `agent_run` 拉起 Agent）。LifeIndex 修改会入队受限双向同步；`tokio`/进程/通知**绝不进 core**。
- **规则引擎 → 响应规划器**：finding → `ResponsePolicy`(Immediate/Deferred/Debounce/Suppress) → `core::enqueue_action`。这是"立即 vs 延后 vs 防打扰"的可拓展 seam。

**“何时触发”是确定性的活（在 app/引擎），“建议什么”才是 agent 的判断；最终是否打扰还要过宿主的冷却、去重和隐私策略**——别把判断塞进调度器（呼应 §1.1）。

**LifeIndex 双向同步**：每日或 LifeDB 变更触发 `agent_run(mode=lifeindex_sync)`；Agent 经受限网关读取唯一 Notion 页面，三方合并后更新 LifeDB，再把 Core 生成的普通 Markdown 写回该页。其它主动建议仍使用 `RunMode::Proactive`，只读且只返回一条建议。

**现状（2026-07）**：调度器骨架、`ResponsePolicy` seam、`notify`/`pet_message`/`agent_run` 执行器、动态检测规则和 LifeIndexSync 均已落地；Pi 与 Codex 两基座连同一 `sisyphus-mcp`，工具面按 `Interactive / Proactive / LifeIndexSync` 运行模式白名单隔离（见 [agent.md](agent.md)）。

完整数据模型、响应策略、执行器、可靠性与 MVP 边界见 [proactive-triggers.md](proactive-triggers.md)。
