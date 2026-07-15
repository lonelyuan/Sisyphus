# Sisyphus 事件协议 v1.0

状态：MVP 基线规范  
日期：2026-06-28  
本文件是**唯一权威定义**（source of truth）。各端（Kotlin / Rust / TypeScript）的类型定义都必须与本文件保持一致；本文件改动时，所有端在同一次提交内一起改。

---

## 0. 设计目标

1. **跨端统一**：手机、电脑、浏览器产生的事件长成同一个样子，否则未来无法联合建模。
2. **多层次、可重合**：触控、应用前台、应用内浏览、会话、规则命中处在不同层次，时间上互相重合是正常的——协议保留分层，不压平。
3. **无损可重算**：原始事件 append-only、不可变、带稳定 id 与精确时间戳，未来可用新的特征方案重新计算、离线重放。
4. **为后期 RL/中等模型留路**：决策日志带倾向分（propensity），结果日志带多窗口 outcome，使 (state, action, reward) 可被重建。

---

## 1. 统一信封（Envelope）

所有事件——无论是原始采集、聚合会话、规则命中还是干预结果——都共用同一套信封字段，差异放进 `payload`。

| 字段 | 类型 | 必填 | 说明 |
|---|---|---|---|
| `schema_version` | string | ✅ | 协议版本，当前 `"1.0"`。加字段时向后兼容，破坏性变更升大版本。 |
| `event_id` | uuid (string) | ✅ | 端侧生成。**幂等主键**，重复上报无副作用。 |
| `user_id` | string | ✅ | MVP 单用户固定 `"local-user"`。 |
| `device_id` | string | ✅ | 如 `"pixel-8"`、`"macbook-1"`、`"chrome-ext-1"`。 |
| `seq_no` | int64 | ✅ | **单设备内单调递增**，用于增量拉取与查漏。 |
| `source` | enum | ✅ | 见 §2。 |
| `layer` | enum | ✅ | 事件所在层次，见 §3。 |
| `type` | string | ✅ | 该层内的具体类型，见 §4。 |
| `time_mode` | `"point"` \| `"interval"` | ✅ | 点事件 vs 区间事件。 |
| `event_time` | RFC3339 string | 点事件必填 | `time_mode="point"` 时使用。 |
| `start_time` | RFC3339 string | 区间必填 | `time_mode="interval"` 时使用。 |
| `end_time` | RFC3339 string | 区间可空 | 进行中的区间可为 null，结束后回填。 |
| `entity` | string | 可空 | 主体标识：包名 / 域名，如 `tv.danmaku.bili`、`bilibili.com`。 |
| `category` | string | 可空 | 分类，如 `entertainment.video`、`work.doc`。MVP 用硬编码白名单。 |
| `payload` | object | ✅ | 该 `type` 专属字段，见 §4。可为 `{}`。 |
| `parent_event_ids` | uuid[] | ✅ | 血缘：派生事件指向其来源事件。raw 层为 `[]`。 |
| `privacy_level` | `"L0"`..`"L3"` | ✅ | 见 §5。 |
| `produced_at` | RFC3339 string | ✅ | 端侧生成该事件的时刻。 |

---

## 2. `source` 枚举

```
android_usage        Android 使用情况采集
android_accessibility Android 无障碍（滚动等，v1.1+）
desktop_agent        桌面端活动窗口/进程采集
browser_extension    浏览器插件 tab/url 采集
engine               习惯引擎产出（finding/decision/intervention）
manual               用户手动输入（目标、反馈）
agent                LLM Agent 产出
```

## 3. `layer` 枚举（分层模型）

借鉴 EDR/SIEM 的分层。派生层通过 `parent_event_ids` 指回来源层。

```
raw          原始采集（或端侧预聚合后的原始信号），append-only，不再聚合
session      聚合后的行为片段（娱乐会话、工作会话）
finding      规则命中的可干预机会
decision     策略引擎的选择记录（含倾向分）
intervention 实际对用户执行的提醒/对话/遮罩
outcome      干预的近端结果（10/30/60 分钟窗口）
feedback     用户显式反馈
```

## 4. `type` 与 `payload` 约定（MVP 子集）

### raw 层

