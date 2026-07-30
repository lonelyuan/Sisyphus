# 博客导出（Quartz）

把 `kb/` 一键发布成人类高度可读的静态博客站：文件夹→栏目、tag→标签页、`[[wikilink]]`→超链、自带关系图谱/反链/全文搜索。**只发 `kb/`，绝不发 `sources/` 和 `work-mihoyo/`（公司，仅本地）。**

## 一次性搭建

```bash
# 1. 拉 Quartz v4 到任意目录（独立于 vault，不污染知识库）
git clone https://github.com/jackyzha0/quartz.git ~/mihoyosec-blog
cd ~/mihoyosec-blog && npm i

# 2. 让 Quartz 的 content 指向知识图谱 kb/（sources/ 是 kb 的兄弟目录，天然被排除）
rm -rf content && ln -s /Users/chenzhongyuan/Obsidian/mihoyosec/kb content
```

## 关键配置（`quartz.config.ts`）

- **排除公司内容**（硬保险，防止误发）：在 `configuration.ignorePatterns` 里加 `"work-mihoyo"`、`"work-mihoyo/**"`（连同默认的 `private`、`.obsidian`）。
- `baseUrl` 改成你的部署域名（GitHub Pages / Cloudflare 等）。
- 栏目 = `kb/` 下的文件夹（web-security / network-infra / ai-redteam / personal）；每个文件夹的 `index.md` 是栏目落地页。
- 标签页 = 卡片 frontmatter 的 `tags`（type 标签 `theory/news/...`、状态标签都会成为可浏览标签）。
- 关系图谱 / 反链 = Quartz 默认开启，直接吃 `[[wikilink]]`。

## 日常导出

```bash
cd ~/mihoyosec-blog
npx quartz build --serve      # 本地预览 http://localhost:8080
npx quartz build              # 产出静态站到 public/，再部署
```

因为 content 是指向 `kb/` 的软链，Obsidian 里更新知识 → 重新 build 即同步到站点。

## 发布安全（务必核对）

- 构建后**检查 `public/` 里没有 `work-mihoyo/` 任何内容**（现状/最佳实践讨论都可能含公司信息）。
- `sources/` 原文本就在 `kb/` 之外，不会被发；即便手滑也不该出现在 `public/`。
- 只有通用知识（`theory` / 通用 `best-practice` / 非涉密 `ai-redteam` 研究）适合对外；涉司内容永远留本地。

## 备选

- **Hugo**：想要更"正式博客"的主题生态时用，但 `[[wikilink]]`/图谱需插件适配、导出要一层转换。
- **极简自建**：写个脚本把 `kb/*.md` 按 folder/tag 转成静态 HTML——最可控，但图谱/搜索要自己实现。
当前选定 Quartz（最贴合"链接化知识花园 + 零改写发布 Obsidian vault"）。

## 排除规则（2026-07-30 起改为按元信息）

⚠️ **不能再按路径排除**：原始材料已经不住在 `sources/` 了，而是**就地放在 `kb/` 的话题文件夹里**
（见 [../SKILL.md](../SKILL.md) 数据模型）。按 `sources/` 排除会一条都排不掉，把他人版权原文发出去。

排除靠两个元信息，两者都由写入器自动打上：

| 要排除的 | 标记 | 来源 |
|---|---|---|
| 逐字原文（他人版权） | frontmatter `publish: false` + tag `source` | `save_source` 自动写入 |
| 公司内容 | 路径 `kb/work-mihoyo/**` + tag `公司` | 分类学约定 |

Quartz 侧配置要点：
- 开启 `explicit`/frontmatter 过滤，只发布 `publish` 不为 `false` 的页面；
- 再按 tag `source` 与路径 `kb/work-mihoyo` 双重排除（冗余是故意的：漏一层还有另一层）；
- 发布前用 `kb_doctor` 确认没有"公司卡被非公司卡引用"造成的意外泄漏路径。

领域落地页：枢纽笔记**不叫 `index.md`**（它们改成了有意义的领域名，见 SKILL.md），
因此接 Quartz 时要么让它自动生成文件夹页，要么显式配置枢纽为栏目首页。这一步在真正接博客时验证。
