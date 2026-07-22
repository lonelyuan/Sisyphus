---
name: sisyphus-daily
description: 用 Sisyphus MCP 工具做原声笔记（capture→意图→artifact）与每日规划/复盘；并可一句话设成定时任务。当用户说“记一下…/做今日规划/晚间复盘/处理收件箱”或“把规划/复盘设成每天定时”时使用。
---

# Sisyphus 每日例程 + 原声笔记

本 skill 通过 Sisyphus 的 MCP server 驱动**原声笔记**与**每日规划/复盘**两类例程，并支持把它们注册为定时任务。

**工具面**：
`capture`（记一句话→事件日志，返回 event_id）· `list_captures`（收件箱，未处理的 capture）·
`propose_intents`（对一条 capture 生成意图候选，你负责分类）· `accept_intent` / `ignore_intent`（落库 / 忽略）·
`query_context`（今日目标/娱乐时长/未完成任务/到期提醒）· `today_actions`（1–3 条最小行动）· `set_goal`（今日目标）。

**分工铁律**：分类与意图提取是**你（Codex）**的活；MCP/Core 只做数据结构保存，不做推断。所有 artifact 只能经 `propose_intents`→`accept_intent` 落库（带来源/置信度/可回滚），不要绕过。

## 前置：安装 MCP server（一次）

1. 构建二进制：
   ```bash
   cd sisyphus/src-tauri && cargo build --release -p sisyphus-mcp
   # 产物：sisyphus/src-tauri/target/release/sisyphus-mcp
   ```
2. 在**你自己的** Codex 配置（`~/.codex/config.toml`）里注册（把路径换成上面产物的绝对路径）：
   ```toml
   [mcp_servers.sisyphus]
   command = "/ABS/PATH/TO/sisyphus-mcp"
   args = []
   # 可选：覆盖 DB 路径（默认 ~/Library/Application Support/com.sisyphus/sisyphus.db，与桌面 App 同库）
   # env = { SISYPHUS_DB = "/path/to/sisyphus.db" }
   ```
   Claude Code 等其他 MCP 客户端同理，填等价的 stdio server 配置。
3. 重启 Codex，确认能看到 `sisyphus` 的工具（`capture` / `list_captures` / `propose_intents` / `accept_intent` / `ignore_intent` / `query_context` / `today_actions` / `set_goal`）。

---

---

## 模式 C：原声笔记（capture → 意图 → artifact）

触发语示例：**“记一下：…”** / **“帮我把想法整理一下”** / **“处理收件箱”**

**A. 即时记录**：用户随口说一句（目标/待办/提醒/想法/情绪/材料），先调 `capture(text)` 原样收下，返回 `event_id`。**不要**急着追问细节——零压记录优先。

**B. 意图提取（你来分类）**：对一条 capture（新记的，或 `list_captures` 里的收件箱项），判断它是哪一类，调 `propose_intents(capture_event_id, candidates)`。分类准则：

| 判断 | kind | proposed 字段 |
|---|---|---|
| 今天要专注推进的一件事 | `goal` | `{text}` |
| 具体可执行的待办 | `task` | `{title, due_ms?, priority?, note?}` |
| 某时刻要提醒 | `reminder` | `{text, remind_at_ms(epoch ms), recurrence?}` |
| 想法/素材/偏好/情绪 | `note` | `{title?, body, tags?}`（情绪打 `tags:["mood"]`，素材打 `tags:["material"]`）|

- **只提最小候选**，一条 capture 通常 1 个候选，不要拆成任务海。
- 拿不准时间/优先级就**留空**，别编。`remind_at_ms` 必须是真实 epoch 毫秒。

**C. 确认落库**：向用户复述候选，一句话问「记成这样？」。
- 用户认可 → `accept_intent(intent_id)`。
- 用户改了措辞/时间 → `accept_intent(intent_id, edits={...})`（只传要改的字段）。
- 用户说不用了 → `ignore_intent(intent_id)`。

> 情绪/反馈类若只是倾诉，可只 `capture` 存底、不强行落 artifact。落库的都应是用户认可的、可被后续规划/复盘引用的东西。

---

## 模式 A：今日规划（morning-plan）

触发语示例：**“做今日规划”** / **“帮我定今天的目标”**

1. 调 `query_context` 读今日上下文（是否已有目标、昨日/今日行为)。
2. 若今日**已有**目标：复述它，问是否继续或修改。
3. 若**无**目标：结合最近 `capture` 的内容，提议 **1 个**今日最小行动（不要任务海）。
4. 用户确认后调 `set_goal` 落库。
5. 一句话收尾，说明今天专注这一件即可，完成就放松。

## 模式 B：晚间复盘（evening-review）

触发语示例：**“晚间复盘”** / **“今天怎么样”**

1. 调 `query_context` 读今日目标状态、娱乐时长、干预次数与近期干预。
2. 用**关心但不评判**的语气总结今天：目标推进了吗？娱乐是否超标？
3. 引导用户口述感受与遗留想法，逐条调 `capture` 记录（供明天规划引用）。
4. 不说教、不羞辱；只给一个明确的明日最小下一步建议。

> 语气准则见 `docs/spec/agent.md`：具体（引用真实时长/目标）、最小行动、不制造焦虑。

---

## 把例程设成定时任务

用户一句话即可，例如：**“把今日规划设成每天早 9 点，晚间复盘设成每天 21 点”**。
按用户的 Codex 运行方式二选一执行：

### 方式一：云端 Codex / ChatGPT 的 Tasks（若可用）
把「模式 A」「模式 B」的触发语分别创建为两个 scheduled task（9:00 / 21:00），
任务正文写：“运行 sisyphus-daily 的 morning-plan / evening-review 例程”。前提是该环境已连上 `sisyphus` MCP。

### 方式二：macOS 本地 launchd（可靠、离线，推荐本机自用）
为每个例程生成一个 LaunchAgent，到点用 `codex exec` 跑对应触发语。占位符需替换：
- `<CODEX_BIN>`：你的 codex 可执行路径（`which codex`；若为空说明未装 CLI，改用方式一）。
- 时间按需改 `Hour` / `Minute`。

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
    <string>运行 sisyphus-daily 的 morning-plan 例程：先 query_context，再帮我定今天的最小目标并 set_goal。</string>
  </array>
  <key>StartCalendarInterval</key>
  <dict><key>Hour</key><integer>9</integer><key>Minute</key><integer>0</integer></dict>
  <key>StandardOutPath</key><string>/tmp/sisyphus-morning.log</string>
  <key>StandardErrorPath</key><string>/tmp/sisyphus-morning.err</string>
</dict>
</plist>
```

复盘同理，另存 `com.sisyphus.evening-review.plist`（Hour=21，触发语换 evening-review）。加载：

```bash
launchctl load ~/Library/LaunchAgents/com.sisyphus.morning-plan.plist
launchctl load ~/Library/LaunchAgents/com.sisyphus.evening-review.plist
# 取消： launchctl unload <plist>
```

> 注意：定时任务只是到点**拉起 Codex 跑例程**；真正的读写仍走 `sisyphus` MCP 工具，
> 即使桌面 App 没开也能工作（MCP server 独立开同一个 SQLite）。
