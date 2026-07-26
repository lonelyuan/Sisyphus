import { invoke } from "@tauri-apps/api/core";

export interface AgentTurn {
  role: "user" | "assistant";
  text: string;
}

export interface AgentRunOutput {
  runtime: "pi" | "codex";
  text: string;
  elapsed_ms: number;
}

/**
 * 主窗口与宠物共用的 Agent 入口。会话历史由 UI 持有，runtime 每次仍以无状态、
 * 只读方式运行，避免 Pi/Codex 各自产生另一份用户状态。
 */
export async function askAgent(
  text: string,
  history: AgentTurn[] = [],
  runtime?: "auto" | "pi" | "codex",
  runId?: string,
): Promise<AgentRunOutput> {
  const recent = history.slice(-12);
  const prompt = recent.length
    ? `以下是最近对话，仅用于保持上下文：\n${recent
        .map((m) => `${m.role === "user" ? "用户" : "西西弗斯"}：${m.text}`)
        .join("\n")}\n\n用户的新消息：${text}`
    : text;
  return invoke<AgentRunOutput>("run_agent", {
    prompt,
    runtime: runtime && runtime !== "auto" ? runtime : null,
    runId: runId || null,
  });
}

export async function cancelAgent(runId: string): Promise<boolean> {
  return invoke<boolean>("cancel_agent_run", { runId });
}

/** 兼容旧宠物调用；新代码应使用 askAgent 以取得 runtime 元数据。 */
export async function askPi(text: string): Promise<string> {
  return (await askAgent(text)).text;
}
