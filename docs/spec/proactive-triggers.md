# Spec: 主动触发（调度器 · 动作队列 · Agent 派发）

本文件是西西弗斯**主动能力**的权威文档：系统如何在没有用户当下输入时，自己决定"何时做点什么"，以及把结果投递到通知 / 知识库 / Notion 看板。

上位文档：[architecture.md](architecture.md)（两平面 + 双存储 + `ingest_event` 契约）。本设计横跨两个平面：**"何时触发"是感知平面的确定性活**（不调 LLM），**"做什么/说什么"可以是反思平面的 agent**。相关：[rule-engine.md](rule-engine.md)（行为触发来源）、[notion-integration.md](notion-integration.md)（Notion 投递面）。

---

## 0. 一句话心智模型：一条"待办动作队列"，多个生产者，多个执行器

不要把主动能力理解成"定时任务(cron)"。cron 只是特例。真正的抽象是：

> **一条统一的"到点要做的动作"队列（`scheduled_actions`）。谁都能往里塞一个"在时刻 T 做动作 A"，一个常驻循环到点取出、按类型派发执行。`T = now` 就是"立即"，`T = now+Δ` 就是"延后"——同一条路径。**

这直接满足关键需求：**规则引擎检出行为特征后，其响应策略既可以是"立即提醒"，也可以是"过一会儿再提醒"**——两者都只是往队列塞一个 `due_at` 不同的动作，执行器完全一致。时间触发的"每日 9 点自省"也只是队列里一条带 `recurrence` 的动作。

```
生产者（谁塞动作进队列）
  ① 静态周期计划    每日9点知识自省 / 每日19点支线梳理     （recurrence）
  ② 规则引擎响应策略 检出特征 → 立即/延后/抑制             （核心可拓展点，见 §3）
  ③ Agent/用户显式  "30分钟后提醒我…" / skill 主动排程
                         ↓ enqueue_action(kind, payload, due_at, …)
        ┌───────────────────────────────────────────┐
        │  scheduled_actions 队列 (Artifact store)    │
        └───────────────────────────────────────────┘
                         ↓ 常驻循环 due-check（到点取出）
执行器（按 kind 派发副作用）
  notify        端侧通知（确定性，便宜）
  agent_run     拉起反思平面 agent（Codex CLI/SDK），带 skill+scope+vault
  notion_now    刷新 Notion NOW 挂件（白名单，notion-integration §3.3）
  notion_inbox  append 到 Notion Inbox
  …（可拓展）
                         ↓ 投递面
              通知 · 知识库 vault · Notion 看板
```

---

## 1. 分层：守安卓构建铁律（core 纯净，副作用在 app）

主动能力横跨两个平面，实现上**严格分层**，否则 `tokio`/进程管理/通知会渗进 core、拖垮安卓构建（[architecture.md §6](architecture.md) 铁律）。

| 层 | 职责 | 依赖 |
|---|---|---|
| **`sisyphus-core::scheduler`** | 队列的**纯数据逻辑**：`enqueue_action` / `due_actions(now)` / `mark_fired` / `mark_done` / 周期动作 `reschedule`。**无副作用、无 IO 之外的东西**、纯 `rusqlite`。安卓可编。 | rusqlite（已有） |
| **app（感知平面常驻）** | 常驻 **ticker** 每 N 秒调 `due_actions` → 按 `kind` 执行**平台相关副作用**：通知（`tauri-plugin-notification`）、拉起 codex（`std::process::Command`）、Notion 回写（走 indexer/MCP）。 | tauri、std::process |
| **规则引擎 → 响应规划器** | finding → `ResponsePolicy` → `core::enqueue_action`。规划器是 §3 的可拓展 seam。 | core |

> **铁律**：`tokio`/`rmcp`/进程/通知**绝不进 core**。core 只回答"现在有哪些动作到期了"，**怎么执行**是 app 的事。这样安卓端即使不能拉起 codex，也照样能编译、照样能跑 `notify` 类动作。

---

## 2. 数据模型：`scheduled_actions`（进 Artifact store）

可变、有 `status`、有生命周期 → 按 [architecture.md §2.2](architecture.md) 进 Artifact store，单独建表（不塞进多态大表）。

