# 路线图

本文档描述西西弗斯计划从个人实验项目到可推广产品的阶段性路线。

**架构总纲见 [spec/architecture.md](spec/architecture.md)**（两平面 + 双存储 + SOC 心智模型），本文件只讲阶段与验收。当前判断：不要先做"大而全的个人助理 App"，也不要先设计一套宏大的 Core 对象模型；应先给 SIEM **接上第一根真实的电线**，让感知平面与反思平面各自跑通最小闭环。

核心原则：

- 每个阶段都必须产生可验证闭环，而不是只完成孤立组件。
- 业务能力沉淀在数据层 + 引擎（Core），不写死在某个 Agent prompt、Codex skill 或 runtime session 中。
- Agent runtime 可以替换（现用 Codex/Claude Code，后期自研），数据模型、引擎、调度、反馈必须由项目自己掌控。
- **Core 是萃取出来的，不是提前设计的**：第二个采集源落地前，不新增 artifact 抽象。
- 技术选型服务长期复杂项目，早期可以用过渡方案，但要保留迁移边界，避免不可逆技术债。

---

## 进度快照（2026-07-30）

Phase 0 / 1.0 / 1.1 / 1.2 / 1.3 的核心闭环均已落地并在本机跑通（Tauri v2 + Rust `sisyphus-core` + rmcp MCP + React/Tailwind 深色 UI）。逐项状态见各阶段勾选框。

| 阶段 | 状态 | 一句话 |
|---|---|---|
| Phase 0 技术验证 | ✅（同步除外） | Tauri/SQLite/协议/`ingest_event`/MCP 全通；**schema 迁移机制已补**（`user_version`）；Supabase 同步延后至 Phase 2 |
| 1.0 Core MVP | ✅ | capture→propose_intents→accept_intent→artifact；意图种类扩到 6 类（含 life_item / rule，都进候选桥）|
| 1.1 拖延干预 | ✅ 闭环 + ✅ outcome | macOS 采集器 + Android UsageStats + 动态规则 + 通知 + 反馈 + **近端结果观察**；LLM 文案待做 |
| 1.2 原声笔记 | ✅ | 收件箱 → 分类 → accept/edit/ignore 落 artifact |
| 1.3 第二大脑 | ✅ 核心 + ✅ 约束层 | Obsidian vault + 索引 + **写入器约束 + kb_doctor 体检 + 红链队列 + 结晶化归并** |

**当前主线（2026-07-30 调整）**：把三个模块的**不变量从 prompt 搬进代码**，并为技能树 / 无极时间线打好存储底座。见下方「2026-07-30 全面评审与整改」。

> **kill-criteria 已废弃（2026-07-20 用户决定）**：原以"连续 5 天通知未改变行为→证明无效"当 kill 门。用户判断该门逻辑上无法证伪（永远可归因于"实现不够好"），既不能证伪 idea、反而拖着不敢往下做。故不再当验证门槛，改当**持续打磨的实现质量问题**。"主动提醒能改变行为"作为项目公理，实现形态（宠物/系统通知/遮罩/对话）持续迭代。
>
> 但**近端结果观察不是 kill 门，是学习信号**——已于 2026-07-30 补上（见下）。没有它，阈值只能靠感觉调，Phase 2.1 的可学习策略也没有 label 可学。

---

## 2026-07-30 全面评审与整改

一次覆盖文档 / 代码 / 真实数据的评审。**共同结论：三个模块的规则本身基本正确，但全部只写在 prompt 与 spec 里，没有一条能被拒绝、被测量、被反馈。** 整改的主线因此不是加新架构，而是把约束搬进写入路径，并补上能算指标的体检工具。

### 评审时实测到的问题（全部有数据）

**知识库（对真实 vault 的 98 个 md 做图分析）**

| 问题 | 实测 | 根因 |
|---|---|---|
| wikilink 断链 | **63/354 = 18%** | `vault::slugify` 把标题里的空格换成 `-`（`AD CS` → `ad-cs.md`），而 `## 关联` 按**标题**渲染 `[[AD CS]]`。Obsidian 按**文件名**解析，不看 frontmatter 的 `title` → 任何含空格/标点的标题，其入链必然点不开。57/63 都是这一个原因 |
| 同一张卡两份 | `certighost` 在 `kb/…` 与 `…`（漏 `kb/` 前缀）各一份，内容已分叉 | `write_knowledge_note` 幂等键是 **path** 而不是 title；`folder` 无任何校验，MCP 工具描述还写着"省略则落 vault 根"，与 SKILL.md 的"一律 kb/ 开头"直接冲突 |
| 查重查不到 | — | `search_knowledge` 只 LIKE 标题/标签/路径，**不搜正文**。SKILL.md 要求"按主题查重"在工具层根本做不到 → 只能新建 → 碎片化 |
| 读不回来 | — | MCP **没有读卡工具**，而 types.md 要求"更新前必须先读回现有卡"。整卡覆盖 = 静默丢内容 |
| 标签缺失 | 缺 type 5 张、缺可靠性 8 张、1 张写成字面 `#待确认` | 无校验 |
| 目录超阈值 | identity-system 22 张、network-infra 18 张（阈值 12） | 无体检工具，agent 无法在上下文里可靠统计图结构 |
| 孤儿卡 | 入度为 0 共 6 张 | 同上 |
| 删卡留断链 | `[[游泳知识库]]` 等 | `delete_note` 不改入链，也无重定向机制 |

**反拖延**

