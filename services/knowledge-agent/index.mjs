#!/usr/bin/env node
// Sisyphus 知识 agent 派发脚本（app→Codex TS SDK）。
//
// 用法：
//   node index.mjs "<主题>"                     # 真机：跑 Codex 深研并写入知识库
//   SISYPHUS_AGENT_DRYRUN=1 node index.mjs "x"   # mock：不 spawn codex，只回执（验证派发管道）
//
// 环境变量：
//   SISYPHUS_VAULT         知识库目录（Codex 的 workingDirectory；未设则用当前目录）
//   SISYPHUS_AGENT_DRYRUN  =1 时走 mock，不导入 SDK、不启动 codex（本机可测）
//
// 真机前提：`npm install`（会一并装入 @openai/codex CLI）+ codex 已鉴权
//   （`codex login` 或 CODEX_API_KEY）+ 你的 ~/.codex/config.toml 注册了 sisyphus MCP
//   （见 skills/sisyphus-daily/SKILL.md），这样 Codex 才能调 write_knowledge_note 落库。

const topic = process.argv.slice(2).join(" ").trim();
const dryRun = process.env.SISYPHUS_AGENT_DRYRUN === "1";
const vault = process.env.SISYPHUS_VAULT || process.cwd();

if (!topic) {
  console.error('usage: node index.mjs "<主题>"  (env: SISYPHUS_VAULT, SISYPHUS_AGENT_DRYRUN=1)');
  process.exit(2);
}

// mock 模式：零依赖，仅回执，用于验证 Rust→Node→(将会派发) 的管道接线。
if (dryRun) {
  console.log(JSON.stringify({ ok: true, dryRun: true, topic, vault }));
  process.exit(0);
}

// 真机：动态导入，避免 dry-run 也强依赖 SDK 安装。
const { Codex } = await import("@openai/codex-sdk");

const prompt = `你是 Sisyphus 的第二大脑知识工程师。围绕主题「${topic}」做知识加工：
1. 检索/整理该主题的可验证要点（有 deep research 工具就用）。
2. 产出 3–10 张概念卡片，每张：5 行以内摘要 + 关键要点，标注来源。
3. 用 [[wikilink]] 表达卡片之间及与既有知识的关系。
优先调用 sisyphus MCP 的 write_knowledge_note 工具逐张保存（title/body/tags/links/sources）；
若该工具不可用，则直接在当前工作目录写 {slug}.md（YAML frontmatter: title/tags/sources + 正文 + [[wikilink]]）。
低置信度的关系只在正文注明「待确认」，不要污染知识库。`;

const codex = new Codex();
const thread = codex.startThread({
  workingDirectory: vault,
  skipGitRepoCheck: true, // vault 通常不是 git 仓库
});

const turn = await thread.run(prompt);
console.log(turn.finalResponse ?? "(no final response)");
