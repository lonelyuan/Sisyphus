/// 时间轴读模型（与 core::timeline 的 TimelineResponse 一一对应）。
///
/// 约定：**所有时间边界都由后端给**。前端不用 `Date` 推日界——
/// 换日点是数据库里的用户设置（`settings.day_boundary_hour`），
/// 前端算出来的"午夜"是 UTC 或系统时区午夜，和 rollup 的日桶对不上。

export type Detail = "minute" | "day" | "week" | "life";

/// 折叠档位。线性轴按周期取模，一个周期一行。
/// `day` 即 actogram（作息栅格图），`week` 的 7 列形态就是传统日历。
export type Fold = "none" | "day" | "week" | "month" | "year";

export const FOLDS: Fold[] = ["none", "day", "week", "month", "year"];

export const FOLD_LABEL: Record<Fold, string> = {
  none: "线性",
  day: "按日",
  week: "按周",
  month: "按月",
  year: "按年",
};

export type EventKind =
  | "behavior"
  | "intervention"
  | "system"
  | "capture"
  | "goal"
  | "task"
  | "reminder"
  | "knowledge"
  | "rule";

export interface TimelineEvent {
  /// 显著性等级：0 人生尺度仍可见，3 只在日/分钟尺度可见（LOD 过滤由后端做）。
  level?: number;
  id: string;
  kind: EventKind;
  start_ms: number;
  end_ms: number;
  title: string;
  category: string | null;
  detail: string | null;
  severity: string | null;
}

export interface DaySummary {
  date: string;
  start_ms: number;
  observed_ms: number;
  focus_ms: number;
  entertainment_ms: number;
  neutral_ms: number;
  intervention_count: number;
  /// 0–100 的可解释启发式分。刻意不做默认可见轨道：它比看起来更粗。
  state_score: number;
}

/// 预聚合条带：粗尺度的主数据，桶粒度随可见跨度自动变粗（day → week → month）。
export interface TimeBand {
  bucket_start_ms: number;
  bucket_end_ms: number;
  observed_ms: number;
  focus_ms: number;
  entertainment_ms: number;
  neutral_ms: number;
  top_category: string | null;
}

/// 长期计划图层（LifeDB）：目标/项目/技能的跨度 + 里程碑点，progress 由 Core 算出。
export interface PlanSpan {
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

export interface Tick {
  ms: number;
  label: string;
  /// 0 = 主刻度（带标签），1 = 次刻度（只有短线）。
  tier: number;
  day_start: boolean;
}

export interface FoldRow {
  index: number;
  start_ms: number;
  end_ms: number;
  label: string;
  sub_label: string;
}

export interface FoldGrid {
  fold: string;
  cols: number;
  col_unit: string;
  rows: FoldRow[];
  truncated: boolean;
}

/// 折叠网格里的一格。`row`/`col` 由后端算好，前端只负责落位。
export interface AxisCell {
  start_ms: number;
  end_ms: number;
  row: number;
  col: number;
  observed_ms: number;
  focus_ms: number;
  entertainment_ms: number;
  neutral_ms: number;
  top_category: string | null;
}

export interface TimelineResponse {
  start_ms: number;
  end_ms: number;
  detail: Detail;
  bucket: string;
  events: TimelineEvent[];
  days: DaySummary[];
  bands: TimeBand[];
  plans: PlanSpan[];
  truncated: boolean;
  has_long_term_source: boolean;
  ticks: Tick[];
  tick_unit: string;
  tick_minor_unit: string;
  fold: string;
  grid: FoldGrid;
  /// none | session | hour | day —— 折叠视图这一档用什么粒度铺格子。
  cell_kind: string;
  cells: AxisCell[];
  boundary_hour: number;
}

export interface KeyDuration {
  key: string;
  duration_ms: number;
}

export interface RangeStats {
  windows: number;
  covered_ms: number;
  observed_ms: number;
  focus_ms: number;
  entertainment_ms: number;
  neutral_ms: number;
  session_count: number;
  top_categories: KeyDuration[];
  top_apps: KeyDuration[];
  intervention_count: number;
  intervention_switched: number;
  capture_count: number;
  artifact_count: number;
  truncated: boolean;
}

export const EMPTY: TimelineResponse = {
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
  ticks: [],
  tick_unit: "",
  tick_minor_unit: "",
  fold: "none",
  grid: { fold: "none", cols: 0, col_unit: "none", rows: [], truncated: false },
  cell_kind: "none",
  cells: [],
  boundary_hour: 0,
};

export const MINUTE = 60_000;
export const HOUR = 3_600_000;
export const DAY = 86_400_000;
