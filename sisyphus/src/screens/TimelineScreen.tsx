import { useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Layers3, LocateFixed, Minus, Plus } from "lucide-react";

const MIN_SPAN = 15 * 60_000;
const MAX_SPAN = 10 * 365.25 * 86_400_000;
const DAY = 86_400_000;

type Detail = "minute" | "day" | "week" | "life";

interface TimelineEvent {
  /// 显著性等级：0 人生尺度仍可见，3 只在日/分钟尺度可见（LOD 过滤由后端做）
  level?: number;
  id: string;
  kind:
    | "behavior"
    | "intervention"
    | "system"
    | "capture"
    | "goal"
    | "task"
    | "reminder"
    | "knowledge"
    | "rule";
  start_ms: number;
  end_ms: number;
  title: string;
  category: string | null;
  detail: string | null;
  severity: string | null;
}

interface DaySummary {
  date: string;
  start_ms: number;
  observed_ms: number;
  focus_ms: number;
  entertainment_ms: number;
  neutral_ms: number;
  intervention_count: number;
  state_score: number;
}

/// 预聚合条带：粗尺度的主数据，桶粒度随可见跨度自动变粗（day → week → month）。
/// 代价是 O(可见桶数) 而非 O(事件数)，这是"无极缩放"能成立的前提。
interface TimeBand {
  bucket_start_ms: number;
  bucket_end_ms: number;
  observed_ms: number;
  focus_ms: number;
  entertainment_ms: number;
  neutral_ms: number;
  top_category: string | null;
}

/// 长期计划图层（LifeDB）：目标/项目/技能的跨度 + 里程碑点，progress 由 Core 确定性算出。
interface PlanSpan {
  id: string;
  kind: "life_goal" | "life_project" | "life_milestone" | "life_skill";
  title: string;
  track: string;
  status: string;
  start_ms: number;
  end_ms: number;
  progress: number;
  level: number;
}

interface TimelineResponse {
  start_ms: number;
  end_ms: number;
  detail: Detail;
  /// 本次实际使用的桶粒度：day | week | month | none
  bucket: string;
  events: TimelineEvent[];
  days: DaySummary[];
  /// TODO(UI)：条带与计划图层的绘制是下一步的前端工作，后端数据已就绪。
  bands: TimeBand[];
  plans: PlanSpan[];
  truncated: boolean;
  has_long_term_source: boolean;
}

interface HitRegion {
  x1: number;
  x2: number;
  y1: number;
  y2: number;
  event: TimelineEvent;
}

const EMPTY: TimelineResponse = {
  start_ms: 0,
  end_ms: 0,
  detail: "day",
  bucket: "none",
  events: [],
  days: [],
  bands: [],
  plans: [],
  truncated: false,
  has_long_term_source: false,
};

