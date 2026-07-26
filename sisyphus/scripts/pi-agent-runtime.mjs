#!/usr/bin/env node

import { mkdir } from "node:fs/promises";
import path from "node:path";
import process from "node:process";

import {
  createAgentSession,
  defineTool,
  DefaultResourceLoader,
  ModelRuntime,
  SessionManager,
  VERSION,
} from "@earendil-works/pi-coding-agent";
import { Type } from "@sinclair/typebox";
import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { StdioClientTransport } from "@modelcontextprotocol/sdk/client/stdio.js";

const ZERO_COST = { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 };

function required(name) {
  const value = process.env[name]?.trim();
  if (!value) throw new Error(`Pi SDK 缺少配置 ${name}`);
  return value;
}

function apiFor(format) {
  switch (format) {
    case "anthropic":
    case "anthropic-messages":
      return "anthropic-messages";
    case "google":
    case "google-generative-ai":
      return "google-generative-ai";
    case "openai-completions":
    case "deepseek":
    case "groq":
    case "openrouter":
    case "together":
    case "xai":
      return "openai-completions";
    case "openai":
    case "openai-responses":
    default:
      return "openai-responses";
  }
}

async function readStdin() {
  let input = "";
  process.stdin.setEncoding("utf8");
  for await (const chunk of process.stdin) input += chunk;
  return input.trim();
}

/**
 * 连接一个 MCP stdio server，把它的工具逐个包成 Pi customTool。同一份逻辑给 sisyphus-mcp
 * 和 Notion 官方 notion-mcp-server 共用，两个基座（Pi/Codex）由此拿到同一套工具契约
 * （architecture.md §4）。连接失败时返回空工具集（降级，不让整次运行崩掉）。
 */
async function connectMcpClient(label, command, args, env) {
  const client = new Client({ name: `sisyphus-pi-runtime-${label}`, version: VERSION }, { capabilities: {} });
  try {
    const transport = new StdioClientTransport({ command, args, env });
    await client.connect(transport);
    const { tools: mcpTools } = await client.listTools();
    const tools = (mcpTools ?? []).map((t) =>
      defineTool({
        name: t.name,
        label: t.name,
        description: t.description ?? t.name,
        // MCP inputSchema 是 JSON Schema；Type.Unsafe 让 Pi 原样透传给模型并跳过重复校验。
        parameters: Type.Unsafe(t.inputSchema ?? { type: "object", properties: {} }),
        async execute(_toolCallId, params) {
          const res = await client.callTool({
            name: t.name,
            arguments: params ?? {},
          });
          const content = (res.content ?? [])
            .filter((c) => c && c.type === "text" && typeof c.text === "string")
            .map((c) => ({ type: "text", text: c.text }));
          return {
            content: content.length ? content : [{ type: "text", text: res.isError ? "工具调用失败" : "" }],
            details: null,
          };
        },
      }),
    );
    return { tools, client };
  } catch (error) {
    process.stderr.write(`[pi-runtime] 接入 ${label} 失败，降级为无该工具集：${error}\n`);
    try {
      await client.close();
    } catch {
      // ignore
    }
    return { tools: [], client: null };
  }
}

/** 连 sisyphus-mcp（Rust，stdio），开与 App 同一个库 / vault，继承本次运行的只读模式。 */
async function loadSisyphusTools() {
  const mcpBin = process.env.SISYPHUS_MCP_BIN?.trim();
  if (!mcpBin) return { tools: [], client: null };
  const env = { ...process.env };
  for (const key of ["SISYPHUS_DB", "SISYPHUS_VAULT", "SISYPHUS_READ_ONLY"]) {
    const value = process.env[key];
    if (value !== undefined) env[key] = value;
  }
  return connectMcpClient("sisyphus", mcpBin, [], env);
}

/**
 * 连官方 Notion MCP server（`npx -y @notionhq/notion-mcp-server`），只在配置了
 * SISYPHUS_NOTION_TOKEN 时接入。只读边界靠 Notion 侧的 integration 权限机制保证
 * （建议 token 只给 "Read content" 权限），不靠这里的代码或提示词自觉。
 */
async function loadNotionTools() {
  const token = process.env.SISYPHUS_NOTION_TOKEN?.trim();
  if (!token) return { tools: [], client: null };
  const env = { ...process.env, NOTION_TOKEN: token };
  return connectMcpClient("notion", "npx", ["-y", "@notionhq/notion-mcp-server"], env);
}

function finalAssistantText(messages) {
  const assistant = [...messages].reverse().find((message) => message.role === "assistant");
  if (!assistant) return "";
  if (typeof assistant.content === "string") return assistant.content.trim();
  if (!Array.isArray(assistant.content)) return "";
  return assistant.content
    .filter((part) => part?.type === "text" && typeof part.text === "string")
    .map((part) => part.text)
    .join("")
    .trim();
}

if (process.argv.includes("--version")) {
  process.stdout.write(VERSION);
  process.exit(0);
}

const prompt = await readStdin();
if (!prompt) throw new Error("Pi SDK 收到了空 prompt");