- **通知风暴（最高优先）**：冷却读 `interventions.shown_at`，但只有 `Immediate` 策略写这张表；`Deferred`/`Debounce` 只入队。于是这两种策略下冷却永远 ready，采集器每 5–15s 就重新入队一条——一条 10 分钟延迟的规则会攒出几十条通知同时炸出来。而 skill 与工具描述都把 `deferred` 当可用选项推荐。
- `Debounce` 的 `window_ms` 被整个丢弃（`window_ms: _`），去重只靠"队列里还有 pending"，派发完就又能入队 → 退化成每 30 秒响一次。
- `RuleEngine::evaluate` 返回**第一个**命中且内置规则永远排在前面 → 用户精心建的规则可能永远不触发。
- `interventions.outcome` 列建表就在，**从未回填**。
- 长时间停在同一个 app 时**一条事件都不写**（只在切换前台时才落盘）——看两小时电影 = Event log 空白。

**人生看板**

- 同一件"要做的事"存在三张表：`tasks`（accept_intent 写、today_actions 读）、`life_items`（看板读写）、`lifeindex_cards`（遗留）。同步只有 `tasks → life_items` 单向且不完整。用户可感知的后果：助手记下的待办要等 App 重启才出现在看板；在看板里做完一件事，第二天早上仍会被提醒做它。
- 缺 GTD 的**责任领域**（Horizon 3）一层，于是 `track=main|side` 只能一条条手标——这是"手动维护不丝滑"的主要来源。
- `goal` 无可判定完成条件 → 永远无法收敛；`review_at_ms` 字段在但无人读写 → idea 没有毕业机制。
- `today_actions` 是"目标 + 前 3 个未完成任务（按手填 priority）"，与"永远知道下一步"的承诺不匹配，且不可解释。
- Notion 整页 replace 是唯一不可逆的一步；`life_sync_runs` 一直在存写回前全文，但**没有任何读取入口**。

**地基**

- **无 schema 迁移机制**：只有 `CREATE TABLE IF NOT EXISTS`，加列不生效 → 下次给已有表加字段，所有已装机的库会报 `no such column`。
- **时区口径分裂**："今天"用 UTC 日期算，而 `time_of_day` / `daily@HH:MM` 用本地时区 → UTC+8 用户的日界落在**早上 8 点**。
- **并发静默丢事件**：`seq_no = MAX+1` + `UNIQUE(device_id,seq_no)` + `INSERT OR IGNORE`，两个同 device_id 的写入者并发时后一条被吞掉，却照样返回一个库里不存在的 event_id（Pi 与 Codex 的 MCP 都用 `agent-mcp`）。
- `ingest_event` **不校验**任何东西（架构文档写着"校验信封与 privacy_level"，实现是纯透传）；`category` 列混用了行为分类与 capture 种类两套命名空间。
- 文档漂移与重复：`knowledge-engine-design.md` 与 `references/knownengine-design.md` 是同一份 1758 行（仅差 8 行 banner）；`agent.md` 的 LifeIndex 模式仍写 `upsert_lifeindex_card`；`rule-engine.md` 写的规则引擎路径已过时。

### 已整改（本轮全部落地，测试 36 → 71 通过）

**地基**
- [x] `core::migrations`：`PRAGMA user_version` + 幂等迁移数组（补列 / 重建 `life_items` 放宽 kind / 旧表一次性导入 / 知识卡标题去重 + 部分唯一索引 / 播种默认领域），含"老库形状 → 迁移 → 数据完好"测试。
- [x] `core::clock`：全系统唯一的"一天"定义（本地时区 + 可配置换日点，默认 0 点，可设 4 点适合夜猫子）。`context` / `collector` / `commands` / `android_jni` 全部改走它。
- [x] `core::settings`：Core 行为开关的 KV（换日点等）；凭据仍不进 SQLite。
- [x] `ingest_event` 变成闸门：枚举白名单 + interval 必须有 start/end + end≥start + privacy_level 校验，脏事件拒绝入库。素材改用独立 `type=material_text`，不再污染 `category` 命名空间。
- [x] `seq_no` 改 `device_seq` 计数器表单语句原子自增；`insert_behavior_event` 若没插进去且 event_id 不在库里则**报错**而不是假装成功。

**反拖延**
- [x] 新增 `rule_fires` 事实表：**命中当拍就记**，与"通知有没有真的弹"无关。冷却与 debounce 窗口只看它 → 通知风暴消除；`Suppress` 也记痕（推进冷却）。
- [x] `Debounce.window_ms` 真的生效（`db::debounced_recently`）。
- [x] `RuleEngine::evaluate` 改为跑全部规则、按 severity 取最高，同级时**动态规则优先**于内置。
- [x] 延后派发的文案改为投递前重算（不含"此刻已刷 N 分钟"这类会过期的数字）。
- [x] **近端结果观察闭环**：干预弹出即入队 `observe_outcome`（+10min / +30min）→ ticker 到点算窗口内娱乐占比 → 回填 `outcome`（still_entertainment / mixed / switched / unknown，没观测就是 unknown，不编）→ `intervention::outcome_stats` 给"转移率"。MCP/Tauri 都能读。
- [x] 采集器每 5 分钟落一个闭合切片，长时间停在同一 app 不再是 Event log 空白（Event log 仍是 append-only，不改历史事件）。

