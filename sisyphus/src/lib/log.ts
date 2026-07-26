// 轻量分级日志。控制台级别可运行时配（localStorage sisyphus_log_level，默认 info），
// 并转发到 tauri-plugin-log（写文件 + 终端 stdout，与 Rust 端日志统一）。
//   控制台改级别：setLogLevel('info')  级别：debug<info<warn<error<silent
//   文件位置：应用 log 目录（macOS ~/Library/Logs/com.sisyphus/）。

import { debug as pDebug, info as pInfo, warn as pWarn, error as pError } from "@tauri-apps/plugin-log";

export type Level = "debug" | "info" | "warn" | "error" | "silent";
const ORDER: Record<Level, number> = { debug: 10, info: 20, warn: 30, error: 40, silent: 99 };

function current(): Level {
  try {
    const v = localStorage.getItem("sisyphus_log_level") as Level | null;
    if (v && v in ORDER) return v;
  } catch {
    /* ignore */
  }
  return "info";
}

function fmt(a: unknown): string {
  if (typeof a === "string") return a;
  try {
    return JSON.stringify(a);
  } catch {
    return String(a);
  }
}

function emit(lvl: Exclude<Level, "silent">, scope: string, args: unknown[]) {
  if (ORDER[lvl] < ORDER[current()]) return;
  const color =
    lvl === "error" ? "#f87171" : lvl === "warn" ? "#fbbf24" : lvl === "info" ? "#8b93ff" : "#8a8c93";
  const cfn = lvl === "error" ? console.error : lvl === "warn" ? console.warn : console.log;
  const t = new Date().toISOString().slice(11, 23);
  cfn(`%c${t} [${lvl.toUpperCase()}] ${scope}`, `color:${color};font-weight:600`, ...args);
  // 转发到 tauri-plugin-log（文件 + stdout）。非 Tauri 环境静默忽略。
  const pfn = lvl === "error" ? pError : lvl === "warn" ? pWarn : lvl === "info" ? pInfo : pDebug;
  try {
    void pfn(`${scope} ${args.map(fmt).join(" ")}`)?.catch?.(() => {});
  } catch {
    /* ignore */
  }
}

export function logger(scope: string) {
  return {
    debug: (...a: unknown[]) => emit("debug", scope, a),
    info: (...a: unknown[]) => emit("info", scope, a),
    warn: (...a: unknown[]) => emit("warn", scope, a),
    error: (...a: unknown[]) => emit("error", scope, a),
  };
}

export function setLogLevel(l: Level) {
  try {
    localStorage.setItem("sisyphus_log_level", l);
    console.log(`[log] 控制台级别已设为 ${l}`);
  } catch {
    /* ignore */
  }
}

// 方便在控制台直接调：window.setLogLevel('info')
try {
  (globalThis as unknown as { setLogLevel?: typeof setLogLevel }).setLogLevel = setLogLevel;
} catch {
  /* ignore */
}
