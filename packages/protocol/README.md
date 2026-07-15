# @sisyphus/protocol

跨端事件协议 v1.0。**[SPEC.md](./SPEC.md) 是唯一权威定义。**

## 内容

| 文件 | 用途 |
|---|---|
| `SPEC.md` | 权威规范：信封字段、layer 分层、type/payload、隐私等级、版本策略 |
| `src/events.ts` | TypeScript 类型（浏览器插件、桌面前端、Edge Functions 用） |
| `schema/behavior-event.schema.json` | JSON Schema，用于校验与文档 |

## 跨语言一致性约定

MVP **不引入 codegen**。各端手写等价定义，纪律是“改 SPEC 必同提交改各端”：

- **TypeScript**：本包 `src/events.ts`（浏览器插件 / 桌面前端 / Edge Functions 复用）。
- **Rust**：`sisyphus/src-tauri/crates/core`（`ingest::NewEvent` + `raw_events` schema 与信封 1:1）——数据层唯一事实来源。
- **Kotlin**：不再定义完整信封。安卓采集器（`sisyphus/src-tauri/gen/android/...`）只把原始信号（前台包名、时长等）经 Tauri 插件桥给 Rust，由 Rust `ingest_event` 组装成 `BehaviorEvent`。

任何端新增 `type` 或 `payload` 字段，先改 `SPEC.md` 登记，再落地各端（见本仓库 `note_text` 的加法）。

## 核心设计（速记）

- **统一信封 + `layer` 分层 + `parent_event_ids` 血缘**：容纳多层次、时间重合的事件，不压平。
- **`time_mode`**：点事件（触控、finding）vs 区间事件（前台、会话）。
- **无损 raw + 决策带 `choice_probability`**：保住未来 RL/离线策略评估的可重算性。
- **高频信号端侧预聚合**（如 `scroll_burst`），不逐条上报。
