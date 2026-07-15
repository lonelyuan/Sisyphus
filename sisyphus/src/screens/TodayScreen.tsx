import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Check, X, Plus, Bell, Clock, CircleCheck, Circle, Ban } from "lucide-react";
import { Card, CardLabel } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { cn } from "@/lib/utils";
import { fmtClock } from "@/lib/format";

interface DailyGoal { id: string; date: string; raw_text: string; status: string }
interface Task {
  id: string;
  title: string;
  status: string;
  due_ms: number | null;
  priority: number;
  note: string | null;
  created_at: number;
}
interface Reminder {
  id: string;
  text: string;
  remind_at_ms: number;
  status: string;
  recurrence: string | null;
  created_at: number;
}
interface TodayContext {
  date: string;
  goal: DailyGoal | null;
  entertainment_minutes: number;
  intervention_count: number;
  due_reminders: Reminder[];
}

const GOAL_STATUS: Record<string, { label: string; className: string }> = {
  planned: { label: "计划中", className: "text-muted-foreground" },
  started: { label: "进行中", className: "text-accent" },
  completed: { label: "已完成", className: "text-success" },
  skipped: { label: "已跳过", className: "text-muted-foreground" },
  abandoned: { label: "已放弃", className: "text-danger" },
};

export default function TodayScreen() {
  const [ctx, setCtx] = useState<TodayContext | null>(null);
  const [tasks, setTasks] = useState<Task[]>([]);
  const [goalInput, setGoalInput] = useState("");
  const [taskInput, setTaskInput] = useState("");
  const [saving, setSaving] = useState(false);

  async function load() {
    try {
      const [c, t] = await Promise.all([
        invoke<TodayContext>("get_today_context"),
        invoke<Task[]>("list_tasks"),
      ]);
      setCtx(c);
      setTasks(t);
      if (c.goal && !goalInput) setGoalInput(c.goal.raw_text);
    } catch (e) {
      console.error("load today failed", e);
    }
  }

  useEffect(() => {
    load();
    const id = setInterval(load, 30_000);
    return () => clearInterval(id);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  async function saveGoal() {
    if (!goalInput.trim()) return;
    setSaving(true);
    try {
      await invoke("set_goal", { text: goalInput.trim() });
      await load();
    } finally {
      setSaving(false);
    }
  }

  async function setGoalStatus(status: string) {
    if (!ctx?.goal) return;
    await invoke("update_goal_status", { id: ctx.goal.id, status });
    await load();
  }

  async function addTask() {
    const title = taskInput.trim();
    if (!title) return;
    setTaskInput("");
    await invoke("create_task", { title, dueMs: null, note: null });
    await load();
  }

  async function toggleTask(t: Task) {
    const next = t.status === "done" ? "todo" : "done";
    await invoke("set_task_status", { id: t.id, status: next });
    await load();
  }

  async function removeTask(id: string) {
    await invoke("delete_task", { id });
    await load();
  }

  async function completeReminder(id: string) {
    await invoke("set_reminder_status", { id, status: "done" });
    await load();
  }

  const entMin = Math.round(ctx?.entertainment_minutes ?? 0);
  const goal = ctx?.goal ?? null;
  const dueReminders = ctx?.due_reminders ?? [];
  const st = goal ? GOAL_STATUS[goal.status] ?? { label: goal.status, className: "text-muted-foreground" } : null;
  const goalOpen = goal ? goal.status === "planned" || goal.status === "started" : false;
  const openCount = tasks.filter((t) => t.status === "todo" || t.status === "doing").length;

  return (
    <div className="animate-in mx-auto flex max-w-md flex-col gap-3 p-4">
      {/* 今日目标 */}
      <Card className="flex flex-col gap-3 p-4">
        <div className="flex items-center justify-between">
          <CardLabel>今日目标</CardLabel>
          {st && <span className={cn("text-xs font-medium", st.className)}>{st.label}</span>}
        </div>
        {goal ? (
          <p className="text-[15px] font-medium leading-snug">{goal.raw_text}</p>
        ) : (
          <p className="text-sm text-muted-foreground">还没设定今天要专注的那一件事。</p>
        )}
        <div className="flex gap-2">
          <Input
            value={goalInput}
            onChange={(e) => setGoalInput(e.target.value)}
            placeholder={goal ? "修改目标…" : "输入今日目标…"}
            onKeyDown={(e) => e.key === "Enter" && saveGoal()}
          />
          <Button variant="primary" onClick={saveGoal} disabled={saving}>
            {saving ? "…" : goal ? "更新" : "设定"}
          </Button>
        </div>
        {goal && goalOpen && (
          <div className="flex gap-2">
            <Button variant="success" size="sm" onClick={() => setGoalStatus("completed")}>
              <Check size={15} strokeWidth={2} />
              完成
            </Button>
            <Button variant="ghost" size="sm" onClick={() => setGoalStatus("abandoned")}>
              <Ban size={14} strokeWidth={1.75} />
              放弃今日
            </Button>
          </div>
        )}
      </Card>

      {/* 今日数据 */}
      <Card className="p-4">
        <CardLabel>今日数据</CardLabel>
        <div className="mt-3 grid grid-cols-3 gap-3">
          <Stat value={`${entMin}`} unit="min" label="娱乐时长" warn={entMin > 60} />
          <Stat value={`${ctx?.intervention_count ?? 0}`} label="干预" />
          <Stat value={`${openCount}`} label="待办" />
        </div>
      </Card>

      {/* 任务（增删查改） */}
      <Card className="flex flex-col gap-3 p-4">
        <CardLabel>任务</CardLabel>
        <div className="flex gap-2">
          <Input
            value={taskInput}
            onChange={(e) => setTaskInput(e.target.value)}
            placeholder="加一条任务…"
            onKeyDown={(e) => e.key === "Enter" && addTask()}
          />
          <Button variant="secondary" size="icon" onClick={addTask} aria-label="添加任务">
            <Plus size={16} strokeWidth={2} />
          </Button>
        </div>
        {tasks.length > 0 ? (
          <ul className="flex flex-col">
            {tasks.map((t) => {
              const done = t.status === "done" || t.status === "dropped";
              return (
                <li key={t.id} className="group flex items-center gap-2.5 py-1.5">
                  <button onClick={() => toggleTask(t)} className="shrink-0 text-muted-foreground transition-colors hover:text-accent" aria-label="切换完成">
                    {done ? (
                      <CircleCheck size={17} strokeWidth={1.75} className="text-success" />
                    ) : (
                      <Circle size={17} strokeWidth={1.75} />
                    )}
                  </button>
                  <span className={cn("flex-1 text-sm leading-snug", done && "text-muted-foreground line-through")}>
                    {t.title}
                  </span>
                  <button
                    onClick={() => removeTask(t.id)}
                    className="shrink-0 text-muted-foreground/40 opacity-0 transition-opacity hover:text-danger group-hover:opacity-100"
                    aria-label="删除任务"
                  >
                    <X size={15} strokeWidth={2} />
                  </button>
                </li>
              );
            })}
          </ul>
        ) : (
          <p className="text-sm text-muted-foreground">还没有任务。加一条，或对 Codex 说句话。</p>
        )}
      </Card>

      {/* 到期提醒 */}
      {dueReminders.length > 0 && (
        <Card className="border-warning/30 p-4">
          <div className="flex items-center gap-2">
            <Bell size={14} strokeWidth={1.75} className="text-warning" />
            <CardLabel className="text-warning/90">到期提醒</CardLabel>
          </div>
          <ul className="mt-3 flex flex-col gap-2">
            {dueReminders.map((r) => (
              <li key={r.id} className="flex items-center gap-2 text-sm">
                <Clock size={13} strokeWidth={1.75} className="shrink-0 text-muted-foreground" />
                <span className="flex-1">{r.text}</span>
                <span className="text-[11px] text-muted-foreground">{fmtClock(r.remind_at_ms)}</span>
                <button onClick={() => completeReminder(r.id)} className="text-muted-foreground hover:text-success" aria-label="完成提醒">
                  <Check size={15} strokeWidth={2} />
                </button>
              </li>
            ))}
          </ul>
        </Card>
      )}
    </div>
  );
}

function Stat({ value, unit, label, warn }: { value: string; unit?: string; label: string; warn?: boolean }) {
  return (
    <div className="flex flex-col gap-0.5">
      <div className="flex items-baseline gap-1">
        <span className={cn("font-mono text-2xl font-semibold tabular-nums", warn && "text-warning")}>{value}</span>
        {unit && <span className="text-xs text-muted-foreground">{unit}</span>}
      </div>
      <span className="text-[11px] text-muted-foreground">{label}</span>
    </div>
  );
}
