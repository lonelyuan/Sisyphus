# 意图路由详表

MCP 工具面（`sisyphus` server）完整签名与用法。所有工具即使桌面 App 没开也能工作（MCP 独立开同一 SQLite）。

## 通用

- `capture(text)` → `captured: <event_id>`。**任何用户态输入先调它**；普通代码开发、测试和文档修改指令不调。返回的 event_id 用于 `propose_intents`。
- `list_captures(unprocessed?=true)` → `[{event_id, text, created_at}]`。本地待处理记录；`unprocessed=true` 只列还没生成候选的。它不是 Notion Inbox。
- `query_context()` → `{date, goal, entertainment_minutes, intervention_count, open_tasks[], due_reminders[], recent_interventions[]}`。规划/复盘前先调。
- `today_actions()` → `["…"]`（1–3 条）。

## 笔记 / 素材（→ Core note）

判定：陈述性、要记住/以后查、文章/链接/概念/想学的领域。

- `capture(text)` 保留原始输入。
- `propose_intents(capture_event_id, [{kind:"note", proposed:{title?, body, tags?}}])` 生成最小笔记候选；`body` 保留必要要点与原始链接，不擅自扩写成知识文章。
- 向用户复述候选；确认后 `accept_intent`，修改则通过 `edits` 覆盖，拒绝则 `ignore_intent`。
- Sisyphus 项目禁用知识库工具，不调用 `ingest_document`、`write_knowledge_note`、`search_knowledge` 或 `list_knowledge`。

## 任务 / 提醒（→ 落库 + 到点提醒）

判定：可执行、有下一步、或有明确时间点（“提醒我…”）。

- `propose_intents(capture_event_id, candidates)`，`candidates=[{kind, proposed, confidence?}]`：
  - `kind="task"`，`proposed={title, due_ms?, priority?, note?}`。
  - `kind="reminder"`，`proposed={text, remind_at_ms, recurrence?}`。**`remind_at_ms` 必填、真实 epoch 毫秒**——到点端侧（macOS 采集器 / Android 前台服务）会自动弹通知。
  - `kind="goal"`，`proposed={text}`（等价 set_goal）。
  - `kind="note"`，`proposed={title?, body, tags?}`（想法 / 素材 / 偏好；情绪打 `tags:["mood"]`）。
- `accept_intent(intent_id, edits?)` → 落成 artifact。`edits` 是覆盖候选字段的 JSON（用户就地改）。
- `ignore_intent(intent_id)` → 忽略（不落库）。
- 拿不准时间/优先级就**留空别编**。一条 capture 通常只提 1 个候选。

## 习惯 / 拖延（→ 启动西西弗斯计划）

判定：“想少刷/戒掉某 app”、“容易分心”、“今天要专注做 X”。

- `set_goal(text)` → 设/更新今日目标（规则引擎判断“目标未完成”的依据；没有目标就不会触发干预）。
- `list_monitored_apps()` → 看当前监控名单（内置 + 自定义）。
- `add_monitored_app(id, category)` → 把某娱乐 app 纳入监控。`id`=bundle id / 包名，`category`=`entertainment.video|game|social|news`。桌面 + 安卓即时生效。
- `remove_monitored_app(id)` → 移除。
- 机制：用户设了目标 + 停留在监控名单内的 app 超过阈值（debug 1min / release 15min）+ 冷却满足 → 端侧自动弹「不羞辱、引用真实时长和目标」的干预通知（四按钮：开始任务 / 休息 / 继续 / 放弃今日）。你不需要逐拍驱动，配好即可。
- 常见包名：抖音 `com.ss.android.ugc.aweme`、B站 `tv.danmaku.bili`、快手 `com.kuaishou.nebula`、小红书/微博 `com.sina.weibo`、YouTube `com.google.android.youtube`（多数已内置，`list_monitored_apps` 可查）。

## LifeIndex / LifeDB（→ 长短期人生看板）

判定：长期目标、项目、具体行动、日常习惯、想发展的主线/支线，或用户明确说“放进人生看板”。

- 五种形态：`idea | goal | project | action | routine`。
- 两个正交维度：`track=main|side|neutral|undecided`，`horizon=now|next|later|someday|unscheduled`。
- 先 `capture` 原话，复述你准备写入的最小结构；用户认可后调用 `upsert_life_item(origin="agent")`。
- 未知字段保持 `undecided/unscheduled/inbox`，不要为了整齐猜时间、主次和优先级。
- 目标/项目包含子项时用 `link_life_items(relation="contains")`；不要一次生成任务海。
- App/Agent 写入会自动标 `local_dirty` 并排队；普通会话不直接改 Notion。
- 专用 `LifeIndexSync` 按 [notion-integration.md](../../../docs/spec/notion-integration.md) 三方合并；更新已有项带 `id + expected_revision`，写回成功才 complete。

## 多意图处理

一条输入可含多类：先 `capture` 一次，再对**同一个 capture_event_id** 分别路由（笔记走 note 候选，任务走 propose/accept，习惯走 set_goal + add_monitored_app）。给用户一个合并的、最小的确认，而不是四条分开的问句。
