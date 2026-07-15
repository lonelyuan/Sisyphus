import { useEffect, useState, type ReactNode } from "react";
import { invoke, addPluginListener, type PluginListener } from "@tauri-apps/api/core";
import { Target, ScrollText, Settings2 } from "lucide-react";
import TodayScreen from "./screens/TodayScreen";
import RecordsScreen from "./screens/RecordsScreen";
import SettingsScreen from "./screens/SettingsScreen";
import { cn } from "@/lib/utils";

type Tab = "today" | "records" | "settings";

interface UsageEvent {
  pkg: string;
  category: string;
  active_ms: number;
}

interface FindingOutput {
  rule_id: string;
  severity: string;
  message: string;
  intervention_id: string;
}

export default function App() {
  const [tab, setTab] = useState<Tab>("today");

  // 监听 Kotlin UsagePlugin 推送的前台 app 事件，触发 Rust 规则评估（Android）
  useEffect(() => {
    let listener: PluginListener | null = null;
    addPluginListener<UsageEvent>("usage", "usage_event", async (event) => {
      try {
        const finding = await invoke<FindingOutput | null>("evaluate_rules", {
          ctx: {
            current_app: event.pkg || null,
            current_category: event.category || null,
            active_entertainment_ms: event.active_ms ?? 0,
            media_playing_since_ms: 0,
            recent_scroll_count: 0,
          },
        });
        if (finding) {
          await invoke("plugin:notification|showIntervention", {
            message: finding.message,
            interventionId: finding.intervention_id,
          });
        }
      } catch (e) {
        console.error("evaluate_rules error", e);
      }
    })
      .then((l) => {
        listener = l;
      })
      .catch(console.error);
    return () => {
      listener?.unregister();
    };
  }, []);

  // 监听通知按钮响应事件
  useEffect(() => {
    let listener: PluginListener | null = null;
    addPluginListener<{ intervention_id: string; action: string }>(
      "notification",
      "action_taken",
      async ({ intervention_id, action }) => {
        try {
          await invoke("record_feedback", { interventionId: intervention_id, action });
        } catch (e) {
          console.error("record_feedback error", e);
        }
      },
    )
      .then((l) => {
        listener = l;
      })
      .catch(console.error);
    return () => {
      listener?.unregister();
    };
  }, []);

  return (
    <div className="flex h-screen flex-col bg-background text-foreground">
      {/* 顶栏 */}
      <header className="flex shrink-0 items-center justify-between border-b border-border px-5 py-3">
        <div className="flex items-center gap-2">
          <span className="text-[15px] leading-none">⛰</span>
          <span className="text-sm font-semibold tracking-tight">西西弗斯</span>
        </div>
        <div className="flex items-center gap-1.5 text-[11px] text-muted-foreground">
          <span className="h-1.5 w-1.5 rounded-full bg-success" />
          常驻中
        </div>
      </header>

      {/* 内容区（唯一滚动区域） */}
      <main className="flex-1 overflow-y-auto">
        {tab === "today" && <TodayScreen />}
        {tab === "records" && <RecordsScreen />}
        {tab === "settings" && <SettingsScreen />}
      </main>

      {/* 底部 tab 栏 */}
      <nav className="flex shrink-0 border-t border-border">
        <TabItem
          label="今日"
          icon={<Target size={18} strokeWidth={1.75} />}
          active={tab === "today"}
          onClick={() => setTab("today")}
        />
        <TabItem
          label="记录"
          icon={<ScrollText size={18} strokeWidth={1.75} />}
          active={tab === "records"}
          onClick={() => setTab("records")}
        />
        <TabItem
          label="设置"
          icon={<Settings2 size={18} strokeWidth={1.75} />}
          active={tab === "settings"}
          onClick={() => setTab("settings")}
        />
      </nav>
    </div>
  );
}

function TabItem({
  label,
  icon,
  active,
  onClick,
}: {
  label: string;
  icon: ReactNode;
  active: boolean;
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      className={cn(
        "flex flex-1 flex-col items-center gap-1 py-2.5 text-[11px] transition-colors",
        active ? "text-accent" : "text-muted-foreground hover:text-foreground",
      )}
    >
      {icon}
      <span>{label}</span>
    </button>
  );
}