**知识工程（约束层）**
- [x] 文件名 = 标题（只替换文件系统非法字符），不再 slugify；`aliases` 进 frontmatter 做重定向。**已对真实 vault 执行迁移：断链 18% → 1.1%**（剩 4 条是真正还不存在的卡，现在作为红链进调研队列）。迁移前后各有 git 快照。
- [x] 幂等键改为 **title**；同标题换 folder = 移动而不是复制；`knowledge_notes.title` 存活行部分唯一索引。重复的 `certighost` 已合并（保留内容更全的版本）。
- [x] 写入校验：`folder` 必须 `kb/` 开头、标题长度上限、tags 恰好一个类型 + 一个可靠性档、`links` 至少一条（MOC 豁免）、**`已复现`/`已验证` 必须有 sources**（"模型自身知识只配待确认"从提示变成约束）。
- [x] `search_knowledge` 搜正文并返回片段（碎片化的机械原因消除）。
- [x] `read_knowledge_note` + `append_section`（补充式增长成为原子操作）+ `merge_notes`（留别名、改入链、删碎卡，不留断链）。
- [x] `kb_doctor`：断链/断链率、孤儿、无出链、重复、缺标签、无据高档、领域拆并建议、同前缀碎卡簇、MOC 目录漂移、散落文件、未被引用的原文。`rebalance`/`defragment` 从"靠感觉扫"变成"跑 lint → 修 top N"。
- [x] `kb_wanted` 红链队列 = **主动调研的输入**（三场景闭环：①留红链 → ③按热度深研 → ①补齐）。
- [x] `reindex_vault`：vault 的 `.md` 是本体、索引是可重建投影；启动时追平，吸收用户在 Obsidian 里的手改/移动/删除。
- [x] vault 自动 `git init` + 每次写入后快照：整卡覆盖从"不可逆"变成"可 diff 可回滚"，顺带得到版本历史。

**人生看板 / LifeDB**
- [x] `life_areas`（GTD Horizon 3 责任领域，播种 6 个默认领域）+ `life_items.area_id`。
- [x] `success_criteria` / `target_value` / `current_value` / `unit`：完成可判定，进度可算。
- [x] `tasks` 收敛：`accept_intent(kind=task)` 直接写 `life_items`（kind=action），`query_context.open_items` / `today_actions` 全部改读 LifeDB。看板与今日视图第一次一致。
- [x] `lifetree::next_actions`：确定性 + **每条带理由**（逾期 → 已排在当下 → 临近截止 → 主线/重点领域下最浅的未完成 → 今日日常 → 候选 → 待安排兜底，不因过滤丢失）。
- [x] `lifetree::review_queue`：周回顾要问的问题由 SQL/Rust 算好（到期审查 / 停滞 / 未拆解 / 缺完成条件 / 滞留 inbox），agent 只负责问与措辞；每天 20:10 的 job 只在周日发问。
- [x] `list_sync_runs` + `sync_run_remote_before`：Notion 整页替换终于有了恢复入口。

**无极时间线**
- [x] `time_rollups` 预聚合桶（day/week/month）+ `rollup_state` 水位增量重建 + 读路径自愈追平。年尺度代价从 O(事件数) 降为 O(可见桶数)。跨午夜会话按逻辑日拆分，周/月桶由日桶再聚合（三尺度口径一致）。
- [x] 事件显著性等级 `lod_level`：粗尺度按**重要性**保留图层，不再"按表顺序截断"。
- [x] 补上缺失的**长期计划图层**：LifeDB 的目标/项目/技能跨度 + 里程碑点，进度用技能树同一套算法（`has_long_term_source` 不再硬编码 false）。

**知识工程二轮（同日追加，起因：用户反馈"sources 一大坨没人看"+"图谱里全是 index 占位符"）**
- [x] **原始材料从物理隔离改为元信息隔离**：`sources/` 目录取消，原文**就地存放在它讲的那个话题的
  文件夹里**；隔离靠 `type: source` + **单向连接**（卡片/枢纽 → 原文，原文自己不出链，工具层会拒绝有出链的 source）
  + `publish: false`（博客按此排除，**不能再按路径排除**）。质量门槛：外部原文必须有 url，
  目录/索引类快照直接拒收。
- [x] 真实 vault 迁移：9 份高价值原文按引用关系/内容归位话题夹；**删 11 份 KM 副本**
  （3 份目录快照共 2220 行纯链接 + 6 份可直接开 URL 的单篇 + README），引用侧改为直接引用 KM 链接
  ——这正是项目自己"第一方成品只链接不复制"规则要求的；根目录散落文件也归位。
- [x] **图谱能看出层级**（Obsidian 图谱**不渲染文件夹节点**，节点标签用文件名）：
  9 个 `index.md` 改名为领域名（`网络与基础设施`/`Web 安全`…）；新增 `hub` **角色**标记
  （`moc` 只是类型，一个领域可以有多张概念地图卡——早先按"路径最短的 moc"猜，把自动目录写进了
  「安全基建」）；父子枢纽互链成脊椎；`refresh_mocs` **确定性生成**子领域/卡片/原始材料清单
  （marker 之外的领域叙述原样保留）；`graph.json` 配 9 组按 `path`/`tag` 上色 + 打开箭头。
- [x] 修 `extract_wikilinks`：跳过围栏代码块与行内代码——文档里写 `` `[[链接]]` `` 讲语法
  不再被误判成永远补不上的假红链。
- [x] 迁移后体检：**断链 1%（3 条真缺口）· 孤儿 0 · 目录漂移 0 · 散落文件 0 · 无人引用的原文 0
  · 违反单向的原文 0 · 缺类型 0 · 缺可靠性 0 · 无据高档 0**。

**文档**
- [x] 删掉重复的 1758 行设计文档；修正 `agent.md` / `rule-engine.md` / `local-storage.md` / `notion-integration.md` / `proactive-triggers.md` / `architecture.md` 的漂移；更新 skills 契约与 MCP instructions。

### 尚未做（明确留白，不假装完成）

