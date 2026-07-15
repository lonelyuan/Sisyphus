# Sisyphus

跨端、低摩擦、可自我迭代的个人反拖延助理。**架构总纲见 [`docs/spec/architecture.md`](docs/spec/architecture.md)**，背景见 [`docs/background.md`](docs/background.md)。

> **心智模型:一个 SOC。** 采集源(手机 / 桌面 / 浏览器 / 手动输入)→ 归一化事件(`BehaviorEvent`)→ 习惯引擎 → 干预。
>
> **两个平面:**
> - **感知 / 响应平面** —— 常驻、确定性、实时,跑在 Tauri App 里(采集 → 规则引擎 → 本地通知)。
> - **反思平面** —— 人类节奏、自然语言,现阶段用 Codex / Claude 经 MCP 工具驱动(记录 / 规划 / 复盘 / 知识)。
>
> 两个平面共享同一数据层:append-only **Event log** + 可变 **Artifact store**。唯一写入契约 `ingest_event`。

## 仓库结构（顶层按“消费者 / 构建方式”划分）

```
Sisyphus/
  sisyphus/    Tauri 客户端 = Rust workspace(app + core + mcp)+ React WebView UI（桌面 / 安卓）
  packages/    给「代码」用的共享库:protocol（事件协议唯一权威）、browser-extension（采集源，延后）
  services/    「部署运行」的服务端:ingest（Supabase，跨端同步 = Phase 2）
  skills/      交付给「Agent 运行时」(Codex/Claude) 的技能包:sisyphus-daily
  docs/        规格（docs/spec/）与路线图
```

> 目录层面的 monorepo,不强求统一构建;顶层按消费者分,不是按“是否交付物”（app、扩展、skill 都是交付物）。详见 [`docs/spec/architecture.md`](docs/spec/architecture.md) §6。

## 快速开始

```bash
# 客户端（感知平面）
cd sisyphus && pnpm install && pnpm tauri dev

# 反思平面 MCP server（交付物：构建后注册进你自己的 Codex/Claude）
cd sisyphus/src-tauri && cargo build --release -p sisyphus-mcp
```

- 客户端开发入门:[`docs/README.md`](docs/README.md)
- MCP 安装与每日例程:[`skills/sisyphus-daily/SKILL.md`](skills/sisyphus-daily/SKILL.md)

## 文档

| 文档 | 内容 |
|---|---|
| [docs/spec/architecture.md](docs/spec/architecture.md) | **架构总纲**:两平面、双存储、`ingest_event`、MCP 工具面、目录映射 |
| [docs/roadmap.md](docs/roadmap.md) | 阶段路线与近期下一步（A/B 双轨） |
| [docs/spec/](docs/spec/) | 各专项规格:protocol / local-storage / rule-engine / agent / sync / android-collection |
| [docs/background.md](docs/background.md) | 项目缘起 |

## 前置工具链

- Node + **pnpm**（客户端前端）
- Rust / Cargo（Tauri 后端、core、mcp）:`curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`
- Android:Android Studio + NDK + `rustup target add`（见 [docs/spec/android-collection.md](docs/spec/android-collection.md)）
- Supabase CLI（仅 Phase 2 同步需要）:`brew install supabase/tap/supabase`
