# Spec: LifeIndex · LifeDB · Notion 双向同步

本文件是 LifeIndex 与 Notion 集成的权威契约。上位架构见 [architecture.md](architecture.md)，本地表见 [local-storage.md](local-storage.md)，工具签名见 [api.md](api.md)。

## 0. 决策

> **SQLite LifeDB 是结构化事实源；App 是严格视图与编辑面；Notion 是用户可自由修改的普通文本输入/投影层；Agent 负责语义编译与三方合并。**

不依赖 Notion Database、公式、Relation、Rollup 或付费 Dashboard。Notion 页面只保存 Markdown 表面文本，不承载业务约束。用户可以在 App 或 Notion 任一侧修改，最终都汇入同一 LifeDB。

## 1. 三个产品对象

### LifeItem

`life_items` 统一承载人生规划域中的五类对象：

- `idea`：模糊意图、念头、待澄清方向。
- `goal`：希望达到的结果。
- `project`：为目标组织的一段工作。
- `action`：具体事项，或有明确时间节点的事。
- `routine`：需要反复发生的日常/习惯。
- `skill`：能力节点（技能树的根；前置关系用 `depends_on` 边）。
- `milestone`：可判定的检查点（目标/技能的分解产物）。

技能树的形状与进度算法见 [lifeindex-mind-system.md](lifeindex-mind-system.md)。

正交维度避免强迫用户只选一种分类：

- `track = main | side | neutral | undecided`：主线、支线、中性、未判断。
- `horizon = now | next | later | someday | unscheduled`：时间尺度。
- `area_id → life_areas`：责任领域（GTD Horizon 3，无完成态）。`focus=1` 的领域参与主线推导与今日行动选择——
  这一层的缺失曾是"主次只能一条条手标、维护起来不丝滑"的主要来源。

可判定与度量：`success_criteria`、`target_value` / `current_value` / `unit`、`review_at_ms`（idea 的毕业审查）。

`life_item_edges` 用邻接表表达 `contains / supports / depends_on / blocks / derived_from / related`。SQLite recursive CTE 足以覆盖当前目标→项目→行动/日常结构，不引入图数据库。

### LifeDB

LifeDB 由 SQLite 的 `life_items`、`life_areas`、`life_item_edges`、`life_item_external_refs`、`life_sync_state`、`life_sync_runs` 组成。
**人生规划域内只有一个事实源**：旧 `tasks` 表已收敛（`accept_intent(kind=task)` 直接落 `action`，
`query_context` / `today_actions` 读 LifeDB），不再存在"看板与今日视图各说一套"的情况。约束、revision、同步脏标记和审计基线由 Core 确定性执行；Agent 不能绕过枚举和外键约束。

### LifeIndex

App 只展示同一 LifeDB 的四个重叠过滤视图：

| 视图 | 过滤条件 | 回答的问题 |
|---|---|---|
| 事项 | `kind=action` | 具体要做什么 |
| 日常 | `kind=routine` | 什么需要反复发生 |
| 主线 | `track=main` | 什么提高长期核心竞争力 |
| 支线 | `track=side` | 什么让我开心、愿意发展 |

同一个 `action` 可以同时出现在“事项”和“主线/支线”。没有进入四视图的对象必须出现在“待整理”，不得因过滤丢失。

## 2. Notion 权限边界

设置页保存：

- integration token（配置文件权限 `0600`，不返回前端、不进入 prompt）；
- 唯一 LifeIndex page ID；
- `sync_enabled` 开关。

用户在 Notion integration 中只授予 `Read content`、`Update content`，并只连接 LifeIndex 页面。

模型不直连通用 Notion MCP。`notion-lifeindex-gateway.mjs` 在隔离进程中持有 token/page ID，内部连接官方 `@notionhq/notion-mcp-server`，只暴露：

- `read_lifeindex_page()`：读取固定页面完整 Markdown；
- `replace_lifeindex_page(markdown)`：仅在 `LifeIndexSync` 模式出现，替换固定页面正文。

工具参数中没有 `page_id`，因此模型不能把写入重定向到其它页面。替换请求禁止删除子页面/数据库。普通交互和主动推荐模式只读。

## 3. 三方语义合并

每次 `LifeIndexSync` 必须按顺序执行：