const projectDir = required("SISYPHUS_PROJECT_DIR");
const agentDir = required("SISYPHUS_PI_AGENT_DIR");
const skillDir = required("SISYPHUS_SKILL_DIR");
const format = required("SISYPHUS_PI_FORMAT");
const modelId = required("SISYPHUS_PI_MODEL");
const apiKey = required("SISYPHUS_PI_API_KEY");
const baseUrl = process.env.SISYPHUS_PI_BASE_URL?.trim() ?? "";
// 主动/只读模式由宿主经 SISYPHUS_READ_ONLY 注入（"1"=只读）。决定系统提示与 MCP 写门禁。
const readOnly = process.env.SISYPHUS_READ_ONLY?.trim() === "1";

await mkdir(agentDir, { recursive: true });

const modelRuntime = await ModelRuntime.create({
  authPath: path.join(agentDir, "auth.json"),
  modelsPath: null,
  allowModelNetwork: false,
});

let providerId = format;
let model = baseUrl ? undefined : modelRuntime.getModel(providerId, modelId);

if (baseUrl) {
  providerId = `sisyphus-${format}`;
  modelRuntime.registerProvider(providerId, {
    name: `Sisyphus ${format}`,
    baseUrl,
    api: apiFor(format),
    authHeader: true,
    models: [
      {
        id: modelId,
        name: modelId,
        reasoning: true,
        input: ["text", "image"],
        cost: ZERO_COST,
        contextWindow: 128000,
        maxTokens: 16384,
      },
    ],
  });
  model = modelRuntime.getModel(providerId, modelId);
}

if (!model) {
  throw new Error(
    `Pi SDK 找不到模型 ${format}/${modelId}；自定义模型请同时配置 API Endpoint`,
  );
}
await modelRuntime.setRuntimeApiKey(providerId, apiKey, { allowNetwork: false });

const resourceLoader = new DefaultResourceLoader({
  cwd: projectDir,
  agentDir,
  additionalSkillPaths: [skillDir],
  noContextFiles: true,
  noPromptTemplates: true,
  noThemes: true,
  // 关键：不加载项目/全局 pi 扩展（如 `.pi/extensions/pi-permission-system`）。这些是为交互式
  // 终端 UI 写的，会在无 UI 的 headless 子进程里对每次工具调用返回
  // "requires approval, but no interactive UI is available" 而拒绝执行——我们自己的只读/可写
  // 门禁已经在 MCP server 侧（SISYPHUS_READ_ONLY/LIFEINDEX_ONLY）做了，不需要它。
  noExtensions: true,
  systemPrompt: readOnly
    ? `你是西西弗斯个人助手。当前为主动/只读模式：可以读取用户授权的本地状态和外部信息源，但严格禁止创建、修改或删除本地文件、数据库、Notion 文档、任务、目标或知识库。优先遵循 sisyphus skill，给用户的建议最多聚焦一件当下值得做的事。`
    : `你是西西弗斯个人助手（交互模式）。

【工具授权】你已经被完全授权直接使用所有 sisyphus 本地工具，无需任何额外授权、审批、弹窗或确认入口——它们随时可用，直接调用即可。绝对不要声称"需要授权""没有审批界面""无法读取/写入"之类；那是错误的，你有权限。
【读取】query_context / list_lifeindex / list_detection_rules / list_monitored_apps / list_captures / search_knowledge 等只读工具：需要时**直接调用**，不要征求许可，也不要让用户手动贴数据。
【写入】set_goal / add_monitored_app / create_detection_rule / propose_intents+accept_intent / write_knowledge_note / upsert_lifeindex_card 等：先用一句话向用户复述你要做什么并等其认可，认可后**立即调用工具真正落库**（不要只说"已设置"却不调用）。
【看板刷新】"刷新看板"= 先 list_lifeindex + query_context 读现状（直接调用），再据此 upsert_lifeindex_card 更新；外部 Notion 只读、绝不回写。
语气关心不评判、具体、只提最小下一步。优先遵循 sisyphus skill。`,
});
await resourceLoader.reload({ resolveProjectTrust: async () => true });

const [sisyphusLoaded, notionLoaded] = await Promise.all([loadSisyphusTools(), loadNotionTools()]);
const allTools = [...sisyphusLoaded.tools, ...notionLoaded.tools];
const mcpClients = [sisyphusLoaded.client, notionLoaded.client].filter(Boolean);

const { session } = await createAgentSession({
  cwd: projectDir,
  agentDir,
  model,
  thinkingLevel: "low",
  modelRuntime,
  resourceLoader,
  // 关键：必须用 tools 允许名单显式激活自定义工具。SDK 里 noTools:"builtin"（或任何真值）
  // 且未提供 tools 时，initialActiveToolNames=[] → 工具虽注册却不激活、不发给模型
  // （文档说"保留 custom tools"与实现不符）。只列出我们的工具 = 不启用内置 read/bash/edit/write。
  tools: allTools.map((t) => t.name),
  customTools: allTools,
  sessionManager: SessionManager.inMemory(projectDir),
});

try {
  await session.prompt(prompt);
  const error = session.agent.state.errorMessage;
  if (error) throw new Error(error);
  const text = finalAssistantText(session.agent.state.messages);
  if (!text) throw new Error("Pi SDK 未返回文本");
  process.stdout.write(text);
} finally {
  session.dispose();
  await Promise.all(
    mcpClients.map((client) =>
      client.close().catch(() => {
        // ignore
      }),
    ),
  );
}
