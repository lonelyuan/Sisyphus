/// 技能树的绘制层。**只投影与绘制，不做判断**——四态、进度、掌握度、扇区角都是 Core 给的。
///
/// 图层顺序（也是"哪些是背景、哪些是前景"的答案）：
///   领域扇区（背景）→ 深度环 → 雷达多边形（背景摘要）→ 前置边 → 技能点 → 里程碑刻度
///   → 想法星尘（环外）→ 标签 → 中心 Lv 读数
///
/// 颜色只表达属性（track），位置只表达结构（领域 / 依赖深度 / 掌握度）。

import {
  ideaCounts,
  ideaPolar,
  nodeSize,
  satellite,
  screenRadius,
  sectorVertex,
  toScreen,
  type Point,
  type Viewport,
} from "./projection";
import type { Frame } from "./playback";
import { stateAt } from "./playback";
import type { NodeState, SkillMap, SkillNode } from "./types";

export interface HitRegion {
  x: number;
  y: number;
  r: number;
  node?: SkillNode;
  idea?: string;
}

export interface DrawInput {
  map: SkillMap;
  /** 已由物理松弛落定的屏幕坐标（技能与孤立里程碑）。 */
  positions: Map<string, Point>;
  view: Viewport;
  /** 0 = 树投影，1 = 雷达投影。 */
  t: number;
  /** 播放中的历史帧；null = 现在。 */
  frame: Frame | null;
  selectedId: string | null;
  hoverId: string | null;
  /** 缩放足够大时里程碑从刻度展开为卫星节点。 */
  expandMilestones: boolean;
  showLabels: boolean;
}

const TRACK_COLOR: Record<string, string> = {
  main: "#8b93ff",
  side: "#e99d54",
  neutral: "#69c4a4",
  undecided: "#657084",
};

function trackColor(track: string) {
  return TRACK_COLOR[track] ?? TRACK_COLOR.undecided;
}

/** 四态 → 亮度与描边。这是"我会什么 / 还差什么"的唯一视觉编码。 */
function stateStyle(state: NodeState) {
  switch (state) {
    case "attained":
      return { alpha: 1, dash: [] as number[], stroke: 1.6 };
    case "in_progress":
      return { alpha: 0.9, dash: [], stroke: 1.4 };
    case "available":
      return { alpha: 0.62, dash: [3, 3], stroke: 1.1 };
    case "locked":
      return { alpha: 0.3, dash: [2, 4], stroke: 1 };
  }
}

