/// 技能树的**唯一坐标出口**。
///
/// 不变量（与无极时间轴的"折叠 = 换投影"同源）：**雷达不是第二个视图，是同一份布局的另一个投影**。
/// `t=0` 画树（半径 = 依赖深度，回答"要先会什么"），`t=1` 画雷达（半径 = 掌握度，回答"我在这个领域到哪一步"），
/// 中间是连续插值。一段绘制代码、一条渲染路径；时间播放只改 `progress`，同样不新开路径。
///
/// 角度永远来自 Core 给的扇区角区间，任何投影都不会改变它——所以"角度 = 领域"这条图例在任何 `t` 下都成立。

import type { SkillMap, SkillNode, SkillSector } from "./types";

export interface Viewport {
  width: number;
  height: number;
  /** 缩放倍数（滚轮），1 = 适配画布。 */
  scale: number;
  /** 平移（屏幕像素）。 */
  offsetX: number;
  offsetY: number;
}

export interface Polar {
  angle: number;
  /** 归一化半径 0–1（1 = 最外环）。 */
  radius: number;
}

export interface Point {
  x: number;
  y: number;
}

/** 最内圈留给中心的 "Lv" 读数，最外圈留给想法星尘。 */
const INNER = 0.16;
const OUTER = 0.9;
/** 雷达投影里 0 掌握度也给一点半径，否则所有未开始的节点会挤在圆心。 */
const RADAR_FLOOR = 0.18;

export function clamp(value: number, min: number, max: number) {
  return Math.min(max, Math.max(min, value));
}

/** 扇区内按槽位分角：`(slot + 0.5) / slots`，两端各留半格，不会贴在扇区边界上。 */
export function sectorAngle(sector: SkillSector, slot: number, slots: number) {
  const span = sector.end_angle - sector.start_angle;
  const denominator = Math.max(1, slots);
  return sector.start_angle + ((slot + 0.5) / denominator) * span;
}

/** 树投影的归一化半径：依赖深度越深越靠外。 */
export function treeRadius(depth: number, maxDepth: number) {
  return INNER + ((depth + 1) / (maxDepth + 2)) * (OUTER - INNER);
}

/** 雷达投影的归一化半径：自身进度越高越靠外。 */
export function radarRadius(progress: number) {
  return INNER + (RADAR_FLOOR + (1 - RADAR_FLOOR) * clamp(progress, 0, 1)) * (OUTER - INNER);
}

/**
 * 节点的目标极坐标。`progress` 单独传入，让时间播放能用历史进度而不必伪造节点对象。
 * 有父技能的里程碑是父节点周围的卫星，由 `satellite()` 处理，不走这里。
 */
export function place(
  node: Pick<SkillNode, "depth" | "slot" | "slot_count" | "sector">,
  sector: SkillSector,
  maxDepth: number,
  progress: number,
  t: number,
): Polar {
  const angle = sectorAngle(sector, node.slot, node.slot_count);
  const tree = treeRadius(node.depth, maxDepth);
  const radar = radarRadius(progress);
  return { angle, radius: tree + (radar - tree) * clamp(t, 0, 1) };
}

/** 雷达多边形顶点：扇区中线上，半径由领域掌握度决定。`t` 让它在树投影下收成一个淡淡的底圈。 */
export function sectorVertex(sector: SkillSector, mastery: number, t: number): Polar {
  const angle = (sector.start_angle + sector.end_angle) / 2;
  const radar = radarRadius(mastery);
  const idle = INNER + 0.34 * (OUTER - INNER);
  return { angle, radius: idle + (radar - idle) * clamp(t, 0, 1) };
}

/** 想法星尘：最外环之外，按序号在扇区内铺开。它们没有进度，所以不随 `t` 移动。 */
export function ideaPolar(sector: SkillSector, index: number, count: number): Polar {
  return {
    angle: sectorAngle(sector, index, count),
    radius: OUTER + 0.055 + 0.03 * (index % 2),
  };
}

/** 里程碑卫星：父节点周围的小环。半径是屏幕像素（不随投影变化，它是"刻度"不是"位置"）。 */
export function satellite(parent: Point, slot: number, slots: number, distance: number): Point {
  const angle = (slot / Math.max(1, slots)) * Math.PI * 2 - Math.PI / 2;
  return { x: parent.x + Math.cos(angle) * distance, y: parent.y + Math.sin(angle) * distance };
}

/** 极坐标 → 屏幕像素。画布中心 + 缩放 + 平移。 */
export function toScreen(polar: Polar, view: Viewport): Point {
  const base = Math.min(view.width, view.height) / 2;
  const r = polar.radius * base * view.scale;
  return {
    x: view.width / 2 + view.offsetX + Math.cos(polar.angle) * r,
    y: view.height / 2 + view.offsetY + Math.sin(polar.angle) * r,
  };
}

/** 屏幕半径（画扇区、圆环用）。 */
export function screenRadius(radius: number, view: Viewport) {
  return radius * (Math.min(view.width, view.height) / 2) * view.scale;
}

/** 节点画多大：技能是技能点，里程碑是刻度，量级必须不同。 */
export function nodeSize(node: Pick<SkillNode, "kind" | "parent_id">) {
  if (node.kind === "skill") return 13;
  return node.parent_id ? 5.5 : 9;
}

/** 每个扇区的想法数（星尘铺开用）。 */
export function ideaCounts(map: SkillMap) {
  const counts = new Map<number, number>();
  for (const idea of map.ideas) counts.set(idea.sector, (counts.get(idea.sector) ?? 0) + 1);
  return counts;
}
