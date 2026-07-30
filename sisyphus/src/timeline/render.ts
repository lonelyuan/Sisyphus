/// Canvas 渲染。
///
/// 三条渲染约束：
/// 1. **同一条轨道在不同尺度换表示，但位置不变**。轨道的 y 区间是稳定锚点，
///    只有内部画法随 LOD 切换（会话条 → 密度条 → 格子），读者不必重新找东西在哪。
/// 2. **不做会破坏可比性的装饰**。此前边缘上翘（curl）让同样 60 分钟的会话
///    在画布中间和边缘的竖直位置不同，等高比较直接失效——已删除。
/// 3. **折叠只是换投影**。焦点轨道用 [`Projection.place`] 画一次即可，
///    线性/转场/折叠三种状态走同一段代码。

import type { AxisCell, PlanSpan, TimelineEvent, TimelineResponse } from "./types";
import { DAY, HOUR } from "./types";
import { Projection } from "./projection";
import { clamp, smoothstep, type Layout } from "./layout";
import {
  COLOR,
  assignLanes,
  cellPaint,
  categoryColor,
  effectiveVisible,
  eventColor,
  laneCount,
  scoreColor,
  type TrackId,
  type TrackViewState,
} from "./tracks";

export interface HitRegion {
  x1: number;
  x2: number;
  y1: number;
  y2: number;
  event?: TimelineEvent;
  cell?: AxisCell;
  plan?: PlanSpan;
}

export interface Selection {
  kind: "linear" | "phase";
  a: number;
  b: number;
}

export interface Scene {
  layout: Layout;
  projection: Projection;
  data: TimelineResponse;
  tracks: TrackViewState[];
  focus: TrackId;
  selection: Selection | null;
  now: number;
}

const FONT_MONO = "10px ui-monospace, SFMono-Regular, Menlo, monospace";
const FONT_UI = "10px -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif";

export function drawScene(ctx: CanvasRenderingContext2D, scene: Scene): HitRegion[] {
  const { layout, projection: proj, data } = scene;
  const hits: HitRegion[] = [];
  ctx.clearRect(0, 0, layout.width, layout.height);
  ctx.fillStyle = "#0b0c0e";
  ctx.fillRect(0, 0, layout.width, layout.height);

  drawFutureShade(ctx, scene);
  drawGrid(ctx, scene);
  drawRuler(ctx, scene);

  const t = proj.t;
  const visible = effectiveVisible(scene.tracks);
  for (const track of visible) {
    const box = layout.tracks.find((candidate) => candidate.id === track.id);
    if (!box) continue;
    // 焦点轨道用 place() 自己插值（折叠时它是唯一铺满画布的轨道），
    // 其余轨道在折叠过程中淡出——纵向空间被行占满了，留着只会互相压扁。
    const alpha = track.id === scene.focus ? 1 : 1 - t;
    if (alpha < 0.02) continue;
    ctx.save();
    ctx.globalAlpha = alpha;
    drawTrack(ctx, scene, track.id, box.top, box.height, box.collapsed, hits);
    ctx.restore();
  }

  if (t > 0.02 && data.cells.length && scene.focus === "behavior") {
    ctx.save();
    ctx.globalAlpha = t;
    drawCells(ctx, scene, hits);
    ctx.restore();
  }

  drawPlayhead(ctx, scene);
  drawSelection(ctx, scene);
  return hits;
}

// ── 底层图元 ────────────────────────────────────────────────────────────────

