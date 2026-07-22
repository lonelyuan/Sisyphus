#!/usr/bin/env node
// Sisyphus 知识 agent 派发脚本（app→Codex TS SDK）。知识工程 v2。
//
// 用法（对应知识工程四大场景，见 skills/knowledge-engine/references/pipelines.md）：
//   node index.mjs "<主题>"                 # 场景③ 主动探索：搜索引擎深研并写入知识库
//   node index.mjs --batch <文件夹>          # 场景② 批量学习：吸收一个现有知识库（融入非照搬）
//   SISYPHUS_AGENT_DRYRUN=1 node index.mjs … # mock：不 spawn codex，只回执（验证派发管道）
//
// 场景①（日常对话后反思）在对话内完成，不走本脚本。
// 场景④（沙箱实践验证 / CTF）验证器尚未实现——见 pipelines.md ④，暂人工复现后手动标 已复现。
//
// 环境变量：
//   SISYPHUS_VAULT         知识库目录（Codex 的 workingDirectory；未设则用当前目录）
//   SISYPHUS_AGENT_DRYRUN  =1 时走 mock，不导入 SDK、不启动 codex（本机可测）
//
// 真机前提：`npm install`（含 @openai/codex CLI）+ codex 已鉴权（`codex login` 或 CODEX_API_KEY）
//   + ~/.codex/config.toml 注册了 sisyphus MCP（见 skills/sisyphus/SKILL.md），
//   这样 Codex 才能调 write_knowledge_note / save_source 落库。数据模型见 skills/knowledge-engine/。

const args = process.argv.slice(2);
const dryRun = process.env.SISYPHUS_AGENT_DRYRUN === "1";
const vault = process.env.SISYPHUS_VAULT || process.cwd();

let mode, target;
if (args[0] === "--batch") {
  mode = "batch";
  target = args.slice(1).join(" ").trim();
} else {
  mode = "topic";
  target = args.join(" ").trim();
}

if (!target) {
  console.error('usage: node index.mjs "<主题>"  |  node index.mjs --batch <文件夹>');
  process.exit(2);
}

// mock 模式：零依赖，仅回执，验证 Rust→Node→(派发) 接线。
if (dryRun) {
  console.log(JSON.stringify({ ok: true, dryRun: true, mode, target, vault }));
  process.exit(0);
}

// 知识工程 v2 契约（内联，确保脱离 skill 也遵守核心规则）。
const CONTRACT = `知识库在工作目录（Obsidian vault）。**两个物理隔离的存储**：
- kb/ = 你总结的知识卡片（结构化 + [[wikilink]]，即博客内容）。
- sources/ = 原文逐字归档（不进图谱、不导出博客）。
**分类两轴**：① 话题领域 = write_knowledge_note 的 folder，**值以 kb/ 开头**：kb/web-security | kb/network-infra | kb/ai-redteam | kb/work-mihoyo/state | kb/work-mihoyo/best-practice | kb/personal（想不到就提议新领域，别丢根目录）。② 文章类型 = tags：theory|news|state|best-practice|personal + 状态 待确认|已验证。
**铁律**：**卡=结晶主题，不是对话；默认归并非新建**——同主题多轮/多篇 → 长同一颗卡的 H2 小节，别每次另开碎卡；建新卡前先过双筛「独立可检索 且（多处被引用 或 会撑爆母卡）」，过不了的作为已有结晶的小节。宁缺毋滥；证据不足标 #待确认、不编造；每张卡必有 folder + type 标签 + ≥1 条 links；sources/ 原文绝不进 kb；state（我司现状）与 best-practice（理想）绝不写进同一张卡，用 gap 链接表达差距；work-mihoyo/* 一律 #公司、仅本地。
**工具**：建/长卡前先 list_knowledge / search_knowledge 按**主题**查重；已有则读回按类型生命周期更新（theory 补充式：先读旧卡→加/精一个小节→写超集，绝不删已验证内容）。发现同主题碎卡簇 → 用 write_knowledge_note 写合并超集卡 + 改 [[wikilink]] + delete_note 删旧碎卡（结晶化/defragment）。值得原文留存的先 save_source(title,content,url,source_type) 归档到 sources/，再在 kb 卡 sources 里引用其路径。`;

const prompts = {
  topic: `你是 Sisyphus 第二大脑知识工程师。围绕主题「${target}」做**主动探索 / 深研（deep research）**：
1. 有 deep research / 联网工具就用，**多角度、尽可能丰富**地检索可验证要点；抓到的重要原文用 save_source 归档到 sources/。
2. 结合已有知识库（先 list_knowledge / search_knowledge），**重点补齐已有卡片里标 #待确认 的缺口**，以及该主题缺失的核心概念。
3. 产出结晶卡片，每张分类归位（folder + type），5 行内摘要 + 关键要点 + [[wikilink]] + sources；**默认归并进已有结晶**（长 H2 小节），只在过原子性双筛时才新建。
4. 结束时报告：新建/更新了哪些卡、归档了哪些原文、各相关领域现在卡数（供 rebalance）。
${CONTRACT}`,

  batch: `你是 Sisyphus 第二大脑知识工程师。**批量学习 / 吸收**一个现有知识库「${target}」——目标是尽可能完整梳理后**融入我自己的体系，而不是照搬**：
1. 遍历该文件夹下的文档（.md/.txt/.pdf 文本等）。逐篇：值得原文留存的先 save_source 归档到 sources/（保留 url/来源）。压缩包先解压。
2. **去芜存菁**：抽取每篇值得沉淀的概念，丢掉水分/营销/重复；不是每篇都建卡。
3. **按我的分类学归位，绝不复制对方目录结构或原文段落**：概念进我自己的话题树 + 结晶格式 + 可靠性标签；先 list_knowledge / search_knowledge 按主题查重，**默认归并进已有结晶**（长 H2 小节），无则新建。
4. **结构反哺**：若对方的抽象/分类比我更稳更清，提议优化我自己的树（MOVE/SPLIT/新领域，rebalance）——学的是组织方式，不只是内容。
5. 全部处理完，给 rebalance 建议（>~12 拆子话题、<2 上并、树深≤3、同主题碎卡合并），更新受影响领域 index.md。
6. 报告：处理多少篇、归档多少原文、新建/更新/合并多少卡、结构调整建议。
${CONTRACT}`,
};

// 真机：动态导入，避免 dry-run 也强依赖 SDK。
const { Codex } = await import("@openai/codex-sdk");
const codex = new Codex();
const thread = codex.startThread({
  workingDirectory: vault,
  skipGitRepoCheck: true, // vault 通常不是 git 仓库
});

const turn = await thread.run(prompts[mode]);
console.log(turn.finalResponse ?? "(no final response)");
