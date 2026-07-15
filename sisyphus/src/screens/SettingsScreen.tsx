import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

export default function SettingsScreen() {
  const [usageGranted, setUsageGranted] = useState<boolean | null>(null);
  const [collecting, setCollecting] = useState(false);
  const [errMsg, setErrMsg] = useState("");
  const [platformName, setPlatformName] = useState("");

  // 检测平台 + 刷新权限，独立 effect 避免闭包捕获过期值
  useEffect(() => {
    detectPlatformAndRefresh();
  }, []);

  async function detectPlatformAndRefresh() {
    // 通过 Tauri 命令安全获取平台名，不依赖 plugin-os 初始化时序
    try {
      const p = await invoke<string>("plugin:os|platform");
      setPlatformName(p);
      if (p === "android") await refreshPermission();
    } catch {
      // plugin-os 未初始化时降级：直接尝试 checkPermission，失败则是非 Android
      try {
        const res = await invoke<{ granted: boolean }>("plugin:usage|checkPermission");
        setPlatformName("android");
        setUsageGranted(res.granted);
      } catch {
        setPlatformName("desktop");
      }
    }
  }

  async function refreshPermission() {
    try {
      const res = await invoke<{ granted: boolean }>("plugin:usage|checkPermission");
      setUsageGranted(res.granted);
    } catch (e: unknown) {
      setErrMsg("checkPermission 失败: " + String(e));
    }
  }

  async function requestPermission() {
    setErrMsg("");
    try {
      await invoke("plugin:usage|requestPermission");
      // 等用户从设置页返回后刷新（onResume 会触发，但 WebView 内无此回调，轮询一次）
      setTimeout(refreshPermission, 1500);
      setTimeout(refreshPermission, 3000);
    } catch (e: unknown) {
      setErrMsg("requestPermission 失败: " + String(e));
    }
  }

  async function toggleCollector() {
    setErrMsg("");
    try {
      if (collecting) {
        await invoke("plugin:usage|stopCollector");
        setCollecting(false);
      } else {
        await invoke("plugin:usage|startCollector");
        setCollecting(true);
      }
    } catch (e: unknown) {
      setErrMsg("采集服务操作失败: " + String(e));
    }
  }

  const isAndroid = platformName === "android";

  return (
    <div style={s.container}>
      <h2 style={s.title}>设置</h2>

      {/* 错误提示 */}
      {errMsg ? (
        <div style={s.errBox}>
          <strong>错误</strong>
          <p style={{ margin: "4px 0 0", fontSize: "12px", wordBreak: "break-all" }}>{errMsg}</p>
        </div>
      ) : null}

      {/* 平台调试信息 */}
      <div style={s.card}>
        <p style={s.label}>平台</p>
        <p style={s.desc}>{platformName || "检测中…"}</p>
      </div>

      {isAndroid && (
        <>
          <div style={s.card}>
            <p style={s.label}>应用使用情况权限</p>
            <div style={s.permRow}>
              <div>
                <span style={{ ...s.badge, background: usageGranted ? "#22c55e" : "#ef4444" }}>
                  {usageGranted === null ? "检查中…" : usageGranted ? "已授权" : "未授权"}
                </span>
                <p style={s.desc}>采集前台 app 切换事件，用于分析娱乐时长</p>
              </div>
              {!usageGranted && (
                <button style={s.btnPrimary} onClick={requestPermission}>
                  前往授权
                </button>
              )}
            </div>
          </div>

          <div style={s.card}>
            <p style={s.label}>采集服务</p>
            <div style={s.row}>
              <span style={s.desc}>后台持续采集（前台通知常驻）</span>
              <button
                style={collecting ? s.btnStop : s.btnStart}
                onClick={toggleCollector}
                disabled={!usageGranted}
              >
                {collecting ? "停止" : "启动"}
              </button>
            </div>
            {!usageGranted && <p style={s.hint}>需先授予"应用使用情况"权限</p>}
          </div>
        </>
      )}

      {!isAndroid && platformName && (
        <div style={s.card}>
          <p style={s.label}>桌面端</p>
          <p style={s.desc}>活动窗口采集在 Phase 2 实现。</p>
        </div>
      )}

      <div style={s.card}>
        <p style={s.label}>版本</p>
        <p style={s.desc}>Sisyphus v0.1.0 · Phase 1</p>
      </div>
    </div>
  );
}

const s: Record<string, React.CSSProperties> = {
  container: { padding: "16px", fontFamily: "sans-serif", maxWidth: "480px", margin: "0 auto" },
  title: { fontSize: "20px", fontWeight: 700, marginBottom: "16px" },
  card: { background: "#f8f8f8", borderRadius: "12px", padding: "16px", marginBottom: "12px" },
  errBox: {
    background: "#fef2f2", border: "1px solid #fca5a5", borderRadius: "12px",
    padding: "12px 16px", marginBottom: "12px", color: "#b91c1c",
  },
  label: { fontSize: "12px", color: "#888", marginBottom: "8px", textTransform: "uppercase" },
  permRow: { display: "flex", justifyContent: "space-between", alignItems: "flex-start" },
  badge: { fontSize: "12px", color: "#fff", padding: "2px 8px", borderRadius: "999px" },
  desc: { fontSize: "13px", color: "#555", marginTop: "4px" },
  hint: { fontSize: "12px", color: "#f59e0b", marginTop: "8px" },
  row: { display: "flex", justifyContent: "space-between", alignItems: "center" },
  btnPrimary: {
    padding: "6px 14px", borderRadius: "8px", background: "#3b82f6",
    color: "#fff", border: "none", cursor: "pointer", fontSize: "13px", whiteSpace: "nowrap",
  },
  btnStart: {
    padding: "6px 14px", borderRadius: "8px", background: "#22c55e",
    color: "#fff", border: "none", cursor: "pointer", fontSize: "13px",
  },
  btnStop: {
    padding: "6px 14px", borderRadius: "8px", background: "#6b7280",
    color: "#fff", border: "none", cursor: "pointer", fontSize: "13px",
  },
};
