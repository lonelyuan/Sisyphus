import { useEffect, useMemo, useRef, useState } from "react";
import {
  Bot,
  Check,
  Copy,
  CornerDownLeft,
  LoaderCircle,
  MessageSquarePlus,
  Pencil,
  Plus,
  RotateCcw,
  ShieldCheck,
  Sparkles,
  Square,
  Trash2,
  X,
} from "lucide-react";
import { invoke } from "@tauri-apps/api/core";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { askAgent, cancelAgent, type AgentTurn } from "@/agent/agent";
import {
  createConversation,
  loadActiveChatId,
  loadChats,
  saveActiveChatId,
  saveChats,
  titleFromMessage,
  type ChatConversation,
  type ChatMessage,
  type ChatRuntime,
} from "@/agent/chat-store";
import { cn } from "@/lib/utils";

interface RuntimeStatus {
  configured: ChatRuntime;
  resolved: "pi" | "codex" | null;
}

interface PendingRun {
  runId: string;
  conversationId: string;
  startedAt: number;
}

const STARTERS = ["结合我最近的状态，帮我理清今天", "我现在有点拖延，陪我拆一下", "看看最近行为里有什么值得注意"];

export default function AgentScreen({ isVisible = true }: { isVisible?: boolean }) {
  const initialChats = useRef<ChatConversation[] | null>(null);
  if (!initialChats.current) initialChats.current = loadChats();

  const [chats, setChats] = useState<ChatConversation[]>(initialChats.current);
  const [activeId, setActiveId] = useState(() => loadActiveChatId(initialChats.current!));
  const [pending, setPending] = useState<PendingRun | null>(null);
  const [resolvedRuntime, setResolvedRuntime] = useState<string>("auto");
  const [editingId, setEditingId] = useState<string | null>(null);
  const [editingTitle, setEditingTitle] = useState("");
  const [copiedId, setCopiedId] = useState<string | null>(null);
  const cancelledRuns = useRef(new Set<string>());
  const endRef = useRef<HTMLDivElement>(null);
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  const active = chats.find((chat) => chat.id === activeId) ?? chats[0];
  const orderedChats = useMemo(
    () => [...chats].sort((a, b) => b.updatedAt - a.updatedAt),
    [chats],
  );
  const activeBusy = pending?.conversationId === active.id;
  const lastAssistantId = [...active.messages]
    .reverse()
    .find((message) => message.role === "assistant")?.id;
  const displayRuntime =
    [...active.messages].reverse().find((message) => message.runtime)?.runtime ??
    (active.runtime === "auto" ? resolvedRuntime : active.runtime);

  useEffect(() => saveChats(chats), [chats]);
  useEffect(() => saveActiveChatId(activeId), [activeId]);

  useEffect(() => {
    invoke<RuntimeStatus>("get_agent_runtime_status")
      .then((status) => setResolvedRuntime(status.resolved || status.configured || "auto"))
      .catch(() => setResolvedRuntime("auto"));
  }, []);

  useEffect(() => {
    if (isVisible && (activeBusy || active.messages.length)) {
      endRef.current?.scrollIntoView({ behavior: "smooth", block: "end" });
    }
  }, [active.messages, activeBusy, isVisible]);

  useEffect(() => {
    const element = textareaRef.current;
    if (!element) return;
    element.style.height = "0px";
    element.style.height = `${Math.min(element.scrollHeight, 160)}px`;
  }, [active.draft]);

  function updateChat(id: string, update: (chat: ChatConversation) => ChatConversation) {
    setChats((current) => current.map((chat) => (chat.id === id ? update(chat) : chat)));
  }

  function selectChat(id: string) {
    setActiveId(id);
    setEditingId(null);
  }

  function newChat() {
    const chat = createConversation();
    setChats((current) => [chat, ...current]);
    setActiveId(chat.id);
    setEditingId(null);
    requestAnimationFrame(() => textareaRef.current?.focus());
  }

  async function deleteChat(id: string) {
    const chat = chats.find((item) => item.id === id);
    if (!chat) return;
    // 不用 window.confirm：Tauri webview 里原生 confirm 常被禁用/返回 false，会导致删除永远不生效。
    if (pending?.conversationId === id) await stopGeneration();
    const remaining = chats.filter((item) => item.id !== id);
    const next = remaining.length ? remaining : [createConversation()];
    setChats(next);
    if (activeId === id) setActiveId(next[0].id);
  }

  function beginRename(chat: ChatConversation) {
    setEditingId(chat.id);
    setEditingTitle(chat.title);
  }

  function commitRename() {
    if (!editingId) return;
    const title = editingTitle.trim();
    if (title) updateChat(editingId, (chat) => ({ ...chat, title, updatedAt: Date.now() }));
    setEditingId(null);
  }

  async function runMessage(
    conversation: ChatConversation,
    text: string,
    history: AgentTurn[],
    appendUser: boolean,
  ) {
    if (pending) return;
    const runId = crypto.randomUUID();
    const now = Date.now();
    if (appendUser) {
      const userMessage: ChatMessage = {
        id: crypto.randomUUID(),
        role: "user",
        text,
        createdAt: now,
      };
      updateChat(conversation.id, (chat) => ({
        ...chat,
        title: chat.messages.length ? chat.title : titleFromMessage(text),
        draft: "",
        lastError: undefined,
        updatedAt: now,
        messages: [...chat.messages, userMessage],
      }));
    } else {
      updateChat(conversation.id, (chat) => ({ ...chat, lastError: undefined }));
    }
    setPending({ runId, conversationId: conversation.id, startedAt: now });

    try {
      const out = await askAgent(text, history, conversation.runtime, runId);
      if (cancelledRuns.current.has(runId)) return;
      setResolvedRuntime(out.runtime);
      const assistant: ChatMessage = {
        id: crypto.randomUUID(),
        role: "assistant",
        text: out.text,
        runtime: out.runtime,
        createdAt: Date.now(),
      };
      updateChat(conversation.id, (chat) => ({
        ...chat,
        lastError: undefined,
        updatedAt: Date.now(),
        messages: [...chat.messages, assistant],
      }));
    } catch (error) {
      if (!cancelledRuns.current.has(runId)) {
        updateChat(conversation.id, (chat) => ({
          ...chat,
          lastError: String(error),
          updatedAt: Date.now(),
        }));
      }
    } finally {
      cancelledRuns.current.delete(runId);
      setPending((current) => (current?.runId === runId ? null : current));
    }
  }

  function send(raw = active.draft) {
    const text = raw.trim();
    if (!text || pending) return;
    const history = active.messages.map(({ role, text: body }) => ({ role, text: body }));
    void runMessage(active, text, history, true);
  }

  async function stopGeneration() {
    if (!pending) return;
    const current = pending;
    cancelledRuns.current.add(current.runId);
    updateChat(current.conversationId, (chat) => ({
      ...chat,
      lastError: "生成已停止，可以重试上一条消息。",
      updatedAt: Date.now(),
    }));
    setPending(null);
    try {
      await cancelAgent(current.runId);
    } catch {
      // 进程可能恰好已经退出；本地 run id 仍会阻止迟到结果落入对话。
    }
  }

  function retryLast() {
    if (pending) return;
    let userIndex = -1;
    for (let index = active.messages.length - 1; index >= 0; index -= 1) {
      if (active.messages[index].role === "user") {
        userIndex = index;
        break;
      }
    }
    if (userIndex < 0) return;
    const user = active.messages[userIndex];
    const history = active.messages
      .slice(0, userIndex)
      .map(({ role, text }) => ({ role, text }));
    const snapshot = { ...active, messages: active.messages.slice(0, userIndex + 1) };
    updateChat(active.id, (chat) => ({
      ...chat,
      messages: chat.messages.slice(0, userIndex + 1),
      lastError: undefined,
    }));
    void runMessage(snapshot, user.text, history, false);
  }

  async function copyMessage(message: ChatMessage) {
    try {
      await navigator.clipboard.writeText(message.text);
      setCopiedId(message.id);
      window.setTimeout(() => setCopiedId((id) => (id === message.id ? null : id)), 1400);
    } catch {
      // 某些 WebView 不开放 clipboard，不影响对话。
    }
  }

  return (
    <section className="agent-screen animate-in">
      <aside className="agent-history" aria-label="对话历史">
        <div className="agent-history-head">
          <span>对话</span>
          <button onClick={newChat} aria-label="新建对话" title="新建对话">
            <Plus size={15} />
          </button>
        </div>
        <div className="agent-history-list">
          {orderedChats.map((chat) => (
            <div key={chat.id} className={cn("agent-history-row", chat.id === active.id && "active")}>
              {editingId === chat.id ? (
                <form
                  className="agent-history-rename"
                  onSubmit={(event) => {
                    event.preventDefault();
                    commitRename();
                  }}
                >
                  <input
                    autoFocus
                    value={editingTitle}
                    onChange={(event) => setEditingTitle(event.target.value)}
                    onBlur={commitRename}
                    aria-label="对话标题"
                  />
                </form>
              ) : (
                <button className="agent-history-select" onClick={() => selectChat(chat.id)}>
                  <span>{chat.title}</span>
                  <small>
                    {pending?.conversationId === chat.id
                      ? "正在回复…"
                      : chat.messages[chat.messages.length - 1]?.text.slice(0, 32) || "还没有消息"}
                  </small>
                </button>
              )}
              {editingId !== chat.id && (
                <div className="agent-history-actions">
                  <button onClick={() => beginRename(chat)} aria-label="重命名对话" title="重命名">
                    <Pencil size={11} />
                  </button>
                  <button onClick={() => void deleteChat(chat.id)} aria-label="删除对话" title="删除">
                    <Trash2 size={11} />
                  </button>
                </div>
              )}
            </div>
          ))}
        </div>
      </aside>

      <div className="agent-chat">
        <header className="agent-heading">
          <div>
            <p className="eyebrow">AGENT</p>
            <h1>{active.title}</h1>
            <p>对话保存在本地；Pi 与 Codex 共用同一份只读上下文。</p>
          </div>
          <div className="agent-heading-actions">
            <button className="agent-new-chat" onClick={newChat}>
              <MessageSquarePlus size={13} /> 新对话
            </button>
            <div className="runtime-pill" title="智能体只读：不会修改 Notion 或本地内容">
              <ShieldCheck size={13} />
              {displayRuntime === "auto" ? "自动选择" : displayRuntime}
              <span>只读</span>
            </div>
          </div>
        </header>

        <div className="agent-conversation" aria-live="polite">
          {active.messages.length === 0 ? (
            <div className="agent-empty">
              <div className="agent-orb"><Sparkles size={22} /></div>
              <h2>今天的石头，推到哪里了？</h2>
              <p>我会按需读取行为记录与已授权信息源，但不会替你改动它们。</p>
              <div className="starter-grid">
                {STARTERS.map((starter) => (
                  <button key={starter} disabled={Boolean(pending)} onClick={() => send(starter)}>{starter}</button>
                ))}
              </div>
            </div>
          ) : (
            active.messages.map((message) => (
              <article key={message.id} className={cn("agent-message", message.role)}>
                <div className="agent-avatar">
                  {message.role === "assistant" ? <Bot size={15} /> : "你"}
                </div>
                <div className="agent-message-body">
                  <div className="agent-message-meta">
                    {message.role === "assistant" ? "西西弗斯" : "你"}
                    {message.runtime && <span>{message.runtime}</span>}
                  </div>
                  {message.role === "assistant" ? (
                    <div className="agent-markdown">
                      <ReactMarkdown
                        remarkPlugins={[remarkGfm]}
                        components={{
                          a: ({ href, children }) => <a href={href} target="_blank" rel="noreferrer">{children}</a>,
                        }}
                      >
                        {message.text}
                      </ReactMarkdown>
                    </div>
                  ) : (
                    <p>{message.text}</p>
                  )}
                  <div className="agent-message-actions">
                    <button onClick={() => void copyMessage(message)} title="复制">
                      {copiedId === message.id ? <Check size={12} /> : <Copy size={12} />}
                      {copiedId === message.id ? "已复制" : "复制"}
                    </button>
                    {message.role === "assistant" && message.id === lastAssistantId && !pending && (
                      <button onClick={retryLast} title="重新生成">
                        <RotateCcw size={12} /> 重新生成
                      </button>
                    )}
                  </div>
                </div>
              </article>
            ))
          )}

          {activeBusy && (
            <article className="agent-message assistant">
              <div className="agent-avatar"><Bot size={15} /></div>
              <div className="agent-message-body">
                <div className="agent-message-meta">西西弗斯</div>
                <p className="agent-thinking"><LoaderCircle size={14} /> 正在读取上下文并思考…</p>
              </div>
            </article>
          )}

          {active.lastError && (
            <div className="agent-error">
              <span>Agent 暂时没有回应：{active.lastError}</span>
              <div>
                <button onClick={retryLast} disabled={Boolean(pending)}><RotateCcw size={12} /> 重试</button>
                <button onClick={() => updateChat(active.id, (chat) => ({ ...chat, lastError: undefined }))}>
                  <X size={12} /> 关闭
                </button>
              </div>
            </div>
          )}

          {pending && !activeBusy && (
            <button className="agent-other-run" onClick={() => selectChat(pending.conversationId)}>
              另一个对话正在回复，点击返回
            </button>
          )}
          <div ref={endRef} />
        </div>

        <form
          className="agent-composer"
          onSubmit={(event) => {
            event.preventDefault();
            send();
          }}
        >
          <div className="agent-composer-main">
            <textarea
              ref={textareaRef}
              value={active.draft}
              onChange={(event) =>
                updateChat(active.id, (chat) => ({ ...chat, draft: event.target.value }))
              }
              onKeyDown={(event) => {
                if (event.key === "Enter" && !event.shiftKey) {
                  event.preventDefault();
                  send();
                }
              }}
              placeholder="说说你正在想什么…"
              rows={1}
            />
            <div className="agent-composer-bar">
              <label>
                <span>模型</span>
                <select
                  value={active.runtime}
                  disabled={Boolean(pending)}
                  onChange={(event) =>
                    updateChat(active.id, (chat) => ({
                      ...chat,
                      runtime: event.target.value as ChatRuntime,
                    }))
                  }
                >
                  <option value="auto">自动</option>
                  <option value="pi">Pi</option>
                  <option value="codex">Codex</option>
                </select>
              </label>
              <span>Enter 发送 · Shift+Enter 换行</span>
            </div>
          </div>
          {activeBusy ? (
            <button type="button" className="agent-stop" onClick={() => void stopGeneration()} aria-label="停止生成">
              <Square size={14} fill="currentColor" />
            </button>
          ) : (
            <button type="submit" disabled={!active.draft.trim() || Boolean(pending)} aria-label="发送">
              <CornerDownLeft size={17} />
            </button>
          )}
        </form>
      </div>
    </section>
  );
}