function drawFutureShade(ctx: CanvasRenderingContext2D, scene: Scene) {
  const { layout, projection: proj, now } = scene;
  if (proj.folded) return;
  const x = proj.linearX(now);
  if (x >= layout.plotLeft + layout.plotWidth) return;
  const from = Math.max(layout.plotLeft, x);
  const right = layout.plotLeft + layout.plotWidth;
  ctx.save();
  // 只裁到"未来"那一块：斜纹是往左下画的，不裁的话纹理会铺满整个过去区。
  ctx.beginPath();
  ctx.rect(from, layout.plotTop, right - from, layout.plotHeight);
  ctx.clip();
  ctx.fillStyle = "rgba(0,0,0,.28)";
  ctx.fillRect(from, layout.plotTop, right - from, layout.plotHeight);
  // 未来区只有"计划"有意义：这里的空白是信息，不是缺数据。
  ctx.strokeStyle = "rgba(255,255,255,.028)";
  ctx.lineWidth = 1;
  for (let hatch = from; hatch < right + layout.plotHeight; hatch += 9) {
    ctx.beginPath();
    ctx.moveTo(hatch, layout.plotTop);
    ctx.lineTo(hatch - layout.plotHeight, layout.plotTop + layout.plotHeight);
    ctx.stroke();
  }
  ctx.restore();
}

function drawGrid(ctx: CanvasRenderingContext2D, scene: Scene) {
  const { layout, projection: proj, data } = scene;
  ctx.save();
  ctx.lineWidth = 1;
  if (proj.folded) {
    for (const tick of phaseTicks(scene)) {
      ctx.strokeStyle = tick.tier === 0 ? "rgba(255,255,255,.07)" : "rgba(255,255,255,.03)";
      ctx.beginPath();
      ctx.moveTo(Math.round(tick.x) + 0.5, layout.plotTop);
      ctx.lineTo(Math.round(tick.x) + 0.5, layout.plotTop + layout.plotHeight);
      ctx.stroke();
    }
    if (layout.rowHeight >= 9) {
      ctx.strokeStyle = "rgba(255,255,255,.035)";
      for (const row of layout.rows) {
        ctx.beginPath();
        ctx.moveTo(layout.plotLeft, Math.round(row.top) + 0.5);
        ctx.lineTo(layout.plotLeft + layout.plotWidth, Math.round(row.top) + 0.5);
        ctx.stroke();
      }
    }
  } else {
    for (const tick of data.ticks) {
      if (tick.tier !== 0 && !tick.day_start) continue;
      const x = proj.linearX(tick.ms);
      if (x < layout.plotLeft - 2 || x > layout.width + 2) continue;
      ctx.strokeStyle = tick.day_start ? "rgba(255,255,255,.075)" : "rgba(255,255,255,.035)";
      ctx.beginPath();
      ctx.moveTo(Math.round(x) + 0.5, layout.plotTop);
      ctx.lineTo(Math.round(x) + 0.5, layout.plotTop + layout.plotHeight);
      ctx.stroke();
    }
  }
  ctx.restore();
}

interface PhaseTick {
  x: number;
  label: string;
  tier: number;
}

/// 折叠模式下的相位刻度。横轴不再是绝对时间，而是"日内时刻"或"列"。
export function phaseTicks(scene: Scene): PhaseTick[] {
  const { projection: proj, data } = scene;
  const anchor = proj.rows[0]?.start_ms ?? data.start_ms;
  const out: PhaseTick[] = [];
  if (proj.fold === "day") {
    const width = proj.layout.plotWidth;
    const hours = proj.phaseSpan / HOUR;
    const step = width / hours >= 34 ? 1 : width / hours >= 12 ? 3 : 6;
    for (let hour = 0; hour <= hours; hour += step) {
      const at = anchor + hour * HOUR;
      out.push({
        x: proj.phaseX(hour * HOUR),
        label: `${String(new Date(at).getHours()).padStart(2, "0")}`,
        tier: hour % (step * 2) === 0 ? 0 : 1,
      });
    }
    return out;
  }
  if (proj.fold === "week") {
    const names = ["一", "二", "三", "四", "五", "六", "日"];
    for (let col = 0; col < 7; col += 1) {
      out.push({ x: proj.phaseX(col + 0.5), label: names[col], tier: 0 });
    }
    return out;
  }
  if (proj.fold === "month") {
    for (let col = 0; col < proj.cols; col += 1) {
      if (col % 5 !== 0) continue;
      out.push({ x: proj.phaseX(col + 0.5), label: `${col + 1}`, tier: 0 });
    }
    return out;
  }
  // 年折叠：列是"一年里的第几天"，只在月初打标签。
  const firstRow = proj.rows[0];
  if (!firstRow) return out;
  for (const cell of data.cells) {
    if (cell.row !== firstRow.index) continue;
    const date = new Date(cell.start_ms);
    if (date.getDate() !== 1) continue;
    out.push({ x: proj.phaseX(cell.col + 0.5), label: `${date.getMonth() + 1}`, tier: 0 });
  }
  return out;
}

