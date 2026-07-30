import { useEffect, useState, type ReactNode } from "react";
import { invoke, addPluginListener, type PluginListener } from "@tauri-apps/api/core";
import { platform as osPlatform } from "@tauri-apps/plugin-os";
import { MessageCircle, Settings2, Waves, LayoutGrid, Network } from "lucide-react";
import AgentScreen from "./screens/AgentScreen";
import TimelineScreen from "./screens/TimelineScreen";
import LifeIndexScreen from "./screens/LifeIndexScreen";
import SkillTreeScreen from "./screens/SkillTreeScreen";
import SettingsScreen from "./screens/SettingsScreen";
import { cn } from "@/lib/utils";

type Tab = "agent" | "timeline" | "lifeindex" | "skilltree" | "settings";

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
  const [tab, setTab] = useState<Tab>("agent");

  // 监听 Kotlin UsagePlugin 推送的前台 app 事件，触发 Rust 规则评估（Android）
  useEffect(() => {
    if (!("__TAURI_INTERNALS__" in window)) return;
    let listener: PluginListener | null = null;
    let cancelled = false;
    try {
      if (osPlatform() !== "android" || cancelled) return;
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
          if (cancelled) void l.unregister();
          else listener = l;
        })
        .catch(console.error);
    } catch (e) {
      console.error("platform detection error", e);
    }
    return () => {
      cancelled = true;
      listener?.unregister();
    };
  }, []);

  // 监听通知按钮响应事件
  useEffect(() => {
    if (!("__TAURI_INTERNALS__" in window)) return;
    let listener: PluginListener | null = null;
    let cancelled = false;
    try {
      if (osPlatform() !== "android" || cancelled) return;
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
          if (cancelled) void l.unregister();
          else listener = l;
        })
        .catch(console.error);
    } catch (e) {
      console.error("platform detection error", e);
    }
    return () => {
      cancelled = true;
      listener?.unregister();
    };
  }, []);

  return (
    <div className="app-shell">
      <header className="app-topbar">
        <div className="flex items-center gap-2">
          <span className="brand-mark">S</span>
          <div>
            <span className="brand-name">西西弗斯</span>
            <span className="brand-subtitle">LOCAL COMPANION</span>
          </div>
        </div>
        <div className="resident-status">
          <span /> 感知中
        </div>
      </header>

      <div className="app-body">
        <nav className="app-sidebar" aria-label="主导航">
          <TabItem label="Agent" icon={<MessageCircle size={18} />} active={tab === "agent"} onClick={() => setTab("agent")} />
          <TabItem label="时间轴" icon={<Waves size={18} />} active={tab === "timeline"} onClick={() => setTab("timeline")} />
          <TabItem label="看板" icon={<LayoutGrid size={18} />} active={tab === "lifeindex"} onClick={() => setTab("lifeindex")} />
          <TabItem label="技能树" icon={<Network size={18} />} active={tab === "skilltree"} onClick={() => setTab("skilltree")} />
          <div className="sidebar-spacer" />
          <TabItem label="设置" icon={<Settings2 size={18} />} active={tab === "settings"} onClick={() => setTab("settings")} />
        </nav>

        <main className={cn("app-content", tab === "timeline" && "timeline-active")}>
          {/* Agent 始终挂载：切页时保留进行中的请求、草稿、滚动位置和组件状态。 */}
          <div className="app-panel agent-panel" hidden={tab !== "agent"}>
            <AgentScreen isVisible={tab === "agent"} />
          </div>
          {tab === "timeline" && (
            <div className="app-panel timeline-panel">
              <TimelineScreen />
            </div>
          )}
          {tab === "lifeindex" && (
            <div className="app-panel lifeindex-panel">
              <LifeIndexScreen />
            </div>
          )}
          {tab === "skilltree" && (
            <div className="app-panel skilltree-panel">
              <SkillTreeScreen />
            </div>
          )}
          {tab === "settings" && (
            <div className="app-panel settings-panel">
              <SettingsScreen />
            </div>
          )}
        </main>
      </div>

      <nav className="app-mobile-nav" aria-label="主导航">
        <TabItem label="Agent" icon={<MessageCircle size={18} />} active={tab === "agent"} onClick={() => setTab("agent")} />
        <TabItem label="时间轴" icon={<Waves size={18} />} active={tab === "timeline"} onClick={() => setTab("timeline")} />
        <TabItem label="看板" icon={<LayoutGrid size={18} />} active={tab === "lifeindex"} onClick={() => setTab("lifeindex")} />
        <TabItem label="技能树" icon={<Network size={18} />} active={tab === "skilltree"} onClick={() => setTab("skilltree")} />
        <TabItem label="设置" icon={<Settings2 size={18} />} active={tab === "settings"} onClick={() => setTab("settings")} />
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
        "nav-item",
        active && "active",
      )}
    >
      {icon}
      <span>{label}</span>
    </button>
  );
}
