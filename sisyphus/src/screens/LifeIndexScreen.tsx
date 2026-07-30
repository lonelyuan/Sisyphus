import { useCallback, useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Check, Clock3, Flame, LayoutGrid, Plus, RefreshCw, Repeat2, Route, Sparkles } from "lucide-react";
import { Card, CardLabel } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import ItemDraftDialog from "@/lifeindex/ItemDraftDialog";
import {
  dateValue,
  emptyDraft,
  kindLabel,
  horizonLabel,
  payload,
  toDraft,
  type Draft,
  type LifeArea,
  type LifeItem,
  type LifeStatus,
} from "@/lifeindex/model";

interface SyncOverview {
  configured: boolean;
  sync_enabled: boolean;
  target_id: string;
  projection: { dirty_count: number; item_count: number; max_revision: number };
  state: {
    last_summary: string;
    last_success_at_ms: number | null;
    last_attempt_at_ms: number | null;
    last_error: string | null;
  } | null;
}

function ItemCard({ item, onEdit, onToggle }: { item: LifeItem; onEdit: () => void; onToggle: () => void }) {
  const dirty = item.sync_status !== "clean";
  return (
    <Card
      className={cn(
        "group flex cursor-pointer flex-col gap-2 p-3 transition hover:border-muted-foreground/40",
        item.status === "done" && "opacity-55",
        item.sync_status === "conflict" && "border-warning/60",
      )}
      onClick={onEdit}
    >
      <div className="flex items-start gap-2">
        <button
          className={cn(
            "mt-0.5 flex h-4 w-4 shrink-0 items-center justify-center rounded-full border text-transparent transition",
            item.status === "done" ? "border-success bg-success text-white" : "border-muted-foreground/50 hover:border-success",
          )}
          title={item.status === "done" ? "重新打开" : "标记完成"}
          onClick={(event) => {
            event.stopPropagation();
            onToggle();
          }}
        >
          <Check size={11} strokeWidth={3} />
        </button>
        <div className="min-w-0 flex-1">
          <p className={cn("text-sm font-medium text-foreground", item.status === "done" && "line-through")}>{item.title}</p>
          {item.body && <p className="mt-1 line-clamp-2 text-[11px] leading-relaxed text-muted-foreground">{item.body}</p>}
        </div>
        {dirty && <span className="mt-1 h-1.5 w-1.5 shrink-0 rounded-full bg-accent" title="等待同步" />}
      </div>
      <div className="flex flex-wrap gap-1 text-[10px] text-muted-foreground">
        <span className="rounded bg-muted px-1.5 py-0.5">{kindLabel[item.kind]}</span>
        <span className="rounded bg-muted px-1.5 py-0.5">{horizonLabel[item.horizon]}</span>
        {item.due_at_ms && <span className="rounded bg-warning/10 px-1.5 py-0.5 text-warning">至 {dateValue(item.due_at_ms)}</span>}
        {item.recurrence && <span className="rounded bg-muted px-1.5 py-0.5">↻ {item.recurrence}</span>}
      </div>
    </Card>
  );
}

