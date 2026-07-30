# Spec: 本地存储

本地存储由 **Rust 侧（rusqlite）** 管理，Core schema 定义在 `sisyphus/src-tauri/crates/core/src/db.rs`。

不使用 ORM 或注解处理器（Room/KSP），schema 直接用 SQL 字符串定义，避免编译器插件依赖。

**演进规则（务必遵守）**：`db.rs` 的 `SCHEMA` 始终是**最新**定义（新库一次建好）；把老库补齐到同一形状是
`core::migrations` 的活（`PRAGMA user_version` + 幂等迁移数组）。`CREATE TABLE IF NOT EXISTS` 对已存在的表
是**空操作**——只改 SCHEMA 加列，已装机的库会在查询时报 `no such column`。加列 / 改 CHECK 一律加一条迁移。

---

## 两个存储：Event log vs Artifact store

见 [architecture.md](architecture.md) §2。本地库分两类表，**语义不同，禁止混用**：

| 类别 | 表 | 性质 | 统一吗 |
|---|---|---|---|
| **Event log** | `raw_events`（+ 派生层同表） | append-only，不可变 | 统一（信封 + payload） |
| **Artifact store** | `daily_goals`、`interventions`、`rule_fires`、`notes`/`reminders`/`intent_candidates`、`scheduled_actions`、`detection_rules`、`knowledge_notes`/`knowledge_links`、`monitored_apps` | 可变，有 status | 不统一，分表 |
| **LifeDB** | `life_items`、`life_areas`、`life_item_edges`、`life_item_external_refs`、`life_sync_state`、`life_sync_runs` | 人生规划域的结构化事实、关系和同步审计 | 统一对象 + 邻接表 |
| **派生缓存** | `time_rollups`、`rollup_state` | 无极时间线的预聚合桶；**可从 Event log 完全重建** | — |
| **配置 / 计数** | `settings`、`device_seq` | Core 行为开关（换日点）、每设备 seq 计数器 | — |
| **兼容投影** | `tasks`、`lifeindex_cards` | 已收敛进 LifeDB，只保留读兼容，**不再接收新行** | 可移除 |
| 传输 | `outbox` | 上传队列 | — |

