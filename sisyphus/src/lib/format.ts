/** 展示用小工具：时间、时长、app 名美化。 */

export function fmtClock(ms: number): string {
  const d = new Date(ms);
  return `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
}

export function fmtDay(ms: number): string {
  const d = new Date(ms);
  return `${d.getMonth() + 1}/${d.getDate()}`;
}

export function fmtDuration(ms: number): string {
  const min = Math.round(ms / 60000);
  if (min < 1) return "<1m";
  if (min < 60) return `${min}m`;
  const h = Math.floor(min / 60);
  const m = min % 60;
  return m ? `${h}h${m}m` : `${h}h`;
}

/** com.google.Chrome → Chrome；tv.danmaku.bili → Bili。 */
export function prettyApp(bundle?: string | null): string {
  if (!bundle) return "未知";
  const seg = bundle.split(".").pop() || bundle;
  return seg.charAt(0).toUpperCase() + seg.slice(1);
}

/** 娱乐分类的中文短标签 + 是否娱乐。 */
export function categoryLabel(category?: string | null): string | null {
  if (!category) return null;
  const map: Record<string, string> = {
    "entertainment.video": "视频",
    "entertainment.game": "游戏",
    "entertainment.social": "社交",
    "entertainment.news": "资讯",
  };
  return map[category] ?? category;
}

export function isEntertainment(category?: string | null): boolean {
  return !!category && category.startsWith("entertainment");
}

/** 到期时刻的友好显示：今天→时钟；明天→「明天 HH:MM」；更远→「M/D HH:MM」。 */
export function fmtDue(ms: number): string {
  const now = new Date();
  const d = new Date(ms);
  const days = Math.round(
    (new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime() -
      new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime()) /
      86400000,
  );
  if (days === 0) return fmtClock(ms);
  if (days === 1) return `明天 ${fmtClock(ms)}`;
  return `${fmtDay(ms)} ${fmtClock(ms)}`;
}

/** 周期串 "daily@09:00" → "每日 09:00"。 */
export function recurrenceLabel(rec?: string | null): string | null {
  if (!rec) return null;
  const m = rec.match(/^daily@(\d{1,2}):(\d{2})$/);
  if (m) return `每日 ${m[1].padStart(2, "0")}:${m[2]}`;
  return rec;
}

/** 可靠性阶梯标签 → 配色（非可靠性标签返回 null，按普通话题标签渲染）。 */
const RELIABILITY: Record<string, string> = {
  待确认: "bg-warning/15 text-warning",
  多源印证: "bg-accent/15 text-accent",
  已复现: "bg-success/15 text-success",
  已验证: "bg-success/15 text-success",
  stale: "bg-muted text-muted-foreground",
  有反证: "bg-danger/15 text-danger",
};
export function reliabilityClass(tag: string): string | null {
  return RELIABILITY[tag] ?? null;
}
export function actionLabel(kind: string, payloadJson: string): string {
  let p: Record<string, unknown> = {};
  try {
    p = JSON.parse(payloadJson);
  } catch {
    /* ignore */
  }
  const topic = typeof p.topic === "string" ? p.topic : "";
  switch (kind) {
    case "agent_run":
      if (p.mode === "introspect") return "知识库自省";
      if (p.mode === "proactive_recommendation") return "主动推荐";
      return topic ? `深研 · ${topic.length > 16 ? topic.slice(0, 16) + "…" : topic}` : "知识任务";
    case "notify":
      return typeof p.title === "string" && p.title ? p.title : "提醒";
    case "pet_message":
      return typeof p.title === "string" && p.title ? p.title : "宠物提醒";
    default:
      return kind;
  }
}
