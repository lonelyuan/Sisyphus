import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  Circle,
  Link2,
  LocateFixed,
  Network,
  Pause,
  Play,
  Plus,
  Radar,
  Sparkles,
} from "lucide-react";
import ItemDraftDialog from "@/lifeindex/ItemDraftDialog";
import { emptyDraft, payload, toDraft, type Draft, type LifeArea, type LifeItem } from "@/lifeindex/model";
import { draw, hitTest, type HitRegion } from "@/skilltree/draw";
import { createBody, step, type Body } from "@/skilltree/physics";
import { buildTimeline, frameAt, stateAt, type Frame, type Timeline } from "@/skilltree/playback";
import { clamp, nodeSize, place, toScreen, type Point, type Viewport } from "@/skilltree/projection";
import { EMPTY_MAP, type Growth, type SkillMap, type SkillNode } from "@/skilltree/types";

const DAY = 86_400_000;
/** 里程碑从"环上刻度"展开成卫星节点的缩放阈值。 */
const EXPAND_AT = 1.45;

interface ReviewItem {
  item_id: string;
  title: string;
  question: string;
}
interface ReviewQueue {
  due_review: ReviewItem[];
  stalled: ReviewItem[];
  undecomposed: ReviewItem[];
  no_success_criteria: ReviewItem[];
  stale_inbox: ReviewItem[];
}

