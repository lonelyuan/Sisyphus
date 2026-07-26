# Sisyphus 文档导航

西西弗斯计划：多端行为采集 → 习惯引擎 → 智能干预。

项目背景见 [background.md](background.md)。**架构总纲见 [spec/architecture.md](spec/architecture.md)**（两平面 + 双存储 + SOC 心智模型，读 spec 前先读它）。

## 技术栈一览

| 层 | 技术 |
|---|---|
| 跨端框架 | Tauri v2（Rust + React/TypeScript） |
| 本地存储 | rusqlite（SQLite，Rust 管理） |
| 云端存储（同步，Phase 2）| Supabase（Postgres）|
| Android 特权 API | Kotlin Tauri 插件 |
| 反思平面（现阶段）| Codex / Claude Code，经 MCP 工具调用 |
| 反思平面（长期）| 迁移自研 Agent 基座 |

代码入口：`sisyphus/`（Tauri 项目）。

## 规格文档（spec/）

| 文档 | 内容 |
|---|---|
| [spec/architecture.md](spec/architecture.md) | **总体架构**：两平面、双存储、`ingest_event` 契约、MCP 工具面 |
| [spec/protocol.md](spec/protocol.md) | 事件信封 schema、枚举、隐私等级 |
| [spec/local-storage.md](spec/local-storage.md) | 本地 SQLite 表结构与访问模式 |
| [spec/sync.md](spec/sync.md) | outbox 上传协议（Phase 2）、Rust/TS 分工 |
| [spec/rule-engine.md](spec/rule-engine.md) | 规则引擎接口与 MVP 规则 |
| [spec/android-collection.md](spec/android-collection.md) | Android Kotlin 插件 API 与权限 |
| [spec/agent.md](spec/agent.md) | 反思平面：MCP 工具面与 Agent 边界 |
| [spec/notion-integration.md](spec/notion-integration.md) | LifeIndex：Notion 用户独占编辑、只读同步与多源上下文 |
| [spec/proactive-triggers.md](spec/proactive-triggers.md) | 调度器、主动推荐和宠物 / 通知投递 |

## 路线图

[roadmap.md](roadmap.md) — Phase 0–4 及可验证目标，「近期推荐下一步」给 A/B 双轨。

## 研究与设计参考

[research/](research/) — 早于当前架构、但仍有价值的研究与思考（SOC 类比来源、JITAI/BCT 证据、RL/bandit 理论、陪伴型产品参考等）。**不是当前设计**，以 spec/ 为准。

## 开发入门

```bash
cd sisyphus
pnpm install
pnpm tauri dev                     # Desktop
pnpm tauri android init            # 首次初始化 Android 工程
pnpm tauri android dev             # Android 开发
```

Android 前置：Android Studio、NDK、Rust Android targets（见 [spec/android-collection.md](spec/android-collection.md)）。
