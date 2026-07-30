/// 弹性松弛：让节点不重叠、让依赖边尽量径向，同时**不许把语义弄脏**。
///
/// 硬边界（越界即 bug，`scripts/skilltree-geometry-check.ts` 会断言）：
/// - 角度只能在 Core 给的**扇区角区间内**移动（角度 = 领域，这条图例不能被物理破坏）；
/// - 半径只能在目标环 `±RADIUS_SLACK` 内移动（半径 = 依赖深度 / 掌握度，同理）。
///
/// 确定性：初始扰动来自 `hash(id)` 而不是 `Math.random()`。同一份输入两次松弛得到同一结果，
/// 于是节点的位置跨会话稳定——用户会形成空间记忆，位置每次都变会把这份记忆毁掉。

import { clamp, type Polar } from "./projection";

/** 半径允许的偏移（归一化半径单位）。够躲开重叠，不够跨到隔壁环。 */
export const RADIUS_SLACK = 0.035;
/** 角度距扇区边界至少留这么多，避免节点压在分界线上。 */
const ANGLE_MARGIN = 0.02;

export interface Body {
  id: string;
  /** Core 给的目标位置（锚点）。 */
  target: Polar;
  /** 允许游走的扇区角区间。 */
  minAngle: number;
  maxAngle: number;
  /** 当前位置。 */
  angle: number;
  radius: number;
  /** 角/径方向速度。 */
  va: number;
  vr: number;
  /** 碰撞半径（归一化半径单位换算后的近似值）。 */
  size: number;
  /** 前置节点 id（弱角度吸引，让边更径向、更好读）。 */
  depends: string[];
}

const ANCHOR = 0.06;
const DAMPING = 0.86;
const REPULSION = 0.0022;
const DEPEND_PULL = 0.012;

/** 稳定哈希（FNV-1a 的 32 位变体）→ [0,1)，代替 Math.random。 */
export function hash01(id: string) {
  let h = 0x811c9dc5;
  for (let i = 0; i < id.length; i += 1) {
    h ^= id.charCodeAt(i);
    h = Math.imul(h, 0x01000193) >>> 0;
  }
  return (h >>> 8) / 0x01000000;
}

/** 把角度差归一到 (-π, π]，避免绕圈。 */
function angleDelta(a: number, b: number) {
  let d = a - b;
  while (d > Math.PI) d -= Math.PI * 2;
  while (d <= -Math.PI) d += Math.PI * 2;
  return d;
}

export function createBody(
  id: string,
  target: Polar,
  bounds: { start: number; end: number },
  size: number,
  depends: string[],
): Body {
  const jitter = (hash01(id) - 0.5) * 2;
  const minAngle = Math.min(bounds.start + ANGLE_MARGIN, bounds.end - ANGLE_MARGIN);
  const maxAngle = Math.max(bounds.start + ANGLE_MARGIN, bounds.end - ANGLE_MARGIN);
  return {
    id,
    target,
    minAngle,
    maxAngle,
    // 确定性的初始扰动：看起来是"活的"，但每次都一样。
    angle: clamp(target.angle + jitter * ANGLE_MARGIN, minAngle, maxAngle),
    radius: target.radius + jitter * RADIUS_SLACK * 0.5,
    va: 0,
    vr: 0,
    size,
    depends,
  };
}

/**
 * 推进一步。`aspect` 是"1 归一化半径 ≈ 多少个 size 单位"的换算，用来让斥力在屏幕上等距。
 * 纯函数式副作用：原地更新 bodies（调用方自己决定跑几步）。
 */
export function step(bodies: Body[], aspect: number) {
  const byId = new Map(bodies.map((b) => [b.id, b]));
  for (const body of bodies) {
    // 1. 锚点弹簧：始终把节点拉回 Core 给的位置。
    body.va += -angleDelta(body.angle, body.target.angle) * ANCHOR;
    body.vr += (body.target.radius - body.radius) * ANCHOR;

    // 2. 邻接斥力：只在屏幕上真的挨上了才推开。
    for (const other of bodies) {
      if (other === body) continue;
      const dr = body.radius - other.radius;
      const da = angleDelta(body.angle, other.angle);
      // 近似屏幕距离：角度差乘以半径得到弧长。
      const arc = da * ((body.radius + other.radius) / 2);
      const distance = Math.hypot(dr, arc);
      const minimum = (body.size + other.size) / aspect;
      if (distance >= minimum || distance === 0) continue;
      const push = (minimum - distance) * REPULSION;
      body.va += (arc >= 0 ? 1 : -1) * push;
      body.vr += (dr >= 0 ? 1 : -1) * push;
    }

    // 3. 依赖弱吸引：让"前置 → 后继"尽量落在同一条射线上，边才好读。
    for (const dependency of body.depends) {
      const prerequisite = byId.get(dependency);
      if (!prerequisite) continue;
      body.va += -angleDelta(body.angle, prerequisite.angle) * DEPEND_PULL;
    }
  }
  for (const body of bodies) {
    body.va *= DAMPING;
    body.vr *= DAMPING;
    body.angle = clamp(body.angle + body.va, body.minAngle, body.maxAngle);
    body.radius = clamp(
      body.radius + body.vr,
      body.target.radius - RADIUS_SLACK,
      body.target.radius + RADIUS_SLACK,
    );
  }
}

/** 跑到稳定（断言脚本与首帧用）。同输入必得同输出。 */
export function relax(bodies: Body[], steps: number, aspect: number) {
  for (let i = 0; i < steps; i += 1) step(bodies, aspect);
  return bodies;
}
