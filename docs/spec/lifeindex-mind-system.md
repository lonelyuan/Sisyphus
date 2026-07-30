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

### 2.5 视图契约（`core::skillmap` + `src/skilltree/`）

技能树视图不是"把 `life_tree` 画出来"，它是一张**能力地图**：读模型 [`core::skillmap`](../../sisyphus/src-tauri/crates/core/src/skillmap.rs) 把 LifeDB 的图投影成扇区 + 节点 + 前置边，前端只做投影与绘制。

**图元角色（这是全部要点）。** 四个概念在 LifeDB 里各有其位，图元角色必须不同，否则地图会退化成思维导图：

| 概念 | 存储 | 图元 | 为什么必须这样 |
|---|---|---|---|
| **领域** | `life_areas` | **背景扇区，永不是节点** | 领域没有完成态、只需维持标准。画成节点它永远填不满，进度就失去意义 |
| **技能** | `kind=skill` | 技能点（大节点） | 唯一"拥有 / 未拥有"可判定的能力单位 |
| **里程碑** | `kind=milestone` | 父技能环上的**刻度**（放大后展开为卫星节点） | 它是等级刻度不是并列节点；`contains` 是组成关系，画成"属于"而不是画成边 |
| **想法** | `kind=idea` | **环外星尘**（无边） | 还没决定要不要变成能力；看它向内迁移＝树在生长 |
| **事实进展** | `progress` / 度量 / 子 `action` | **不是图元**：节点填充、刻度亮数、度量读数、时间播放 | 把进展画成节点＝地图变成任务清单，且违反"人生尺度只画重要的" |

- **`depends_on` 是唯一进图的边**（前置能力）。`A blocks B` 在读模型里规范化为 `B depends_on A`。
- `contains` / `supports` **不画成边**（由 `parent_id` 表达）；`related` / `derived_from` 只在选中时浮现。
- `goal` / `project` 不进画布（它们是产出不是能力），挂在技能上作角标 `goal_count`。

**节点四态（无阈值魔数，全部由进度与前置推出）：**

```text
attained    progress >= 1.0（含 status=done）
in_progress 0 < progress < 1
available   progress == 0 且所有前置 attained
locked      progress == 0 且存在未 attained 的前置 → blocked_by 说得出是谁
```

于是"已拥有的能力"＝明亮内核，"未来目标"＝暗淡外缘。**agent 不得把 `locked` 的技能推荐为下一步。**

**三条几何不变量（与 §3 时间轴同源）：**

1. **几何算术归 Core**。扇区角区间、依赖深度、四态、领域掌握度、环上槽位（`slot`/`slot_count`）都由 `skillmap` 给出；前端不做任何依赖 DB 语义的算术。
2. **雷达 = 换投影**。`src/skilltree/projection.ts` 的 `place(node, sector, maxDepth, progress, t)` 在 `t=0`（树：半径=依赖深度）与 `t=1`（雷达：半径=掌握度）之间插值；角度与 `t` 无关。一条渲染路径，时间播放只是第三个参数。
3. **位置 = 结构维度，颜色 = 属性**。扇区只能是 `area`；`track`（主线/支线）与 `status` 只能给颜色/亮度，**不许各占一个扇区**。
4. 弹性布局的硬边界：弹簧只能在扇区角区间内、目标环 `±RADIUS_SLACK` 内移动节点；种子用 `hash(id)` 而非 `Math.random`，**位置跨会话稳定**（`slot` 因此按 `(created_at, id)` 排而不是 `updated_at`）。

**每个视觉通道都要能说出它独占回答的一个问题**（沿用时间轴那条验收标准；说不出的通道砍掉）：扇区=属于哪个领域；半径(t=0)=要先会什么；半径(t=1)=在这个领域到哪一步；填充=完成到几成；刻度亮数=Lv 几/几；描边亮度=四态；边实线/虚线=前置是否满足；颜色=主次；扇区底色=是否 focus；环外星尘=有哪些还没定的想法；时间滑杆=这棵树怎么长成的。**明确不给通道**：人生总分、跨领域排行、状态分折线。

### 2.6 进度账本（时间播放的唯一诚实数据源）

