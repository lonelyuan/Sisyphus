#!/usr/bin/env node

/**
 * LifeIndex 专用 Notion MCP 网关。
 *
 * 模型永远拿不到 integration token，也不能提交 page_id：网关把所有读写固定到设置页配置的
 * 单个页面。普通交互只暴露读取；只有 LifeIndexSync 进程会拿到整页 Markdown 替换工具。
 */

import process from "node:process";

import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { StdioClientTransport as ClientTransport } from "@modelcontextprotocol/sdk/client/stdio.js";
import { Server } from "@modelcontextprotocol/sdk/server/index.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { CallToolRequestSchema, ListToolsRequestSchema } from "@modelcontextprotocol/sdk/types.js";

function required(name) {
  const value = process.env[name]?.trim();
  if (!value) throw new Error(`LifeIndex Notion 网关缺少 ${name}`);
  return value;
}

function normalizePageId(raw) {
  const compact = raw.match(/[0-9a-fA-F]{32}/g)?.at(-1) ?? raw.replaceAll("-", "");
  if (!/^[0-9a-fA-F]{32}$/.test(compact)) {
    throw new Error("SISYPHUS_NOTION_PAGE_ID 必须是 Notion page ID 或含该 ID 的页面 URL");
  }
  return `${compact.slice(0, 8)}-${compact.slice(8, 12)}-${compact.slice(12, 16)}-${compact.slice(16, 20)}-${compact.slice(20)}`;
}

const token = required("SISYPHUS_NOTION_TOKEN");
const pageId = normalizePageId(required("SISYPHUS_NOTION_PAGE_ID"));
const allowWrite = process.env.SISYPHUS_NOTION_ALLOW_WRITE?.trim() === "1";

const upstream = new Client(
  { name: "sisyphus-lifeindex-gateway", version: "0.1.0" },
  { capabilities: {} },
);
await upstream.connect(
  new ClientTransport({
    command: "npx",
    args: ["-y", "@notionhq/notion-mcp-server@2.5.1"],
    env: { ...process.env, NOTION_TOKEN: token },
  }),
);

const upstreamTools = (await upstream.listTools()).tools ?? [];
const readTool = upstreamTools.find((tool) => tool.name.endsWith("retrieve-page-markdown"));
const writeTool = upstreamTools.find((tool) => tool.name.endsWith("update-page-markdown"));
if (!readTool) throw new Error("当前 Notion MCP 不支持整页 Markdown 读取");
if (allowWrite && !writeTool) throw new Error("当前 Notion MCP 不支持整页 Markdown 更新");

const server = new Server(
  { name: "sisyphus-notion-lifeindex", version: "0.1.0" },
  {
    capabilities: { tools: {} },
    instructions: "只允许读取配置好的 LifeIndex 页面；专用同步模式可替换该页正文。page_id 由网关固定。",
  },
);

server.setRequestHandler(ListToolsRequestSchema, async () => ({
  tools: [
    {
      name: "read_lifeindex_page",
      description: "读取设置页中唯一配置的 Notion LifeIndex 页面，返回完整 Markdown。无需也不能传 page_id。",
      inputSchema: { type: "object", properties: {}, additionalProperties: false },
      annotations: { readOnlyHint: true },
    },
    ...(allowWrite
      ? [
          {
            name: "replace_lifeindex_page",
            description:
              "用完整 Markdown 替换设置页中唯一配置的 Notion LifeIndex 页面。仅同步任务可用；不会删除子页面或数据库。",
            inputSchema: {
              type: "object",
              properties: {
                markdown: {
                  type: "string",
                  description: "render_lifeindex_projection 生成的完整 markdown，禁止自行删减事项。",
                },
              },
              required: ["markdown"],
              additionalProperties: false,
            },
            annotations: { destructiveHint: true },
          },
        ]
      : []),
  ],
}));

server.setRequestHandler(CallToolRequestSchema, async (request) => {
  if (request.params.name === "read_lifeindex_page") {
    return upstream.callTool({
      name: readTool.name,
      arguments: { page_id: pageId, include_transcript: false },
    });
  }
  if (request.params.name === "replace_lifeindex_page" && allowWrite) {
    const markdown = request.params.arguments?.markdown;
    if (typeof markdown !== "string" || !markdown.trim()) {
      return {
        isError: true,
        content: [{ type: "text", text: "markdown 不能为空" }],
      };
    }
    return upstream.callTool({
      name: writeTool.name,
      arguments: {
        page_id: pageId,
        type: "replace_content",
        replace_content: { new_str: markdown, allow_deleting_content: false },
      },
    });
  }
  return {
    isError: true,
    content: [{ type: "text", text: `LifeIndex Notion 网关拒绝工具：${request.params.name}` }],
  };
});

async function close() {
  await Promise.allSettled([server.close(), upstream.close()]);
}
process.once("SIGINT", () => void close().finally(() => process.exit(0)));
process.once("SIGTERM", () => void close().finally(() => process.exit(0)));

await server.connect(new StdioServerTransport());
