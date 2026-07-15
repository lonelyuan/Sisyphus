import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { platform as osPlatform } from "@tauri-apps/plugin-os";
import { enable, disable, isEnabled } from "@tauri-apps/plugin-autostart";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { Power, FolderOpen, Radar, Activity, Info, Eye, Plus, X } from "lucide-react";
import { Card, CardLabel } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { prettyApp, categoryLabel } from "@/lib/format";

interface MonitoredApp {
  id: string;
  category: string;
  platform: string;
  source: string;
}

export default function SettingsScreen() {
  const [platform, setPlatform] = useState("");
  const [autostart, setAutostart] = useState<boolean | null>(null);
  const [vault, setVault] = useState("");
  const [usageGranted, setUsageGranted] = useState<boolean | null>(null);
  const [collecting, setCollecting] = useState(false);
  const [monitored, setMonitored] = useState<MonitoredApp[]>([]);
  const [newId, setNewId] = useState("");
  const [newCat, setNewCat] = useState("entertainment.video");
  const [err, setErr] = useState("");

  useEffect(() => {
    (async () => {
      let p = "";
      try {
        // 官方 JS API（比裸 invoke 可靠）：返回 "android" | "macos" | "windows" | ...
        p = await osPlatform();
      } catch {
        // 兜底：os 插件不可用时，android-only 的 usage 命令能调通即安卓。
        try {
          await invoke("check_usage_permission");
          p = "android";
        } catch {
          p = "desktop";
        }
      }
      setPlatform(p);
      if (p === "android") await refreshPermission();
    })();
    isEnabled().then(setAutostart).catch(() => setAutostart(null));
    invoke<string>("get_vault_path").then(setVault).catch(() => {});
    loadMonitored();
  }, []);

  async function refreshPermission() {
    try {
      const granted = await invoke<boolean>("check_usage_permission");
      setUsageGranted(granted);
    } catch (e) {
      setErr("检查权限失败: " + String(e));
    }
  }

  async function toggleAutostart() {
    setErr("");
    try {
      if (autostart) {
        await disable();
        setAutostart(false);
      } else {
        await enable();
        setAutostart(true);
      }
    } catch (e) {
      setErr("开机自启切换失败: " + String(e));
    }
  }

  async function openVault() {
    try {
      await revealItemInDir(vault);
    } catch (e) {
      setErr("打开知识库失败: " + String(e));
    }
  }

  function loadMonitored() {
    invoke<MonitoredApp[]>("list_monitored_apps").then(setMonitored).catch(() => {});
  }

  async function addApp() {
    const id = newId.trim();
    if (!id) return;
    setErr("");
    try {
      await invoke("add_monitored_app", { id, category: newCat });
      setNewId("");
      loadMonitored();
    } catch (e) {
      setErr("添加失败: " + String(e));
    }
  }

  async function removeApp(id: string) {
    try {
      await invoke("remove_monitored_app", { id });
      loadMonitored();
    } catch (e) {
      setErr("删除失败: " + String(e));
    }
  }

  async function requestPermission() {
    setErr("");
    try {
      await invoke("request_usage_permission");
      setTimeout(refreshPermission, 1500);
      setTimeout(refreshPermission, 3000);
    } catch (e) {
      setErr("跳转授权失败: " + String(e));
    }
  }

  async function toggleCollector() {
    setErr("");
    try {
      await invoke(collecting ? "stop_collector" : "start_collector");
      setCollecting(!collecting);
    } catch (e) {
      setErr("采集服务操作失败: " + String(e));
    }
  }

  const isAndroid = platform === "android";
  const apps = monitored.filter(
    (m) => m.platform === (isAndroid ? "android" : "desktop") || m.platform === "custom",
  );

  return (
    <div className="animate-in mx-auto flex max-w-md flex-col gap-3 p-4">
      {err && (
        <Card className="border-danger/40 bg-danger/10 p-3 text-xs text-danger">{err}</Card>
      )}

      {/* 监控名单（增删查改） */}
      <Card className="flex flex-col gap-3 p-4">
        <div className="flex items-center gap-2">
          <Eye size={14} strokeWidth={1.75} className="text-muted-foreground" />
          <CardLabel>监控名单（{apps.length}）</CardLabel>
        </div>

        {/* 增：包名 + 分类 */}
        <div className="flex gap-2">
          <Input
            value={newId}
            onChange={(e) => setNewId(e.target.value)}
            placeholder={isAndroid ? "包名，如 com.ss.android.ugc.aweme" : "bundle id，如 com.apple.TV"}
            onKeyDown={(e) => e.key === "Enter" && addApp()}
          />
          <select
            value={newCat}
            onChange={(e) => setNewCat(e.target.value)}
            className="h-9 shrink-0 rounded-md border border-input bg-input px-2 text-xs text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/40"
          >
            <option value="entertainment.video">视频</option>
            <option value="entertainment.game">游戏</option>
            <option value="entertainment.social">社交</option>
            <option value="entertainment.news">资讯</option>
          </select>
          <Button variant="secondary" size="icon" onClick={addApp} aria-label="添加监控 app">
            <Plus size={16} strokeWidth={2} />
          </Button>
        </div>

        {/* 查 + 删 */}
        {apps.length ? (
          <ul className="flex flex-col gap-1.5">
            {apps.map((m) => (
              <li key={m.platform + m.id} className="group flex items-center gap-2">
                <span className="flex-1 truncate text-sm">{prettyApp(m.id)}</span>
                <code className="max-w-[120px] truncate font-mono text-[10px] text-muted-foreground/60">
                  {m.id}
                </code>
                <span className="shrink-0 rounded bg-warning/15 px-1.5 py-0.5 text-[10px] text-warning">
                  {categoryLabel(m.category)}
                </span>
                {m.source === "user" ? (
                  <button
                    onClick={() => removeApp(m.id)}
                    className="shrink-0 text-muted-foreground/40 transition-colors hover:text-danger"
                    aria-label="删除"
                  >
                    <X size={14} strokeWidth={2} />
                  </button>
                ) : (
                  <span className="w-[14px] shrink-0 text-center text-[10px] text-muted-foreground/40">·</span>
                )}
              </li>
            ))}
          </ul>
        ) : (
          <p className="text-sm text-muted-foreground">当前平台暂无监控项。</p>
        )}
        <p className="text-[11px] leading-relaxed text-muted-foreground">
          停留在名单内的 app 超阈值触发干预。自定义项跨端即时生效（安卓无需重编）；内置项不可删，可用同名自定义项覆盖分类。
          {!isAndroid && " 桌面浏览器内刷视频需浏览器插件（延后）。"}
        </p>
      </Card>

      {!isAndroid && (
        <>
          {/* 后台常驻 */}
          <Card className="flex items-start gap-3 p-4">
            <Radar size={16} strokeWidth={1.75} className="mt-0.5 shrink-0 text-accent" />
            <div className="flex flex-col gap-1">
              <CardLabel>后台常驻</CardLabel>
              <p className="text-xs leading-relaxed text-muted-foreground">
                关窗后从程序坞隐藏、仅保留菜单栏图标（不占程序坞）；采集器持续在后台运行。点菜单栏图标唤回窗口，「退出」才结束进程。
              </p>
            </div>
          </Card>

          {/* 开机自启 */}
          <Card className="flex items-center justify-between gap-3 p-4">
            <div className="flex items-start gap-3">
              <Power size={16} strokeWidth={1.75} className="mt-0.5 shrink-0 text-muted-foreground" />
              <div className="flex flex-col gap-1">
                <CardLabel>开机自启</CardLabel>
                <p className="text-xs text-muted-foreground">登录时自动启动，跨重启常驻采集</p>
              </div>
            </div>
            <Switch
              checked={autostart ?? false}
              disabled={autostart === null}
              onCheckedChange={toggleAutostart}
            />
          </Card>

          {/* 知识库 */}
          <Card className="flex flex-col gap-3 p-4">
            <div className="flex items-center gap-2">
              <FolderOpen size={14} strokeWidth={1.75} className="text-muted-foreground" />
              <CardLabel>第二大脑知识库</CardLabel>
            </div>
            <code className="block truncate rounded-md border border-border bg-muted px-2.5 py-2 font-mono text-[11px] text-muted-foreground">
              {vault || "…"}
            </code>
            <Button variant="secondary" size="sm" className="self-start" onClick={openVault}>
              <FolderOpen size={14} strokeWidth={1.75} />
              在 Finder 打开（可作为 Obsidian 库）
            </Button>
          </Card>
        </>
      )}

      {isAndroid && (
        <>
          <Card className="flex flex-col gap-3 p-4">
            <div className="flex items-center gap-2">
              <Activity size={14} strokeWidth={1.75} className="text-muted-foreground" />
              <CardLabel>应用使用情况权限</CardLabel>
            </div>
            <div className="flex items-center justify-between">
              <span
                className={
                  "rounded-full px-2 py-0.5 text-[11px] " +
                  (usageGranted ? "bg-success/15 text-success" : "bg-danger/15 text-danger")
                }
              >
                {usageGranted === null ? "检查中…" : usageGranted ? "已授权" : "未授权"}
              </span>
              {!usageGranted && (
                <Button size="sm" onClick={requestPermission}>
                  前往授权
                </Button>
              )}
            </div>
          </Card>

          <Card className="flex items-center justify-between gap-3 p-4">
            <div className="flex flex-col gap-1">
              <CardLabel>采集服务</CardLabel>
              <p className="text-xs text-muted-foreground">后台持续采集（前台通知常驻）</p>
            </div>
            <Button
              variant={collecting ? "secondary" : "primary"}
              size="sm"
              disabled={!usageGranted}
              onClick={toggleCollector}
            >
              {collecting ? "停止" : "启动"}
            </Button>
          </Card>
        </>
      )}

      {/* 页脚 */}
      <div className="mt-1 flex items-center justify-between px-1 text-[11px] text-muted-foreground">
        <span className="flex items-center gap-1.5">
          <Info size={12} strokeWidth={1.75} />
          Sisyphus v0.1.0 · Phase 1
        </span>
        <span>{platform || "…"}</span>
      </div>
    </div>
  );
}