`life_items` 只有会被覆盖的 `updated_at`，因此"这棵树是怎么长成现在这样的"此前无法回答。`life_item_progress`（append-only）补上这件事：

- 一行 = 一次真实变更后的**全量三元组**（`status` / `current_value` / `target_value`），NULL 就是"没有度量"；
- 写入点 `lifedb::upsert_item` / `archive_item`，**只在 status 或度量真的变化时**追加（改标题不是进展）；
- 边的诞生时间用现成的 `life_item_edges.created_at`；
- 迁移 v6 为老库回填两个锚点（`created_at` 的初始态、`updated_at` 的当前态），`origin='backfill'` **明确标注为近似**——不假装拥有从未记录过的中间过程；
- `skill_map(Some(T))` 用账本在 T 之前的最后一行覆盖字段，再跑**同一套** `lifetree` 算法：回放与"现在"共用一个口径；
- `growth(from,to)` 在每个变化时刻算一遍，**delta 编码**只输出进度真的变了的点。舞台（节点身份与位置）来自 `skill_map(None)`，**播放期间位置绝不移动**——所以回放回答的是"现在这棵树是怎么长成的"，已归档的旧节点不参与。

---

## 3. 无极时间线

### 3.1 三条硬约束

1. **代价与缩放解耦**。粗尺度不扫 `raw_events`，读 `time_rollups` 预聚合桶。年尺度代价 = O(可见桶数)。
2. **不同尺度显示不同抽象层次**。每个点事件带显著性等级，尺度越粗保留的层越少——粗尺度留下的是**重要的**，不是"恰好排在前面的"。
3. **长期计划必须在时间轴上**。人生尺度上唯一有意义的图层是目标/项目/技能的跨度与里程碑点。

外加一条几何约束（见 §3.5）：**时间的几何由 Core 给**。刻度与折叠网格的边界一律是逻辑日/周/月，前端不做时区算术。

### 3.2 预聚合（`time_rollups` + `rollup_state`）

- 桶按**逻辑日**切（[`core::clock`](../../sisyphus/src-tauri/crates/core/src/clock.rs)：本地时区 + 换日点），周/月桶**由日桶再聚合** —— 保证"这周 = 7 天之和"对得上。
- 维度三个：`category`（专注/娱乐/中性拆分与主导分类）、`app`、`hour`。
  - `hour` 的 key 是 `"HH|category"`，`HH` 是**逻辑日内小时序号**（0 = 日起点那一小时）——这正好就是折叠视图的横轴位置。
  - 它在 Rust 侧算（按本地小时切分会话在 SQL 里要生成序列且要处理 DST），且**只存在于日桶**：
    "日内第几小时"跨周聚合没有含义，所以周/月再聚合显式排除 `dimension='hour'`。
- 增量：`rollup_state.watermark_ms` 记已处理到的 `ingested_at`；只重算新事件触碰到的日桶，且是"删该桶重算"——幂等可重跑。
- 跨午夜会话按日界拆分时长（有测试）；`hour` 维度同样按小时边界拆（有测试）。
- 自愈：读路径 `catch_up` 在有新事件时先追平（无新事件只花一次索引查询），ticker 每 30s 也追一次。**改换日点必须整体重算**（`invalidate_all` + `rebuild(full)`），否则旧桶口径失效——`set_day_boundary_hour` 命令已经带上这一步。

### 3.3 显著性等级（LOD）

| 等级 | 含义 | 内容 |
|---|---|---|
| 0 | 人生尺度仍可见 | `life_goal` / `life_skill` / `life_milestone` |
| 1 | 月尺度可见 | 今日目标、`life_project`、知识结晶、检测规则创建 |
| 2 | 周尺度可见 | 事项、提醒、干预（干预事件上直接挂 `outcome`，一眼看出哪次提醒真起作用了）|
| 3 | 日/分钟尺度可见 | 原始行为会话、零散 capture |

`detail`（前端连续 zoom 推导）→ 最大等级；`detail` 缺失时按可见跨度兜底。
**例外**：`fold=day` 且行数 ≤ 62 时强制放开到等级 3 —— actogram 的格子要由原始会话铺，
否则周尺度的 LOD 会把它们过滤掉、格子全空。