function drawRuler(ctx: CanvasRenderingContext2D, scene: Scene) {
  const { layout, projection: proj, data } = scene;
  ctx.save();
  ctx.fillStyle = "rgba(255,255,255,.022)";
  ctx.fillRect(0, 0, layout.width, layout.rulerH);
  ctx.strokeStyle = "rgba(255,255,255,.09)";
  ctx.lineWidth = 1;
  ctx.beginPath();
  ctx.moveTo(0, layout.rulerH - 0.5);
  ctx.lineTo(layout.width, layout.rulerH - 0.5);
  ctx.stroke();

  ctx.font = FONT_MONO;
  ctx.textAlign = "center";
  ctx.textBaseline = "alphabetic";

  if (proj.folded) {
    for (const tick of phaseTicks(scene)) {
      const height = tick.tier === 0 ? 7 : 4;
      ctx.strokeStyle = tick.tier === 0 ? "rgba(255,255,255,.28)" : "rgba(255,255,255,.14)";
      ctx.beginPath();
      ctx.moveTo(Math.round(tick.x) + 0.5, layout.rulerH - height);
      ctx.lineTo(Math.round(tick.x) + 0.5, layout.rulerH);
      ctx.stroke();
      if (tick.tier === 0 && tick.label) {
        ctx.fillStyle = "rgba(233,233,236,.5)";
        ctx.fillText(tick.label, tick.x, layout.rulerH - 12);
      }
    }
    ctx.restore();
    return;
  }

  let lastLabelRight = -Infinity;
  for (const tick of data.ticks) {
    const x = proj.linearX(tick.ms);
    if (x < -30 || x > layout.width + 30) continue;
    const major = tick.tier === 0;
    const height = major ? (tick.day_start ? 9 : 7) : 4;
    ctx.strokeStyle = tick.day_start
      ? "rgba(255,255,255,.38)"
      : major
        ? "rgba(255,255,255,.24)"
        : "rgba(255,255,255,.12)";
    ctx.beginPath();
    ctx.moveTo(Math.round(x) + 0.5, layout.rulerH - height);
    ctx.lineTo(Math.round(x) + 0.5, layout.rulerH);
    ctx.stroke();
    if (!major || !tick.label) continue;
    // 标签防重叠：宁可少画一个标签，也不要糊成一片。
    const width = ctx.measureText(tick.label).width;
    if (x - width / 2 < lastLabelRight + 6) continue;
    lastLabelRight = x + width / 2;
    ctx.fillStyle = tick.day_start ? "rgba(233,233,236,.68)" : "rgba(233,233,236,.42)";
    ctx.fillText(tick.label, x, layout.rulerH - 13);
  }
  ctx.restore();
}