export default function SkillTreeScreen() {
  const [map, setMap] = useState<SkillMap>(EMPTY_MAP);
  const [areas, setAreas] = useState<LifeArea[]>([]);
  const [review, setReview] = useState<ReviewQueue | null>(null);
  const [growth, setGrowth] = useState<Growth | null>(null);
  const [size, setSize] = useState({ width: 900, height: 520 });
  const [scale, setScale] = useState(1);
  const [offset, setOffset] = useState({ x: 0, y: 0 });
  /** 0 = 树投影（半径 = 依赖深度），1 = 雷达投影（半径 = 掌握度）。 */
  const [projection, setProjection] = useState(0);
  const [playAt, setPlayAt] = useState<number | null>(null);
  const [playing, setPlaying] = useState(false);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [hoverId, setHoverId] = useState<string | null>(null);
  const [draft, setDraft] = useState<Draft | null>(null);
  const [saving, setSaving] = useState(false);
  const [linkFrom, setLinkFrom] = useState<string | null>(null);
  const [error, setError] = useState("");

  const canvasRef = useRef<HTMLCanvasElement>(null);
  const surfaceRef = useRef<HTMLDivElement>(null);
  const hitsRef = useRef<HitRegion[]>([]);
  const bodiesRef = useRef<Map<string, Body>>(new Map());
  const positionsRef = useRef<Map<string, Point>>(new Map());
  const dragRef = useRef<{ x: number; y: number; ox: number; oy: number; moved: boolean } | null>(null);
  const frameRef = useRef<number | null>(null);

  const load = useCallback(async () => {
    try {
      const [nextMap, nextAreas, nextReview] = await Promise.all([
        invoke<SkillMap>("skill_map", { atMs: null }),
        invoke<LifeArea[]>("list_life_areas"),
        invoke<ReviewQueue>("review_queue", { idleDays: 7 }),
      ]);
      setMap(nextMap);
      setAreas(nextAreas);
      setReview(nextReview);
      setError("");
    } catch (reason) {
      if ("__TAURI_INTERNALS__" in window) setError(String(reason));
    }
  }, []);

  useEffect(() => {
    void load();
    const unlisten = listen("lifeindex-updated", () => void load());
    return () => {
      void unlisten.then((stop) => stop());
    };
  }, [load]);

  // 生长史只在真要播放时才取（它要在每个变化时刻跑一遍地图，不该在打开页面时白算）。
  const bornAt = useMemo(() => {
    const times = map.nodes.map((node) => node.created_at);
    return times.length ? Math.min(...times) : Date.now() - 30 * DAY;
  }, [map.nodes]);

  const loadGrowth = useCallback(async () => {
    if (growth) return growth;
    try {
      const next = await invoke<Growth>("skill_tree_growth", {
        fromMs: bornAt - DAY,
        toMs: Date.now(),
      });
      setGrowth(next);
      return next;
    } catch (reason) {
      setError(String(reason));
      return null;
    }
  }, [bornAt, growth]);

  const timeline: Timeline | null = useMemo(() => buildTimeline(growth), [growth]);
  const frame: Frame | null = useMemo(
    () => (timeline && playAt !== null ? frameAt(timeline, playAt) : null),
    [timeline, playAt],
  );

  useEffect(() => {
    const node = surfaceRef.current;
    if (!node) return;
    const observer = new ResizeObserver(([entry]) => {
      setSize({
        width: Math.max(360, entry.contentRect.width),
        height: Math.max(360, entry.contentRect.height),
      });
    });
    observer.observe(node);
    return () => observer.disconnect();
  }, []);

  const view: Viewport = useMemo(
    () => ({ width: size.width, height: size.height, scale, offsetX: offset.x, offsetY: offset.y }),
    [size, scale, offset],
  );

  // 物理体：Core 给锚点（扇区角 + 环半径），弹簧只在扇区角区间与环 ±slack 内游走。
  // 播放时用历史进度算锚点——树投影下半径只看依赖深度（位置不动），
  // 雷达投影下半径就是掌握度，节点必须随回放一起向外走，否则会和多边形对不上。
  useEffect(() => {
    const next = new Map<string, Body>();
    const sectors = map.sectors;
    for (const node of map.nodes) {
      if (node.parent_id && node.kind === "milestone") continue; // 卫星跟着父节点，不参与松弛
      const sector = sectors[node.sector];
      if (!sector) continue;
      const progress = stateAt(frame, node.id)?.progress ?? node.progress;
      const target = place(node, sector, map.max_depth, progress, projection);
      const existing = bodiesRef.current.get(node.id);
      const body = createBody(
        node.id,
        target,
        { start: sector.start_angle, end: sector.end_angle },
        nodeSize(node),
        node.depends_on,
      );
      // 已有节点保留当前位置，只换锚点——切投影时是"弹过去"，不是重排。
      if (existing) {
        body.angle = existing.angle;
        body.radius = existing.radius;
        body.va = existing.va;
        body.vr = existing.vr;
      }
      next.set(node.id, body);
    }
    bodiesRef.current = next;
  }, [map, projection, frame]);

  // 每帧推进一步物理并重绘：看得见它"落定"，且同输入必收敛到同一处。
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const dpr = Math.min(window.devicePixelRatio || 1, 2);
    canvas.width = Math.round(size.width * dpr);
    canvas.height = Math.round(size.height * dpr);
    canvas.style.width = `${size.width}px`;
    canvas.style.height = `${size.height}px`;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;

    const render = () => {
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      const bodies = Array.from(bodiesRef.current.values());
      // aspect：把归一化半径换算成屏幕像素，斥力才在视觉上等距。
      const base = Math.min(size.width, size.height) / 2;
      step(bodies, Math.max(1, base * scale));
      const positions = new Map<string, Point>();
      for (const body of bodies) {
        positions.set(body.id, toScreen({ angle: body.angle, radius: body.radius }, view));
      }
      positionsRef.current = positions;
      hitsRef.current = draw(ctx, {
        map,
        positions,
        view,
        t: projection,
        frame,
        selectedId,
        hoverId,
        expandMilestones: scale >= EXPAND_AT,
        showLabels: scale >= 0.85,
      });
      frameRef.current = requestAnimationFrame(render);
    };
    frameRef.current = requestAnimationFrame(render);
    return () => {
      if (frameRef.current !== null) cancelAnimationFrame(frameRef.current);
    };
  }, [map, view, size, scale, projection, frame, selectedId, hoverId]);

  // 播放：把时间轴推着走，到头停下。
  useEffect(() => {
    if (!playing || playAt === null) return;
    const to = Date.now();
    const timer = window.setInterval(() => {
      setPlayAt((current) => {
        if (current === null) return current;
        const stepMs = Math.max(DAY / 2, (to - bornAt) / 240);
        const next = current + stepMs;
        if (next >= to) {
          setPlaying(false);
          return to;
        }
        return next;
      });
    }, 40);
    return () => window.clearInterval(timer);
  }, [playing, playAt, bornAt]);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const onWheel = (event: WheelEvent) => {
      event.preventDefault();
      setScale((current) => clamp(current * Math.exp(-event.deltaY * 0.0016), 0.45, 3.2));
    };
    canvas.addEventListener("wheel", onWheel, { passive: false });
    return () => canvas.removeEventListener("wheel", onWheel);
  }, []);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      if (linkFrom) setLinkFrom(null);
      else setSelectedId(null);
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [linkFrom]);

  const selected = map.nodes.find((node) => node.id === selectedId) ?? null;
  const titleOf = useCallback(
    (id: string) => map.nodes.find((node) => node.id === id)?.title ?? id,
    [map.nodes],
  );

  async function saveDraft() {
    if (!draft?.title.trim()) return;
    setSaving(true);
    try {
      await invoke("upsert_life_item", { input: payload({ ...draft, title: draft.title.trim() }) });
      setDraft(null);
      setGrowth(null);
      await load();
    } catch (reason) {
      setError(String(reason));
    } finally {
      setSaving(false);
    }
  }

  async function editNode(node: SkillNode) {
    try {
      const items = await invoke<LifeItem[]>("list_life_items", { includeArchived: false });
      const item = items.find((candidate) => candidate.id === node.id);
      if (item) setDraft(toDraft(item));
    } catch (reason) {
      setError(String(reason));
    }
  }

  async function toggleMilestone(node: SkillNode) {
    try {
      const items = await invoke<LifeItem[]>("list_life_items", { includeArchived: false });
      const item = items.find((candidate) => candidate.id === node.id);
      if (!item) return;
      const next = toDraft(item);
      next.status = item.status === "done" ? "active" : "done";
      await invoke("upsert_life_item", { input: payload(next) });
      setGrowth(null);
      await load();
    } catch (reason) {
      setError(String(reason));
    }
  }

  async function linkTo(target: SkillNode) {
    if (!linkFrom || linkFrom === target.id) {
      setLinkFrom(null);
      return;
    }
    try {
      // 依赖者 depends_on 前置：先点的是要解锁的技能，后点的是它的前置。
      await invoke("link_life_items", {
        fromItemId: linkFrom,
        toItemId: target.id,
        relation: "depends_on",
        sortOrder: 0,
      });
      setLinkFrom(null);
      setGrowth(null);
      await load();
    } catch (reason) {
      setError(String(reason));
      setLinkFrom(null);
    }
  }

  const pending =
    (review?.no_success_criteria.length ?? 0) +
    (review?.undecomposed.length ?? 0) +
    (review?.due_review.length ?? 0);

  return (
    <section className="skilltree-screen animate-in">
      <header className="skilltree-heading">
        <div>
          <p className="eyebrow">SKILL TREE</p>
          <h1>人生技能树</h1>
          <p>角度是责任领域，半径是要先会什么，填充是已经会了多少。</p>
        </div>
        <div className="skilltree-stat">
          <Network size={14} />
          <span>
            已掌握 {map.attained}/{map.total}
          </span>
          <small>{map.nodes.length} 节点 · {map.edges.length} 前置 · {map.ideas.length} 想法</small>
        </div>
      </header>

      <div className="skilltree-toolbar">
        <div className="skilltree-projection" role="group" aria-label="投影">
          <button className={projection < 0.5 ? "active" : ""} onClick={() => setProjection(0)}>
            <Network size={13} /> 依赖树
          </button>
          <button className={projection >= 0.5 ? "active" : ""} onClick={() => setProjection(1)}>
            <Radar size={13} /> 领域雷达
          </button>
        </div>
        <input
          aria-label="投影插值"
          type="range"
          min={0}
          max={1}
          step="0.01"
          value={projection}
          onChange={(event) => setProjection(Number(event.target.value))}
        />
        <button
          className={linkFrom ? "active" : ""}
          title="连前置：先点要解锁的技能，再点它的前置"
          onClick={() => setLinkFrom(linkFrom ? null : selectedId)}
          disabled={!selectedId && !linkFrom}
        >
          <Link2 size={13} /> {linkFrom ? "选前置…" : "连前置"}
        </button>
        <button onClick={() => setDraft({ ...emptyDraft, kind: "skill", status: "active" })}>
          <Plus size={13} /> 新技能
        </button>
        <button
          className="skilltree-now"
          onClick={() => {
            setScale(1);
            setOffset({ x: 0, y: 0 });
            setPlayAt(null);
            setPlaying(false);
          }}
        >
          <LocateFixed size={13} /> 现在
        </button>
      </div>

      <div className="skilltree-surface" ref={surfaceRef}>
        <canvas
          ref={canvasRef}
          onPointerDown={(event) => {
            event.currentTarget.setPointerCapture(event.pointerId);
            dragRef.current = { x: event.clientX, y: event.clientY, ox: offset.x, oy: offset.y, moved: false };
          }}
          onPointerMove={(event) => {
            const rect = event.currentTarget.getBoundingClientRect();
            const drag = dragRef.current;
            if (drag) {
              const dx = event.clientX - drag.x;
              const dy = event.clientY - drag.y;
              if (Math.abs(dx) > 3 || Math.abs(dy) > 3) drag.moved = true;
              setOffset({ x: drag.ox + dx, y: drag.oy + dy });
              return;
            }
            const hit = hitTest(hitsRef.current, event.clientX - rect.left, event.clientY - rect.top);
            setHoverId(hit?.node?.id ?? null);
          }}
          onPointerUp={(event) => {
            const drag = dragRef.current;
            dragRef.current = null;
            if (!drag || drag.moved) return;
            const rect = event.currentTarget.getBoundingClientRect();
            const hit = hitTest(hitsRef.current, event.clientX - rect.left, event.clientY - rect.top);
            if (!hit?.node) {
              setSelectedId(null);
              return;
            }
            if (linkFrom) void linkTo(hit.node);
            else setSelectedId(hit.node.id);
          }}
          onPointerCancel={() => {
            dragRef.current = null;
          }}
        />
        {!map.nodes.length && (
          <div className="skilltree-empty">
            <Sparkles size={20} className="text-accent" />
            <strong>技能树还是空的</strong>
            <span>
              新建一个 <b>技能</b>，给它挂几个可判定的 <b>里程碑</b>，再用「连前置」把要先会的能力连起来。
              领域是背景扇区，在设置里维护。
            </span>
          </div>
        )}
        {error && <p className="skilltree-error">{error}</p>}
      </div>

      <div className="skilltree-playback">
        <button
          onClick={async () => {
            const data = await loadGrowth();
            if (!data) return;
            if (playAt === null) setPlayAt(data.from_ms);
            setPlaying(!playing);
          }}
          title="按时间播放这棵树的生长"
        >
          {playing ? <Pause size={13} /> : <Play size={13} />}
        </button>
        <input
          aria-label="时间"
          type="range"
          min={bornAt - DAY}
          max={Date.now()}
          step={DAY / 4}
          value={playAt ?? Date.now()}
          onChange={async (event) => {
            const value = Number(event.target.value);
            await loadGrowth();
            setPlaying(false);
            setPlayAt(value);
          }}
        />
        <span className="skilltree-clock">
          {playAt === null
            ? "现在"
            : new Date(playAt).toLocaleDateString("zh-CN", { year: "numeric", month: "short", day: "numeric" })}
        </span>
        {playAt !== null && (
          <button
            onClick={() => {
              setPlayAt(null);
              setPlaying(false);
            }}
          >
            回到现在
          </button>
        )}
      </div>

      {selected && (
        <aside className="skilltree-detail">
          <div className="skilltree-detail-head">
            <div>
              <strong>{selected.title}</strong>
              <small>
                {selected.kind === "skill" ? "技能" : "里程碑"} ·{" "}
                {selected.state === "attained"
                  ? "已掌握"
                  : selected.state === "in_progress"
                    ? "在进展"
                    : selected.state === "available"
                      ? "可解锁"
                      : "锁定"}{" "}
                · {Math.round(selected.progress * 100)}%
                {selected.total_leaves > 1 ? ` · Lv ${selected.done_leaves}/${selected.total_leaves}` : ""}
              </small>
            </div>
            <button onClick={() => setSelectedId(null)}>×</button>
          </div>
          {selected.blocked_by.length > 0 && (
            <p className="skilltree-blocked">需先完成：{selected.blocked_by.map(titleOf).join("、")}</p>
          )}
          {selected.success_criteria ? (
            <p className="skilltree-criteria">完成条件：{selected.success_criteria}</p>
          ) : (
            <p className="skilltree-criteria warn">还没有可判定的完成条件——它永远无法收敛。</p>
          )}
          {selected.target_value !== null && (
            <p className="skilltree-metric">
              度量：{selected.current_value ?? 0} / {selected.target_value} {selected.unit ?? ""}
            </p>
          )}
          {/* 事实进展在这里列，不上画布——否则地图会变成任务清单。 */}
          <ul className="skilltree-children">
            {map.nodes
              .filter((node) => node.parent_id === selected.id)
              .map((child) => (
                <li key={child.id}>
                  <button
                    className={child.progress >= 1 ? "done" : ""}
                    onClick={() => void toggleMilestone(child)}
                    title={child.progress >= 1 ? "重新打开" : "标记完成"}
                  >
                    <Circle size={9} />
                  </button>
                  <span>{child.title}</span>
                  {child.due_at_ms && <em>{new Date(child.due_at_ms).toLocaleDateString("zh-CN")}</em>}
                </li>
              ))}
            {!map.nodes.some((node) => node.parent_id === selected.id) && (
              <li className="muted">还没有里程碑。拆一个可判定的检查点出来，进度才有刻度。</li>
            )}
          </ul>
          <div className="skilltree-detail-actions">
            <button onClick={() => void editNode(selected)}>编辑</button>
            <button
              onClick={() =>
                setDraft({
                  ...emptyDraft,
                  kind: "milestone",
                  status: "active",
                  area_id: selected.area_id ?? "",
                })
              }
            >
              加里程碑
            </button>
            <button onClick={() => setLinkFrom(selected.id)}>连前置</button>
          </div>
          {draft?.kind === "milestone" && !draft.id && (
            <p className="skilltree-hint">保存后用「连前置」把它挂到技能下（contains 由 link 建立）。</p>
          )}
        </aside>
      )}

      {pending > 0 && (
        <footer className="skilltree-review">
          <span>{pending} 项待回顾</span>
          {review?.no_success_criteria.length ? <em>{review.no_success_criteria.length} 个缺可判定标准</em> : null}
          {review?.undecomposed.length ? <em>{review.undecomposed.length} 个还没拆</em> : null}
          {review?.due_review.length ? <em>{review.due_review.length} 个想法该毕业审查</em> : null}
        </footer>
      )}

      {draft && (
        <ItemDraftDialog
          draft={draft}
          areas={areas}
          saving={saving}
          onChange={setDraft}
          onClose={() => setDraft(null)}
          onSave={() => void saveDraft()}
        />
      )}
    </section>
  );
}
