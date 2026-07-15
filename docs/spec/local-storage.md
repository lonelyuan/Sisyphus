# Spec: 本地存储

本地存储由 **Rust 侧（rusqlite）** 管理，schema 定义在 `sisyphus/src-tauri/src/db.rs`。

不使用 ORM 或注解处理器（Room/KSP），schema 直接用 SQL 字符串定义，避免编译器插件依赖。

---

## 两个存储：Event log vs Artifact store

见 [architecture.md](architecture.md) §2。本地库分两类表，**语义不同，禁止混用**：

| 类别 | 表 | 性质 | 统一吗 |
|---|---|---|---|
| **Event log** | `raw_events`（+ 派生层同表） | append-only，不可变 | 统一（信封 + payload） |
| **Artifact store** | `daily_goals`、`interventions`、（后续）`notes`/`tasks`/`knowledge_nodes` | 可变，有 status | 不统一，分表 |
| 传输 | `outbox` | 上传队列 | — |

**铁律**：不造多态大表 `artifacts` 用 type 兜住所有对象；每种有状态对象各自建表。**第二个采集源落地前，不新增 artifact 表**（见 [architecture.md](architecture.md) §2.3）。

---

## 表结构

### `raw_events` — 本地事件日志（Event log）

字段与 `packages/protocol/SPEC.md` 信封 1:1 对应。规则引擎从此表查询历史数据。

关键约束：
- `(device_id, seq_no)` 唯一索引，防止重复写入
- `produced_at` 为 epoch ms（本地时间戳，非 RFC3339，上传时转换）
- append-only，**禁止 UPDATE / DELETE**

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
| `outcome` | 干预后结果（延迟回填） |

---

## 本地 schema vs Supabase schema

两份 schema **独立维护**，存在漂移风险：

| 位置 | 文件 |
|---|---|
| 本地 SQLite | `sisyphus/src-tauri/src/db.rs`（SCHEMA 字符串）|
| Supabase | `services/ingest/supabase/migrations/0001_init.sql` |

**约束**：`raw_events` 表字段含义必须一致；本地用 epoch ms 存时间，上传时序列化为 RFC3339。Supabase 表有额外字段（`ingested_at`），本地无需同步。