```sql
CREATE TABLE IF NOT EXISTS scheduled_actions (
  id            TEXT PRIMARY KEY,
  kind          TEXT NOT NULL,              -- notify | agent_run | notion_now | notion_inbox | …（可拓展枚举）
  payload_json  TEXT NOT NULL DEFAULT '{}', -- kind 相关：agent_run={skill,prompt,scope,vault}; notify={title,body}
  due_at_ms     INTEGER NOT NULL,           -- 到点时刻；= now 即"立即"
  recurrence    TEXT,                       -- NULL=一次性；如 "daily@09:00" / cron 串=周期（fire 后重排下一次）
  status        TEXT NOT NULL DEFAULT 'pending', -- pending | fired | done | failed | cancelled
  dedup_key     TEXT,                       -- 可选防打扰：同 key 已有 pending 则不重复入队
  origin_event_id TEXT,                     -- 溯源：哪条 finding/事件触发的（append-only 溯源锚点）
  created_by    TEXT NOT NULL,              -- rule_engine | scheduler | agent | manual
  created_at_ms INTEGER NOT NULL,
  fired_at_ms   INTEGER
);
CREATE INDEX IF NOT EXISTS idx_sched_due ON scheduled_actions (status, due_at_ms);
```

**幂等/防重复两道闸**：
1. `dedup_key`——入队时若已有同 key 的 `pending`，跳过（防"同一 finding 反复排提醒"打扰用户）。
2. `status`——`due_actions` 只取 `pending` 且 `due_at<=now`；取出即置 `fired`，执行完置 `done`/`failed`。防一个动作被并发/重启重复执行。

**溯源**：`origin_event_id` 指回触发它的 finding/事件；动作 fire 时可另写一条 `intervention`/`action_fired` 事件进 Event log（[architecture.md §2.1](architecture.md)），保持"发生过的事都在事件流里"。

---

## 3. 规则引擎的响应策略（核心可拓展点）

现状：规则引擎命中 → **硬编码**"立即弹通知"。这不可拓展。改成：**finding → `ResponsePolicy` → 响应规划器 → enqueue**。规则只表达"检出了什么 + 该怎么回应"，不直接产生副作用。

```rust
// core，纯数据；规则引擎产出
enum ResponsePolicy {
    Immediate(Action),                 // 立即：enqueue due_at=now
    Deferred { action: Action, at: DueTime }, // 延后：at=绝对时刻 或 相对 now+Δ
    Debounce { action: Action, window_ms: i64, dedup_key: String }, // 窗口内只提醒一次
    Suppress,                          // 不打扰（如冷却期内、夜间免打扰）
}
enum DueTime { At(i64), After(i64) }   // 绝对 epoch ms / 相对延迟 ms
```

**"响应规划器"** 把 policy 翻译成 `enqueue_action`：`Immediate`→`due_at=now`；`Deferred{After(Δ)}`→`due_at=now+Δ`；`Debounce`→带 `dedup_key` 入队。**MVP 规划器可以极简（一律 Immediate→notify），但这个 seam 必须先立住**——日后加"检测到 doom-scrolling 但现在别烦他、45 分钟后若仍在刷再提醒""夜间 policy=Suppress""下班闲暇时段才推支线"都只是新增 policy 分支，不动执行器、不动队列。

> 这也把 [notion-integration.md §3.3/§4.4](notion-integration.md) 的"引擎检测闲暇→挑一条低精力短时行动→刷 NOW+通知"纳入同一框架：那是 `Deferred/Immediate` + `kind=notion_now`+`kind=notify` 两个动作。

---

## 4. 执行器：按 `kind` 派发（app 层）

常驻 ticker 每 N 秒（复用感知平面已有循环节奏）：`due = core::due_actions(now)`；逐个按 `kind`：

| kind | 执行（app 层） | 平台 | 现状 |
|---|---|---|---|
| `notify` | `tauri-plugin-notification` / 安卓 InterventionNotification | 桌面+安卓 | ✅ 通道已有 |
| `agent_run` | `std::process::Command` 拉起 codex（复用 `services/knowledge-agent` 派发；传 `SISYPHUS_VAULT`+skill+prompt+scope）→ agent 经 MCP 写回卡片/reminder → 完成后可再 `enqueue(notify)` 把结果推给用户 | **仅桌面**（手机无 node/codex） | 🟡 待接 |
| `notion_now` | 覆盖写 Notion 🔄NOW 块（白名单，见 [notion-integration.md §2.4](notion-integration.md)） | 桌面 | 🔴 待 indexer |
| `notion_inbox` | append 到 📥Inbox | 桌面 | 🔴 待 indexer |

