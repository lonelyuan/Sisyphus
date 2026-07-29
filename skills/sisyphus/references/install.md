# 安装

## 1. 构建 MCP server（一次）

```bash
cd sisyphus/src-tauri && cargo build --release -p sisyphus-mcp
# 产物：sisyphus/src-tauri/target/release/sisyphus-mcp
```

## 2. 注册进 Codex，并按项目限制工具面

**Codex**（`~/.codex/config.toml`，路径换成上面产物的绝对路径）：

```toml
[mcp_servers.sisyphus]
command = "/ABS/PATH/TO/sisyphus-mcp"
args = []
enabled = true
# 可选：覆盖 DB / 知识库路径（默认与桌面 App 同库：~/Library/Application Support/com.sisyphus/）
# env = { SISYPHUS_DB = "/path/sisyphus.db", SISYPHUS_VAULT = "/path/vault" }
```

在 Sisyphus 仓库的 `.codex/config.toml` 用 `enabled_tools` 只开放 Core/意图工具，例如 `capture`、`list_captures`、`propose_intents`、`accept_intent`、`ignore_intent`、`query_context`、`today_actions`、`set_goal`、监控和检测规则工具；不要开放 `ingest_document`、`write_knowledge_note`、`search_knowledge`、`list_knowledge` 等知识库工具。

Claude Code / 其它 MCP 客户端填等价的 stdio server 配置。

把本 skill 通过仓库级 `.agents/skills/sisyphus` 暴露，只在 Sisyphus 项目使用；不要链接到用户级 skills 目录。

## 3.（可选）每日例程设成定时任务

**macOS 本地 launchd**（可靠、离线）。为每个例程生成一个 LaunchAgent，到点用 `codex exec` 跑对应触发语。占位符：`<CODEX_BIN>`=`which codex`。

`~/Library/LaunchAgents/com.sisyphus.morning-plan.plist`：

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>com.sisyphus.morning-plan</string>
  <key>ProgramArguments</key>
  <array>
    <string><CODEX_BIN></string>
    <string>exec</string>
    <string>用 sisyphus 做今日规划：先 query_context 和 list_life_items，再结合未处理 capture，只推荐一个最小目标并 set_goal；普通规划模式不要写 Notion。</string>
  </array>
  <key>StartCalendarInterval</key>
  <dict><key>Hour</key><integer>9</integer><key>Minute</key><integer>0</integer></dict>
  <key>StandardOutPath</key><string>/tmp/sisyphus-morning.log</string>
  <key>StandardErrorPath</key><string>/tmp/sisyphus-morning.err</string>
</dict>
</plist>
```

复盘同理（另存 `com.sisyphus.evening-review.plist`，Hour=21，触发语换晚间复盘）。加载：

```bash
launchctl load ~/Library/LaunchAgents/com.sisyphus.morning-plan.plist
launchctl load ~/Library/LaunchAgents/com.sisyphus.evening-review.plist
```

> App 自带的每日 8:30 `LifeIndexSync` 负责 Notion 双向同步，不需要 launchd。上面的规划/复盘任务保持只读；只有隔离的同步模式会拿到固定单页替换工具。

## 4.（可选）知识 agent 派发（第二大脑深研）

系统深研一个主题、批量写知识卡片：`services/knowledge-agent`（Node + `@openai/codex-sdk`）。

```bash
cd services/knowledge-agent && npm install     # 会一并装入 codex CLI
codex login                                     # 或设 CODEX_API_KEY
SISYPHUS_VAULT="$HOME/Library/Application Support/com.sisyphus/vault" node index.mjs "AI 安全"
```

它跑一个 Codex 线程做深研，并调 `sisyphus` MCP 的 `write_knowledge_note` 把概念卡片写进知识库（需第 2 步已注册 MCP）。桌面 App 也可经命令 `run_knowledge_agent(topic)` 派发（配 `SISYPHUS_KNOWLEDGE_AGENT_SCRIPT` 指向 index.mjs）。详见 `services/knowledge-agent/README.md`。