export default function TimelineScreen() {
  const [center, setCenter] = useState(() => Date.now());
  const [span, setSpan] = useState(DAY);
  const [data, setData] = useState<TimelineResponse>(EMPTY);
  const [size, setSize] = useState({ width: 900, height: 430 });
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");
  const [selected, setSelected] = useState<TimelineEvent | null>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const surfaceRef = useRef<HTMLDivElement>(null);
  const hitRegions = useRef<HitRegion[]>([]);
  const viewRef = useRef({ center, span });
  const dragRef = useRef<{ x: number; center: number; moved: boolean } | null>(null);

  viewRef.current = { center, span };
  const detail = useMemo(() => detailForSpan(span), [span]);
  const start = center - span / 2;
  const end = center + span / 2;

  useEffect(() => {
    const node = surfaceRef.current;
    if (!node) return;
    const observer = new ResizeObserver(([entry]) => {
      const width = Math.max(320, entry.contentRect.width);
      const height = Math.max(300, entry.contentRect.height);
      setSize({ width, height });
    });
    observer.observe(node);
    return () => observer.disconnect();
  }, []);

  useEffect(() => {
    let cancelled = false;
    const timer = window.setTimeout(async () => {
      setLoading(true);
      try {
        const response = await invoke<TimelineResponse>("query_timeline", {
          startMs: Math.round(start),
          endMs: Math.round(end),
          detail,
          maxItems: 1800,
        });
        if (!cancelled) {
          setData(response);
          setError("");
        }
      } catch (reason) {
        if (!cancelled && "__TAURI_INTERNALS__" in window) setError(String(reason));
      } finally {
        if (!cancelled) setLoading(false);
      }
    }, 120);
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [start, end, detail]);

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
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    hitRegions.current = drawTimeline(ctx, size.width, size.height, center, span, data);
  }, [size, center, span, data]);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const onWheel = (event: WheelEvent) => {
      event.preventDefault();
      const { center: currentCenter, span: currentSpan } = viewRef.current;
      if (event.shiftKey && !event.ctrlKey && !event.metaKey) {
        setCenter(currentCenter + (event.deltaY / Math.max(1, size.width)) * currentSpan);
        return;
      }
      const rect = canvas.getBoundingClientRect();
      const ratio = clamp((event.clientX - rect.left) / rect.width, 0, 1);
      const anchor = currentCenter + (ratio - 0.5) * currentSpan;
      const nextSpan = clamp(currentSpan * Math.exp(event.deltaY * 0.0017), MIN_SPAN, MAX_SPAN);
      setSpan(nextSpan);
      setCenter(anchor - (ratio - 0.5) * nextSpan);
    };
    canvas.addEventListener("wheel", onWheel, { passive: false });
    return () => canvas.removeEventListener("wheel", onWheel);
  }, [size.width]);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") setSelected(null);
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, []);

  function zoom(factor: number) {
    setSpan((current) => clamp(current * factor, MIN_SPAN, MAX_SPAN));
  }

  return (
    <section className="timeline-screen animate-in">
      <header className="timeline-heading">
        <div>
          <p className="eyebrow">INFINITE TIMELINE</p>
          <h1>无极时间轴</h1>
          <p>每个微小行为，都在同一条时间上聚合成人生方向。</p>
        </div>
        <div className="timeline-lod">
          <Layers3 size={14} />
          <span>{detailLabel(detail)}</span>
          <small>{formatSpan(span)}</small>
        </div>
      </header>

      <div className="timeline-toolbar">
        <button onClick={() => zoom(0.55)} aria-label="放大时间轴"><Plus size={15} /></button>
        <input
          aria-label="时间尺度"
          type="range"
          min={Math.log10(MIN_SPAN)}
          max={Math.log10(MAX_SPAN)}
          step="0.001"
          value={Math.log10(span)}
          onChange={(event) => setSpan(10 ** Number(event.target.value))}
        />
        <button onClick={() => zoom(1.8)} aria-label="缩小时间轴"><Minus size={15} /></button>
        <button
          className="timeline-now"
          onClick={() => {
            setCenter(Date.now());
            setSpan(DAY);
          }}
        >
          <LocateFixed size={14} /> 今天
        </button>
      </div>

      <div className="timeline-surface" ref={surfaceRef}>
        <canvas
          ref={canvasRef}
          onPointerDown={(event) => {
            event.currentTarget.setPointerCapture(event.pointerId);
            dragRef.current = { x: event.clientX, center, moved: false };
          }}
          onPointerMove={(event) => {
            const drag = dragRef.current;
            if (!drag) return;
            const delta = event.clientX - drag.x;
            if (Math.abs(delta) > 3) drag.moved = true;
            setCenter(drag.center - (delta / size.width) * span);
          }}
          onPointerUp={(event) => {
            const drag = dragRef.current;
            dragRef.current = null;
            if (!drag || drag.moved) return;
            const rect = event.currentTarget.getBoundingClientRect();
            const x = event.clientX - rect.left;
            const y = event.clientY - rect.top;
            const hit = hitRegions.current.find((region) => x >= region.x1 && x <= region.x2 && y >= region.y1 && y <= region.y2);
            setSelected(hit?.event || null);
          }}
          onPointerCancel={() => { dragRef.current = null; }}
        />
        <div className="timeline-edge timeline-edge-left" />
        <div className="timeline-edge timeline-edge-right" />
        {loading && <span className="timeline-loading">更新中</span>}
        {detail === "life" && !data.has_long_term_source && (
          <div className="timeline-life-empty">
            <strong>长期方向保持为空</strong>
            <span>在 LifeIndex 新建或从 Notion 同步后，这里会呈现你的长期计划。</span>
          </div>
        )}
      </div>

      <footer className="timeline-footer">
        <span>{formatBoundary(start)}</span>
        <span>滚轮缩放 · 拖拽平移 · Shift + 滚轮横移</span>
        <span>{formatBoundary(end)}</span>
      </footer>

      {selected && (
        <button className="timeline-selection" onClick={() => setSelected(null)}>
          <span style={{ background: eventColor(selected) }} />
          <div>
            <strong>{selected.title}</strong>
            <small>{kindLabel(selected.kind)} · {formatEventRange(selected)}{selected.detail && selected.detail !== selected.kind ? ` · ${selected.detail}` : ""}</small>
          </div>
          <kbd>esc</kbd>
        </button>
      )}
      {data.truncated && <p className="timeline-note">当前窗口事件较多，已按可见范围裁剪；继续放大可查看细节。</p>}
      {error && <p className="timeline-error">时间轴暂时无法读取：{error}</p>}
    </section>
  );
}