**agent_run 的往返**：app 到点 → 拉 codex（不思考，只执行）→ codex 在本地 db/vault 上干活（整理/找缺口/挑支线/写 reminder artifact）→ app 通知投递。**"何时触发"确定性在 app，"做什么/要不要打扰"判断在 agent**——别把判断塞进调度器。

---

## 5. 打通：app → 知识库 → Notion 看板

主动队列是**触发器**；跨模块的数据一致靠**投影管道**（`outbox` → projectors）。两者配合才叫"打通"，但不是把三个库揉成一个——**各模块真相源不同，各自幂等投影**：

| 模块 | 真相源 | 投影 |
|---|---|---|
| Core（事件/artifact） | 本地 SQLite（[architecture.md §2](architecture.md)） | — |
| 第二大脑（知识散文） | Obsidian vault `.md` | 由 write_knowledge_note 落盘 + 索引（可重建投影） |
| LifeIndex（作战看板） | **Notion**（[notion-integration.md §0](notion-integration.md)） | 本地只读镜像 `notion_actions` + 白名单回写 |

**打通的两条管道**：
1. **投影管道（数据一致）**：canonical artifact 变更 → `outbox` 事件 → projectors：知识 → Obsidian；LifeIndex 动作/NOW → Notion（白名单）。`outbox` 表已存在（[architecture.md §7](architecture.md)），是这条管道的地基。
2. **主动管道（跨模块触发）**：本 spec 的动作队列把三者串起来。典型串联：

```
19:00 触发（scheduled_actions, recurrence=daily@19:00, kind=agent_run）
  → 拉 codex(assistant skill, scope=personal)
  → 读 query_context(目标/任务) + Notion 镜像(#低精力#now 行动) + 知识库 #待确认 缺口
  → 挑一件最可能感兴趣的支线，写 reminder artifact
  → enqueue(kind=notion_now, 刷 🔄NOW 挂件) + enqueue(kind=notify, 端侧提醒)
```
一次主动动作，跨 Core（reminder artifact）→ 知识库（读缺口）→ Notion（刷 NOW）→ 端侧（通知）。这就是三者打通的落地形态。

---

## 6. 可靠性

- **补跑（catch-up）**：ticker 每次检查"周期动作今天该跑的时刻已过、且今天未 fire → 现在补跑"（靠 `recurrence` + `fired_at`）。机器 9 点在睡，10 点唤醒也补上。
- **常驻依赖**：app 内 ticker 只在 app 运行时转。MVP 靠 B 轨已有的**后台常驻 + autostart**。更强的存活保证（app 崩溃/长睡后）留给 OS 调度兜底（macOS `launchd` / 安卓 `WorkManager`）——它只负责"保证 app/job 被唤起"，作 Phase 2。
- **安卓现实**：手机拉不起 codex CLI → `agent_run` **仅桌面**；安卓端只跑 `notify` 类（确定性），LLM 结论靠同步过去（[architecture.md §5](architecture.md) Phase 2）。
- **周期重排**：`recurrence` 动作 fire 后立即算出下一次 `due_at` 重排（或插入下一条 pending），保证不断链。
- **幂等**：`dedup_key` 防重复入队；`status=fired` 防重复执行；`origin_event_id` 可回溯。

---

## 7. 落地边界（MVP vs 后续）

**MVP（最小骨架，本次）：**
1. `core::scheduler`：`scheduled_actions` 表 + `enqueue_action` / `due_actions` / `mark_fired` / `mark_done` / 周期 `reschedule` + 单测。
2. app 常驻 ticker：`due-check` → 派发 **`notify`** 与 **`agent_run`**（spawn codex，缺 codex 时优雅降级）。
3. 种一个静态 job：**每日 9 点知识库自省**（`kind=agent_run`, skill=knowledge-engine, 跑去碎片化+找缺口）。

**后续（按需，勿提前造）：**
- 响应规划器接规则引擎（`Deferred`/`Debounce`/`Suppress` 落地真实防打扰）。
- `notion_now`/`notion_inbox` 执行器（依赖 `notion_indexer`，见 notion-integration §6）。
- `launchd`/`WorkManager` 存活兜底；跨端同步动作。
- 夜间免打扰、精力/时段情境筛选（GTD context，notion-integration §4.4）。

**明确舍弃/暂缓**：不引入独立消息队列/编排平台；不把三库合一；调度绝不写进某个 agent 基座（保持基座可换）。
