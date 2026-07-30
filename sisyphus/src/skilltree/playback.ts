/// 时间播放：把 Core 的 delta 编码变化点折成"任意时刻 T 的进度"。
///
/// 阶梯函数，不插值出中间值——进度是事实，不该被动画伪造。视觉上的平滑由绘制层的
/// 缓动负责（画的是"从上一个真实值走到下一个真实值"），数值本身永远是账本里的那个。

import type { Growth, NodeState, ProgressChange } from "./types";

export interface Frame {
  at_ms: number;
  progress: Map<string, ProgressChange>;
  mastery: Map<number, number>;
}

/** 预处理成"按节点分组、按时间升序"的变化序列，播放时二分即可。 */
export interface Timeline {
  from_ms: number;
  to_ms: number;
  byNode: Map<string, ProgressChange[]>;
  bySector: Map<number, { at_ms: number; mastery: number }[]>;
}

export function buildTimeline(growth: Growth | null): Timeline | null {
  if (!growth) return null;
  const byNode = new Map<string, ProgressChange[]>();
  for (const change of growth.changes) {
    const list = byNode.get(change.item_id);
    if (list) list.push(change);
    else byNode.set(change.item_id, [change]);
  }
  const bySector = new Map<number, { at_ms: number; mastery: number }[]>();
  for (const change of growth.sectors) {
    const list = bySector.get(change.sector);
    if (list) list.push({ at_ms: change.at_ms, mastery: change.mastery });
    else bySector.set(change.sector, [{ at_ms: change.at_ms, mastery: change.mastery }]);
  }
  return { from_ms: growth.from_ms, to_ms: growth.to_ms, byNode, bySector };
}

/** 取 `at_ms <= t` 的最后一条（二分）。没有则返回 null = 那时它还不存在。 */
function lastAt<T extends { at_ms: number }>(list: T[] | undefined, t: number): T | null {
  if (!list || !list.length || list[0].at_ms > t) return null;
  let low = 0;
  let high = list.length - 1;
  while (low < high) {
    const mid = Math.ceil((low + high) / 2);
    if (list[mid].at_ms <= t) low = mid;
    else high = mid - 1;
  }
  return list[low];
}

export function frameAt(timeline: Timeline, t: number): Frame {
  const progress = new Map<string, ProgressChange>();
  for (const [id, list] of timeline.byNode) {
    const hit = lastAt(list, t);
    if (hit) progress.set(id, hit);
  }
  const mastery = new Map<number, number>();
  for (const [sector, list] of timeline.bySector) {
    const hit = lastAt(list, t);
    if (hit) mastery.set(sector, hit.mastery);
  }
  return { at_ms: t, progress, mastery };
}

/** 播放时某节点的状态；节点在 T 时刻还没出生则返回 null（不画）。 */
export function stateAt(frame: Frame | null, id: string): { progress: number; state: NodeState; done: number; total: number } | null {
  if (!frame) return null;
  const hit = frame.progress.get(id);
  if (!hit) return null;
  return { progress: hit.progress, state: hit.state, done: hit.done_leaves, total: hit.total_leaves };
}
