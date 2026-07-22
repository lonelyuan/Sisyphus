//! macOS 前台应用采集器（感知平面第一根真实电线）。
//!
//! 后台线程每隔几秒轮询前台 app，经 `sisyphus_core::ingest_event` 写 `app_foreground`
//! 事件，喂给规则引擎；命中即弹本地通知。刻意做到最蠢：只取前台 app bundle id + 硬编码分类，
//! 不做浏览器 URL、不做 Android（见 docs/roadmap.md B 轨）。
//!
//! 仅 macOS 编译（`#[cfg(target_os = "macos")]` 在 lib.rs 处 gate）。

use std::path::PathBuf;
use std::time::Duration;

use chrono::Utc;
use rusqlite::Connection;
use tauri::AppHandle;
use tauri_plugin_notification::NotificationExt;

use sisyphus_core::category::categorize;
use sisyphus_core::ingest::{ingest_event, NewEvent};
use sisyphus_core::rule_engine::config::RuleConfig;
use sisyphus_core::rule_engine::{RuleContext, RuleEngine};
use sisyphus_core::db;

const DEVICE_ID: &str = "macos-desktop";
const USER_ID: &str = "local-user";

#[cfg(debug_assertions)]
const POLL_SECS: u64 = 5;
#[cfg(not(debug_assertions))]
const POLL_SECS: u64 = 15;

/// 进行中的前台会话（内存态，防漏算：未切走的会话不在 DB）。
struct Session {
    bundle: String,
    category: Option<String>,
    start_ms: i64,
}

/// 线程入口：打开独立连接（WAL 支持与 App 同库并发），循环轮询。
pub fn run(db_path: PathBuf, app: AppHandle) {
    let conn = match db::open(db_path.to_str().unwrap_or_default()) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[collector] open db failed: {e}");
            return;
        }
    };
    let _ = conn.busy_timeout(Duration::from_secs(5));
    let engine = RuleEngine::new(RuleConfig::default());

    let mut session: Option<Session> = None;
    eprintln!("[collector] macOS 前台采集器启动，poll={POLL_SECS}s（监控名单在 App 设置页可增删）");

    loop {
        if let Err(e) = tick(&conn, &engine, &app, &mut session) {
            eprintln!("[collector] tick error: {e}");
        }
        std::thread::sleep(Duration::from_secs(POLL_SECS));
    }
}

fn tick(
    conn: &Connection,
    engine: &RuleEngine,
    app: &AppHandle,
    session: &mut Option<Session>,
) -> Result<(), String> {
    let now = Utc::now().timestamp_millis();
    let front = match frontmost_app() {
        Some(b) => b,
        None => return Ok(()), // 取不到（如锁屏/无权限），本 tick 跳过
    };
    // 统一分类（用户表 + 内置白名单，热生效）。
    let category = categorize(conn, &front).map_err(|e| e.to_string())?;
    eprintln!("[collector] front={front} category={category:?}");

    // 前台 app 变化：关闭上一段会话，写成 app_foreground 区间事件。
    let changed = session.as_ref().map(|s| s.bundle != front).unwrap_or(true);
    if changed {
        if let Some(prev) = session.take() {
            write_foreground_event(conn, &prev, now)?;
        }
        *session = Some(Session {
            bundle: front.clone(),
            category: category.clone(),
            start_ms: now,
        });
    }

    // 当前进行中的娱乐时长（防漏算，注入规则）。
    let active_ms = match session.as_ref() {
        Some(s) if is_entertainment(s.category.as_deref()) => now - s.start_ms,
        _ => 0,
    };

    let today = Utc::now().format("%Y-%m-%d").to_string();
    let goal = db::get_today_goal(conn, &today).map_err(|e| e.to_string())?;

    let ctx = RuleContext {
        now_ms: now,
        user_id: USER_ID.to_string(),
        device_id: DEVICE_ID.to_string(),
        current_app: Some(front),
        current_category: category.clone(),
        active_entertainment_ms: active_ms,
        media_playing_since_ms: 0,
        recent_scroll_count: 0,
        today_goal: goal,
    };

    if let Some(out) = sisyphus_core::intervention::evaluate_and_record(conn, engine, &ctx)
        .map_err(|e| e.to_string())?
    {
        notify(app, &out.message);
        eprintln!("[collector] 干预触发 rule={} sev={}", out.rule_id, out.severity);
    }

    // 到期提醒：到点弹通知（原子标记 fired 防重复）。
    for r in sisyphus_core::artifacts::take_due_reminders(conn, now).map_err(|e| e.to_string())? {
        notify(app, &format!("⏰ 提醒：{}", r.text));
        eprintln!("[collector] 提醒触发: {}", r.text);
    }

    Ok(())
}

fn write_foreground_event(conn: &Connection, s: &Session, end_ms: i64) -> Result<(), String> {
    ingest_event(
        conn,
        USER_ID,
        DEVICE_ID,
        NewEvent {
            event_id: None,
            source: "desktop_agent".to_string(),
            layer: "raw".to_string(),
            event_type: "app_foreground".to_string(),
            time_mode: "interval".to_string(),
            event_time: None,
            start_time: Some(s.start_ms),
            end_time: Some(end_ms),
            entity: Some(s.bundle.clone()),
            category: s.category.clone(),
            payload: serde_json::json!({}),
            parent_event_ids: vec![],
            privacy_level: "L0".to_string(),
        },
    )
    .map(|_| ())
    .map_err(|e| e.to_string())
}

fn is_entertainment(cat: Option<&str>) -> bool {
    cat.map(|c| c.starts_with("entertainment")).unwrap_or(false)
}

/// 取前台应用 bundle id。走 System Events(首次弹一次自动化授权即可,拿 bundle id 无需其他权限)。
/// 无 bundle id 的进程返回 "missing value" → 跳过。
/// 注:若将来要采集**窗口标题**(L1/L2),macOS 还需 Accessibility / 屏幕录制权限,成本更高——MVP 只取 bundle id。
fn frontmost_app() -> Option<String> {
    let out = std::process::Command::new("osascript")
        .arg("-e")
        .arg("tell application \"System Events\" to get bundle identifier of first application process whose frontmost is true")
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() || s == "missing value" {
        return None;
    }
    Some(s)
}

fn notify(app: &AppHandle, body: &str) {
    if let Err(e) = app
        .notification()
        .builder()
        .title("西西弗斯")
        .body(body)
        .show()
    {
        eprintln!("[collector] notification failed: {e}");
    }
}
