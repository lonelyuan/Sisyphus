# Spec: LifeIndex 心智系统（技能树 · 无极时间线）

本文件是 **LifeIndex 从"看板"升级为"心智系统"** 的权威契约：技能树与无极时间线的数据底座、不变量、理论依据与边界。上位架构见 [architecture.md](architecture.md)，同步语义见 [notion-integration.md](notion-integration.md)，本地表见 [local-storage.md](local-storage.md)。

---

## 0. 决策

> **技能树与无极时间线不是可视化糖，是心智系统的两个入口。它们的深度由底座决定，因此底座先于 UX 设计；一切"进度""下一步""该回顾什么"都由 Core 确定性算出，agent 只负责生成文本与措辞。**

两条铁律：

1. **不引入新抽象**：技能树完全由既有的 `life_items` + `life_item_edges` 承载（扩两个 kind、加几个可判定字段），不新建平行模型。呼应 [architecture.md §2.3](architecture.md)「Core 是萃取出来的」。
2. **确定性的部分绝不交给 agent**：进度、下一步选择、回顾队列必须可复现、可解释、可测试。agent 每次在上下文里数一遍既不稳定也无法调试。

---

## 1. 理论依据（每条都落成了字段，不是装饰）

| 框架 | 借它解决什么 | 落成什么 |
|---|---|---|
| **GTD Horizons of Focus**（六个高度） | 缺"责任领域"这一层 → `track=main\|side` 只能一条条手标 | `life_areas` 表 + `life_items.area_id`；`focus=1` 的领域参与主线推导与今日行动选择 |
| **OKR / 可判定的 key result** | `goal` 没有完成条件 → 永远不能 done，看板越长越沉 | `success_criteria`（一句可判定的话）+ `target_value`/`current_value`/`unit`（度量） |
| **PARA** | 第二大脑与人生看板是两棵互不相干的树 | Area 词表两侧共用：`kb/` ≈ Resources，LifeDB ≈ Projects/Areas |
| **GTD Weekly Review + Zeigarnik** | 看板若无回顾节奏，本身会变成新的焦虑源 | `lifetree::review_queue` + 每周日的 `weekly_review` job；判断由 Core 算好，agent 只提问 |
| **渐进式掌握（里程碑分解）** | "学好英语"这种目标无法执行 | `kind=milestone` + `contains` 边；里程碑带 `due_at_ms` 天然进日程与 `next_actions` |

**明确不做**：人生仪表盘打分、跨用户排行、把"维护看板"变成一份新工作的任何机制。

---

## 2. 技能树

### 2.1 形状（零新表）

```text
skill（能力节点，kind=skill）
  ├─ contains   → milestone（等级/阶段，可判定，kind=milestone）
  │                 └─ contains → action（具体要做的事）
  └─ depends_on → skill（前置能力，构成 DAG）
```

- `contains` / `supports` 是**层级**边，构成树；`depends_on` 是**前置**边，构成 DAG，**不参与父子关系**（否则前置节点会被当成子节点，进度就错了）。
- 根节点 = 没有入向层级边的节点。同一棵树可以同时被"技能树视图"和"目标分解视图"读取，只是 `kinds` 过滤不同。

### 2.2 进度：确定性算法（`lifetree`）

```text
叶子节点：status=done → 1.0
          有度量（target_value>0 且 current_value 有值）→ clamp(current/target, 0, 1)
          其余 → 0.0            ← 不给"在做"送分，保持诚实
内部节点：子节点进度的等权平均
额外输出：done_leaves / total_leaves（给 UI 显示 "3/7"）
```

不变量：
- **进度只有一个来源**。时间线的 `plans` 图层复用同一函数，避免两处口径不一致。
- 环形边不得导致栈溢出或死循环：递归带 `seen` 栈与 `MAX_DEPTH=8`（有测试）。
- 归档节点不参与计算。

### 2.3 "目标 → 自动生成里程碑"（待做的例程，边界已定）

agent 产出候选里程碑，**必须走意图候选桥**（`propose_intents(kind="life_item")` → 用户确认 → `accept_intent`），因此每个自动生成的里程碑都带来源、置信度与可回滚状态。Core 不做语义生成，agent 不绕过确认直接写库。

### 2.4 与日程联动

里程碑的 `due_at_ms` 让它天然进入：
- `lifetree::next_actions`（逾期 / 临近截止两条规则直接命中）；
- 无极时间线的 `plans` 图层（等级 0，人生尺度仍可见）；
- 提醒队列（后续可由 `review_at_ms` 触发"里程碑该检查了"）。

---

## 3. 无极时间线

### 3.1 三条硬约束

1. **代价与缩放解耦**。粗尺度不扫 `raw_events`，读 `time_rollups` 预聚合桶。年尺度代价 = O(可见桶数)。
2. **不同尺度显示不同抽象层次**。每个点事件带显著性等级，尺度越粗保留的层越少——粗尺度留下的是**重要的**，不是"恰好排在前面的"。
3. **长期计划必须在时间轴上**。人生尺度上唯一有意义的图层是目标/项目/技能的跨度与里程碑点。