1. `list_life_items(include_archived=true)` 与 `list_life_item_edges()` 读取本地当前状态。
2. `render_lifeindex_projection(target_id)` 取得本地投影、上次成功 `last_snapshot_text` 和逐项 `projected_revisions`。
3. `read_lifeindex_page()` 取得 Notion 当前文本。
4. 对比“Notion 当前文本 / 上次成功快照 / LifeDB 当前状态”：
   - 首次同步逐条吸收全部用户事项；无法判断的原文也保存为 `idea + inbox`。
   - Notion 变更用 `upsert_life_item(origin=notion)`；更新已有项带 `id + expected_revision`。
   - 优先使用投影中的 `<!-- lifeitem:uuid -->` 标记；标记被用户删掉时再做语义匹配。
   - 只有删除含义明确且本地没有并发修改时才归档。
   - 字段缺失留空，不猜时间、主次或优先级；冲突优先保留双方信息和可逆结果。
5. 再次生成投影，把返回的 Markdown 原样交给 `replace_lifeindex_page`。
6. 只有 Notion 写回成功后，才调用 `complete_lifeindex_sync(remote_before_text, snapshot_text, projected_revisions, summary)`。每次成功同步都保存写回前全文和最终全文，首次导入也可恢复。

`expected_revision` 提供乐观并发控制。`complete_lifeindex_sync` 只清理 revision 与本轮投影完全一致的行；同步过程中发生的新 App 修改仍保持 `local_dirty`，会触发下一轮，不会被误标为已同步。

## 4. 调度与交互

- 每天 08:30：`agent_run(mode=lifeindex_sync)` 做入站检查和完整合并。
- App、交互 Agent 或旧 task API 修改 LifeDB：立即入队一个带 dedup key 的出站同步，桌面 ticker 最迟约 30 秒拉起。
- App “立即同步”：直接运行相同的 `LifeIndexSync` 模式。
- 全进程同步锁保证同一时间只运行一个同步 Agent。
- 同步成功后宿主发出 `lifeindex-updated`，App 重新读取 SQLite。

未配置/关闭同步时，LifeDB 和 App 可完全离线工作；定时任务跳过，不影响本地编辑。

## 5. 投影格式

Notion 普通页由 Core 生成固定结构：事项、日常、**技能与里程碑**、主线、支线、待整理。每个条目包含状态、标题、kind、track、horizon、可选截止日期与正文，并带稳定 LifeItem 注释标记。

四视图有意重叠，因此同一条目可能在 Notion 出现两次；两处使用同一 ID，Agent 必须合并为一个 LifeItem。Notion 是可编辑投影，不要求用户维护机器字段。

## 6. 失败与恢复

- **整页替换是这条链路上唯一不可逆的一步**，因此必须有恢复入口：`life_sync_runs` 保存每轮的写回前全文，
  用 `list_lifeindex_runs` 找到那一轮、`lifeindex_rollback_text(run_id)` 取回全文，再由 `LifeIndexSync`
  模式交给 `replace_lifeindex_page` 写回。**开启同步前先确认这条路径可用。**
- Notion 读取/写入失败：不调用 complete，保留 dirty 状态，并写 `life_sync_state.last_error`。
- Agent 进程返回但没有完成写回确认：宿主判定本轮失败。
- 页面含不允许删除的子页面/数据库时，整页替换会失败而不是破坏内容。
- 旧 `tasks` 和 `lifeindex_cards` 在 schema 初始化时幂等导入 LifeDB；旧 API 暂保留兼容，新 UI/MCP 只使用 LifeItem。
- 数据可通过 SQLite、MCP JSON 和 Notion Markdown 导出，不绑定单一云产品。

## 7. 验收

- App 新建/编辑/完成/归档 LifeItem 后，SQLite 立即更新并出现待同步状态。
- Notion 修改、新增或明确删除条目后，下次同步能更新同一 LifeItem，不按标题制造副本。
- 同一 action 能同时显示在事项和主线/支线，未归类项不丢失。
- 同步期间并发 App 修改不会被 complete 误清理。
- Agent 工具面不能写目标、提醒、规则、知识库、文件或其它 Notion 页面。
- Notion 不可用或同步关闭时，App/LifeDB 仍可独立使用。
