# Spec: Notion 集成（用户编辑 · 智能体只读）

本文件是西西弗斯接入 Notion / LifeIndex 的**权威文档**。权限边界、同步语义、主动任务信息流和本地镜像都以本文件为准。

上位文档：[architecture.md](architecture.md)（两平面 + 双存储）。主动触发见 [proactive-triggers.md](proactive-triggers.md)。Notion 连接器属于 app / MCP 适配层，**不进入 `sisyphus-core`**。

---

## 0. 决策

> **Notion 是用户维护的个人上下文源；西西弗斯只读取、理解和缓存，不编辑 Notion。**

本次架构调整废弃早期的 LifeIndex 双向流：

- 不创建或维护 `📥 Inbox`。
- 不创建或维护 `🔄 NOW` / “下一项”挂件。
- 不勾选、划线、追加、移动、归档或重写任何 Notion 内容。
- 不要求用户为了智能体改成固定的 GTD / PARA / 数据库模板。

智能体仍可在一次主动任务中推理出“此刻最适合做的一件事”，但它是**短暂的推荐结果**，只经宠物或系统通知展示，不成为 Notion 中的持久字段，也不回写完成状态。

---

## 1. 状态所有权

系统没有一个能覆盖所有数据的“唯一真相源”。事实按创建者和用途归属：

| 状态 | 权威来源 | 谁能编辑 | 西西弗斯如何使用 |
|---|---|---|---|
| 用户写下的计划、目标、项目、复盘和文档 | Notion | **只有用户** | 只读拉取，生成本地镜像，供推理引用 |
| 行为事件、idle、娱乐时长、干预和反馈 | 本地 SQLite | 采集器 / Core | 作为当前行为与节奏上下文 |
| 知识卡片与长文 | Obsidian vault | 用户及获准的知识工作流 | 作为可选知识上下文 |
| 主动建议、投递记录、冷却和近端结果 | 本地 SQLite | 西西弗斯宿主程序 | 防重复、评估提醒是否有效 |

因此“西西弗斯状态和 Notion 同步”的准确含义是：**用户拥有的状态从 Notion 单向同步到本地只读镜像；系统产生的运行状态留在本地。** 两边不互相覆盖。

---

## 2. 权限边界：机制保证，不靠提示词自觉

### 2.1 Notion 连接

- Notion integration 只申请内容读取能力，只共享用户明确选择的页面 / 数据源。
- 凭据放在 OS Keychain / Secret Store，不写 SQLite、日志或 agent prompt。
- 主动智能体只拿到 `list/read/query` 形态的只读连接器；**不得给它挂通用的、含 create/update/delete 能力的 Notion MCP**。
- 连接器在运行时拒绝所有写方法。即使模型生成了写操作，也无法执行。

### 2.2 本地写入

“智能体只读”特指智能体对用户内容源只读。宿主程序仍可确定性地写本地运行元数据，例如同步游标、镜像快照、通知投递结果和用户反馈；这些写入不改变用户的 Notion 内容。

---

## 3. 可替换的信息源适配器

Notion 是当前首选信息源，但存储和推理层不应写死 `notion_actions`。app / MCP 适配层提供统一只读接口：

```text
ContextSource
  refresh(scope, cursor?) -> RefreshResult
  read_context(scope, limit) -> ContextItem[]
  health() -> SourceHealth
```

首个实现是 `NotionContextSource`；以后可接本地 Markdown、日历或其他任务工具，而不修改调度器和推理协议。

适配器将外部内容归一成一个**只读上下文信封**，同时保留原始引用：

```json
{
  "source": "notion",
  "connection_id": "personal-notion",
  "external_id": "page-or-block-id",
  "kind": "page|database_row|todo|text",
  "title": "用户原文标题",
  "content": "用户原文或确定性提取文本",
  "status": "可选，按源原样映射",
  "due_at_ms": null,
  "tags": [],
  "source_updated_at_ms": 0,
  "observed_at_ms": 0,
  "content_hash": "..."
}
```

