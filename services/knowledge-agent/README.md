# sisyphus-knowledge-agent

Phase 1.3「第二大脑」的**派发链路**：App（Tauri/Rust）→ 本 Node 脚本 → **Codex TS SDK** →
围绕一个主题做深研，把概念卡片写进 Sisyphus 知识库（Obsidian 兼容 vault）。

这是反思平面的一个交付物：**基座用 Codex、与 App 解耦，但经 MCP/SDK 联动**。
App 只做数据结构保存（vault + `knowledge_notes` 索引 + 溯源事件），加工智能在 Codex。

## 数据流

```
App 命令 run_knowledge_agent(topic)
  → spawn: node index.mjs "<topic>"   (env SISYPHUS_VAULT 指向知识库)
    → @openai/codex-sdk: codex.startThread({workingDirectory: vault}).run(prompt)
      → Codex 深研 + 调 sisyphus MCP 的 write_knowledge_note
        → vault/*.md（人类可读投影，Obsidian 打开）
        + knowledge_notes 索引行（可查询真相）
        + Event log knowledge_ingested（溯源）
```

## 前置（真机）

1. **Node ≥ 18**。
2. `npm install`（会一并装入依赖 `@openai/codex` CLI —— SDK 底层 spawn 它）。
3. **codex 鉴权**：`codex login`（ChatGPT 登录）或设置 `CODEX_API_KEY`。
4. **注册 sisyphus MCP**（让 Codex 能落库）：把 `sisyphus-mcp` 加进你的 `~/.codex/config.toml`，
   见 [`skills/sisyphus-daily/SKILL.md`](../../skills/sisyphus-daily/SKILL.md)。未注册时脚本会退化为
   让 Codex 直接往 vault 目录写 `.md`（缺索引/溯源，仅应急）。

## 用法

```bash
# mock（零依赖，验证 App→Node 派发管道；不 spawn codex）
SISYPHUS_AGENT_DRYRUN=1 node index.mjs "AI 安全"
# → {"ok":true,"dryRun":true,"topic":"AI 安全","vault":"<cwd 或 $SISYPHUS_VAULT>"}

# 真机
SISYPHUS_VAULT="$HOME/Library/Application Support/com.sisyphus/vault" node index.mjs "AI 安全"
```

## App 如何调用

App 侧命令 `run_knowledge_agent(topic)` 读环境变量决定如何拉起本脚本，并注入 `SISYPHUS_VAULT`：

```bash
# 脚本绝对路径（作为单个参数传入，路径含空格也安全）
export SISYPHUS_KNOWLEDGE_AGENT_SCRIPT="/ABS/PATH/Sisyphus/services/knowledge-agent/index.mjs"
# 可选：node 可执行路径（默认 "node"）
export SISYPHUS_NODE_BIN="node"
```

命令等价于 `node <SCRIPT> "<topic>"`（env 注入 `SISYPHUS_VAULT`）。
（产品化后可改为 Tauri sidecar 打包，免去手工配置——见 roadmap Phase 2。）

## 环境变量

| 变量 | 作用 |
|---|---|
| `SISYPHUS_VAULT` | 知识库目录（Codex 的 workingDirectory）；App 会自动注入 |
| `SISYPHUS_AGENT_DRYRUN` | `=1` 走 mock，不导入 SDK、不启动 codex |
| `SISYPHUS_KNOWLEDGE_AGENT_SCRIPT` | App 侧：本脚本 index.mjs 的绝对路径 |
| `SISYPHUS_NODE_BIN` | App 侧：node 可执行（默认 `node`） |
| `CODEX_API_KEY` | codex 鉴权（或用 `codex login`） |
