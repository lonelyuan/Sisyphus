# 文章类型：五种材料的不同生命周期与链接规则

用户明确要求：文章类型确实有区别，不能用一套规则处理。`type` 是 frontmatter 字段 + 同名标签，与话题领域（folder，见 [taxonomy.md](taxonomy.md)）正交。**一张卡恰好一个 type。**

> 前置：无论哪个 type，卡的粒度都是**结晶主题**而非对话——同主题的多个侧面是**同一颗结晶的不同 H2 小节**，不是多张卡。先读 [crystallization.md](crystallization.md) 的归并/双筛/拆分规则，再看这里各 type 的生命周期。

## 速查

| type | 是什么 | 迭代方式 | 时效 | 链接规则 |
|---|---|---|---|---|
| `theory` | 理论/教科书：漏洞原理、协议、方法论 | **补充式**（见下） | 长期有效 | 与 theory / best-practice 自由互链 |
| `news` | 安全新闻：漏洞报告、复现解析、事件 | 快照，基本不改 | 会过时，标 `#stale` | 链到相关 theory；可派生 theory |
| `state` | **我司现状**：miHoYo 实际怎么做的 | 现状更新（带日期留痕） | 随公司变 | 仅本地；与 best-practice 建 gap 链，**不合并** |
| `best-practice` | **企业最佳实现**：行业理想/标准 | 补充式 | 长期 | 链 theory；**不掺我司细节** |
| `personal` | 个人/非技术 | 随意 | — | **不强行**链技术卡 |

## theory —— 补充式增长（重点）

用户原话："基本上是 append only（以补充为主，不是只在文章末尾新增）"。理解为：

- 迭代 = **加法**：补充新小节、细化已有小节、补例子/图/链接。
- **绝不删除或改写已验证内容**；纠错也是"标注更正 + 保留原说法脉络"，而非抹掉。
- 因为 `write_knowledge_note` 是整卡覆盖（按标题幂等），所以更新前**必须先读回现有卡**（用户可能手改过），在其上做加法式合并，再写**超集**回去。
- 准确性优先：拿不准的进「待确认」区、标 `#待确认`；被实践证实后升 `#已验证`。
- 正文用结构化小节（见 [format.md](format.md)），便于逐节补充而非"末尾追加流水账"。

## news —— 带日期的快照

- frontmatter 必带 `date`；标题可含日期或版本。
- 记录当时的事实与复现步骤，**不追改**（它是历史快照）。
- 过时（如漏洞已修复、方法失效）→ 加 `#stale`，不删。
- 当多条 news 归纳出通用规律 → **派生**一张 `theory` 卡（news 链向它），而不是把 news 改成 theory。

## state vs best-practice —— 硬隔离（一个现状、一个理想）

用户原话："区分我司现状说明 以及 企业级最佳实现，一个是理论一个是现状，不应混合"。

- `state`（`folder=work-mihoyo/state`）：**我司实际**。每条带**日期 + 来源**（会议/内部 wiki/口述），`#公司`，仅本地。
- `best-practice`（`folder=work-mihoyo/best-practice`）：**行业理想/标准**，公司无关，可对外。
- **绝不写进同一张卡。** 两者的差距用 **gap 链接**表达：在 state 卡的「差距」区写 `与理想差距见 [[某最佳实现]]`，反之亦然。这样"我司现在怎样、业界应该怎样、差在哪"三件事清清楚楚，又不互相污染。
- 这也保护了发布安全：博客只导出通用知识，`work-mihoyo/`（含 state 与公司化的 best-practice 讨论）不外发。

## personal —— 隔离子图

- `folder=personal`，非技术内容（兴趣、生活、随想）。
- **不强行**与技术卡建 `links`；让它自成一个子图，避免污染技术知识图谱的关系网。
- 技术相关的即使是个人笔记，也归到对应技术领域，不放这里。
- **同样按结晶归并**：一个个人主题（如"雅思备考""游泳计划"）多轮对话 → 长同一颗卡的小节，别每次新开。personal 最容易犯"对话即卡"的碎片病（见 [crystallization.md](crystallization.md) 雅思案例）。

## 每种 type 的 frontmatter 标签

除领域标签外，卡片 `tags` 必含：
- type 标签：`theory` / `news` / `state` / `best-practice` / `personal`
- **可靠性阶梯**（技术/安全类卡尤其要标；由低到高，只标当前档）：
  - `待确认` —— **默认**。模型说法或单一来源，证据不足。写成保留句、不写死。
  - `多源印证` —— ≥2 个**独立可信来源**交叉支持（对应 KnownEngine 的 Corroborated）。
  - `已复现` —— **我自己在实操/靶场/沙盒里跑通过**（对应 Reproduced）。AI 红队、漏洞、payload 类结论达到这档才算硬。
  - `已验证` —— 人工确认、长期有效（最高，对应 Human-validated）。
  - `stale` —— 时效性内容已过时（news 常用，不删加标）。`有反证` —— 出现明确反例，标注并保留原说法脉络。
- `公司`：凡 `work-mihoyo/*` 一律带（本地边界标记）

> 这是 [KnownEngine 可靠性状态机](knownengine-design.md#91-状态定义)（Captured→Grounded→Corroborated→Built→Reproduced→Human-validated / Contradicted）的**轻量瘦身版**：我们不建 claim/evidence-span 机制，只用标签表达"这话有多可信"。**模型自身知识只配 `待确认`，绝不能凭空升档。** CTF 场景拓展后再考虑做到 span 级证据绑定。