- [ ] **技能树与无极时间线的前端**：后端数据（`life_tree` / `bands` / `plans` / `level`）已就绪且有类型定义，画布渲染是下一步的 UI 工作。
- [ ] **意图分类 golden set**：30–50 条固定用例（句子 → 期望 kind + 字段），换基座/改 prompt 后跑一遍看漂移。目前换 Pi ↔ Codex 是盲飞。
- [ ] LLM 生成干预文案（现为确定性模板 + 真实数据引用）。
- [ ] 浏览器插件（桌面最大信号缺口：真正的刷视频在浏览器内）。
- [ ] Notion 改为只替换 marker 区块（现在是整页替换，会打乱用户排版）。
- [ ] 知识库 FTS5（现为 LIKE 正文检索；个人库规模够用，不引入新依赖）。
- [ ] **领域拆分**：`network-infra` 16 张、`identity-system` 21 张都超过 12 的阈值（体检已报 split）。
  拆法需要语义判断（如 identity-system 可拆 证书/域/SSO-MFA 三支），留给一次 rebalance 例程。
- [ ] **碎卡簇 8 组**待评估（`证书基础/证书注册/证书映射加固`、`ESC/ESC1/ESC1 环境边界` 等）——
  过双筛后该并的并，需要读内容判断，不宜机械合并。
- [ ] 接 Quartz 时验证：枢纽不再叫 `index.md`，栏目落地页要么靠自动生成、要么显式配置。
- [ ] `move_note` 工具（现由 `write_knowledge_note` 换 folder 时自动移动覆盖）。

---


## 总体架构心智模型

西西弗斯从两个维度组织。

### 横向软件组件

| 组件 | 职责 | 备注 |
|---|---|---|
| 事件与数据层 | 保存行为事件、用户输入、任务、知识、记忆、反馈 | 本地 SQLite + Supabase/Postgres 过渡；后期自建服务端 |
| 状态机层 | 管理意图、任务、提醒、干预、知识节点的合法状态转换 | 避免 loose JSON 失控 |
| Agent 基座层 | LLM 调用、工具调用、上下文构建、流式输出、多 Agent 编排 | 可先用 Codex/ChatGPT skill dogfood，后续评估 Pi runtime / 自研 runtime |
| 策略与调度层 | 今日行动选择、提醒时机、冷却、反馈学习 | 先规则，后模型 |
| 知识处理层 | 文档摄取、摘要、概念抽取、关系合并、检索、剪枝 | 不把原始材料简单堆进知识库 |
| UI 层 | 今日页、收件箱、时间轴、知识视图、设置和权限 | UI 是数据投影，不是核心状态来源 |
| 同步与权限层 | 多端同步、隐私等级、授权、导出、删除 | MVP 单用户，推广阶段必须重做多用户安全边界 |

### 纵向业务模块

| 模块 | 目标 | 核心闭环 |
|---|---|---|
| 总入口 · InputBox + 意图识别（原 1.2 原声笔记） | 无压记录 + 功能路由 | 任意输入 → `capture` → 意图识别 → 路由到下面三模块 |
| 1.1 西西弗斯计划（狭义） | 低效习惯与拖延的数字干预 | 行为采集 → 风险识别 → Agent 干预 → 用户反馈 → **近端结果** |
| LifeIndex · 人生看板 | 低摩擦记录 + 严格结构化展示 + **心智系统**（技能树 / 无极时间线）| Notion 自由文本 ↔ Agent 三方语义合并 ↔ SQLite LifeDB → App 视图（见 [spec/notion-integration.md](spec/notion-integration.md)、[spec/lifeindex-mind-system.md](spec/lifeindex-mind-system.md)）|
| 1.3 第二大脑 | 和人一同进化的知识工程 | 原始材料/目标 → 学习加工 → 知识库(vault)/知识图谱 → 可读输出/教学 |

### 统一入口

用户不应理解内部模块。但要注意：**"统一"发生在数据层，不发生在管道**。系统有两条不同形状的管道，见 [spec/architecture.md](spec/architecture.md)：

```text
感知管道（实时·确定性）：
  行为事件 → ingest_event → Event log → 规则引擎 → finding → 本地干预

反思管道（人类节奏·自然语言）：
  一句话 / 语音 / 文件 / 链接
    → capture（写 Event log）
    → propose_intents / accept_intent
    → Artifact（目标 / 任务 / 提醒 / 知识 / 复盘）
    → 用户确认或安全自动执行
```

二者共享 **Event log（信封统一）**，但行为事件不产生"意图候选"、笔记不进规则引擎——不要用一条 `capture→intent` 管道强行兜住行为事件。

因此，所有业务模块共享这些核心服务（Rust 数据层 + MCP 工具面，见 [spec/architecture.md](spec/architecture.md) §3–4）：

- `capture(text | file | url | event)`：接住原始输入。
- `propose_intents(capture_id)`：生成意图候选。
- `accept_intent(intent_id)`：将候选意图转为正式对象。
- `create_artifact(...)`：创建目标、任务、记忆、知识节点、提醒等。
- `query_context(scope)`：为 Agent 构建最小必要上下文。
- `select_today_actions()`：选择今日最小行动。
- `schedule_reminder(...)`：安排固定或条件提醒。
- `record_feedback(...)`：记录用户反馈。
- `record_outcome(...)`：记录提醒后的近端结果。
- `ingest_document(...)`：处理材料并进入知识系统。

---

## Phase 0 — 技术验证阶段

**目标**：验证核心技术选型可行，建立可扩展、可迭代的工程底座，避免在长期大型复杂项目中留下不可迁移的技术债。

当前技术判断：

