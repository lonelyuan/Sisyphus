import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

interface DailyGoal {
  id: string;
  date: string;
  raw_text: string;
  status: string;
}

interface TodayContext {
  date: string;
  goal: DailyGoal | null;
  entertainment_minutes: number;
  intervention_count: number;
}

const STATUS_LABELS: Record<string, string> = {
  planned: "计划中",
  started: "进行中",
  completed: "已完成",
  skipped: "已跳过",
  abandoned: "已放弃",
};

export default function TodayScreen() {
  const [ctx, setCtx] = useState<TodayContext | null>(null);
  const [goalInput, setGoalInput] = useState("");
  const [saving, setSaving] = useState(false);

  async function loadContext() {
    try {
      const data = await invoke<TodayContext>("get_today_context");
      setCtx(data);
      if (data.goal) setGoalInput(data.goal.raw_text);
    } catch (e) {
      console.error("get_today_context failed", e);
    }
  }

  useEffect(() => {
    loadContext();
    const id = setInterval(loadContext, 30_000);
    return () => clearInterval(id);
  }, []);

  async function saveGoal() {
    if (!goalInput.trim()) return;
    setSaving(true);
    try {
      await invoke("set_goal", { text: goalInput.trim() });
      await loadContext();
    } finally {
      setSaving(false);
    }
  }

  async function markDone() {
    if (!ctx?.goal) return;
    await invoke("update_goal_status", { id: ctx.goal.id, status: "completed" });
    await loadContext();
  }

  const entertainmentMin = Math.round(ctx?.entertainment_minutes ?? 0);
  const interventionCount = ctx?.intervention_count ?? 0;

  return (
    <div style={styles.container}>
      <h2 style={styles.title}>今日</h2>

      {/* 目标卡片 */}
      <div style={styles.card}>
        <p style={styles.label}>今日目标</p>
        {ctx?.goal ? (
          <div>
            <p style={styles.goalText}>{ctx.goal.raw_text}</p>
            <p style={styles.status}>{STATUS_LABELS[ctx.goal.status] ?? ctx.goal.status}</p>
            {ctx.goal.status === "planned" || ctx.goal.status === "started" ? (
              <button style={styles.btnSuccess} onClick={markDone}>标记完成</button>
            ) : null}
          </div>
        ) : (
          <p style={styles.empty}>还没有设置今日目标</p>
        )}
        <div style={styles.inputRow}>
          <input
            style={styles.input}
            value={goalInput}
            onChange={e => setGoalInput(e.target.value)}
            placeholder={ctx?.goal ? "修改目标…" : "输入今日目标…"}
            onKeyDown={e => e.key === "Enter" && saveGoal()}
          />
          <button style={styles.btnPrimary} onClick={saveGoal} disabled={saving}>
            {saving ? "…" : ctx?.goal ? "更新" : "设置"}
          </button>
        </div>
      </div>

      {/* 统计卡片 */}
      <div style={styles.card}>
        <p style={styles.label}>今日数据</p>
        <div style={styles.statsRow}>
          <StatItem
            value={`${entertainmentMin} 分钟`}
            label="娱乐时长"
            warn={entertainmentMin > 60}
          />
          <StatItem value={`${interventionCount} 次`} label="干预次数" />
        </div>
      </div>
    </div>
  );
}

function StatItem({ value, label, warn }: { value: string; label: string; warn?: boolean }) {
  return (
    <div style={styles.statItem}>
      <p style={{ ...styles.statValue, color: warn ? "#e05" : "#222" }}>{value}</p>
      <p style={styles.statLabel}>{label}</p>
    </div>
  );
}

const styles: Record<string, React.CSSProperties> = {
  container: { padding: "16px", fontFamily: "sans-serif", maxWidth: "480px", margin: "0 auto" },
  title: { fontSize: "20px", fontWeight: 700, marginBottom: "16px" },
  card: {
    background: "#f8f8f8", borderRadius: "12px", padding: "16px", marginBottom: "12px",
  },
  label: { fontSize: "12px", color: "#888", marginBottom: "6px", textTransform: "uppercase" },
  goalText: { fontSize: "16px", fontWeight: 600, marginBottom: "4px" },
  status: { fontSize: "13px", color: "#555", marginBottom: "8px" },
  empty: { fontSize: "14px", color: "#aaa", marginBottom: "8px" },
  inputRow: { display: "flex", gap: "8px", marginTop: "8px" },
  input: {
    flex: 1, padding: "8px 12px", borderRadius: "8px",
    border: "1px solid #ddd", fontSize: "14px",
  },
  btnPrimary: {
    padding: "8px 16px", borderRadius: "8px", background: "#3b82f6",
    color: "#fff", border: "none", cursor: "pointer", fontSize: "14px",
  },
  btnSuccess: {
    padding: "6px 12px", borderRadius: "8px", background: "#22c55e",
    color: "#fff", border: "none", cursor: "pointer", fontSize: "13px",
    marginBottom: "8px",
  },
  statsRow: { display: "flex", gap: "24px" },
  statItem: { textAlign: "center" },
  statValue: { fontSize: "22px", fontWeight: 700, margin: 0 },
  statLabel: { fontSize: "12px", color: "#888", margin: 0 },
};
