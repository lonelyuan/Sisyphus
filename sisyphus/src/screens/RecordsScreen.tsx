import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Activity, Bell, BookOpen, Circle } from "lucide-react";
import { Card, CardLabel } from "@/components/ui/card";
import { cn } from "@/lib/utils";
import { fmtClock, fmtDay, fmtDuration, prettyApp, categoryLabel, isEntertainment, reliabilityClass } from "@/lib/format";

interface SessionRow {
  entity: string | null;
  category: string | null;
  start_time: number;
  end_time: number | null;
  duration_ms: number | null;
}
interface InterventionRow {
  id: string;
  rule_id: string;
  shown_at: number;
  severity: string;
  message: string;
  user_response: string | null;
  responded_at: number | null;
  outcome: string | null;
}
interface KnowledgeNote {
  id: string;
  path: string;
  title: string;
  tags: string[];
  sources: string[];
  status: string;
  updated_at: number;
}

const RESP: Record<string, { label: string; className: string }> = {
  start_task: { label: "开始任务", className: "bg-success/15 text-success" },
  take_rest: { label: "休息一下", className: "bg-accent/15 text-accent" },
  continue: { label: "继续娱乐", className: "bg-muted text-muted-foreground" },
  abandon_today: { label: "今天放弃", className: "bg-danger/15 text-danger" },
};

export default function RecordsScreen() {
  const [sessions, setSessions] = useState<SessionRow[]>([]);
  const [interventions, setInterventions] = useState<InterventionRow[]>([]);
  const [knowledge, setKnowledge] = useState<KnowledgeNote[]>([]);

  async function load() {
    try {
      const [s, i, k] = await Promise.all([
        invoke<SessionRow[]>("list_sessions"),
        invoke<InterventionRow[]>("list_interventions"),
        invoke<KnowledgeNote[]>("list_knowledge"),
      ]);
      setSessions(s);
      setInterventions(i);
      setKnowledge(k);
    } catch (e) {
      console.error("load records failed", e);
    }
  }

  useEffect(() => {
    load();
    const id = setInterval(load, 30_000);
    return () => clearInterval(id);
  }, []);

  return (
    <div className="animate-in mx-auto flex max-w-md flex-col gap-3 p-4">
      {/* 干预历史 */}
      <Card className="p-4">
        <div className="flex items-center gap-2">
          <Bell size={14} strokeWidth={1.75} className="text-muted-foreground" />
          <CardLabel>干预历史</CardLabel>
        </div>
        {interventions.length ? (
          <ul className="mt-3 flex flex-col gap-3">
            {interventions.map((it) => {
              const resp = it.user_response ? RESP[it.user_response] : null;
              return (
                <li key={it.id} className="flex flex-col gap-1">
                  <div className="flex items-center gap-2">
                    <span className="font-mono text-[11px] text-muted-foreground">
                      {fmtDay(it.shown_at)} {fmtClock(it.shown_at)}
                    </span>
                    {it.severity === "high" && <span className="text-[11px] text-warning">⚠️ 高</span>}
                    <span
                      className={cn(
                        "ml-auto rounded-full px-2 py-0.5 text-[10px]",
                        resp ? resp.className : "bg-muted text-muted-foreground/60",
                      )}
                    >
                      {resp ? resp.label : "未响应"}
                    </span>
                  </div>
                  <p className="whitespace-pre-line text-xs leading-snug text-foreground/80">{it.message}</p>
                </li>
              );
            })}
          </ul>
        ) : (
          <Empty text="还没有干预。设个目标、刷会儿娱乐 app 就会触发。" />
        )}
      </Card>

      {/* 行为时间轴 */}
      <Card className="p-4">
        <div className="flex items-center gap-2">
          <Activity size={14} strokeWidth={1.75} className="text-muted-foreground" />
          <CardLabel>行为记录</CardLabel>
        </div>
        {sessions.length ? (
          <ul className="mt-3 flex flex-col">
            {sessions.map((s, idx) => {
              const ent = isEntertainment(s.category);
              const cat = categoryLabel(s.category);
              return (
                <li key={idx} className="flex items-center gap-2.5 py-1.5">
                  <Circle
                    size={7}
                    className={cn("shrink-0", ent ? "fill-warning text-warning" : "fill-muted-foreground/40 text-muted-foreground/40")}
                  />
                  <span className="flex-1 truncate text-sm">{prettyApp(s.entity)}</span>
                  {cat && (
                    <span className={cn("rounded px-1.5 py-0.5 text-[10px]", ent ? "bg-warning/15 text-warning" : "bg-muted text-muted-foreground")}>
                      {cat}
                    </span>
                  )}
                  <span className="w-10 text-right font-mono text-[11px] text-muted-foreground">
                    {s.duration_ms != null ? fmtDuration(s.duration_ms) : "进行中"}
                  </span>
                  <span className="w-9 text-right font-mono text-[11px] text-muted-foreground/70">{fmtClock(s.start_time)}</span>
                </li>
              );
            })}
          </ul>
        ) : (
          <Empty text="还没有采集到行为。桌面端需 App 在跑；安卓需授予使用情况权限并启动采集。" />
        )}
      </Card>

      {/* 知识卡片 */}
      <Card className="p-4">
        <div className="flex items-center gap-2">
          <BookOpen size={14} strokeWidth={1.75} className="text-muted-foreground" />
          <CardLabel>知识库（{knowledge.length}）</CardLabel>
        </div>
        {knowledge.length ? (
          <ul className="mt-3 flex flex-col gap-2">
            {knowledge.map((k) => {
              const relTag = k.tags.find((t) => reliabilityClass(t));
              const topicTags = k.tags.filter((t) => !reliabilityClass(t));
              return (
                <li key={k.id} className="flex flex-col gap-1">
                  <div className="flex items-center gap-2">
                    <span className="min-w-0 flex-1 truncate text-sm">{k.title}</span>
                    {relTag && (
                      <span className={cn("shrink-0 rounded-full px-2 py-0.5 text-[10px]", reliabilityClass(relTag))}>
                        {relTag}
                      </span>
                    )}
                  </div>
                  {topicTags.length > 0 && (
                    <div className="flex flex-wrap gap-1">
                      {topicTags.map((t) => (
                        <span key={t} className="rounded bg-muted px-1.5 py-0.5 text-[10px] text-muted-foreground">
                          #{t}
                        </span>
                      ))}
                    </div>
                  )}
                </li>
              );
            })}
          </ul>
        ) : (
          <Empty text="知识库为空。让 Codex 用 write_knowledge_note 写卡片，或在设置里打开 vault。" />
        )}
      </Card>
    </div>
  );
}

function Empty({ text }: { text: string }) {
  return <p className="mt-3 text-sm leading-relaxed text-muted-foreground">{text}</p>;
}
