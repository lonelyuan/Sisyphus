/// 轨道定义与配色。
///
/// **一条轨道 = 一个来源维度，不是一个属性值**。娱乐/社交/工作是行为的*属性*，
/// 该用颜色区分；把它们拆成三条轨道等于把一件事的三个侧面当成三件事，
/// 白白吃掉纵向空间（纵向空间是多轨视图最紧的资源）。
///
/// 每条轨道都必须能说出它**独占回答**的一个问题；答不出来的轨道不该存在。

import type { AxisCell, TimelineEvent } from "./types";

export type TrackId = "behavior" | "state" | "intervention" | "capture" | "plan";

export interface TrackMeta {
  id: TrackId;
  label: string;
  /// 这条轨道独占回答的问题（也是 UI 上的 hint）。
  question: string;
  /// 纵向高度权重。
  weight: number;
  defaultVisible: boolean;
}

export const TRACKS: TrackMeta[] = [
  {
    id: "behavior",
    label: "行为",
    question: "这段时间我实际在做什么？",
    weight: 2.6,
    defaultVisible: true,
  },
  {
    id: "intervention",
    label: "干预",
    question: "哪次提醒真的让我转移了注意力？",
    weight: 0.85,
    defaultVisible: true,
  },
  {
    id: "capture",
    label: "记录",
    question: "想法与知识是在什么状态下产生的？",
    weight: 0.85,
    defaultVisible: true,
  },
  {
    id: "plan",
    label: "计划",
    question: "未来这段时间我承诺了什么？",
    weight: 1.5,
    defaultVisible: true,
  },
  {
    id: "state",
    label: "状态分",
    // 默认关闭：state_score 是一个可解释但很粗的加权公式，
    // 画成醒目曲线会暗示它并不具备的精度。想看的人自己打开。
    question: "每日状态的粗略趋势（启发式，慎读）",
    weight: 1.2,
    defaultVisible: false,
  },
];

export interface TrackViewState {
  id: TrackId;
  visible: boolean;
  solo: boolean;
  collapsed: boolean;
}

export const DEFAULT_TRACK_STATE: TrackViewState[] = TRACKS.map((track) => ({
  id: track.id,
  visible: track.defaultVisible,
  solo: false,
  collapsed: false,
}));

/// solo 优先：有任何轨道 solo 时，只画 solo 的。
export function effectiveVisible(states: TrackViewState[]): TrackViewState[] {
  const solo = states.filter((state) => state.solo && state.visible);
  return solo.length > 0 ? solo : states.filter((state) => state.visible);
}

// ── 配色 ─────────────────────────────────────────────────────────────────────

export const COLOR = {
  focus: "#69c4a4",
  entertainment: "#e99d54",
  social: "#d178b3",
  neutral: "#657084",
  accent: "#8b93ff",
  warn: "#fbbf24",
  good: "#34d399",
};

export function categoryColor(category: string | null): string {
  if (!category) return COLOR.neutral;
  if (category.startsWith("entertainment")) return COLOR.entertainment;
  if (category.includes("social") || category.includes("communication")) return COLOR.social;
  if (category === "(unknown)") return COLOR.neutral;
  return COLOR.focus;
}

export function eventColor(event: TimelineEvent): string {
  if (event.kind === "intervention") return event.severity === "high" ? COLOR.warn : COLOR.accent;
  switch (event.kind) {
    case "capture":
      return "#7dd3fc";
    case "goal":
      return COLOR.good;
    case "task":
      return "#60a5fa";
    case "reminder":
      return "#f0abfc";
    case "knowledge":
      return "#a78bfa";
    case "rule":
      return "#fb923c";
    default:
      return categoryColor(event.category);
  }
}

export function scoreColor(score: number): string {
  if (score >= 70) return "rgba(52,211,153,.78)";
  if (score >= 45) return "rgba(139,147,255,.72)";
  return "rgba(251,191,36,.7)";
}

/// 折叠格子的颜色：色相取主导分类，明度取观测强度。
/// 强度用**占格子时长的比例**而不是绝对值，行与行才可比。
export function cellPaint(cell: AxisCell): { color: string; alpha: number } {
  const width = Math.max(1, cell.end_ms - cell.start_ms);
  const share = Math.min(1, cell.observed_ms / width);
  return {
    color: categoryColor(cell.top_category),
    alpha: 0.12 + share * 0.78,
  };
}

/// 单轨内的重叠错开：前台采集本应互斥，真出现重叠说明数据有问题，
/// 错开画出来比藏起来好。最多 4 条 lane，超出的压回最后一条。
export function assignLanes(events: TimelineEvent[]): Map<string, number> {
  const lanes: number[] = [];
  const out = new Map<string, number>();
  for (const event of [...events].sort((a, b) => a.start_ms - b.start_ms)) {
    let lane = lanes.findIndex((end) => end <= event.start_ms);
    if (lane === -1) {
      lane = Math.min(lanes.length, 3);
      if (lane === lanes.length) lanes.push(0);
    }
    lanes[lane] = Math.max(lanes[lane], event.end_ms);
    out.set(event.id, lane);
  }
  return out;
}

export function laneCount(lanes: Map<string, number>): number {
  let max = 0;
  for (const lane of lanes.values()) max = Math.max(max, lane);
  return max + 1;
}