原则：

- 原文优先，不让 LLM 在同步阶段改写用户内容。
- 字段缺失就留空，不强迫用户采用特定排版。
- 每条上下文都带来源、外部 ID、源更新时间和观测时间，推荐结果可以溯源。
- 大页面可按块 / 段落切片，但切片只是可重建投影，不是新的权威内容。

---

## 4. 本地镜像：缓存，不是第二个编辑面

建议在适配层新增两类通用表，而不是 Notion 专用 artifact：

```sql
CREATE TABLE source_connections (
  id                  TEXT PRIMARY KEY,
  kind                TEXT NOT NULL,          -- notion | markdown | calendar | ...
  access_mode         TEXT NOT NULL,          -- 必须为 read_only
  scope_json          TEXT NOT NULL,          -- 用户授权的 page/data-source id；不含密钥
  enabled             INTEGER NOT NULL DEFAULT 1,
  last_attempt_at_ms  INTEGER,
  last_success_at_ms  INTEGER,
  last_error          TEXT
);

CREATE TABLE source_snapshots (
  connection_id       TEXT NOT NULL,
  external_id         TEXT NOT NULL,
  kind                TEXT NOT NULL,
  source_updated_at_ms INTEGER,
  observed_at_ms      INTEGER NOT NULL,
  content_hash        TEXT NOT NULL,
  payload_json        TEXT NOT NULL,
  deleted_at_ms       INTEGER,
  PRIMARY KEY (connection_id, external_id)
);
```

这些表是**可丢弃、可重建的读取缓存**，不属于 Artifact store 的用户事实。Notion 仍是用户内容的权威来源。同步器可以更新 `source_snapshots`；agent 只能读取它。

推荐、投递、用户响应继续记录在本地 `scheduled_actions` / `interventions` 等运行表中。不要为了 Notion 再造一套任务真相。

---

## 5. 主动任务信息流

```text
时间 / 行为规则触发
  → scheduled_actions: agent_run(mode=proactive_recommendation)
  → 宿主并行读取本地 query_context + 刷新已启用的只读 ContextSource
  → 用本次新结果更新 source_snapshots
  → agent 读取“本地状态 + 新鲜外部上下文 + 历史反馈”
  → 只生成 1 条结构化推荐（不调用 Notion 写 API）
  → 宿主执行防打扰 / 去重 / 隐私校验
  → 宠物气泡和/或系统通知推送给用户
  → 点击、忽略及 10/30 分钟近端结果只回写本地
```

建议的 `agent_run` payload：

```json
{
  "mode": "proactive_recommendation",
  "sources": ["local", "notion"],
  "source_freshness": "refresh_before_reasoning",
  "max_recommendations": 1,
  "delivery": ["pet", "notification"],
  "scope": "personal"
}
```

agent 的输出是数据，不直接产生副作用：

```json
{
  "title": "现在可以推进一小步",
  "body": "花 10 分钟整理毕昇杯验证记录。",
  "reason": "你刚进入空闲窗口，Notion 中该项目今天有更新。",
  "source_refs": ["notion:page-or-block-id"],
  "suggested_duration_min": 10
}
```

推荐必须引用实际上下文，只给一件事；没有足够依据时允许输出 `no_recommendation`，不能为了完成定时任务而硬推。

---

## 6. 刷新、同步与冲突

### 6.1 刷新策略

- **主动任务前强制刷新**：每次推理前读 Notion API，确保用户刚改的内容能进入本次判断。
- **后台增量刷新（可选）**：轮询或 Webhook 只负责让缓存更热；Webhook 到达后仍由连接器读取最新对象。
- 使用 `external_id + source_updated_at + content_hash` 幂等更新镜像。
- API 限流按 `Retry-After` 退避；分页和增量游标由连接器封装。

### 6.2 离线与陈旧数据