- 跨端 App：选择 Tauri 技术栈，复用 Rust 本地能力与 Web UI 生态。
- 本地存储：SQLite 作为端侧事实缓存、outbox 和离线规则查询基础。
- 服务端过渡方案：Supabase/Postgres 用于数据同步、事实汇聚、Realtime/command queue 和早期 pgvector/RAG 实验。
- 长期服务端：后期迁移到自建服务端，承载多用户、重计算、模型训练、复杂权限和调度。
- Agent 运行时：短期可用 Codex/ChatGPT skill、CLI、脚本进行 dogfood；中期评估 Pi agent runtime / SDK 作为可嵌入 Agent 基座；Sisyphus Core 保持运行时无关。

验收标准：

- [x] Tauri Desktop / Android 基础工程可运行，能调用 Rust command。
- [x] 本地 SQLite schema 初始化、读写、迁移机制可验证。（`CREATE TABLE IF NOT EXISTS` 增量演进）
- [x] append-only `raw_events` + outbox 模式可跑通。
- [ ] Supabase/Postgres 能接收批量事件，保持幂等写入。（**延后 Phase 2**：outbox 已排队，未接上传）
- [x] 端侧事件协议、服务端 schema、TypeScript 类型保持一致。（`SPEC.md` ↔ `events.ts`；Supabase schema 待同步时校准）
- [x] 基础隐私等级模型存在：L0/L1 默认，L2/L3 明确授权。（`privacy_level` 字段，采集只产 L0–L1）
- [x] Agent 调用与业务逻辑解耦：Agent 只能通过 MCP 调用 Sisyphus Core。

里程碑：

- 完成“本地输入 → 本地落盘 → 可选同步 → Agent 查询上下文”的最小链路。
- 明确哪些代码属于过渡适配层，哪些属于长期 Core。

---

## Phase 1 — 模块验证阶段

**目标**：按业务模块分别开发和验证关键 idea，避免一开始构建完整产品外壳。阶段结束时，每个模块都应有一个可自用的纵向闭环。

### Phase 1.0 — Sisyphus Core MVP

所有业务模块先共享一个最小 Core。

核心对象：

- `capture_items`：原始输入收件箱。
- `intent_candidates`：Agent 生成的意图候选。
- `artifacts`：目标、项目、任务、笔记、记忆、知识节点、提醒等统一对象。
- `artifact_relations`：对象间关系，支持树状 UI 和图状语义。
- `events` / `raw_events`：行为事件与系统事件。
- `reminders` / `interventions`：提醒和干预。
- `feedback_events` / `outcomes`：用户反馈和近端结果。

> **实现校准**：最终未造多态大表 `artifacts`，改为**每种对象各自建表**（`daily_goals`/`tasks`/`notes`/`reminders`/`knowledge_notes`）；capture 不单列 `capture_items`，而是 Event log 里的 `note_text` 事件；`artifact_relations` 未提前造（1.3 知识关系用 vault `[[wikilink]]`）。见 [spec/architecture.md](spec/architecture.md) §2。

验收标准：

- [x] `capture(text)` 能保存任意自然语言输入。
- [x] `propose_intents(capture_id)` 能输出结构化候选意图。（Codex 生成、MCP 持久化）
- [x] `accept_intent(intent_id)` 能创建或更新 artifact。（含 `edits` 修改、`ignore` 回滚）
- [x] `select_today_actions()` 能选出 1–3 个今日最小行动。（`today_actions`：目标 + 未完成任务）
- [x] 所有 AI 推断有来源、置信度和可回滚状态。（`intent_candidates`：capture_event_id/confidence/status）
- [x] Core 暴露 SDK 接口，供 Codex skill / 自研 UI 调用。（rmcp MCP + App 命令 + Codex TS SDK 派发）

### Phase 1.1 — 西西弗斯计划（狭义）：拖延与低效习惯干预

**目标**：验证数字干预闭环是否有效。重点关注用户行为记录和同步、习惯模型的联合建模、行为检测、干预反馈闭环，以及具有情感价值的智能体提醒。

核心闭环：

```text
跨端行为采集
  → 统一事件库
  → 规则/风险识别
  → 策略选择干预动作
  → Agent 生成提醒
  → 端侧执行
  → 用户反馈和近端结果
```

MVP 范围：

- Android Usage Stats 采集前台 app。
- Desktop 活动窗口/浏览器插件作为后续扩展。
- 娱乐/信息流规则：目标未完成 + 娱乐会话超过阈值 + 冷却满足。
- 通知干预：开始任务、合理休息、继续娱乐、放弃今日。
- 今日最小行动与行为数据联动。

验收标准：

- [x] 手机端刷 B 站/抖音超过阈值时，能基于今日目标触发提醒。（Android UsageStats→JNI→规则→Kotlin 通知，后台可弹；⏳ 真机连续验证中）
- [x] 用户点击反馈后写入本地数据库和 outbox。（通知按钮→`record_feedback`；事件入 outbox）
- [ ] 系统能观察提醒后 10/30/60 分钟近端结果。（**未做**：`interventions.outcome` 列已留，回填逻辑待写）
- [~] Agent 能生成不羞辱、不说教、引用实际上下文的提醒文案。（现为**确定性模板**：引用真实时长+目标、不羞辱；LLM 生成待做）
- [~] 规则和策略分离：规则只识别机会，策略决定是否提醒和如何提醒。（规则识别✓；策略层仅冷却，contextual bandit 属 Phase 2.1）

### Phase 1.2 — 原声笔记：无压记录的个人助手

**目标**：验证“零压力记录 + 意图取代 TODO”的产品假设。用户可以输入任意时间、琐碎、非结构化的内容，系统从中提取意图，并自动整理为日程、习惯、提醒、素材、知识或今日行动。

核心闭环：

