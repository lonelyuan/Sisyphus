# Spec: 反思平面（Agent）

本文件定义**反思平面**。架构定位见 [architecture.md](architecture.md) §1.2。

---

## 架构定位

反思平面是系统"人类节奏"的那一半：对话、每日规划、复盘、知识加工。它**不负责实时监控**（那是感知平面 + 规则引擎的活，见 [rule-engine.md](rule-engine.md)）。

```
当前 App runtime：
  Pi = Rust scheduler / Tauri command → Node sidecar → Pi JS SDK
       直接 import createAgentSession / ModelRuntime，不调用 pi CLI
  Codex = Rust scheduler / Tauri command → Codex runtime
  两者共用 Sisyphus skill 和只读 MCP 契约

后期：
  可替换 Agent 基座或 sidecar 打包方式；Core 与 MCP 契约不变
  可加 RAG（pgvector）、个性化模型、多 Agent 编排
```

**核心边界**：

- Agent 只能通过 **MCP 工具 / CLI** 访问 Core，不得直接读写 SQLite，不得内联业务逻辑。
- Agent **读取**引擎产出（findings、today-context），**不逐拍驱动**引擎。引擎常驻自转，Agent 查它的结果。
- 不用 Agent 做高频判定（如"在不在摸鱼"）——那是确定性引擎的活，便宜、实时。**引擎是常驻的确定性后端，Agent 是按需的交互前端。**

### 运行模式（写门禁按模式区分）

宿主（`agent_runtime::RunMode`）按场景决定 Agent 的写权限，MCP server 据 `SISYPHUS_READ_ONLY` / `SISYPHUS_LIFEINDEX_ONLY` 硬门禁执行：

| 模式 | 触发 | 本地写 | 外部源（Notion） |
|---|---|---|---|
| **Interactive** | 主对话 / 宠物（用户主动） | 可写（经用户确认可 set_goal / 建规则 / 写知识 / 落任务等） | 只读 |
| **Proactive** | 定时 / 规则触发的推荐 | 只读 | 只读 |
| **LifeIndex** | 每日看板刷新 | 仅看板卡片（`upsert_lifeindex_card`），其它写禁用 | 只读 |

两个基座（Pi JS SDK / Codex）连同一个 `sisyphus-mcp`，工具面与门禁一致；换基座只换交互前端。

### 个人看板主动模式的额外边界

`agent_run(mode=proactive_recommendation)` 是一个**只读推理任务**：

- 只读取本地 `query_context`、历史反馈和已配置的 `ContextSource`。
- Notion 工具面只有 list / read / query，不暴露 create / update / append / delete。
- 不在 Notion 维护 Inbox、NOW、“下一项”或完成状态；Notion 全部内容只有用户编辑。
- agent 只返回一条结构化 recommendation 或 `no_recommendation`，不直接发通知、不写数据库。
- 宿主程序负责新鲜度检查、冷却 / 去重 / 隐私策略、记录投递结果，并把同一 recommendation 推给宠物和/或系统通知。

这不影响交互式会话在用户确认后通过 MCP 写本地 artifact，也不影响知识工作流写获准的 Obsidian vault；限制针对的是主动任务及用户拥有的外部内容源。

---

## MCP 工具面

App 侧运行一个 MCP server（数据层之上的薄适配器），暴露下列工具给 Codex / Claude Code。完整清单见 [architecture.md](architecture.md) §4，这里给语义与调用约定。

| 工具 | 输入 | 输出 | 副作用 |
|---|---|---|---|
| `capture` | `{ text \| url \| file }` | `{ capture_id }` | 写一条 `manual/note_text` 事件到 Event log |
| `query_context` | `{ scope?: "today" }` | 今日上下文 JSON（见下） | 只读 |
| `today_actions` | `{}` | `{ actions: [1–3 条最小行动] }` | 只读 |
| `propose_intents` | `{ capture_id }` | `{ candidates: [...] }` | 生成意图候选（带来源与置信度） |
| `accept_intent` | `{ intent_id, edits? }` | `{ artifact_id }` | 落成 / 更新一个 artifact |
| `record_feedback` | `{ intervention_id, label, text? }` | `{}` | 写 feedback 事件 |
| `ingest_document` | `{ url \| file }` | `{ doc_id, summary, concepts }` | 材料摄取（第二大脑） |

### `query_context` 输出示例

由数据层从本地 SQLite 构建（离线可用），只含 L0–L1 数据：

```json
{
  "date": "2026-07-14",
  "goal": { "text": "完成论文第三章", "status": "started" },
  "stats": { "entertainment_minutes": 42, "work_minutes": 15, "intervention_count": 2 },
  "recent_interventions": [
    { "shown_at": "14:32", "response": "continue" },
    { "shown_at": "15:18", "response": "start_task" }
  ]
}
```

---

## 系统提示与语气

无论承载在 Codex/Claude Code 还是自研基座，反思平面对用户说话的原则一致：

```
你是用户的习惯助手。你了解用户今天的行为数据和目标（来自 query_context）。
以关心但不评判的语气回应。不要羞辱或说教。
提醒要具体（引用实际时长和目标），不要泛泛而谈。
只提出最小下一步，不生成任务海。
个人看板内容只读；建议只通过宠物或通知呈现，不回写用户文档。
```

---

## 数据隐私

- 上下文默认只含 **L0–L1** 数据（时长、目标文本、分类）。
- 不向 LLM 发送完整 URL、聊天内容、截图（L2+），除非用户在设置中显式授权。
- 采集端不得在未授权时产出高于授权等级的 payload（见 [protocol.md](protocol.md) §5）。

---

## Pi 配置与 API Key

Pi 不使用全局 CLI 的 `/login` 状态。在 App「设置 → Pi JS SDK 模型配置」填写：

1. Provider / API 协议（例如 OpenAI Responses、Anthropic Messages）。
2. API Endpoint；官方 provider 可留空，内网网关或兼容接口填完整 base URL。
3. Model ID 和 API Key。
4. 点「保存并测试」，由 Pi SDK 发起最小请求验证。

Key 不返回 WebView；Rust 只在启动 SDK sidecar 时通过子进程环境传入。配置文件在 Unix 上限制为 `0600`。后续仍应迁移到 OS Keychain / 安全存储。