- `app_foreground`（interval）：前台应用。`entity`=包名。`payload`: `{}`。
- `url_visit`（interval）：浏览器 tab 访问。`entity`=域名。`payload`: `{ "url_hash"?: string, "title"?: string }`（完整 url/title 属 L2，默认不传）。
- `window_active`（interval）：桌面活动窗口。`entity`=进程名。`payload`: `{ "window_title"?: string }`（title 属 L1/L2）。
- `idle`（interval）：无操作。`payload`: `{}`。
- `note_text`（point）：用户手动输入的自然语言 capture。`source`=`manual`，`payload`: `{ "text": string }`（属 L1）。反思平面 `capture` 工具写入此类型，是 Event log 与 Artifact store 之间"意图提取"的输入。
- `knowledge_ingested`（point，v1.0+）：第二大脑写入一张知识卡片的**溯源面包屑**。`source`=`agent`，`payload`: `{ "title": string, "path": string, "sources": string[], "concept_count"?: int }`（title 属 L1）。可读知识本体存 Obsidian vault 的 `.md` 文件；`knowledge_notes` 索引行为可查询真相；本事件只记「发生过 + 指针」，让知识写入也进入统一 Event log（见 architecture.md §2「统一发生在数据层」）。
- `scroll_burst`（interval, v1.1+）：**端侧预聚合**的滚动信号，不要逐次上报。`payload`: `{ "scroll_count": int, "window_sec": int, "avg_interval_ms": int }`。

### session 层

- `entertainment_session` / `work_session`（interval）。`parent_event_ids`=聚合的 raw 事件。`payload`: `{ "primary_device": string, "features": {...} }`。

### finding 层

- `procrastination_risk`（point）。`payload`: `{ "rule_id": string, "rule_version": int, "severity": string, "confidence": number, "context_snapshot": {...}, "recommended_intervention_types": string[], "status": "shadow"|"suppressed"|"actioned"|"dismissed" }`。

### decision 层

- `policy_decision`（point）。`payload`: `{ "policy_id": string, "policy_version": string, "feature_schema_version": string, "available_actions": string[], "chosen_action": string, "choice_probability": number, "scores": {...}, "exploration": bool, "constraints": {...} }`。
  - **`choice_probability`（倾向分）必填**：未来做离线策略评估 / off-policy 学习的前提。

### intervention 层

- `notification` | `dialog` | `chat` | `page_overlay` | `process_control`（point）。`payload`: `{ "target_device_id": string, "message": string, "options": string[], "bct_type"?: string }`。

### outcome 层

- `proximal_outcome`（point）。`parent_event_ids`=对应 decision/intervention。`payload`: `{ "window": "10m"|"30m"|"60m"|"day_end", "observed": {...}, "reward_components": {...}, "reward_total": number }`。

### feedback 层

- `user_feedback`（point）。`payload`: `{ "intervention_id": string, "label": "helpful"|"annoying"|"guilt"|"normal_rest"|"wrong_time"|..., "text"?: string }`。

> 新增 `type` 时：先在本文件登记，再在各端落地。`payload` 内部演进用 §6 的版本策略。

## 5. 隐私等级 `privacy_level`

```
L0  时间、应用包名、域名、类别、时长、滚动计数
L1  页面标题、用户目标、反馈按钮
L2  完整 URL、聊天内容、语音转写
L3  截图、OCR、输入内容、敏感 app 内容
```

MVP 默认只采 **L0–L1**。L2–L3 必须单独授权。采集端不得在未授权时产出高于授权等级的 payload 字段。

## 6. 版本与兼容

- `schema_version` 跟随本协议大版本。
- **加字段** = 向后兼容，不升大版本；消费端必须容忍未知字段。
- **删字段 / 改语义** = 破坏性，升大版本，并提供迁移说明。
- `feature_schema_version`（在 decision payload 内）独立演进，与协议版本解耦——特征方案可频繁迭代而不动事件协议。

## 7. 幂等与汇聚

- 端侧本地先写 outbox（append-only），再批量上传。
- 中央 `raw_events` 表以 `event_id` 为主键，`INSERT ... ON CONFLICT (event_id) DO NOTHING`。
- 上传失败指数退避重试，幂等保证重传无副作用。
- 云端实时订阅只做“状态刷新”，**不是唯一事实来源**；事实来源永远是 append-only 事件表。
