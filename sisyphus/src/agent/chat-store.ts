import type { AgentTurn } from "./agent";

const STORAGE_KEY = "sisyphus.agent.chats.v1";
const ACTIVE_KEY = "sisyphus.agent.active-chat.v1";
const MAX_CHATS = 80;

export type ChatRuntime = "auto" | "pi" | "codex";

export interface ChatMessage extends AgentTurn {
  id: string;
  createdAt: number;
  runtime?: "pi" | "codex";
}

export interface ChatConversation {
  id: string;
  title: string;
  createdAt: number;
  updatedAt: number;
  runtime: ChatRuntime;
  draft: string;
  messages: ChatMessage[];
  lastError?: string;
}

export function createConversation(now = Date.now()): ChatConversation {
  return {
    id: crypto.randomUUID(),
    title: "新对话",
    createdAt: now,
    updatedAt: now,
    runtime: "auto",
    draft: "",
    messages: [],
  };
}

export function loadChats(): ChatConversation[] {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return [createConversation()];
    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [createConversation()];
    const chats = parsed.filter(isConversation).slice(0, MAX_CHATS);
    return chats.length ? chats : [createConversation()];
  } catch {
    return [createConversation()];
  }
}

export function saveChats(chats: ChatConversation[]) {
  localStorage.setItem(STORAGE_KEY, JSON.stringify(chats.slice(0, MAX_CHATS)));
}

export function loadActiveChatId(chats: ChatConversation[]): string {
  const stored = localStorage.getItem(ACTIVE_KEY);
  return chats.some((chat) => chat.id === stored) ? stored! : chats[0].id;
}

export function saveActiveChatId(id: string) {
  localStorage.setItem(ACTIVE_KEY, id);
}

export function titleFromMessage(text: string): string {
  const title = text.replace(/\s+/g, " ").trim();
  return title.length > 24 ? `${title.slice(0, 24)}…` : title || "新对话";
}

function isConversation(value: unknown): value is ChatConversation {
  if (!value || typeof value !== "object") return false;
  const chat = value as Partial<ChatConversation>;
  return (
    typeof chat.id === "string" &&
    typeof chat.title === "string" &&
    typeof chat.createdAt === "number" &&
    typeof chat.updatedAt === "number" &&
    (chat.runtime === "auto" || chat.runtime === "pi" || chat.runtime === "codex") &&
    typeof chat.draft === "string" &&
    Array.isArray(chat.messages) &&
    chat.messages.every(isMessage)
  );
}

function isMessage(value: unknown): value is ChatMessage {
  if (!value || typeof value !== "object") return false;
  const message = value as Partial<ChatMessage>;
  return (
    typeof message.id === "string" &&
    (message.role === "user" || message.role === "assistant") &&
    typeof message.text === "string" &&
    typeof message.createdAt === "number"
  );
}