function drawPlayhead(ctx: CanvasRenderingContext2D, scene: Scene) {
  const { layout, projection: proj, now } = scene;
  ctx.save();
  if (proj.folded) {
    const row = proj.rowContaining(now);
    const box = row ? layout.rows.find((candidate) => candidate.index === row.index) : undefined;
    if (row && box) {
      const x = proj.foldX(now, row);
      ctx.strokeStyle = "rgba(139,147,255,.75)";
      ctx.lineWidth = 1.5;
      ctx.beginPath();
      ctx.moveTo(x, box.top);
      ctx.lineTo(x, box.top + Math.max(3, box.height));
      ctx.stroke();
    }
  } else {
    const x = proj.linearX(now);
    if (x >= layout.plotLeft && x <= layout.width) {
      ctx.strokeStyle = "rgba(139,147,255,.6)";
      ctx.lineWidth = 1;
      ctx.setLineDash([3, 5]);
      ctx.beginPath();
      ctx.moveTo(x, layout.plotTop);
      ctx.lineTo(x, layout.plotTop + layout.plotHeight);
      ctx.stroke();
      ctx.setLineDash([]);
      // 播放头把方向说清楚：左边是发生过的，右边只有计划。
      ctx.fillStyle = "#8b93ff";
      ctx.beginPath();
      ctx.moveTo(x - 4, layout.rulerH - 8);
      ctx.lineTo(x + 4, layout.rulerH - 8);
      ctx.lineTo(x, layout.rulerH - 1);
      ctx.closePath();
      ctx.fill();
    }
  }
  ctx.restore();
}

function drawSelection(ctx: CanvasRenderingContext2D, scene: Scene) {
  const { layout, projection: proj, selection } = scene;
  if (!selection) return;
  ctx.save();
  ctx.fillStyle = "rgba(139,147,255,.12)";
  ctx.strokeStyle = "rgba(139,147,255,.5)";
  ctx.lineWidth = 1;
  if (selection.kind === "phase") {
    if (!proj.folded) {
      ctx.restore();
      return;
    }
    const x1 = proj.phaseX(Math.min(selection.a, selection.b));
    const x2 = proj.phaseX(Math.max(selection.a, selection.b));
    ctx.fillRect(x1, layout.plotTop, x2 - x1, layout.plotHeight);
    ctx.beginPath();
    ctx.moveTo(Math.round(x1) + 0.5, layout.rulerH - 10);
    ctx.lineTo(Math.round(x1) + 0.5, layout.plotTop + layout.plotHeight);
    ctx.moveTo(Math.round(x2) + 0.5, layout.rulerH - 10);
    ctx.lineTo(Math.round(x2) + 0.5, layout.plotTop + layout.plotHeight);
    ctx.stroke();
    ctx.restore();
    return;
  }
  // 时间选区：用同一个投影落位，所以折叠模式下会自动落在对应的行上。
  const from = Math.min(selection.a, selection.b);
  const to = Math.max(selection.a, selection.b);
  for (const rect of proj.place(from, to, layout.plotTop, layout.plotHeight)) {
    const width = Math.max(1, rect.x2 - rect.x1);
    ctx.fillRect(rect.x1, rect.y, width, rect.h);
    ctx.beginPath();
    ctx.moveTo(Math.round(rect.x1) + 0.5, rect.y);
    ctx.lineTo(Math.round(rect.x1) + 0.5, rect.y + rect.h);
    ctx.moveTo(Math.round(rect.x1 + width) + 0.5, rect.y);
    ctx.lineTo(Math.round(rect.x1 + width) + 0.5, rect.y + rect.h);
    ctx.stroke();
  }
  ctx.restore();
}

// ── 轨道 ─────────────────────────────────────────────────────────────────────

function drawTrack(
  ctx: CanvasRenderingContext2D,
  scene: Scene,
  id: TrackId,
  top: number,
  height: number,
  collapsed: boolean,
  hits: HitRegion[],
) {
  switch (id) {
    case "behavior":
      drawBehavior(ctx, scene, top, height, collapsed, hits);
      return;
    case "state":
      drawState(ctx, scene, top, height);
      return;
    case "intervention":
      drawMarkers(ctx, scene, top, height, hits, (event) => event.kind === "intervention");
      return;
    case "capture":
      drawMarkers(
        ctx,
        scene,
        top,
        height,
        hits,
        (event) => event.kind === "capture" || event.category === "milestone",
      );
      return;
    case "plan":
      drawPlans(ctx, scene, top, height, hits);
      return;
  }
}