export function draw(ctx: CanvasRenderingContext2D, input: DrawInput): HitRegion[] {
  const { map, positions, view, t, frame, selectedId, hoverId } = input;
  const hits: HitRegion[] = [];
  const center = { x: view.width / 2 + view.offsetX, y: view.height / 2 + view.offsetY };

  ctx.clearRect(0, 0, view.width, view.height);
  const bg = ctx.createLinearGradient(0, 0, 0, view.height);
  bg.addColorStop(0, "#0d0e11");
  bg.addColorStop(1, "#090a0c");
  ctx.fillStyle = bg;
  ctx.fillRect(0, 0, view.width, view.height);

  const outer = screenRadius(0.96, view);

  // ── 背景：领域扇区。永远不是节点——领域没有完成态，画成节点它永远填不满。
  for (const sector of map.sectors) {
    ctx.beginPath();
    ctx.moveTo(center.x, center.y);
    ctx.arc(center.x, center.y, outer, sector.start_angle, sector.end_angle);
    ctx.closePath();
    ctx.fillStyle = sector.focus ? "rgba(139,147,255,.055)" : "rgba(255,255,255,.018)";
    ctx.fill();
    ctx.strokeStyle = "rgba(255,255,255,.045)";
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.moveTo(center.x, center.y);
    ctx.lineTo(
      center.x + Math.cos(sector.start_angle) * outer,
      center.y + Math.sin(sector.start_angle) * outer,
    );
    ctx.stroke();
  }

  // ── 深度环：半径的刻度（树投影下才有意义，雷达投影里淡出）。
  ctx.globalAlpha = 1 - t;
  ctx.strokeStyle = "rgba(255,255,255,.05)";
  for (let depth = 0; depth <= map.max_depth; depth += 1) {
    const r = screenRadius(0.16 + ((depth + 1) / (map.max_depth + 2)) * (0.9 - 0.16), view);
    ctx.beginPath();
    ctx.arc(center.x, center.y, r, 0, Math.PI * 2);
    ctx.stroke();
  }
  ctx.globalAlpha = 1;

  // ── 背景摘要：雷达多边形。顶点半径 = 领域掌握度，和节点填充同源，不可能互相矛盾。
  if (map.sectors.length >= 3) {
    ctx.beginPath();
    map.sectors.forEach((sector, index) => {
      const mastery = frame?.mastery.get(sector.index) ?? sector.mastery;
      const point = toScreen(sectorVertex(sector, mastery, t), view);
      if (index === 0) ctx.moveTo(point.x, point.y);
      else ctx.lineTo(point.x, point.y);
    });
    ctx.closePath();
    ctx.fillStyle = `rgba(139,147,255,${0.05 + 0.09 * t})`;
    ctx.fill();
    ctx.strokeStyle = `rgba(139,147,255,${0.28 + 0.34 * t})`;
    ctx.lineWidth = 1.2;
    ctx.stroke();
  }

  // ── 领域名：贴在外缘中线上。
  ctx.font = "10px -apple-system, BlinkMacSystemFont, sans-serif";
  ctx.textAlign = "center";
  ctx.textBaseline = "middle";
  for (const sector of map.sectors) {
    const mid = (sector.start_angle + sector.end_angle) / 2;
    const r = outer * 0.995;
    const x = center.x + Math.cos(mid) * r;
    const y = center.y + Math.sin(mid) * r;
    ctx.fillStyle = sector.focus ? "rgba(139,147,255,.85)" : "rgba(233,233,236,.34)";
    ctx.fillText(sector.name, x, y);
    if (sector.total > 0) {
      ctx.fillStyle = "rgba(233,233,236,.24)";
      ctx.font = "9px ui-monospace, monospace";
      ctx.fillText(`${sector.attained}/${sector.total}`, x, y + 12);
      ctx.font = "10px -apple-system, BlinkMacSystemFont, sans-serif";
    }
  }

  const live = new Map(map.nodes.map((node) => [node.id, node]));
  const visible = (id: string) => (frame ? frame.progress.has(id) : true);
  const progressOf = (node: SkillNode) => stateAt(frame, node.id)?.progress ?? node.progress;
  const stateOf = (node: SkillNode) => stateAt(frame, node.id)?.state ?? node.state;

  // ── 前置边：唯一进图的边。实线 = 前置已达成，虚线 = 还锁着。
  for (const edge of map.edges) {
    const from = positions.get(edge.from);
    const to = positions.get(edge.to);
    if (!from || !to) continue;
    if (!visible(edge.from) || !visible(edge.to)) continue;
    const target = live.get(edge.to);
    const satisfied = target ? progressOf(target) >= 1 : edge.satisfied;
    ctx.save();
    ctx.strokeStyle = satisfied ? "rgba(139,147,255,.5)" : "rgba(233,233,236,.14)";
    ctx.lineWidth = satisfied ? 1.4 : 1;
    ctx.setLineDash(satisfied ? [] : [3, 4]);
    ctx.beginPath();
    ctx.moveTo(to.x, to.y);
    // 控制点略偏向圆心：边呈放射状，"压在什么之上"一眼可读。
    const cx = (from.x + to.x) / 2 + (center.x - (from.x + to.x) / 2) * 0.18;
    const cy = (from.y + to.y) / 2 + (center.y - (from.y + to.y) / 2) * 0.18;
    ctx.quadraticCurveTo(cx, cy, from.x, from.y);
    ctx.stroke();
    ctx.restore();
  }

  // ── 技能点与孤立里程碑。
  for (const node of map.nodes) {
    if (node.parent_id && node.kind === "milestone") continue;
    const point = positions.get(node.id);
    if (!point || !visible(node.id)) continue;
    const progress = progressOf(node);
    const style = stateStyle(stateOf(node));
    const size = nodeSize(node);
    const color = trackColor(node.track);
    const emphasised = node.id === selectedId || node.id === hoverId;

    ctx.save();
    ctx.globalAlpha = style.alpha;
    // 底环 = 还没完成的部分
    ctx.beginPath();
    ctx.arc(point.x, point.y, size, 0, Math.PI * 2);
    ctx.fillStyle = "rgba(12,13,16,.9)";
    ctx.fill();
    ctx.setLineDash(style.dash);
    ctx.strokeStyle = color;
    ctx.lineWidth = style.stroke;
    ctx.stroke();
    ctx.setLineDash([]);
    // 进度弧 = 事实进展。从 12 点顺时针，量满即一整圈。
    if (progress > 0) {
      ctx.beginPath();
      ctx.moveTo(point.x, point.y);
      ctx.arc(point.x, point.y, size - 1.6, -Math.PI / 2, -Math.PI / 2 + Math.PI * 2 * progress);
      ctx.closePath();
      ctx.fillStyle = color;
      ctx.globalAlpha = style.alpha * 0.75;
      ctx.fill();
      ctx.globalAlpha = style.alpha;
    }
    if (stateOf(node) === "locked") {
      // 锁定：中心一道短横，不用图标也能看出"还不能点"。
      ctx.strokeStyle = "rgba(233,233,236,.55)";
      ctx.lineWidth = 1.4;
      ctx.beginPath();
      ctx.moveTo(point.x - 3.2, point.y);
      ctx.lineTo(point.x + 3.2, point.y);
      ctx.stroke();
    }
    if (emphasised) {
      ctx.globalAlpha = 1;
      ctx.strokeStyle = "rgba(255,255,255,.75)";
      ctx.lineWidth = 1;
      ctx.beginPath();
      ctx.arc(point.x, point.y, size + 4.5, 0, Math.PI * 2);
      ctx.stroke();
    }
    if (node.goal_count > 0) {
      // 目标/项目是产出不是能力：只作角标，不进画布。
      ctx.globalAlpha = 1;
      ctx.fillStyle = "#34d399";
      ctx.beginPath();
      ctx.arc(point.x + size * 0.78, point.y - size * 0.78, 2.6, 0, Math.PI * 2);
      ctx.fill();
    }
    ctx.restore();
    hits.push({ x: point.x, y: point.y, r: size + 5, node });

    // 里程碑刻度：低缩放画成节点环上的刻度，放大后展开成卫星节点。
    const milestones = map.nodes.filter((m) => m.parent_id === node.id && m.kind === "milestone");
    if (!milestones.length) continue;
    if (!input.expandMilestones) {
      milestones.forEach((milestone, index) => {
        if (!visible(milestone.id)) return;
        const angle = (index / milestones.length) * Math.PI * 2 - Math.PI / 2;
        const lit = progressOf(milestone) >= 1;
        const inner = size + 3;
        const length = lit ? 5 : 3.2;
        ctx.save();
        ctx.globalAlpha = lit ? 0.95 : 0.34;
        ctx.strokeStyle = lit ? color : "rgba(233,233,236,.6)";
        ctx.lineWidth = 1.8;
        ctx.beginPath();
        ctx.moveTo(point.x + Math.cos(angle) * inner, point.y + Math.sin(angle) * inner);
        ctx.lineTo(
          point.x + Math.cos(angle) * (inner + length),
          point.y + Math.sin(angle) * (inner + length),
        );
        ctx.stroke();
        ctx.restore();
      });
    } else {
      milestones.forEach((milestone, index) => {
        if (!visible(milestone.id)) return;
        const spot = satellite(point, index, milestones.length, size + 16);
        const done = progressOf(milestone) >= 1;
        const style = stateStyle(stateOf(milestone));
        ctx.save();
        ctx.globalAlpha = style.alpha;
        ctx.strokeStyle = "rgba(233,233,236,.2)";
        ctx.lineWidth = 1;
        ctx.beginPath();
        ctx.moveTo(point.x, point.y);
        ctx.lineTo(spot.x, spot.y);
        ctx.stroke();
        ctx.beginPath();
        ctx.arc(spot.x, spot.y, nodeSize(milestone), 0, Math.PI * 2);
        ctx.fillStyle = done ? color : "rgba(12,13,16,.9)";
        ctx.fill();
        ctx.strokeStyle = color;
        ctx.lineWidth = 1.1;
        ctx.stroke();
        ctx.restore();
        hits.push({ x: spot.x, y: spot.y, r: nodeSize(milestone) + 4, node: milestone });
      });
    }
  }

  // ── 想法星尘：环外，无边。看它向内迁移＝树在生长。
  const counts = ideaCounts(map);
  const seen = new Map<number, number>();
  for (const idea of map.ideas) {
    const sector = map.sectors[idea.sector];
    if (!sector) continue;
    if (frame && idea.created_at > frame.at_ms) continue;
    const index = seen.get(idea.sector) ?? 0;
    seen.set(idea.sector, index + 1);
    const point = toScreen(ideaPolar(sector, index, counts.get(idea.sector) ?? 1), view);
    ctx.save();
    ctx.globalAlpha = idea.due_review ? 0.85 : 0.42;
    ctx.fillStyle = idea.due_review ? "#fbbf24" : "rgba(233,233,236,.7)";
    ctx.beginPath();
    ctx.arc(point.x, point.y, 2.4, 0, Math.PI * 2);
    ctx.fill();
    if (idea.due_review) {
      ctx.strokeStyle = "rgba(251,191,36,.5)";
      ctx.lineWidth = 1;
      ctx.beginPath();
      ctx.arc(point.x, point.y, 5.5, 0, Math.PI * 2);
      ctx.stroke();
    }
    ctx.restore();
    hits.push({ x: point.x, y: point.y, r: 7, idea: idea.id });
  }

  // ── 标签：只给技能，且只在放得开的时候。
  if (input.showLabels) {
    ctx.font = "10px -apple-system, BlinkMacSystemFont, sans-serif";
    ctx.textAlign = "center";
    for (const node of map.nodes) {
      if (node.kind !== "skill") continue;
      const point = positions.get(node.id);
      if (!point || !visible(node.id)) continue;
      const emphasised = node.id === selectedId || node.id === hoverId;
      ctx.fillStyle = emphasised ? "rgba(233,233,236,.95)" : "rgba(233,233,236,.55)";
      ctx.fillText(node.title, point.x, point.y + nodeSize(node) + 13);
      if (node.total_leaves > 1) {
        ctx.fillStyle = "rgba(233,233,236,.3)";
        ctx.font = "9px ui-monospace, monospace";
        const done = stateAt(frame, node.id)?.done ?? node.done_leaves;
        const total = stateAt(frame, node.id)?.total ?? node.total_leaves;
        ctx.fillText(`Lv ${done}/${total}`, point.x, point.y + nodeSize(node) + 24);
        ctx.font = "10px -apple-system, BlinkMacSystemFont, sans-serif";
      }
    }
  }

  // ── 中心：全局 Lv。已掌握的能力聚成明亮内核，这就是"我会什么"的一句话回答。
  const attained = frame
    ? map.nodes.filter((n) => n.kind === "skill" && (stateAt(frame, n.id)?.state ?? null) === "attained").length
    : map.attained;
  const total = frame
    ? map.nodes.filter((n) => n.kind === "skill" && frame.progress.has(n.id)).length
    : map.total;
  ctx.textAlign = "center";
  ctx.fillStyle = "rgba(233,233,236,.9)";
  ctx.font = "600 15px ui-monospace, monospace";
  ctx.fillText(`${attained}/${total}`, center.x, center.y - 2);
  ctx.fillStyle = "rgba(233,233,236,.32)";
  ctx.font = "9px -apple-system, BlinkMacSystemFont, sans-serif";
  ctx.fillText("已掌握", center.x, center.y + 12);

  return hits;
}

export function hitTest(hits: HitRegion[], x: number, y: number) {
  let best: HitRegion | null = null;
  let bestDistance = Infinity;
  for (const hit of hits) {
    const distance = Math.hypot(hit.x - x, hit.y - y);
    if (distance <= hit.r && distance < bestDistance) {
      best = hit;
      bestDistance = distance;
    }
  }
  return best;
}