### 3.2 预聚合（`time_rollups` + `rollup_state`）

- 桶按**逻辑日**切（[`core::clock`](../../sisyphus/src-tauri/crates/core/src/clock.rs)：本地时区 + 换日点），周/月桶**由日桶再聚合** —— 保证"这周 = 7 天之和"对得上。
- 维度两个：`category`（专注/娱乐/中性拆分与主导分类）与 `app`（后续做 app 条带）。
- 增量：`rollup_state.watermark_ms` 记已处理到的 `ingested_at`；只重算新事件触碰到的日桶，且是"删该桶重算"——幂等可重跑。
- 跨午夜会话按日界拆分时长（有测试）。
- 自愈：读路径 `catch_up` 在有新事件时先追平（无新事件只花一次索引查询），ticker 每 30s 也追一次。**改换日点必须整体重算**（`invalidate_all` + `rebuild(full)`），否则旧桶口径失效——`set_day_boundary_hour` 命令已经带上这一步。

### 3.3 显著性等级（LOD）

| 等级 | 含义 | 内容 |
|---|---|---|
| 0 | 人生尺度仍可见 | `life_goal` / `life_skill` / `life_milestone` |
| 1 | 月尺度可见 | 今日目标、`life_project`、知识结晶、检测规则创建 |
| 2 | 周尺度可见 | 事项、提醒、干预（干预事件上直接挂 `outcome`，一眼看出哪次提醒真起作用了）|
| 3 | 日/分钟尺度可见 | 原始行为会话、零散 capture |

`detail`（前端连续 zoom 推导）→ 最大等级；`detail` 缺失时按可见跨度兜底。

### 3.4 返回结构

`query_timeline` 返回 `bands`（条带）、`plans`（计划跨度）、`events`（带 `level` 的点/区间）、`days`（日聚合，由日桶推导）、`bucket`（本次实际桶粒度）。前端只做投影与绘制，不重复聚合。

---

## 4. 确定性心智服务（`core::lifetree`）

| 服务 | 语义 | 为什么在 Core |
|---|---|---|
| `forest(kinds)` / `subtree(root)` | 技能树 / 目标分解树 + 进度 | 进度必须可复现 |
| `next_actions(limit)` | 今日最小行动，**每条带 `reason`** | "永远知道下一步"是承诺，必须可解释、可测试 |
| `review_queue(idle_days)` | 周回顾要问的问题 | agent 无法在上下文里可靠统计几十上百个节点 |
| `schedule_review(id, days)` | 给 idea 排毕业审查 | 让"手动维护"变成"每周回答几个二选一" |

`next_actions` 的优先级（命中即取，去重后截断）：

```text
1. 已逾期的事项/里程碑
2. 已排在"当下"（horizon=now）且在做
3. 三天内到期
4. 主线或重点领域目标/技能下**最浅**的未完成事项（广度优先）
5. 今日日常
6. horizon=next 里最久没动的
7. 兜底：任何未完成的可执行事项（刚记下、还没排期的也必须能被看见）
```

第 7 条是**不变量**：呼应 [notion-integration.md §1](notion-integration.md)「不得因过滤而丢失」——没有元数据的新事项不该在今日视图里凭空消失。

---

## 5. 验收

- 技能树：两个里程碑完成一个 → 根节点进度 50%；度量型里程碑（5.25/7）→ 75%；`depends_on` 不构成父子；环形边不死循环。
- 时间线：日桶 = 会话与桶的交集之和；周桶 = 覆盖日桶之和；跨午夜会话两天各算一半；无新事件时 `catch_up` 返回 0。
- 下一步：逾期项排第一且理由为「已逾期」；主线目标下的行动理由含「主线」；新记下的无元数据事项仍会出现（理由「待安排」）。
- 回顾：无子项的目标进 `undecomposed`；缺 `success_criteria` 的目标进 `no_success_criteria`；`review_at` 到期的 idea 进 `due_review`。
- 换日点改为 4 点后，"今天"的起点是本地 04:00，且 rollup 已整体重算。

（以上均有自动化测试，见 `crates/core/src/lifetree.rs`、`rollups.rs`、`clock.rs` 的 `mod tests`。）

---

## 6. 边界

- **不引入图数据库**：邻接表 + 递归/内存建树足够；个人库规模下代价可忽略。
- **不做跨用户/服务端同步**：Phase 3 的事。
- **不让 agent 写进度**：`current_value` 由用户或明确的度量来源更新，进度永远是算出来的。
- **不把 Notion 当结构源**：技能树的结构只在 LifeDB；Notion 投影里技能与里程碑出现在「🌳 技能与里程碑」区块，仍是可编辑文本。