function drawBehavior(
  ctx: CanvasRenderingContext2D,
  scene: Scene,
  top: number,
  height: number,
  collapsed: boolean,
  hits: HitRegion[],
) {
  const { data, projection: proj } = scene;
  const span = proj.span;
  const sessions = data.events.filter((event) => event.kind === "behavior");
  // LOD 交叉淡入：会话条与密度条在 4–14 天之间接力。折叠成日时会话永远是主角。
  const sessionAlpha = proj.folded && data.cell_kind === "session"
    ? 1
    : 1 - smoothstep(4 * DAY, 14 * DAY, span);
  const bandAlpha = proj.folded ? 0 : smoothstep(2 * DAY, 6 * DAY, span);

  if (bandAlpha > 0.02 && data.bands.length) {
    ctx.save();
    ctx.globalAlpha = bandAlpha;
    for (const band of data.bands) {
      const x1 = proj.linearX(band.bucket_start_ms);
      const x2 = proj.linearX(band.bucket_end_ms);
      const width = Math.max(1, x2 - x1 - 1);
      if (x2 < proj.layout.plotLeft || x1 > proj.layout.width) continue;
      const total = Math.max(1, band.observed_ms);
      // 堆叠比例：专注 / 中性 / 娱乐。轨道高度固定，所以行与行、桶与桶可比。
      let y = top + height;
      for (const [value, color] of [
        [band.entertainment_ms, COLOR.entertainment],
        [band.neutral_ms, COLOR.neutral],
        [band.focus_ms, COLOR.focus],
      ] as Array<[number, string]>) {
        if (value <= 0) continue;
        const share = (value / total) * height * Math.min(1, band.observed_ms / bandCapacity(band));
        ctx.fillStyle = color;
        ctx.fillRect(x1, y - share, width, share);
        y -= share;
      }
    }
    ctx.restore();
  }

  if (sessionAlpha < 0.02 || !sessions.length) return;
  const lanes = assignLanes(sessions);
  const count = collapsed ? 1 : Math.min(laneCount(lanes), 4);
  ctx.save();
  ctx.globalAlpha = sessionAlpha;
  for (const event of sessions) {
    const lane = collapsed ? 0 : Math.min(lanes.get(event.id) ?? 0, count - 1);
    const laneH = height / count;
    const laneTop = top + lane * laneH;
    // 会话块封顶：DAW 里 clip 填满音轨，但一个 5 分钟的会话铺成 300px 高的竖条
    // 会读成柱状图。封顶后轨道仍是条带，多出的空间留给重叠错开。
    const barH = Math.max(2, Math.min(laneH - (laneH > 10 ? 3 : 1), 46));
    const barTop = laneTop + Math.max(0, (laneH - barH) / 2);
    const minWidth = (proj.span / proj.layout.plotWidth) * 3;
    for (const rect of proj.place(
      event.start_ms,
      Math.max(event.end_ms, event.start_ms + minWidth),
      barTop,
      barH,
    )) {
      const width = Math.max(1.5, rect.x2 - rect.x1);
      if (rect.x2 < proj.layout.plotLeft - 4 || rect.x1 > proj.layout.width + 4) continue;
      ctx.fillStyle = categoryColor(event.category);
      roundRect(ctx, rect.x1, rect.y, width, rect.h, Math.min(4, rect.h / 2));
      ctx.fill();
      if (width > 54 && rect.h >= 12) {
        ctx.save();
        ctx.beginPath();
        ctx.rect(rect.x1 + 4, rect.y, width - 8, rect.h);
        ctx.clip();
        ctx.fillStyle = "rgba(8,9,11,.8)";
        ctx.font = FONT_UI;
        ctx.textAlign = "left";
        ctx.fillText(shortTitle(event.title), rect.x1 + 6, rect.y + rect.h / 2 + 3.5);
        ctx.restore();
      }
      hits.push({
        x1: rect.x1,
        x2: rect.x1 + width,
        y1: rect.y - 2,
        y2: rect.y + rect.h + 2,
        event,
      });
    }
  }
  ctx.restore();
}