function drawTimeline(
  ctx: CanvasRenderingContext2D,
  width: number,
  height: number,
  center: number,
  span: number,
  data: TimelineResponse,
): HitRegion[] {
  const start = center - span / 2;
  const end = center + span / 2;
  const xAt = (time: number) => ((time - start) / span) * width;
  const curl = (x: number, base: number, amount = 16) => {
    const edge = Math.abs((x / width - 0.5) * 2);
    return base + edge ** 3 * amount;
  };
  const hits: HitRegion[] = [];

  ctx.clearRect(0, 0, width, height);
  const bg = ctx.createLinearGradient(0, 0, 0, height);
  bg.addColorStop(0, "#0d0e11");
  bg.addColorStop(1, "#090a0c");
  ctx.fillStyle = bg;
  ctx.fillRect(0, 0, width, height);

  const tick = tickForSpan(span);
  const firstTick = Math.floor(start / tick) * tick;
  ctx.font = "10px ui-monospace, SFMono-Regular, Menlo, monospace";
  ctx.textAlign = "center";
  for (let time = firstTick; time <= end + tick; time += tick) {
    const x = xAt(time);
    if (x < -20 || x > width + 20) continue;
    ctx.strokeStyle = "rgba(255,255,255,.055)";
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.moveTo(x, 42);
    ctx.lineTo(x, height - 36);
    ctx.stroke();
    ctx.fillStyle = "rgba(233,233,236,.38)";
    ctx.fillText(formatTick(time, tick), x, 26);
  }

  const nowX = xAt(Date.now());
  if (nowX >= 0 && nowX <= width) {
    ctx.strokeStyle = "rgba(139,147,255,.6)";
    ctx.setLineDash([3, 5]);
    ctx.beginPath();
    ctx.moveTo(nowX, 38);
    ctx.lineTo(nowX, height - 28);
    ctx.stroke();
    ctx.setLineDash([]);
  }

  const rawOpacity = 1 - smoothstep(2 * DAY, 12 * DAY, span);
  const dayOpacity = smoothstep(2 * DAY, 12 * DAY, span) * (1 - smoothstep(90 * DAY, 240 * DAY, span));

  if (rawOpacity > 0.02) {
    ctx.globalAlpha = rawOpacity;
    for (const event of data.events) {
      const x1 = xAt(event.start_ms);
      const x2 = xAt(Math.max(event.end_ms, event.start_ms + span / width * 4));
      if (x2 < 0 || x1 > width) continue;
      if (event.kind === "intervention") {
        const x = xAt(event.start_ms);
        const y = curl(x, 65, 9);
        ctx.fillStyle = event.severity === "high" ? "#fbbf24" : "#8b93ff";
        ctx.beginPath();
        ctx.moveTo(x, y - 6);
        ctx.lineTo(x + 6, y);
        ctx.lineTo(x, y + 6);
        ctx.lineTo(x - 6, y);
        ctx.closePath();
        ctx.fill();
        hits.push({ x1: x - 9, x2: x + 9, y1: y - 9, y2: y + 9, event });
        continue;
      }
      // artifact 里程碑（目标/任务/提醒/知识/规则）与 capture：点事件，画成小圆标记，独立轨道。
      if (event.category === "milestone" || event.kind === "capture") {
        const x = xAt(event.start_ms);
        const y = curl(x, 86, 7);
        ctx.fillStyle = eventColor(event);
        ctx.beginPath();
        ctx.arc(x, y, 4.5, 0, Math.PI * 2);
        ctx.fill();
        hits.push({ x1: x - 7, x2: x + 7, y1: y - 7, y2: y + 7, event });
        continue;
      }
      const lane = laneFor(event.category);
      const baseY = 108 + lane * 48;
      const y = curl((x1 + x2) / 2, baseY);
      const barWidth = Math.max(4, x2 - x1);
      const barHeight = span < 6 * 60 * 60_000 ? 24 : 17;
      ctx.fillStyle = eventColor(event);
      roundRect(ctx, x1, y, barWidth, barHeight, 5);
      ctx.fill();
      if (barWidth > 52) {
        ctx.save();
        ctx.beginPath();
        ctx.rect(x1 + 5, y, barWidth - 10, barHeight);
        ctx.clip();
        ctx.fillStyle = "rgba(8,9,11,.82)";
        ctx.font = "10px -apple-system, BlinkMacSystemFont, sans-serif";
        ctx.textAlign = "left";
        ctx.fillText(shortTitle(event.title), x1 + 7, y + barHeight / 2 + 3.5);
        ctx.restore();
      }
      hits.push({ x1, x2: x1 + barWidth, y1: y - 3, y2: y + barHeight + 3, event });
    }
    ctx.globalAlpha = 1;
  }

  if (dayOpacity > 0.02 && data.days.length) {
    ctx.globalAlpha = dayOpacity;
    let previous: { x: number; y: number } | null = null;
    for (const day of data.days) {
      const x = xAt(day.start_ms + DAY / 2);
      if (x < -30 || x > width + 30) continue;
      const y = curl(x, height - 86 - (day.state_score / 100) * Math.min(180, height * 0.42), 12);
      const barWidth = Math.max(5, Math.min(28, (DAY / span) * width * 0.62));
      ctx.fillStyle = scoreColor(day.state_score);
      roundRect(ctx, x - barWidth / 2, y, barWidth, height - 56 - y, Math.min(5, barWidth / 2));
      ctx.fill();
      if (previous) {
        ctx.strokeStyle = "rgba(139,147,255,.32)";
        ctx.lineWidth = 1.5;
        ctx.beginPath();
        ctx.moveTo(previous.x, previous.y);
        ctx.lineTo(x, y);
        ctx.stroke();
      }
      previous = { x, y };
      if ((DAY / span) * width > 38) {
        ctx.fillStyle = "rgba(233,233,236,.62)";
        ctx.font = "10px ui-monospace, monospace";
        ctx.textAlign = "center";
        ctx.fillText(String(day.state_score), x, y - 7);
      }
    }
    ctx.globalAlpha = 1;
  }

  ctx.strokeStyle = "rgba(255,255,255,.12)";
  ctx.lineWidth = 1;
  ctx.beginPath();
  for (let x = 0; x <= width; x += 8) {
    const y = curl(x, height - 38, 12);
    if (x === 0) ctx.moveTo(x, y); else ctx.lineTo(x, y);
  }
  ctx.stroke();
  return hits;
}