### 3.4 返回结构

`query_timeline(start, end, detail, max_items, fold)` 返回：

| 字段 | 含义 |
|---|---|
| `bands` | 预聚合条带（线性视图粗尺度的主数据） |
| `plans` | 长期计划跨度（LifeDB 图层） |
| `events` | 带 `level` 的点/区间 |
| `days` | 日聚合（由日桶推导） |
| `bucket` | 本次实际桶粒度 |
| `ticks` / `tick_unit` / `tick_minor_unit` | 日历对齐的两级刻度（`tier` 0 主 / 1 次，`day_start` 标记日界） |
| `grid` | 折叠网格：`rows`（周期行，带标签）、`cols`、`col_unit`、`truncated` |
| `cells` / `cell_kind` | 折叠单元格与粒度：`none` / `session` / `hour` / `day` |
| `boundary_hour` | 换日点（展示用） |

前端只做投影与绘制，不重复聚合、**也不自己算任何时间边界**。

### 3.5 折叠（相位）视图

折叠不是第二个视图，而是同一条轴换投影：按周期取模，一个周期一行。
`fold=day` 即时间生物学的 actogram（作息栅格图），`fold=week` 的 7 列形态就是传统日历。

| 档位 | 一行 | 横轴 | 单元格 |
|---|---|---|---|
| `day` | 逻辑日 | 日内时刻（双绘时 48h） | 行数 ≤ 62 用原始会话；更长用 `hour` 桶 |
| `week` | 逻辑周 | 星期（7 列） | 日桶 |
| `month` | 逻辑月 | 日（31 列，短月右侧留空） | 日桶 |
| `year` | 年 | 一年里的第几天（366 列） | 日桶 |

三条设计约定：

1. **是 2D 折叠，不是 3D 螺旋**。折叠唯一的价值是"不同日期的同一时刻落在同一条竖线上"；
   3D 螺旋的遮挡与透视恰好毁掉这一点。"弹簧"只保留在**转场动画**里（线性与折叠坐标按 `t` 插值）。
2. **日历折叠的网格必须补齐**。没有观测的日子也要有单元格：`没有观测` 与 `有观测但没在娱乐`
   不能长得一样；而且列选区要靠单元格换算成时间窗口，缺格子会让那一行整段漏掉。
3. **列宽固定**。短月/短年右侧留空，而不是拉伸填满 —— 否则列在行之间不可比。

### 3.6 选区统计（`range_stats`）

`range_stats(windows)` 接受**多个**时间窗口：线性选区给 1 段，折叠视图的相位选区
（"每天 22:00–02:00"这一竖条）给每行 1 段。两者走同一个函数、同一个口径，
不存在"折叠视图的统计和线性视图对不上"。

- 时长按**与窗口的交集**精确计算（不按整日近似）。
- 同时给出分类/app 排行、干预次数与其中 `switched` 的次数、capture 与 artifact 计数。
- 窗口数上限 400，超出时 `truncated=true`（不静默截断）。

---

## 4. 确定性心智服务（`core::lifetree`）

