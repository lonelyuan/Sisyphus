# 安装

## 1. 构建 MCP server（一次）

```bash
cd sisyphus/src-tauri && cargo build --release -p sisyphus-mcp
# 产物：sisyphus/src-tauri/target/release/sisyphus-mcp
```

## 2. 注册进你自己的 Codex / Claude

**Codex**（`~/.codex/config.toml`，路径换成上面产物的绝对路径）：

```toml
[mcp_servers.sisyphus]
command = "/ABS/PATH/TO/sisyphus-mcp"
args = []
# 可选：覆盖 DB / 知识库路径（默认与桌面 App 同库：~/Library/Application Support/com.sisyphus/）
# env = { SISYPHUS_DB = "/path/sisyphus.db", SISYPHUS_VAULT = "/path/vault" }
```

Claude Code / 其它 MCP 客户端填等价的 stdio server 配置。重启后应能看到 `sisyphus` 的工具：
`capture` · `list_captures` · `propose_intents` · `accept_intent` · `ignore_intent` · `query_context` · `today_actions` · `set_goal` · `ingest_document` · `write_knowledge_note` · `search_knowledge` · `list_knowledge` · `list_monitored_apps` · `add_monitored_app` · `remove_monitored_app`。

把本 skill（`SKILL.md` + `references/`）放进你的 skills 目录即可作为一站式入口。

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
    <string>用 sisyphus 做今日规划：先 query_context，再结合未处理 capture 帮我定今天的最小目标并 set_goal。</string>
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

> 定时任务只是到点拉起 Codex 跑例程；真正读写仍走 `sisyphus` MCP，桌面 App 没开也能工作。

## 4.（可选）知识 agent 派发（第二大脑深研）

系统深研一个主题、批量写知识卡片：`services/knowledge-agent`（Node + `@openai/codex-sdk`）。

```bash
cd services/knowledge-agent && npm install     # 会一并装入 codex CLI
codex login                                     # 或设 CODEX_API_KEY
SISYPHUS_VAULT="$HOME/Library/Application Support/com.sisyphus/vault" node index.mjs "AI 安全"
```

它跑一个 Codex 线程做深研，并调 `sisyphus` MCP 的 `write_knowledge_note` 把概念卡片写进知识库（需第 2 步已注册 MCP）。桌面 App 也可经命令 `run_knowledge_agent(topic)` 派发（配 `SISYPHUS_KNOWLEDGE_AGENT_SCRIPT` 指向 index.mjs）。详见 `services/knowledge-agent/README.md`。
