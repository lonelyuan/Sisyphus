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
use tauri::{AppHandle, Emitter};
use tauri_plugin_notification::NotificationExt;

use sisyphus_core::category::categorize;
use sisyphus_core::db;
use sisyphus_core::ingest::{ingest_event, NewEvent};
use sisyphus_core::rule_engine::config::RuleConfig;
use sisyphus_core::rule_engine::{RuleContext, RuleEngine};

const DEVICE_ID: &str = "macos-desktop";
const USER_ID: &str = "local-user";

#[cfg(debug_assertions)]
const POLL_SECS: u64 = 5;
#[cfg(not(debug_assertions))]
const POLL_SECS: u64 = 15;

/// 进行中会话的落盘间隔。**不能只在切换应用时才写事件**：一直待在同一个 app（看两小时
/// 电影、连续刷两小时）时，Event log 里会一条记录都没有——时间线、rollup、近端结果观察
/// 全都看不到这段时间。每 5 分钟把"已经过去的这一段"作为闭合区间落盘（Event log 保持
/// append-only，不改历史事件）。
const FLUSH_SECS: i64 = 300;

/// 进行中的前台会话（内存态，防漏算：未切走的会话不在 DB）。
struct Session {
    bundle: String,
    category: Option<String>,
    start_ms: i64,
    /// 已落盘到哪一刻（切片式落盘的水位）。
    flushed_until_ms: i64,
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
    // 前台 app 变化：关闭上一段会话，写成 app_foreground 区间事件。
    let changed = session.as_ref().map(|s| s.bundle != front).unwrap_or(true);
    if changed {
        if let Some(prev) = session.take() {
            write_foreground_event(conn, &prev, prev.flushed_until_ms, now)?;
        }
        *session = Some(Session {
            bundle: front.clone(),
            category: category.clone(),
            start_ms: now,
            flushed_until_ms: now,
        });
    } else if let Some(s) = session.as_mut() {
        // 长时间停在同一个 app：每 FLUSH_SECS 落一个闭合切片，避免"两小时无记录"。
        if now - s.flushed_until_ms >= FLUSH_SECS * 1000 {
            let from = s.flushed_until_ms;
            let snapshot = Session {
                bundle: s.bundle.clone(),
                category: s.category.clone(),
                start_ms: s.start_ms,
                flushed_until_ms: from,
            };
            write_foreground_event(conn, &snapshot, from, now)?;
            s.flushed_until_ms = now;
        }
    }

    // 当前进行中的娱乐时长（防漏算，注入规则）。
    let active_ms = match session.as_ref() {
        Some(s) if is_entertainment(s.category.as_deref()) => now - s.start_ms,
        _ => 0,
    };
    // 当前前台会话总时长（不限分类），供动态规则补入未闭合会话。
    let active_session_ms = session.as_ref().map(|s| now - s.start_ms).unwrap_or(0);

    // 「今天」统一由 core::clock 定义（本地时区 + 换日点），不再用 UTC 日期。
    let today = sisyphus_core::clock::today_str(conn);
    let goal = db::get_today_goal(conn, &today).map_err(|e| e.to_string())?;

    let ctx = RuleContext {
        now_ms: now,
        user_id: USER_ID.to_string(),
        device_id: DEVICE_ID.to_string(),
        current_app: Some(front),
        current_category: category.clone(),
        active_entertainment_ms: active_ms,
        active_session_ms,
        media_playing_since_ms: 0,
        recent_scroll_count: 0,
        today_goal: goal,
    };

    if let Some(out) = sisyphus_core::intervention::evaluate_and_record(conn, engine, &ctx)
        .map_err(|e| e.to_string())?
    {
        // ResponsePolicy::Immediate 的命中当拍派发；Deferred/Debounce 已入队交给 ticker。
        match out.kind.as_str() {
            "pet_message" => {
                let _ = app.emit("pet-message", out.message.clone());
            }
            _ => notify(app, &out.message),
        }
        eprintln!(
            "[collector] 干预触发 rule={} sev={} kind={}",
            out.rule_id, out.severity, out.kind
        );
    }

    // 到期提醒：到点弹通知（原子标记 fired 防重复）。
    for r in sisyphus_core::artifacts::take_due_reminders(conn, now).map_err(|e| e.to_string())? {
        notify(app, &format!("⏰ 提醒：{}", r.text));
        eprintln!("[collector] 提醒触发: {}", r.text);
    }

    Ok(())
}

fn write_foreground_event(
    conn: &Connection,
    s: &Session,
    from_ms: i64,
    end_ms: i64,
) -> Result<(), String> {
    if end_ms <= from_ms {
        return Ok(());
    }
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
            start_time: Some(from_ms),
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