| 服务 | 语义 | 为什么在 Core |
|---|---|---|
| `forest(kinds)` / `subtree(root)` | 技能树 / 目标分解树 + 进度 | 进度必须可复现 |
| `progress_index(items, edges)` | 全库 id → 进度。历史回放传入"按账本覆盖过的 items" | 与 `forest` 同源，回放与现在不可能给出矛盾数字 |
| `skillmap::skill_map(at_ms)` | 技能树地图（扇区/节点/前置边/想法 + 四态 + 几何） | 位置的含义必须可测试，见 §2.5 |
| `skillmap::growth(from,to)` | 生长史（delta 编码的进度变化点） | 播放不能靠前端重算聚合 |
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
- 技能树视图：前置未达成 → `locked` 且 `blocked_by` 命中；`A blocks B` 与 `B depends_on A` 同图；扇区角度和为 2π 且空领域仍有角宽；`mastery` 只算技能不把里程碑计两次；只改 `updated_at` 不改任何 `slot`。
- 进度账本：改标题不产生账本行，改状态/度量产生一行；`skill_map(T)` 在里程碑完成前给 0、完成后给 1；出生之前节点不存在；v6 回填可重复执行。
- 前端几何：`t=0` 半径由深度决定、`t=1` 由掌握度决定且中间单调；角度与 `t` 无关；同输入两次松弛坐标一致；角度不出扇区、半径不出目标环 ±slack。
- 时间线：日桶 = 会话与桶的交集之和；周桶 = 覆盖日桶之和；跨午夜会话两天各算一半；无新事件时 `catch_up` 返回 0。
- 刻度：日界刻度等于 `clock::day_start_at`（换日点 0 与 4 都成立）；月刻度落在每月 1 日而不是每 30 天；人生尺度刻度数有界。
- `hour` 维度：09:30→11:15 的会话切成 30/60/15 分钟三格；小时之和 = 当天该分类总时长；周桶里查不到 `hour` 维度；重复 `rebuild` 结果不变。
- 折叠：`fold=day` 短跨度 `cell_kind=session` 且能拿到原始会话，长跨度自动降级为 `hour`；`fold=week` 的列号 = 星期几，且每行恰好 7 格（含零观测日）；`fold=month` 每行格子数 = 该月实际天数。
- 选区：只取 10:00–11:00 时，09:00–11:00 的会话只计 1 小时；相位选区把连续两天的 22:00–23:00 加成 2 小时且不含白天的工作；空窗口与反向窗口被丢弃。
- 前端几何（`src/timeline/*`，用 jiti 跑 `Projection`/`computeLayout` 的断言）：跨午夜会话切成相邻两行且首段贴行尾、次段贴行首；不同日期的同一时刻同 x；转场中间态落在线性与折叠坐标之间；双绘副本落在上一行且右移半屏；相位选区 = 每行一个窗口。
- 下一步：逾期项排第一且理由为「已逾期」；主线目标下的行动理由含「主线」；新记下的无元数据事项仍会出现（理由「待安排」）。
- 回顾：无子项的目标进 `undecomposed`；缺 `success_criteria` 的目标进 `no_success_criteria`；`review_at` 到期的 idea 进 `due_review`。
- 换日点改为 4 点后，"今天"的起点是本地 04:00，且 rollup 已整体重算。

（以上均有自动化测试：`crates/core/src/lifetree.rs`、`skillmap.rs`、`lifedb.rs`、`migrations.rs`、`rollups.rs`、`clock.rs` 的 `mod tests`；前端几何用 `sisyphus/scripts/skilltree-geometry-check.ts` 断言脚本，
项目**没有前端测试框架，也不要加**：`node node_modules/.pnpm/jiti@*/node_modules/jiti/lib/jiti-cli.mjs scripts/skilltree-geometry-check.ts`。）

---

## 6. 边界

- **不引入图数据库**：邻接表 + 递归/内存建树足够；个人库规模下代价可忽略。
- **不引入前端依赖**：技能树是 canvas 2D + 手写弹簧，与时间轴同一套做法。
- **不做跨用户/服务端同步**：Phase 3 的事。
- **不让 agent 写进度**：`current_value` 由用户或明确的度量来源更新，进度永远是算出来的。
- **不把 Notion 当结构源**：技能树的结构只在 LifeDB；Notion 投影里技能与里程碑出现在「🌳 技能与里程碑」区块，仍是可编辑文本。

### 6.1 已知待做（不在技能树首版内，理由写清）

- **目标 → 自动生成里程碑**：必须走 `propose_intents(kind="life_item")` 意图候选桥，是独立例程（§2.3 已定边界）。
- **Notion 投影升级**：把 `🌳 技能与里程碑` 渲染成嵌套缩进 + `Lv 2/5 · 40%` + `前置：X`，并改名避开用户手工页里已有的 `🌳 知识体系` callout 标记。涉及用户亲手维护的页面，需单独确认。
- **把行为采集时长映射到技能**：`raw_events.category` 与 `life_areas` 是两套词表，硬映射就是猜。要做得先加 `life_areas.rollup_keys` 让用户显式声明对应关系。
- **`life_areas` 嵌套**：Notion 侧「缩进 = area 树」而本地 `life_areas` 是平的。扇区正好要平结构，暂不动，记为已知差异。