- Notion 临时不可用时，可回退到最后一次成功快照，但必须把 `last_success_at` 传给 agent。
- 超过用户配置的陈旧阈值（默认建议 24 小时）时，不基于该源生成具体任务；可跳过本次推送。
- 读取失败不影响本地行为干预和固定提醒。

### 6.3 冲突模型

因为同步方向只有 `Notion → 本地镜像`，不存在双向写冲突。用户删除或移动内容后，下一次刷新将镜像标记为删除 / 越界；本地历史推荐保留 source ref 供审计，但不得把旧内容写回 Notion。

---

## 7. 隐私与可观测性

- 用户按页面 / 数据源授权，默认不扫描整个 workspace。
- 日志只记录 object id、耗时、条数、hash 和错误码，不记录页面正文。
- 进入模型的内容遵守 [protocol.md](protocol.md) 隐私等级；敏感页可排除或只做本地推理。
- 每次主动推荐记录：触发原因、使用的数据源、各源新鲜度、source refs、最终渠道和用户响应。
- 设置页应能看到连接健康度、上次成功同步时间，并可一键停用 / 清空本地镜像；停用不改 Notion。

---

## 8. MVP 与验收

### 现状（2026-07）：MVP 已接入，走 agent 直连而非本地镜像

实现路径比 §3/§4 描述的 `NotionContextSource`/`source_snapshots` 更薄：两个基座（Pi/Codex）各自
接入官方 **`@notionhq/notion-mcp-server`**（Pi 侧 `pi-agent-runtime.mjs` 起第二个 MCP client 合并
工具面；Codex 侧 `agent_runtime.rs` 用 `-c mcp_servers.notion.*` 注入）。**只读边界由 Notion 侧的
integration 权限机制保证**——设置页引导用户建 token 时只勾 "Read content"，即使模型误调用写工具，
Notion API 会直接拒绝，不靠我们代码或提示词自觉（守住 §2.1 的铁律，只是执行点在 Notion 侧而非本地
适配层）。Token 存 `data_dir/notion_config.json`（0600，同 `llm_config.json` 模式）。

**与原设计的差异（已知简化，非最终态）**：
- 没有本地 `source_snapshots` 镜像/缓存——agent 每次直接实时调用 Notion 工具读取，不经确定性同步器刷新游标。
- 没有页面范围白名单校验在我们代码里做——范围完全由用户在 Notion 侧 Share 给 integration 的页面决定。
- 设置页只显示 token 有无，不显示"最后同步时间/错误状态"（因为没有本地同步这一步）。

`source_snapshots` 缓存留作后续（若直连延迟/速率限制成为问题再补）。

### MVP

1. 只读 `NotionContextSource`：限定页面范围，支持分页、刷新和本地快照。
2. `proactive_recommendation` agent job：同时读取 `query_context` 与刷新后的 Notion 上下文。
3. 结构化推荐经宠物 / 系统通知投递；反馈和投递结果仅存本地。
4. 设置页展示只读权限、授权范围、最后同步时间和错误状态。

### 架构验收条件

- 用只读凭据运行，系统仍能完成“触发 → 读取两类上下文 → 推理 → 推送”闭环。
- 代码和 agent 工具面不存在 Notion create / update / append / delete 路径。
- 用户在 Notion 修改内容后，下一次主动任务能读到新版本；Notion API 失败时能清楚降级。
- 更换为另一个 `ContextSource` 不需要修改 scheduler / agent 输出 / delivery 协议。
- 宠物和系统通知消费同一份推荐结果，不各自重复推理。

---

## 9. 明确不做

- 不把西西弗斯做成 Notion 编辑器或任务管理器。
- 不维护 Notion Inbox、NOW、下一项或 AI 生成看板。
- 不从通知按钮回写 Notion 完成状态。
- 不把 Notion 当行为事件库，也不把本地行为数据反向同步进 Notion。
- 不承诺毫秒实时；主动任务前的新鲜读取比常驻高频轮询更重要。