**铁律**：不造能兜住所有业务的多态大表 `artifacts`。LifeItem 只统一人生规划域的
idea/goal/project/action/routine/**skill/milestone**；note/reminder/intervention 等仍有自己的表。

---

## 表结构

### `raw_events` — 本地事件日志（Event log）

字段与 `packages/protocol/SPEC.md` 信封 1:1 对应。规则引擎从此表查询历史数据。

关键约束：
- `(device_id, seq_no)` 唯一索引，防止重复写入。`seq_no` 来自 `device_seq` 计数器表的**单语句原子自增**，
  不是 `MAX+1`（后者在并发下会撞唯一索引并被 `INSERT OR IGNORE` 静默吞掉）
- `produced_at` 为 epoch ms（本地时间戳，非 RFC3339，上传时转换）
- append-only，**禁止 UPDATE / DELETE**
- 写入前必须过 `ingest::validate`（枚举白名单 + interval 必须有 start/end）
- 桌面采集器长时间停在同一 app 时，每 5 分钟落一个**闭合切片**（不是等切换应用才写），
  否则看两小时电影在 Event log 里是一段空白

主要查询（规则引擎用）：
```sql
-- 窗口内娱乐 app 总时长
SELECT COALESCE(SUM(end_time - start_time), 0)
FROM raw_events
WHERE user_id = ? AND layer = 'raw' AND type = 'app_foreground'
  AND category LIKE 'entertainment%'
  AND start_time >= ?         -- 窗口起点 epoch ms
  AND end_time IS NOT NULL;

-- 最近一条前台事件
SELECT * FROM raw_events
WHERE user_id = ? AND layer = 'raw' AND type = 'app_foreground'
ORDER BY start_time DESC LIMIT 1;
```

### `outbox` — 上传队列

待上传到 Supabase 的事件队列。sync 模块从此表读取 pending 记录批量上传。

| 字段 | 说明 |
|---|---|
| `event_id` | 与 raw_events 相同的 UUID |
| `payload_json` | 完整序列化的 BehaviorEvent（上传用）|
| `sync_status` | `pending` → `uploading` → `done` \| `failed` |
| `retry_count` | 失败重试计数 |
| `created_at` | epoch ms |

生命周期：写入时 `pending`，批量上传成功后标记 `done`，定期清理 `done` 记录。

### `daily_goals` — 今日目标

| 字段 | 说明 |
|---|---|
| `id` | UUID |
| `date` | `"2026-06-29"` |
| `raw_text` | 用户输入的目标文本 |
| `status` | `planned` \| `started` \| `completed` \| `skipped` \| `abandoned` |

规则引擎读取 `status` 判断目标是否未完成。

### `interventions` — 干预记录

| 字段 | 说明 |
|---|---|
| `id` | UUID |
| `rule_id` | 触发的规则 ID |
| `shown_at` | 弹出时刻（epoch ms） |
| `user_response` | 用户点击的按钮：`start_task` \| `take_rest` \| `continue` \| `abandon_today` |
| `outcome` | 近端结果：`still_entertainment` \| `mixed` \| `switched` \| `unknown`（干预后 10/30 分钟回填，只填一次）|
| `outcome_detail` | 观测明细，如 `娱乐 7.2min / 观测 9.0min` |
| `outcome_at_ms` | 观测时刻 |

### `rule_fires` — 规则响应事实（append-only）

命中当拍就写，**与"通知有没有真的弹"无关**。冷却与 debounce 窗口只看这张表——见
[rule-engine.md](rule-engine.md) 的 CooldownStore 小节（这是一个曾经出过通知风暴的地方）。

| 字段 | 说明 |
|---|---|
| `rule_id` / `fired_at_ms` | 哪条规则、什么时候响应的 |
| `policy` | `immediate` \| `deferred` \| `debounce` \| `suppress` |
| `dedup_key` | debounce 窗口判断用 |

### `knowledge_notes` / `knowledge_links` — 第二大脑索引

`.md` 是本体，本表是**可重建的投影**（`knowledge::reindex_vault`）。关键约束：

- `title` 对存活行（`status NOT IN ('pruned','duplicate')`）**部分唯一索引** —— 一个概念一张卡。
  幂等键是 `title` 而非 `path`：同标题换 folder = 移动，不是复制。
- `body` 进索引，让查重能搜正文（只按标题查重是碎片化的机械原因）。
- `aliases_json` 是重定向（合并碎卡后旧 `[[链接]]` 仍可解析）。
- `knowledge_links` 保存出链边；`resolved=0` 的就是**红链** = 知识缺口 = 主动调研队列。
  提取时跳过代码区（`` `[[链接]]` `` 是讲语法的示意写法，不是链接）。
- **两种节点共用这张表**，靠 `tags_json` 区分：知识卡片（有 type + 可靠性档）与
  **原始材料**（含 `source` 标签，就地存放在话题夹、无出链、`publish: false`）。
  领域枢纽含 `hub` 标签，每个 `folder` 恰好一个；其目录区块由 `knowledge::refresh_mocs` 生成。

### `time_rollups` / `rollup_state` — 无极时间线预聚合

桶按**逻辑日**切（本地时区 + 换日点），周/月桶由日桶再聚合。`rollup_state.watermark_ms` 支撑增量重建。
维度三个：`category` / `app` / `hour`（`hour` 的 key 是 `"HH|category"`，`HH` 为逻辑日内小时序号，
**只存在于日桶**，周/月再聚合显式排除）。
**改换日点必须整体重算**（桶边界变了，旧桶口径失效）。详见 [lifeindex-mind-system.md](lifeindex-mind-system.md) §3.2。

### LifeDB — `life_items` / `life_item_edges`

`life_items` 的 `kind` 表示对象从模糊到落地的形态（idea/goal/project/action/routine/**skill/milestone**），
`track` 表示主线/支线，`horizon` 表示时间尺度，`area_id` 指向责任领域（GTD Horizon 3）；四者正交。
`revision + sync_status` 支持乐观并发和 Notion 双向同步。

可判定与度量字段（技能树进度的来源，见 [lifeindex-mind-system.md](lifeindex-mind-system.md)）：
`success_criteria`（一句可判定的完成条件）、`target_value` / `current_value` / `unit`（度量）、`review_at_ms`（毕业审查）。

`life_item_edges` 是邻接表，保存 contains/supports/depends_on/blocks/derived_from/related。
**`contains`/`supports` 是层级边（构成树），`depends_on` 是前置边（构成 DAG，不参与父子关系）** ——
技能树的形状完全由这两类边表达，不新建平行模型。当前规模用 SQLite 索引与内存建树；没有引入图数据库的理由。

`life_areas` 是责任领域：无完成态，只需维持标准；`focus=1` 参与主线推导与今日行动选择。

### Notion 同步审计 — `life_item_external_refs` / `life_sync_state`

- `life_item_external_refs`：外部稳定 ID、URL、更新时间、hash、最后发布 revision；不保存 token。
- `life_sync_state`：唯一页面的上次成功 Markdown 快照、摘要、成功/尝试时间与错误。
- `life_sync_runs`：每次成功同步的 Notion 写前全文与最终投影；用于恢复首次导入或错误语义合并。

Notion token 和 page ID 由 app 数据目录的 `notion_config.json` 保存（Unix `0600`）；token 不进 SQLite、不返回前端。完整语义见 [notion-integration.md](notion-integration.md)。

---

## 本地 schema vs Supabase schema

两份 schema **独立维护**，存在漂移风险：

| 位置 | 文件 |
|---|---|
| 本地 SQLite | `sisyphus/src-tauri/crates/core/src/db.rs`（SCHEMA 字符串）|
| Supabase | `services/ingest/supabase/migrations/0001_init.sql` |

**约束**：`raw_events` 表字段含义必须一致；本地用 epoch ms 存时间，上传时序列化为 RFC3339。Supabase 表有额外字段（`ingested_at`），本地无需同步。