/// 桶的"满格"时长：让密度条表达**观测覆盖率**，空档不会被拉伸成满格。
function bandCapacity(band: { bucket_start_ms: number; bucket_end_ms: number }): number {
  return Math.max(1, band.bucket_end_ms - band.bucket_start_ms);
}

function drawCells(ctx: CanvasRenderingContext2D, scene: Scene, hits: HitRegion[]) {
  const { data, projection: proj, layout } = scene;
  for (const cell of data.cells) {
    const box = layout.rows.find((candidate) => candidate.index === cell.row);
    if (!box) continue;
    const range = data.cell_kind === "hour" ? proj.hourRange(cell.col) : proj.colRange(cell.col);
    const width = Math.max(1, range.x2 - range.x1 - (layout.rowHeight > 6 ? 1 : 0));
    const height = Math.max(1, box.height - (layout.rowHeight > 6 ? 1 : 0));
    if (cell.observed_ms > 0) {
      const paint = cellPaint(cell);
      ctx.save();
      ctx.globalAlpha = ctx.globalAlpha * paint.alpha;
      ctx.fillStyle = paint.color;
      ctx.fillRect(range.x1, box.top, width, height);
      ctx.restore();
    } else if (layout.rowHeight >= 6) {
      // 空格子也画出来：没有观测和"没在娱乐"是两件事，不能长得一样。
      ctx.save();
      ctx.globalAlpha = ctx.globalAlpha * 0.14;
      ctx.strokeStyle = "rgba(255,255,255,.35)";
      ctx.strokeRect(range.x1 + 0.5, box.top + 0.5, width - 1, height - 1);
      ctx.restore();
    }
    hits.push({
      x1: range.x1,
      x2: range.x1 + width,
      y1: box.top,
      y2: box.top + height,
      cell,
    });
    if (proj.doublePlot && data.cell_kind === "hour") {
      const previous = layout.rows.find((candidate) => candidate.index === cell.row - 1);
      if (previous && cell.observed_ms > 0) {
        const paint = cellPaint(cell);
        ctx.save();
        ctx.globalAlpha = ctx.globalAlpha * paint.alpha;
        ctx.fillStyle = paint.color;
        ctx.fillRect(range.x1 + proj.foldWidth, previous.top, width, height);
        ctx.restore();
      }
    }
  }
}

function drawState(
  ctx: CanvasRenderingContext2D,
  scene: Scene,
  top: number,
  height: number,
) {
  const { data, projection: proj } = scene;
  if (proj.folded || data.days.length === 0) return;
  ctx.save();
  let previous: { x: number; y: number } | null = null;
  for (const day of data.days) {
    const x = proj.linearX(day.start_ms + DAY / 2);
    if (x < proj.layout.plotLeft - 30 || x > proj.layout.width + 30) continue;
    const y = top + height - (day.state_score / 100) * height;
    const width = clamp((DAY / proj.span) * proj.layout.plotWidth * 0.6, 2, 22);
    ctx.fillStyle = scoreColor(day.state_score);
    ctx.fillRect(x - width / 2, y, width, top + height - y);
    if (previous) {
      ctx.strokeStyle = "rgba(139,147,255,.3)";
      ctx.lineWidth = 1.2;
      ctx.beginPath();
      ctx.moveTo(previous.x, previous.y);
      ctx.lineTo(x, y);
      ctx.stroke();
    }
    previous = { x, y };
  }
  ctx.restore();
}