function detailForSpan(span: number): Detail {
  if (span <= 12 * 60 * 60_000) return "minute";
  if (span <= 8 * DAY) return "day";
  if (span <= 180 * DAY) return "week";
  return "life";
}

function detailLabel(detail: Detail) {
  return { minute: "事件细节", day: "行为区间", week: "每日状态", life: "长期方向" }[detail];
}

function tickForSpan(span: number) {
  const candidates = [5 * 60_000, 15 * 60_000, 60 * 60_000, 3 * 60 * 60_000, 6 * 60 * 60_000, 12 * 60 * 60_000, DAY, 7 * DAY, 30 * DAY, 90 * DAY, 365 * DAY];
  return candidates.find((candidate) => span / candidate <= 11) || 365 * DAY;
}

function formatTick(ms: number, tick: number) {
  const date = new Date(ms);
  if (tick < DAY) return date.toLocaleTimeString("zh-CN", { hour: "2-digit", minute: "2-digit" });
  if (tick < 30 * DAY) return date.toLocaleDateString("zh-CN", { month: "short", day: "numeric" });
  if (tick < 365 * DAY) return date.toLocaleDateString("zh-CN", { year: "2-digit", month: "short" });
  return String(date.getFullYear());
}

function formatSpan(span: number) {
  if (span < 60 * 60_000) return `${Math.round(span / 60_000)} 分钟`;
  if (span < 2 * DAY) return `${(span / 3_600_000).toFixed(span < 12 * 3_600_000 ? 1 : 0)} 小时`;
  if (span < 120 * DAY) return `${Math.round(span / DAY)} 天`;
  if (span < 730 * DAY) return `${(span / (30 * DAY)).toFixed(1)} 个月`;
  return `${(span / (365 * DAY)).toFixed(1)} 年`;
}