```text
散乱输入
  → 本地 Capture 待处理队列（不是 Notion Inbox）
  → 意图提取
  → 候选对象
  → 用户确认/安全自动落盘
  → 今日行动或未来提醒
```

典型输入：

- “我想学吉他。”
- “最近想增强社交能力，避免孤独。”
- “周末提醒我约朋友吃饭。”
- “这篇 AI Infra 文章之后要看。”
- “我今天什么都不想干，只想刷视频。”

验收标准：

- [x] 输入一句自然语言后，系统能判断是目标、任务、提醒、材料、偏好、情绪还是反馈。（Codex 分类为 goal/task/reminder/note；情绪→打标 note，材料→知识库，反馈→事件）
- [x] 系统只提出最小下一步，不生成任务海。（`SKILL.md` 明确「只提最小候选」）
- [x] 用户可一键接受、修改、忽略。（`accept_intent` / `accept_intent(edits)` / `ignore_intent`）
- [x] 今日页只展示 1–3 个最小行动。（`today_actions` 上限 3）
- [x] 已接受意图能被提醒、复盘和后续对话引用。（`query_context` 含未完成任务 + 到期提醒）

### Phase 1.3 — 第二大脑：和人一同进化的知识工程

**目标**：验证“材料不是堆放，而是学习加工和内化”的知识系统。给定目标领域或原始材料引用，系统可使用 deep research 等工具自主扩展资料，并将可验证知识总结为知识库/知识图谱，保持人类高度可读。

核心闭环：

```text
目标领域 / 原始材料
  → 来源保存
  → 摘要和概念抽取
  → 关联已有知识节点
  → 待确认关系
  → 可读知识卡片 / 博客式输出 / 图谱视图
  → 定期剪枝和更新
```

MVP 范围：

- 原始材料保存来源和 hash。
- 文档 chunking、摘要、概念提取。
- `knowledge_node` 与 `artifact_relations` 构成图谱底座。
- 使用 Markdown/博客式长文作为人类可读输出。
- 低置信度合并进入待确认队列，不污染知识库。

验收标准：

- [x] 输入一篇文章或链接，系统能生成 5 行以内摘要和 3–10 个概念节点。（Codex + `write_knowledge_note`；已真机写出「游泳知识库」5 张卡片）
- [~] 系统能判断材料与已有节点的关系。（关系靠 Codex 写 `[[wikilink]]`；无自动关系判定/合并/共享根节点推断）
- [~] 用户能纠正分类和关系。（`.md` 可直接在 Obsidian 编辑；App 内知识列表暂只读）
- [x] 知识节点能被后续对话、今日行动、学习计划引用。（`search_knowledge` / `list_knowledge` + query）
- [ ] 系统能标记过期、重复、低价值节点，进入剪枝候选。（**未做**：`status` 列已留 stale/duplicate/pruned，判定逻辑待写）

---

## Phase 2 — Feature 完善阶段

**目标**：在坚实的 Core、状态机、数据层和模块闭环上，开发预期特色功能。此阶段才开始追求“产品形状”。

### Phase 2.1 — 西西弗斯计划：跨端同步与联合建模

- [ ] 行为采集扩展到 Desktop、浏览器、更多 Android 信号。
- [ ] 支持更多 IoT / 可穿戴设备，例如睡眠、运动、久坐、心率等 proxy。
- [ ] 多端事件聚合为跨端 session。
- [ ] 从固定规则升级为可学习策略：风险模型、contextual bandit、离线策略评估。
- [ ] 使用更复杂 AI 模型学习用户行为，输出更精准、更高成功率的干预措施。
- [ ] 建立策略回放和评估系统，避免提醒疲劳和误报伤害用户体验。

### Phase 2.2 — 原声笔记：进化为数字员工

- [ ] 从“记录和提醒”升级为“处理现实工作”。
- [ ] 接入邮件、日历、文档、浏览器、文件系统等用户授权视野。
- [ ] 支持把自然语言意图转为多步骤工作流。
- [ ] 支持主动跟进：等待结果、检查状态、提醒用户决策。
- [ ] 支持 computer use / browser use，但必须受权限、确认和审计约束。
- [ ] 建立长期用户模型：偏好、工作节奏、社交关系、常用流程。

### Phase 2.3 — 第二大脑：自主学习、自主探索、自主验证

- [ ] 支持围绕一个领域自动制定学习路线。
- [ ] 接入 deep research、论文/网页/视频/书籍等多源材料。
- [ ] 自动生成知识图谱、学习地图、博客式综述和问答卡片。
- [ ] 对关键知识进行来源追踪、交叉验证和置信度评估。
- [ ] 周期性更新知识节点，淘汰过时内容。
- [ ] 不只保存知识，还要能“教会用户”：生成课程、练习、测验和复盘。

---

## Phase 3 — 推广阶段

**目标**：在个人自用验证充分后，考虑将项目推广为创业项目，面向多用户提供服务。

关键任务：

- [ ] 从单用户本地优先架构迁移到多用户服务架构。
- [ ] 自建服务端替换 Supabase 过渡能力，或将 Supabase 限定为明确边界内的基础设施。
- [ ] 重新设计认证、授权、RLS/租户隔离、密钥管理和审计。
- [ ] 建立数据导出、删除、隐私授权、模型调用透明度和合规策略。
- [ ] 明确商业定位：个人效率工具、AI 助理、知识管理、行为干预，还是垂直人群解决方案。
- [ ] 建立可观测系统：事件处理延迟、提醒效果、用户留存、干预厌烦率、知识复用率。
- [ ] 设计付费模型和成本控制：LLM 调用、向量检索、后台任务、存储、同步、推理成本。