function drawMarkers(
  ctx: CanvasRenderingContext2D,
  scene: Scene,
  top: number,
  height: number,
  hits: HitRegion[],
  predicate: (event: TimelineEvent) => boolean,
) {
  const { data, projection: proj } = scene;
  const radius = clamp(height / 3.2, 2.5, 5);
  const centerY = top + height / 2;
  for (const event of data.events) {
    if (!predicate(event)) continue;
    for (const rect of proj.places(event.start_ms, centerY - radius, radius * 2)) {
      const x = rect.x1;
      const y = rect.y + rect.h / 2;
      if (x < proj.layout.plotLeft - 6 || x > proj.layout.width + 6) continue;
      ctx.fillStyle = eventColor(event);
      if (event.kind === "intervention") {
        ctx.beginPath();
        ctx.moveTo(x, y - radius);
        ctx.lineTo(x + radius, y);
        ctx.lineTo(x, y + radius);
        ctx.lineTo(x - radius, y);
        ctx.closePath();
        ctx.fill();
        // 干预的近端结果直接画在标记上：转移成功的加一圈绿环。
        if (event.detail?.includes("switched")) {
          ctx.strokeStyle = COLOR.good;
          ctx.lineWidth = 1.4;
          ctx.beginPath();
          ctx.arc(x, y, radius + 2.4, 0, Math.PI * 2);
          ctx.stroke();
        }
      } else {
        ctx.beginPath();
        ctx.arc(x, y, radius, 0, Math.PI * 2);
        ctx.fill();
      }
      hits.push({
        x1: x - radius - 3,
        x2: x + radius + 3,
        y1: y - radius - 3,
        y2: y + radius + 3,
        event,
      });
    }
  }
}

function drawPlans(
  ctx: CanvasRenderingContext2D,
  scene: Scene,
  top: number,
  height: number,
  hits: HitRegion[],
) {
  const { data, projection: proj } = scene;
  if (!data.plans.length) return;
  const rowH = clamp(height / Math.min(4, data.plans.length), 6, 20);
  data.plans.forEach((plan, index) => {
    const lane = index % Math.max(1, Math.floor(height / rowH));
    const y = top + lane * rowH;
    const barH = Math.max(4, rowH - 3);
    for (const rect of proj.place(plan.start_ms, plan.end_ms, y, barH)) {
      const width = Math.max(3, rect.x2 - rect.x1);
      if (rect.x2 < proj.layout.plotLeft || rect.x1 > proj.layout.width) continue;
      ctx.fillStyle = "rgba(139,147,255,.16)";
      roundRect(ctx, rect.x1, rect.y, width, rect.h, 3);
      ctx.fill();
      ctx.fillStyle = plan.kind === "life_milestone" ? COLOR.warn : "rgba(139,147,255,.55)";
      roundRect(ctx, rect.x1, rect.y, Math.max(2, width * clamp(plan.progress, 0, 1)), rect.h, 3);
      ctx.fill();
      if (width > 60 && barH >= 10) {
        ctx.save();
        ctx.beginPath();
        ctx.rect(rect.x1 + 4, rect.y, width - 8, rect.h);
        ctx.clip();
        ctx.fillStyle = "rgba(233,233,236,.8)";
        ctx.font = FONT_UI;
        ctx.textAlign = "left";
        ctx.fillText(plan.title, rect.x1 + 6, rect.y + barH / 2 + 3.5);
        ctx.restore();
      }
      hits.push({ x1: rect.x1, x2: rect.x1 + width, y1: rect.y, y2: rect.y + barH, plan });
    }
  });
}

function roundRect(
  ctx: CanvasRenderingContext2D,
  x: number,
  y: number,
  width: number,
  height: number,
  radius: number,
) {
  const r = Math.max(0, Math.min(radius, Math.abs(width) / 2, Math.abs(height) / 2));
  ctx.beginPath();
  ctx.roundRect(x, y, width, height, r);
}

function shortTitle(title: string): string {
  const parts = title.split(".");
  return parts[parts.length - 1] || title;
}
