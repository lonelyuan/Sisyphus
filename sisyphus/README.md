# Sisyphus 客户端（Tauri）

Sisyphus 的跨端客户端（桌面 / 安卓）：Rust workspace（app + `sisyphus-core` + `sisyphus-mcp`）+ React WebView UI。
架构见 [`../docs/spec/architecture.md`](../docs/spec/architecture.md)，事件协议见 [`../packages/protocol/SPEC.md`](../packages/protocol/SPEC.md)。

## 开发

```bash
pnpm install
pnpm tauri dev                 # 桌面（含 macOS 前台采集器）
pnpm tauri android init        # 首次初始化 Android 工程
pnpm tauri android dev         # 安卓
```

## Rust workspace（`src-tauri/`）

| 成员 | crate | 职责 |
|---|---|---|
| `src-tauri/`（根）| App（`[lib] name = sisyphus_lib`）| Tauri 桌面/安卓外壳 + macOS 前台采集器 + Tauri 命令。**lib 名勿改**（安卓 `System.loadLibrary("sisyphus_lib")` 绑定它）。 |
| `crates/core` | `sisyphus-core`（rlib）| **唯一事实来源**:db + `ingest_event` + 查询 + 规则引擎 + 分类 |
| `crates/mcp` | `sisyphus-mcp`（bin）| 反思平面 rmcp stdio server，复用 core |

> 约束:tokio / rmcp 等只放 `crates/mcp`，**绝不进 core**，否则拖垮安卓构建（core 只放跨端能编的依赖）。

- 前端在 `src/`（今日页等，React）。
- Android 特权采集用 Kotlin 插件桥接（`src-tauri/gen/android/`），见 [`../docs/spec/android-collection.md`](../docs/spec/android-collection.md)。