阶段性判断标准：

- 个人自用场景连续数周稳定产生价值。
- 核心闭环能被非开发者用户理解和使用。
- 数据安全边界清楚。
- Agent 自动行为有审计、撤销和用户控制。
- 产品不是 Notion/Todo/Calendar 的简单替代，而是能证明“低摩擦意图系统”带来新增价值。

---

## 近期推荐下一步

三条轨道——A 反思平面、B 感知平面（macOS 采集器）、跨端安卓（UsageStats→JNI 后台闭环）——都已跑通。当前主线是把 **LifeDB 作为人生规划事实层**，让 App 严格展示、Notion 自由编辑，并由受限 Agent 自动双向合并。

### 当前冲刺：以可用性为目标推进（分批，逐批验证 + 更新本节）

目标：让 **Pi / Codex 两个基座都能从 app 内真正驱动**西西弗斯全部功能（反拖延含动态规则、第二大脑、人生看板），并富化时间轴。计划见 `.claude/plans/`。

- **批次 A ✅ 已落地（本次）** — 地基：
  - 解除 in-app 智能体的硬编码只读：`agent_runtime::RunMode{Interactive,Proactive}`；主对话/宠物=可写（可 `set_goal`/`add_monitored_app`/建规则/写知识），定时/规则触发=严格只读。外部源（Notion）恒只读。
  - 修编译期路径依赖：`agent_runtime::init_paths` 用运行期 `resource_dir()` 注入 skills/pi-runner/mcp 路径；release 不再烘焙 `CARGO_MANIFEST_DIR`（退回 `current_exe`）；`tauri.conf.json` 打包 `skills/`、`scripts/`。
  - 修调度器阻塞：`agent_run` 移到 worker 线程，不再卡住 30s ticker 里的 `notify`。
  - 立 `ResponsePolicy` seam（core `rule_engine`：Immediate/Deferred/Debounce/Suppress）：命中即时→notify，延后/防打扰→入队 `scheduled_actions`；补 `pet_message` 派发分支（emit `pet-message`，`Pet.tsx` 已监听）。
- **批次 B ✅ 已落地（本次）** — Pi 基座接 MCP 工具面：`scripts/pi-agent-runtime.mjs` spawn `sisyphus-mcp`（stdio），`listTools` 后用 `Type.Unsafe(inputSchema)` 逐个包成 Pi `customTool`；系统提示按 `SISYPHUS_READ_ONLY` 切交互/只读。新增依赖 `@modelcontextprotocol/sdk`、`@sinclair/typebox`。桥已冒烟验证（连上→17 工具→set_goal 写→query_context 读回）。Pi 与 Codex 现共用同一工具契约。
- **批次 C ✅ 已落地（本次）** — 动态规则引擎（“一句话建规则”）：`detection_rules` 表 + 声明式 `core::rules::DynamicRule`（category_prefix/category_in/app_in + window/threshold + requires_active_goal + time_of_day，跨午夜）；`RuleEngine::evaluate` 每次热加载启用规则。MCP 工具 `create/list/set_enabled/delete_detection_rule`（可写门禁）+ 同名 Tauri 命令 + Settings 规则列表（查看/启停/删）+ skill `references/rules.md`。core 单测 3/3、MCP 建/列/只读拒写均已冒烟验证。
- **批次 D ✅ 已落地（本次）** — 人生看板 LifeIndex：`lifeindex_cards` 表（(section,title) 幂等 upsert，可重建投影）+ MCP `upsert_lifeindex_card`/`list_lifeindex`/`delete_lifeindex_card` + Tauri `list_lifeindex` + 「看板」tab（分区卡片、Notion 溯源链接）。MCP 写门禁细化为三档 `write_scope`（只读 / 仅看板 / 全写）；新增 `RunMode::LifeIndex` + 每日 8:30 `agent_run(mode=lifeindex_refresh)` job：agent 只读参考 Notion + 本地上下文后仅写本地看板，绝不回写 Notion。已冒烟验证（仅看板可写、set_goal 被拒、list 正常）。
- **批次 E ✅ 已落地（本次）** — 时间轴富化：`query_timeline` 新增 artifact 里程碑图层（目标/任务/提醒/知识卡片/规则创建，点事件）+ note_text 重标为 capture 图层；`TimelineScreen` 按 kind 配色、里程碑画独立小圆标记、详情面板显中文类型标签。所有尺度都展示稀疏里程碑。
- **批次 F ✅ 已落地（本次）** — 清理死代码（删 `TodayScreen`/`RecordsScreen` + `list_sessions`/`list_recent_sessions`/`SessionRow`）；回写 spec（rule-engine 动态规则、proactive-triggers §4/§7 状态、architecture §9、local-storage 表清单、agent 运行模式）；更新项目记忆（proactive_triggers / pi_agent_inapp）。
- **批次 G ✅ 已落地（同日追加）** — 排查"智能体没权限"发现两个根因并修复：① `pi-agent-runtime.mjs` 的 `DefaultResourceLoader` 会自动加载项目/全局 `.pi/extensions/pi-permission-system`（给交互终端 UI 写的扩展），在我们的 headless 子进程里对每次工具调用返回"需要审批但无 UI"而拒绝——加 `noExtensions: true` 跳过；② SDK 文档说 `noTools:"builtin"` 会保留 custom tools，但实现里未提供 `tools` 允许名单时不会激活它们——改为显式 `tools: sisyphusTools.map(t=>t.name)`。顺带接入 Notion 只读集成：官方 `@notionhq/notion-mcp-server`，Pi 侧多开一个 MCP client 合并工具面、Codex 侧用 `-c mcp_servers.notion.*` 注入，token 存 `notion_config.json`（0600）+ Settings 新卡片；只读边界由 Notion 侧 integration 权限（仅 "Read content"）机制保证。`AgentScreen` 会话删除失效（`window.confirm` 在 Tauri webview 里常返回 false）一并修复。详见 `docs/spec/notion-integration.md` §8"现状"小节。
- **批次 H ✅ 已落地（2026-07-27）** — LifeDB / LifeItem / LifeIndex：SQLite 新增统一人生规划对象、关系、外部引用和三方合并快照；旧 tasks/LifeIndex 卡片幂等迁入。MCP/Tauri 增加 LifeItem CRUD、关系、投影和同步完成 API；Pi/Codex 新增 `LifeIndexSync` 白名单模式。Notion 改为固定单页受限网关，只暴露整页 Markdown 读/替换；每日 8:30 入站、本地修改后即时出站、App 手动同步。看板 UI 重构为事项/日常/主线/支线四个重叠视图 + 待整理，并可完整编辑字段。详见 `docs/spec/notion-integration.md`。

