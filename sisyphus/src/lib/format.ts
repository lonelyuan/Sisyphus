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
