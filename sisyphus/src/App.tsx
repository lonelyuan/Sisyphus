import { useEffect, useState } from "react";
import { invoke, addPluginListener, type PluginListener } from "@tauri-apps/api/core";
import TodayScreen from "./screens/TodayScreen";
import SettingsScreen from "./screens/SettingsScreen";

type Tab = "today" | "settings";

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

  // 监听 Kotlin UsagePlugin 推送的前台 app 事件，触发 Rust 规则评估
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
    }).then(l => { listener = l; }).catch(console.error);

    return () => { listener?.unregister(); };
  }, []);

  // 监听通知按钮响应事件
  useEffect(() => {
    let listener: PluginListener | null = null;

    addPluginListener<{ intervention_id: string; action: string }>(
      "notification",
      "action_taken",
      async ({ intervention_id, action }) => {
        try {
          await invoke("record_feedback", {
            interventionId: intervention_id,
            action,
          });
        } catch (e) {
          console.error("record_feedback error", e);
        }
      }
    ).then(l => { listener = l; }).catch(console.error);

    return () => { listener?.unregister(); };
  }, []);

  return (
    <div style={styles.root}>
      <div style={styles.content}>
        {tab === "today"    && <TodayScreen />}
        {tab === "settings" && <SettingsScreen />}
      </div>

      {/* 底部 Tab 栏 */}
      <nav style={styles.tabBar}>
        <TabItem label="今日" icon="🎯" active={tab === "today"}    onClick={() => setTab("today")} />
        <TabItem label="设置" icon="⚙️" active={tab === "settings"} onClick={() => setTab("settings")} />
      </nav>
    </div>
  );
}

function TabItem({
  label, icon, active, onClick,
}: { label: string; icon: string; active: boolean; onClick: () => void }) {
  return (
    <button
      style={{ ...styles.tabItem, color: active ? "#3b82f6" : "#888" }}
      onClick={onClick}
    >
      <span style={{ fontSize: "20px" }}>{icon}</span>
      <span style={{ fontSize: "11px" }}>{label}</span>
    </button>
  );
}

const styles: Record<string, React.CSSProperties> = {
  root: {
    display: "flex", flexDirection: "column",
    height: "100vh", overflow: "hidden",
    fontFamily: "sans-serif",
  },
  content: { flex: 1, overflowY: "auto" },
  tabBar: {
    display: "flex", borderTop: "1px solid #e5e7eb",
    background: "#fff", paddingBottom: "env(safe-area-inset-bottom, 0px)",
  },
  tabItem: {
    flex: 1, display: "flex", flexDirection: "column",
    alignItems: "center", padding: "8px 0",
    background: "none", border: "none", cursor: "pointer",
    gap: "2px",
  },
};