### 主线：LifeIndex 从「看板」升级为「心智系统」

心智模型见 [spec/notion-integration.md](spec/notion-integration.md)（**LifeDB 是事实源，App 是严格视图，Notion 是自由文本交互层，Agent 是语义编译器**）与 [spec/lifeindex-mind-system.md](spec/lifeindex-mind-system.md)（技能树 / 无极时间线的底座与理论依据）。

1. **已完成：LifeDB 数据模型**：七种 kind（+skill/milestone）+ area + track + horizon + 可判定完成条件与度量 + 邻接关系。
2. **已完成：受限双向同步**：三方快照、乐观 revision、固定单页网关、失败审计、并发保护、**恢复入口**。
3. **已完成：确定性心智服务**：`life_tree`（技能树 + 进度）、`next_actions`（带理由的下一步）、`review_queue`（周回顾要问什么）。
4. **已完成：无极时间线底座**：预聚合桶 + 显著性分层 + 长期计划图层。
5. **下一步：两个组件的前端**——技能树画布（节点 + 前置边 + 进度环）与无极时间线渲染（条带 + 计划跨度 + LOD 标记）。后端数据与类型都已就绪。
6. **再下一步：同步实测**：用真实 LifeIndex 页面覆盖首次导入、并发编辑、删除冲突、Notion 限流和子页面保护。

### 技能树 · 无极时间线（**规划中的重要组件，不是暂缓项**）

两者都是心智系统的一部分，靠底座支撑而非前端糖。存储底座已在 2026-07-30 落地，设计与不变量见 [spec/lifeindex-mind-system.md](spec/lifeindex-mind-system.md)。

**技能树**：目标 → 自动生成里程碑 → 与日程联动。
- 已有底座：`kind=skill|milestone`；`contains` 边表达等级/阶段分解，`depends_on` 边表达前置能力（DAG）；`success_criteria` + `target_value`/`current_value`/`unit` 让"完成"可判定；`lifetree::forest/subtree` 给出**Core 确定性算出的进度**（叶子看状态或度量，内部节点等权平均），进度绝不由 agent 估。
- 待做：① 前端画布；② "目标 → 里程碑"的生成例程（agent 产出候选 milestone，走 `life_item` 意图候选桥，用户确认后落库——保持可回滚）；③ 里程碑到期进 `next_actions` 与提醒队列（已通过 `due_at_ms` 天然联动）。

**无极时间线**：无限伸缩，不同尺度显示不同抽象层次，给用户对时间利用率的掌控感。
- 已有底座：`time_rollups`（day/week/month 预聚合 + 水位增量重建 + 读路径自愈）让缩放代价与事件量解耦；`lod_level` 让粗尺度按**重要性**保留图层；`plans` 图层把长期计划画进时间轴；`bands` 给出每桶的专注/娱乐/中性拆分与主导分类。
- 待做：① 画布渲染（条带 + 计划跨度 + 里程碑标记 + 连续 zoom 手势）；② 时间利用率视图（周/月对比、目标达成与娱乐占比的关系）；③ app 维度条带（数据已在 rollup 的 `dimension='app'`）。

### 让闭环真正有用的小补丁（按性价比排序，都不需新架构）

1. ~~近端结果 outcome 观察~~ ✅ 已落地（2026-07-30）。
2. **浏览器插件**（桌面最大信号缺口）：`packages/browser-extension` 已有骨架。桌面真正的刷视频在浏览器内、原生 app 白名单抓不到。接一个 tab/URL → `ingest_event(url_visit)` 的最小插件。
3. **LLM 生成干预文案**：现为 Rust 确定性模板（引用真实时长与目标）。让反思平面在冷却窗口预生成几条个性化文案缓存，命中时取用——引擎实时触发 + Agent 措辞温度。
4. **意图分类 golden set**：30–50 条固定用例存进仓库，换基座/改 prompt 后跑一遍。成本一个下午，换掉"盲飞"。

### 之后（Phase 2，明确延后）

Supabase 同步 / 跨端联合分析、可学习策略（contextual bandit / 离线策略评估 —— 现在 outcome 已在产生 label，条件第一次具备）、自研 Agent 基座、多用户安全边界、知识库 FTS5 与向量检索、CTF 沙箱验证器。

### 对拖延症开发者的推进纪律（不变）

- 里程碑按"天"记，每步当天能自用、拿到多巴胺。
- 每个里程碑限时，超预估 2 倍还没通就砍需求，不许加东西。
- 明确**延后不碰**：Supabase 同步、可学习策略、自研基座、多用户——全部等主线闭环稳定再说。
