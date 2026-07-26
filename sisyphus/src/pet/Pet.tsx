import { useEffect, useRef, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { LogicalSize, PhysicalPosition } from "@tauri-apps/api/dpi";
import { listen } from "@tauri-apps/api/event";
import { askAgent } from "@/agent/agent";
import { logger } from "@/lib/log";

const log = logger("pet");

// 西西弗斯桌面宠物：一颗会动的巨石。拖动移动窗口、单击开/关对话气泡（两者严格区分）。
const COLLAPSED: [number, number] = [160, 160];
const OPEN: [number, number] = [340, 460];
const DRAG_THRESHOLD = 5; // px：超过才算拖动，否则算单击

interface Msg {
  role: "user" | "pi";
  text: string;
}

export default function Pet() {
  const [open, setOpen] = useState(false);
  const [msgs, setMsgs] = useState<Msg[]>([]);
  const [input, setInput] = useState("");
  const [busy, setBusy] = useState(false);
  const openRef = useRef(false);
  const drag = useRef<{ x: number; y: number; moved: boolean } | null>(null);
  const win = getCurrentWindow();

  useEffect(() => {
    // 主动推荐（agent_run 投递）与规则命中的宠物气泡（pet_message 派发）共用展示逻辑。
    const unlistens = ["agent-recommendation", "pet-message"].map((evt) =>
      listen<string>(evt, (event) => {
        setMsgs((current) => [...current, { role: "pi", text: event.payload }]);
        if (!openRef.current) void toggle(true);
      }),
    );
    return () => {
      unlistens.forEach((p) => void p.then((fn) => fn()));
    };
  }, []);

  // resize 时同步重定位，让宠物底部中心保持不动（否则从左上角展开会"瞬移"）。
  async function toggle(next: boolean) {
    log.info("toggle chat", next);
    const [oldW, oldH] = openRef.current ? OPEN : COLLAPSED;
    const [newW, newH] = next ? OPEN : COLLAPSED;
    try {
      const factor = await win.scaleFactor();
      const pos = await win.outerPosition(); // physical
      await win.setSize(new LogicalSize(newW, newH));
      const dx = Math.round(((oldW - newW) / 2) * factor); // 保持水平中心
      const dy = Math.round((oldH - newH) * factor); // 保持底边
      await win.setPosition(new PhysicalPosition(pos.x + dx, pos.y + dy));
    } catch (e) {
      log.error("resize/reposition failed", e);
    }
    openRef.current = next;
    setOpen(next);
  }

  // 指针判定：移动超阈值 → 拖窗（startDragging），不 toggle；无移动 → 单击 toggle。
  function onPointerDown(e: React.PointerEvent) {
    drag.current = { x: e.clientX, y: e.clientY, moved: false };
    try {
      (e.target as HTMLElement).setPointerCapture?.(e.pointerId);
    } catch {
      /* ignore */
    }
  }
  function onPointerMove(e: React.PointerEvent) {
    const d = drag.current;
    if (!d || d.moved) return;
    if (Math.hypot(e.clientX - d.x, e.clientY - d.y) > DRAG_THRESHOLD) {
      d.moved = true;
      try {
        (e.target as HTMLElement).releasePointerCapture?.(e.pointerId);
      } catch {
        /* ignore */
      }
      win.startDragging().catch((err) => log.error("startDragging failed", err));
    }
  }
  function onPointerUp() {
    const d = drag.current;
    drag.current = null;
    if (d && !d.moved) toggle(!openRef.current); // 只有"没拖动"才当单击
  }

  async function send() {
    const q = input.trim();
    if (!q || busy) return;
    log.info("send", q);
    setInput("");
    setMsgs((m) => [...m, { role: "user", text: q }]);
    setBusy(true);
    try {
      const history = msgs.map((m) => ({ role: m.role === "user" ? "user" as const : "assistant" as const, text: m.text }));
      const r = await askAgent(q, history);
      setMsgs((m) => [...m, { role: "pi", text: r.text }]);
    } catch (e) {
      log.error("askPi threw", e);
      setMsgs((m) => [...m, { role: "pi", text: "出错: " + String(e) }]);
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="pet-root">
      {open && (
        <div className="pet-chat">
          <div className="pet-chat-head">
            <span>西西弗斯</span>
            <button onClick={() => toggle(false)} aria-label="收起">
              ×
            </button>
          </div>
          <div className="pet-chat-body">
            {msgs.length === 0 && <p className="pet-hint">问我点什么。我会读取上下文，但不会替你修改内容。</p>}
            {msgs.map((m, i) => (
              <div key={i} className={m.role === "user" ? "pet-msg pet-msg-user" : "pet-msg pet-msg-pi"}>
                {m.text}
              </div>
            ))}
            {busy && <div className="pet-msg pet-msg-pi pet-typing">…</div>}
          </div>
          <div className="pet-chat-input">
            <input
              value={input}
              onChange={(e) => setInput(e.target.value)}
              placeholder="和西西弗斯说句话…"
              onKeyDown={(e) => e.key === "Enter" && send()}
            />
            <button onClick={send} disabled={busy}>
              发送
            </button>
          </div>
        </div>
      )}

      {/* 拖动=移动窗口；单击=开/关对话（指针位移阈值区分，互不触发） */}
      <div
        className="pet-sprite"
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        title="单击聊天 · 拖动移动"
      >
        <span className="pet-eye pet-eye-l" />
        <span className="pet-eye pet-eye-r" />
      </div>
    </div>
  );
}
