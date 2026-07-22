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

所以 frontmatter 固定是 `title/tags/sources` 三项。`type`、`status` 放 `tags` 里；`domain` 由 `folder` 决定；`date`（news 用）写进正文。

## 五参怎么填

| 参数 | 填什么 |
|---|---|
| `folder` | 话题领域目录，**一律以 `kb/` 开头**（[taxonomy.md](taxonomy.md)）：`kb/web-security` / `kb/network-infra` / `kb/ai-redteam` / `kb/work-mihoyo/state` / `kb/work-mihoyo/best-practice` / `kb/personal` |
| `title` | 概念规范名，简洁可检索，同一概念只用一个 title |
| `tags` | 领域标签 + **type 标签**（必填其一）+ 可靠性标签（见 [types.md](types.md)）+ 视情况 `公司`。如 `[web-security, theory, 待确认]` |
| `links` | 2–5 个**已存在**的相关卡片 title（先 `list_knowledge` 确认存在）；跨 type 遵守 [types.md](types.md) 边界；至少 1 个 |
| `sources` | `sources/` 里的原文路径（先 `save_source` 得到）、或权威 URL、或"2026-Wxx 周会/口述" |
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

## save_source（原始材料）

值得原文留存时，先 `save_source(title, content, url, source_type)` → 落 `sources/{slug}-{hash6}.md`（frontmatter 自动含 url/saved_at/hash/`kind: source`）。再建 kb 卡片，在其 `sources` 里引用该原文相对路径。**原文不进 kb、不打 kb 领域文件夹。**

## 一个例子（theory + 引用原文）

先 `save_source("OWASP SQLi 备忘单", <原文>, "https://owasp.org/...", "article")` → 得 `sources/owasp-sqli-备忘单-54e13e.md`。再：

```
write_knowledge_note(
  folder="kb/web-security",
  title="SQL 注入",
  tags=["web-security","theory","待确认"],
  links=["输入校验","参数化查询"],
  sources=["sources/owasp-sqli-备忘单-54e13e.md"],
  body="> [!info] 一句话定义\n> 未过滤输入被拼进 SQL 语句执行。\n\n## 原理 / 机制\n...\n## 待确认\n- [ ] 我司 ORM 默认是否参数化？"
)
```
落成 `kb/web-security/sql-注入.md`，栏目=Web 安全，type=theory，关联两张卡，溯源指向原文。
