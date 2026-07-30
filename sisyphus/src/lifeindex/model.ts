/// LifeIndex 的共享写入模型：看板与技能树用**同一条写入路径**（`upsert_life_item`），
/// 所以草稿类型、日期换算与 payload 组装只能有一份。
///
/// `origin` 由命令层强制为 `app`，前端不能伪装成已同步的 Notion/import 写入。

export type LifeKind = "idea" | "goal" | "project" | "action" | "routine" | "skill" | "milestone";
export type LifeTrack = "main" | "side" | "neutral" | "undecided";
export type LifeHorizon = "now" | "next" | "later" | "someday" | "unscheduled";
export type LifeStatus = "inbox" | "active" | "waiting" | "done" | "archived";

export interface LifeItem {
  id: string;
  kind: LifeKind;
  title: string;
  body: string;
  track: LifeTrack;
  horizon: LifeHorizon;
  status: LifeStatus;
  area_id: string | null;
  success_criteria: string | null;
  target_value: number | null;
  current_value: number | null;
  unit: string | null;
  start_at_ms: number | null;
  due_at_ms: number | null;
  review_at_ms: number | null;
  recurrence: string | null;
  sync_status: "clean" | "local_dirty" | "notion_dirty" | "conflict";
  revision: number;
  updated_at: number;
}

export interface LifeArea {
  id: string;
  name: string;
  description: string;
  sort_order: number;
  focus: boolean;
}

export interface Draft {
  id?: string;
  expected_revision?: number;
  kind: LifeKind;
  title: string;
  body: string;
  track: LifeTrack;
  horizon: LifeHorizon;
  status: LifeStatus;
  area_id: string;
  success_criteria: string;
  target_value: string;
  current_value: string;
  unit: string;
  start_date: string;
  due_date: string;
  review_date: string;
  recurrence: string;
}

export const kindLabel: Record<LifeKind, string> = {
  idea: "想法",
  goal: "目标",
  project: "项目",
  action: "事项",
  routine: "日常",
  skill: "技能",
  milestone: "里程碑",
};
export const horizonLabel: Record<LifeHorizon, string> = {
  now: "现在",
  next: "近期",
  later: "以后",
  someday: "也许",
  unscheduled: "未定",
};
export const statusLabel: Record<LifeStatus, string> = {
  inbox: "待整理",
  active: "进行中",
  waiting: "等待",
  done: "完成",
  archived: "已归档",
};
export const trackOptions: string[][] = [
  ["undecided", "未判断"],
  ["main", "主线"],
  ["side", "支线"],
  ["neutral", "中性"],
];

export const emptyDraft: Draft = {
  kind: "action",
  title: "",
  body: "",
  track: "undecided",
  horizon: "unscheduled",
  status: "inbox",
  area_id: "",
  success_criteria: "",
  target_value: "",
  current_value: "",
  unit: "",
  start_date: "",
  due_date: "",
  review_date: "",
  recurrence: "",
};

export function dateValue(ms: number | null) {
  return ms ? new Date(ms).toISOString().slice(0, 10) : "";
}

export function dateMs(value: string) {
  return value ? new Date(`${value}T12:00:00`).getTime() : null;
}

function numberValue(value: number | null) {
  return value === null || value === undefined ? "" : String(value);
}

function numberMs(value: string) {
  const trimmed = value.trim();
  if (!trimmed) return null;
  const parsed = Number(trimmed);
  return Number.isFinite(parsed) ? parsed : null;
}

export function toDraft(item: LifeItem): Draft {
  return {
    id: item.id,
    expected_revision: item.revision,
    kind: item.kind,
    title: item.title,
    body: item.body,
    track: item.track,
    horizon: item.horizon,
    status: item.status,
    area_id: item.area_id ?? "",
    success_criteria: item.success_criteria ?? "",
    target_value: numberValue(item.target_value),
    current_value: numberValue(item.current_value),
    unit: item.unit ?? "",
    start_date: dateValue(item.start_at_ms),
    due_date: dateValue(item.due_at_ms),
    review_date: dateValue(item.review_at_ms),
    recurrence: item.recurrence ?? "",
  };
}

export function payload(draft: Draft) {
  return {
    id: draft.id ?? null,
    expected_revision: draft.expected_revision ?? null,
    kind: draft.kind,
    title: draft.title,
    body: draft.body,
    track: draft.track,
    horizon: draft.horizon,
    status: draft.status,
    area_id: draft.area_id || null,
    success_criteria: draft.success_criteria.trim() || null,
    target_value: numberMs(draft.target_value),
    current_value: numberMs(draft.current_value),
    unit: draft.unit.trim() || null,
    start_at_ms: dateMs(draft.start_date),
    due_at_ms: dateMs(draft.due_date),
    review_at_ms: dateMs(draft.review_date),
    recurrence: draft.recurrence.trim() || null,
    source_event_id: null,
    intent_id: null,
    origin: "app",
    external_ref: null,
  };
}

/** 技能树上的可判定形态：这些 kind 该填完成条件与度量，否则永远无法收敛。 */
export function wantsCriteria(kind: LifeKind) {
  return kind === "goal" || kind === "milestone" || kind === "skill";
}