function formatBoundary(time: number) {
  return new Date(time).toLocaleDateString("zh-CN", { year: "numeric", month: "short", day: "numeric" });
}

function formatEventRange(event: TimelineEvent) {
  const start = new Date(event.start_ms);
  const end = new Date(event.end_ms);
  const left = start.toLocaleString("zh-CN", { month: "numeric", day: "numeric", hour: "2-digit", minute: "2-digit" });
  if (event.end_ms <= event.start_ms) return left;
  return `${left}–${end.toLocaleTimeString("zh-CN", { hour: "2-digit", minute: "2-digit" })}`;
}

function laneFor(category: string | null) {
  if (category?.startsWith("entertainment")) return 2;
  if (category?.includes("communication") || category?.includes("social")) return 1;
  return 0;
}

function kindLabel(kind: TimelineEvent["kind"]) {
  switch (kind) {
    case "intervention":
      return "干预";
    case "capture":
      return "记录";
    case "goal":
      return "目标";
    case "task":
      return "任务";
    case "reminder":
      return "提醒";
    case "knowledge":
      return "知识";
    case "rule":
      return "规则";
    case "system":
      return "系统";
    default:
      return "行为";
  }
}

function eventColor(event: TimelineEvent) {
  if (event.kind === "intervention") return event.severity === "high" ? "#fbbf24" : "#8b93ff";
  switch (event.kind) {
    case "capture":
      return "#7dd3fc";
    case "goal":
      return "#34d399";
    case "task":
      return "#60a5fa";
    case "reminder":
      return "#f0abfc";
    case "knowledge":
      return "#a78bfa";
    case "rule":
      return "#fb923c";
  }
  if (event.category?.startsWith("entertainment")) return "#e99d54";
  if (event.category?.includes("social") || event.category?.includes("communication")) return "#d178b3";
  if (event.category) return "#69c4a4";
  return "#657084";
}

function scoreColor(score: number) {
  if (score >= 70) return "rgba(52,211,153,.72)";
  if (score >= 45) return "rgba(139,147,255,.68)";
  return "rgba(251,191,36,.65)";
}

function shortTitle(title: string) {
  const parts = title.split(".");
  return parts[parts.length - 1] || title;
}

function smoothstep(a: number, b: number, value: number) {
  const x = clamp((value - a) / (b - a), 0, 1);
  return x * x * (3 - 2 * x);
}

function clamp(value: number, min: number, max: number) {
  return Math.min(max, Math.max(min, value));
}

function roundRect(ctx: CanvasRenderingContext2D, x: number, y: number, width: number, height: number, radius: number) {
  const r = Math.min(radius, Math.abs(width) / 2, Math.abs(height) / 2);
  ctx.beginPath();
  ctx.roundRect(x, y, width, height, r);
}
