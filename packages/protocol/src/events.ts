// Sisyphus 事件协议 v1.0 — TypeScript 类型
// 权威定义见 ../SPEC.md。本文件供浏览器插件、桌面前端、Supabase Edge Functions 使用。
// Kotlin / Rust 端各自维护等价定义，须与 SPEC.md 一致。

export const SCHEMA_VERSION = "1.0" as const;

export type Source =
  | "android_usage"
  | "android_accessibility"
  | "desktop_agent"
  | "browser_extension"
  | "engine"
  | "manual"
  | "agent";

export type Layer =
  | "raw"
  | "session"
  | "finding"
  | "decision"
  | "intervention"
  | "outcome"
  | "feedback";

export type TimeMode = "point" | "interval";

export type PrivacyLevel = "L0" | "L1" | "L2" | "L3";

/** 统一信封：所有事件共用。差异放进 payload。 */
export interface BehaviorEvent<P extends object = Record<string, unknown>> {
  schema_version: typeof SCHEMA_VERSION;
  event_id: string; // uuid，端侧生成，幂等主键
  user_id: string; // MVP: "local-user"
  device_id: string;
  seq_no: number; // 单设备单调递增
  source: Source;
  layer: Layer;
  type: string; // 见 SPEC §4
  time_mode: TimeMode;
  event_time?: string; // point 时使用 (RFC3339)
  start_time?: string; // interval 时使用
  end_time?: string | null; // 进行中为 null
  entity?: string | null; // 包名 / 域名
  category?: string | null; // entertainment.video 等
  payload: P;
  parent_event_ids: string[]; // 血缘；raw 层为 []
  privacy_level: PrivacyLevel;
  produced_at: string; // RFC3339
}

// ---- payload 形状（MVP 子集，见 SPEC §4）----

/** manual/note_text：用户手动输入的自然语言 capture（L1）。 */
export interface NoteTextPayload {
  text: string;
}

export interface UrlVisitPayload {
  url_hash?: string;
  title?: string; // L2，默认不传
}

export interface WindowActivePayload {
  window_title?: string; // L1/L2
}

export interface ScrollBurstPayload {
  scroll_count: number;
  window_sec: number;
  avg_interval_ms: number;
}

export interface FindingPayload {
  rule_id: string;
  rule_version: number;
  severity: string;
  confidence: number;
  context_snapshot: Record<string, unknown>;
  recommended_intervention_types: string[];
  status: "shadow" | "suppressed" | "actioned" | "dismissed";
}

export interface PolicyDecisionPayload {
  policy_id: string;
  policy_version: string;
  feature_schema_version: string;
  available_actions: string[];
  chosen_action: string;
  choice_probability: number; // 倾向分，必填
  scores: Record<string, number>;
  exploration: boolean;
  constraints: Record<string, unknown>;
}

export interface InterventionPayload {
  target_device_id: string;
  message: string;
  options: string[];
  bct_type?: string;
}

export interface ProximalOutcomePayload {
  window: "10m" | "30m" | "60m" | "day_end";
  observed: Record<string, unknown>;
  reward_components: Record<string, number>;
  reward_total: number;
}

export interface UserFeedbackPayload {
  intervention_id: string;
  label:
    | "helpful"
    | "annoying"
    | "guilt"
    | "normal_rest"
    | "wrong_time"
    | string;
  text?: string;
}

/** 批量上传请求体：POST /events/batch */
export interface EventBatch {
  events: BehaviorEvent[];
}
