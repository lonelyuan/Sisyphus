# Spec: 反思平面（Agent）

本文件定义**反思平面**。架构定位见 [architecture.md](architecture.md) §1.2。

---

## 架构定位

反思平面是系统"人类节奏"的那一半：对话、每日规划、复盘、知识加工。它**不负责实时监控**（那是感知平面 + 规则引擎的活，见 [rule-engine.md](rule-engine.md)）。

```
现在（Phase 1）：不自建 Agent 基座
  载体 = Codex / Claude Code
  经 MCP 工具读写数据层（与 Tauri App 同一个 SQLite）
  复用其现成的：对话 UI、工具调用循环、每日定时任务

后期（Phase 2+）：迁移到自研 Agent 基座
  Agent 仍只经 MCP / CLI 碰数据 → 换基座只换脸不换脊椎
  可加 RAG（pgvector）、个性化模型、多 Agent 编排
```

**核心边界**：

- Agent 只能通过 **MCP 工具 / CLI** 访问 Core，不得直接读写 SQLite，不得内联业务逻辑。
- Agent **读取**引擎产出（findings、today-context），**不逐拍驱动**引擎。引擎常驻自转，Agent 查它的结果。
- 不用 Agent 做高频判定（如"在不在摸鱼"）——那是确定性引擎的活，便宜、实时。**Agent 是脸，引擎是脊椎。**

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
```

---

## 数据隐私

- 上下文默认只含 **L0–L1** 数据（时长、目标文本、分类）。
- 不向 LLM 发送完整 URL、聊天内容、截图（L2+），除非用户在设置中显式授权。
- 采集端不得在未授权时产出高于授权等级的 payload（见 [protocol.md](protocol.md) §5）。

---

## API Key 管理

现阶段承载在 Codex / Claude Code，Key 由这些工具自身管理，本项目不经手。

后期若自研基座运行在 Tauri WebView 内，则：

- Key 存 OS 加密存储（Tauri Store `keys.dat`），不写入代码或 `.env`。
- 设置页提供输入框，用户自持 Key；为空时 UI 展示引导。
- 再后期迁移到服务端代理：Key 托管服务端，客户端只改 endpoint。
