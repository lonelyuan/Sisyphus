# 规定格式：卡片 frontmatter / 分类载体 / 分 type 正文模板

所有 kb 卡片经 `write_knowledge_note` 落盘。**分类由两处承载，不是靠自由 frontmatter：**
- **话题领域** → `write_knowledge_note` 的 `folder`（= 目录，= 博客栏目）。
- **文章类型 + 可靠性** → `tags`（`theory|news|state|best-practice|personal` + 可靠性阶梯 `待确认|多源印证|已复现|已验证|stale`，见 [types.md](types.md#每种-type-的-frontmatter-标签)）。

## Core 自动生成的骨架（别重复写）

`write_knowledge_note(folder, title, body, tags, links, sources)` 渲染出：

```markdown
---
title: "<title>"
tags: [<tags>]
sources: [<sources>]
---

# <title>          ← 自动，body 别再写
<body>             ← 你只写下面的正文小节
## 关联             ← 自动（来自 links），body 别再写
- [[<link>]]
```

所以 frontmatter 固定是 `title/tags/sources`（+ 有别名时的 `aliases`）。`type`、`status` 放 `tags` 里；`domain` 由 `folder` 决定；`date`（news 用）写进正文。

**文件名 = 标题原样**（只替换 `/ \ : * ? " < > |`）。Obsidian 按文件名解析 `[[链接]]`，
所以标题就是链接名——不要再假设有 slug 转换。`aliases` 由 `merge_knowledge_notes` 自动维护，一般不用手填。

## 五参怎么填

| 参数 | 填什么 |
|---|---|
| `folder` | 话题领域目录，**必填且一律以 `kb/` 开头**（工具层校验，不合规直接拒绝）：`kb/web-security` / `kb/network-infra` / `kb/network-infra/identity-system` / `kb/ai-redteam` / `kb/work-mihoyo/state` / `kb/work-mihoyo/best-practice` / `kb/personal` |
| `title` | **短主题规范名**：默认 2–8 个汉字或 ≤5 个英文词；不写公司名、type、日期、状态、`现状/解析/知识体系/核心输出目录` 等可由上下文表达的词。标准术语本身较长时除外 |
| `tags` | 领域标签 + **type 标签**（必填其一）+ 可靠性标签（见 [types.md](types.md)）+ 视情况 `公司`。如 `[web-security, theory, 待确认]` |
| `links` | 2–5 个**已存在且关系明确**的卡片 title（先 `list_knowledge` 确认存在）；能说明父子/依赖/对照/实例/gap，禁止用泛化链接凑数；跨 type 遵守 [types.md](types.md) 边界 |
| `sources` | **只列外部证据**：原始材料卡的路径（如 `kb/personal/拖延症书籍摘要.md`）、权威 URL、他人文档/会议出处。这是**溯源字段**，不要写 `[[wikilink]]`——图谱边靠正文的 `## 关联` 与领域枢纽提供 |
| `body` | 按 type 选下面模板；不含 `# 标题`、不含 `## 关联` |

## 分 type 正文模板

> 正文一律用 **H2 小节**组织（不写流水账）：一颗结晶靠小节增生长大，每轮对话加/精一节。粒度决策（归并/新建/拆分）见 [crystallization.md](crystallization.md)。

**theory（补充式、重结构，便于逐节增补）**
```markdown
> [!info] 一句话定义
> <一眼想起它是什么>

## 原理 / 机制
## 关键点 / 分类
## 例子 / 复现
## 我的理解 / 与工作的联系
## 待确认
- [ ] <需实践证实的点>
```

**news（带日期的快照）**
```markdown
> 日期：2026-07-21　来源：<url/报告名>

## 事件 / 漏洞摘要
## 复现 / 分析
## 影响面
## 关联理论
- 参见 [[相关 theory 卡]]
```

**state（我司现状，带日期+来源，仅本地）**
```markdown
> 现状截至：2026-07-21　来源：<会议/内部wiki/口述>

## 现状描述
## 与理想的差距
- 对照 [[某企业最佳实现]]（gap，不合并）
## 待确认
- [ ] <该问谁 / 查哪个文档 / 要哪个权限>
```

**best-practice（行业理想，公司无关）**
```markdown
> [!info] 标准做法一句话

## 标准 / 推荐做法
## 为什么（原理链到 theory）
## 权衡 / 适用条件
```

**personal（自由，隔离）**：不套模板，正常写；不强行链技术卡。

**moc（领域枢纽）**：`tags` 含 `moc` 与 `hub`（每领域恰好一个）。你只写「领域范围」与「后续可拆」，
`<!-- kb:auto begin/end -->` 之间的子领域/卡片/原始材料清单由 `refresh_mocs` 生成，别手写。

**source（原始材料）**：不用 `write_knowledge_note`，用 `save_source`。它**不需要**可靠性档位
（可靠性衡量"说法有多可信"，原文的可信度就是它的来源），但**必须有溯源**，且**不能有出链**。

## 来源分流：第一方链接 / 外部原文

先判断作者与维护权，再决定是否 `save_source`：

| 材料 | 动作 |
|---|---|
| 用户本人持续维护、已精心整理的 KM/文档 | 视为**第一方成品/事实源**：从最相关的已有 MOC 或卡片直接链接；不复制到 `sources/`，不再精炼保存，不另建同义摘要卡 |
| 重要外部文章/报告，未来需要逐字溯源或担心失链 | `save_source(folder, title, content, url, source_type)` —— **folder 必填**，就地放进它讲的那个话题的文件夹；kb 卡在 `sources` 引用其路径，并在 `links` 里加它（卡片 → 原文的单向边）|
| 普通外部链接、可替代资料、一次性材料 | 直接引用 URL 或不落库；不为“来过”而保存 |
| 用户本人确认/口述 | 不建原始材料卡，也不写入 `sources` 字段；需要时在正文标“来源：本人确认”。是否建卡仍走归并与双筛 |
| 他人文档/会议结论 | 作为外部 provenance 写入 `sources` 字段或正文日期；仅在确需逐字保留时 `save_source` |

`save_source` 落 `{folder}/{标题}.md`，frontmatter 自动含 `tags: [source, 原始材料]` / `source_type` /
`url` / `saved_at` / **`publish: false`**，且**不渲染 `## 关联`**（原文在图谱里是叶子，只被指向）。

**原文就在话题夹里，和卡片相邻**——`sources/` 目录已取消。隔离靠元信息，不靠路径。

## 一个例子（theory + 引用原文）

先 `save_source(folder="kb/web-security", title="OWASP SQLi 备忘单", content=<原文>, url="https://owasp.org/...", source_type="article")`
→ 得 `kb/web-security/OWASP SQLi 备忘单.md`（就在同一个话题夹里）。再：

```
write_knowledge_note(
  folder="kb/web-security",
  title="SQL 注入",
  tags=["web-security","theory","待确认"],
  links=["输入校验","参数化查询","OWASP SQLi 备忘单"],  # 最后一条 = 卡片→原文的单向边
  sources=["kb/web-security/OWASP SQLi 备忘单.md"],
  body="> [!info] 一句话定义\n> 未过滤输入被拼进 SQL 语句执行。\n\n## 原理 / 机制\n...\n## 待确认\n- [ ] 我司 ORM 默认是否参数化？"
)
```
落成 `kb/web-security/sql-注入.md`，栏目=Web 安全，type=theory，关联两张卡，溯源指向原文。
