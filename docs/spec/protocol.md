# Spec: 事件协议

**权威来源**：`packages/protocol/SPEC.md`（机器可读类型：`packages/protocol/src/events.ts`）

本文件是面向开发者的上下文补充；改动协议时，以 `packages/protocol/SPEC.md` 为准，本文件同步更新。

---

## 为什么放在 `packages/` 而不是 `docs/`

`packages/protocol/` 是一个**可导入的 npm 包**，不只是文档：

- `app/src/` 通过 `import type { BehaviorEvent } from '@sisyphus/protocol'` 获得 TypeScript 类型
- `services/ingest/supabase/functions/` 同样可以导入，保证 Edge Function 与前端类型一致
- `SPEC.md` 与 TS 类型共同维护，强制同步

---

## 信封结构（摘要）

每条事件，无论层次，都共用同一个信封：

```
event_id       UUID，端侧生成，幂等主键
user_id        MVP 固定 "local-user"
device_id      如 "pixel-8", "macbook-1", "chrome-ext-1"
seq_no         单设备内单调递增
source         见下方枚举
layer          见下方枚举
type           该层的具体事件类型
time_mode      "point" | "interval"
event_time     RFC3339，point 事件
start_time     RFC3339，interval 事件
end_time       RFC3339，interval 事件，进行中可为 null
entity         主体：包名 / 域名 / 进程名
category       分类：如 "entertainment.video"
payload        该 type 专属字段，可为 {}
parent_event_ids  派生事件的来源 event_id 列表
privacy_level  L0 | L1 | L2 | L3
produced_at    RFC3339，端侧生成时刻
```

---

## `source` 枚举

| 值 | 含义 |
|---|---|
| `android_usage` | Android UsageStats 采集 |
| `android_accessibility` | Android 滚动检测（v1.1+）|
| `desktop_agent` | 桌面活动窗口/进程采集 |
| `browser_extension` | 浏览器插件 tab/url |
| `engine` | 规则引擎产出（finding/decision）|
| `manual` | 用户手动输入（目标、反馈）|
| `agent` | LLM Agent 产出 |

## `layer` 枚举

```
raw          原始采集，append-only
session      聚合后的行为片段
finding      规则命中的可干预机会
decision     策略选择记录（含倾向分 choice_probability）
intervention 对用户执行的提醒/对话
outcome      干预近端结果（10/30/60min 窗口）
feedback     用户显式反馈
```

## 隐私等级

| 等级 | 内容 | MVP 默认 |
|---|---|---|
| L0 | 时长、包名、域名、类别、滚动计数 | 始终采集 |
| L1 | 页面标题、用户目标文本、反馈按钮 | 始终采集 |
| L2 | 完整 URL、聊天内容 | 需用户授权 |
| L3 | 截图、OCR、输入内容 | 需用户授权 |

> 采集端严禁在未授权时产出高于授权等级的 payload 字段。

---

## 版本规则

- 加字段：向后兼容，不升大版本；消费端必须容忍未知字段
- 删字段 / 改语义：破坏性变更，升大版本，提供迁移说明
- 改协议时：同一次 commit 修改 `SPEC.md` + `packages/protocol/src/events.ts` + Rust 类型定义