export default function LifeIndexScreen() {
  const [items, setItems] = useState<LifeItem[]>([]);
  const [areas, setAreas] = useState<LifeArea[]>([]);
  const [sync, setSync] = useState<SyncOverview | null>(null);
  const [loaded, setLoaded] = useState(false);
  const [draft, setDraft] = useState<Draft | null>(null);
  const [saving, setSaving] = useState(false);
  const [syncing, setSyncing] = useState(false);
  const [error, setError] = useState("");

  const load = useCallback(async () => {
    try {
      const [nextItems, nextSync, nextAreas] = await Promise.all([
        invoke<LifeItem[]>("list_life_items", { includeArchived: false }),
        invoke<SyncOverview>("get_lifeindex_sync_overview"),
        invoke<LifeArea[]>("list_life_areas"),
      ]);
      setItems(nextItems);
      setSync(nextSync);
      setAreas(nextAreas);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setLoaded(true);
    }
  }, []);

  useEffect(() => {
    void load();
    const unlisten = listen("lifeindex-updated", () => void load());
    return () => {
      void unlisten.then((stop) => stop());
    };
  }, [load]);

  const boards = useMemo(
    () => [
      { key: "action", title: "事项", note: "具体要做，或有明确截止", icon: Clock3, items: items.filter((item) => item.kind === "action") },
      { key: "routine", title: "日常", note: "反复发生，不必固定到某天", icon: Repeat2, items: items.filter((item) => item.kind === "routine") },
      { key: "main", title: "主线", note: "长期积累核心竞争力", icon: Route, items: items.filter((item) => item.track === "main") },
      { key: "side", title: "支线", note: "让自己开心，乐于发展", icon: Flame, items: items.filter((item) => item.track === "side") },
    ],
    [items],
  );
  const visibleIds = new Set(boards.flatMap((board) => board.items.map((item) => item.id)));
  const inbox = items.filter((item) => !visibleIds.has(item.id));

  function newFor(board: string) {
    const next = { ...emptyDraft };
    if (board === "routine") next.kind = "routine";
    if (board === "main") {
      next.kind = "project";
      next.track = "main";
    }
    if (board === "side") {
      next.kind = "project";
      next.track = "side";
    }
    setDraft(next);
  }

  async function save() {
    if (!draft?.title.trim()) return;
    setSaving(true);
    setError("");
    try {
      await invoke("upsert_life_item", { input: payload({ ...draft, title: draft.title.trim() }) });
      setDraft(null);
      await load();
    } catch (reason) {
      setError(String(reason));
    } finally {
      setSaving(false);
    }
  }

  async function updateStatus(item: LifeItem, status: LifeStatus) {
    try {
      const next = toDraft(item);
      next.status = status;
      await invoke("upsert_life_item", { input: payload(next) });
      await load();
    } catch (reason) {
      setError(String(reason));
    }
  }

  async function archive() {
    if (!draft?.id) return;
    setSaving(true);
    try {
      await invoke("archive_life_item", { id: draft.id, expectedRevision: draft.expected_revision ?? null });
      setDraft(null);
      await load();
    } catch (reason) {
      setError(String(reason));
    } finally {
      setSaving(false);
    }
  }

  async function runSync() {
    setSyncing(true);
    setError("");
    try {
      await invoke("run_lifeindex_sync");
      await load();
    } catch (reason) {
      setError(String(reason));
    } finally {
      setSyncing(false);
    }
  }

  const lastSync = sync?.state?.last_success_at_ms
    ? new Date(sync.state.last_success_at_ms).toLocaleString([], { month: "numeric", day: "numeric", hour: "2-digit", minute: "2-digit" })
    : "尚未同步";

  return (
    <div className="animate-in mx-auto flex max-w-7xl flex-col gap-4 p-5 md:p-8">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <div className="flex items-center gap-2">
            <LayoutGrid size={17} strokeWidth={1.75} className="text-accent" />
            <h2 className="text-sm font-medium text-foreground">LifeIndex</h2>
            <span className="rounded-full bg-muted px-2 py-0.5 text-[10px] text-muted-foreground">LifeDB · {items.length}</span>
          </div>
          <p className="mt-1.5 text-[11px] text-muted-foreground">一份结构化数据，四个重叠视角。事项/日常描述形态，主线/支线描述意义。能力全景在「技能树」。</p>
        </div>
        <div className="flex items-center gap-2">
          <span className={cn("text-[10px]", sync?.state?.last_error ? "text-danger" : "text-muted-foreground")}>
            {sync?.projection.dirty_count ? `${sync.projection.dirty_count} 项待同步` : lastSync}
          </span>
          <Button variant="secondary" size="sm" disabled={syncing || !sync?.configured || !sync.sync_enabled} onClick={() => void runSync()}>
            <RefreshCw size={13} className={syncing ? "animate-spin" : ""} /> {syncing ? "同步中" : "立即同步"}
          </Button>
          <Button size="sm" onClick={() => setDraft({ ...emptyDraft })}><Plus size={14} /> 新建</Button>
        </div>
      </div>

      {error && <Card className="border-danger/40 bg-danger/10 p-3 text-xs text-danger">{error}</Card>}
      {sync?.state?.last_error && !error && <Card className="border-warning/30 bg-warning/5 p-3 text-xs text-warning">上次同步未完成：{sync.state.last_error}</Card>}
      {loaded && items.length === 0 && (
        <Card className="flex flex-col items-center gap-2 p-8 text-center text-sm text-muted-foreground">
          <Sparkles size={22} className="text-accent" />
          <p>LifeDB 还是空的。先随手丢一个想法，或配置 Notion 后立即同步。</p>
        </Card>
      )}

      <div className="grid items-start gap-3 md:grid-cols-2 xl:grid-cols-4">
        {boards.map((board) => {
          const Icon = board.icon;
          return (
            <section key={board.key} className="min-w-0 rounded-xl border border-border bg-muted/20 p-2.5">
              <div className="mb-2 flex items-start justify-between gap-2 px-1">
                <div>
                  <div className="flex items-center gap-1.5 text-xs font-medium text-foreground"><Icon size={13} className="text-accent" /> {board.title}<span className="text-[10px] text-muted-foreground">{board.items.length}</span></div>
                  <p className="mt-0.5 text-[9px] text-muted-foreground">{board.note}</p>
                </div>
                <button className="rounded p-1 text-muted-foreground hover:bg-muted hover:text-foreground" onClick={() => newFor(board.key)} title={`新增${board.title}`}><Plus size={13} /></button>
              </div>
              <div className="flex flex-col gap-2">
                {board.items.map((item) => <ItemCard key={item.id} item={item} onEdit={() => setDraft(toDraft(item))} onToggle={() => void updateStatus(item, item.status === "done" ? "active" : "done")} />)}
                {!board.items.length && <div className="rounded-lg border border-dashed border-border px-3 py-6 text-center text-[10px] text-muted-foreground">暂无</div>}
              </div>
            </section>
          );
        })}
      </div>

      {inbox.length > 0 && (
        <section className="flex flex-col gap-2">
          <CardLabel>待整理 · 尚未进入四视图（{inbox.length}）</CardLabel>
          <div className="grid gap-2 sm:grid-cols-2 lg:grid-cols-4">
            {inbox.map((item) => <ItemCard key={item.id} item={item} onEdit={() => setDraft(toDraft(item))} onToggle={() => void updateStatus(item, item.status === "done" ? "active" : "done")} />)}
          </div>
        </section>
      )}

      {draft && (
        <ItemDraftDialog
          draft={draft}
          areas={areas}
          saving={saving}
          onChange={setDraft}
          onClose={() => setDraft(null)}
          onSave={() => void save()}
          onArchive={() => void archive()}
        />
      )}
    </div>
  );
}
