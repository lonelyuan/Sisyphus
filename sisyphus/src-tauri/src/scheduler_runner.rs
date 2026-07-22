//! 主动触发 ticker（app 层 · 感知平面常驻）。
//!
//! `sisyphus-core::scheduler` 只给"到期集合"（纯数据、安卓可编）；本模块负责**平台相关副作用**：
//! 到点弹通知 / 拉起反思平面 agent（codex）。守铁律：这些副作用绝不进 core。
//!
//! MVP 骨架：只派发 `notify` 与 `agent_run`；种一个"每日 9 点知识库自省" job。
//! `agent_run` 靠环境变量 `SISYPHUS_AGENT_CMD`（如 `node /abs/path/knowledge-agent/index.mjs`）
//! 拉起；未配置则优雅降级（打日志、标 failed），不硬依赖 codex 就位。

use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use serde_json::Value;
use tauri::AppHandle;
use tauri_plugin_notification::NotificationExt;

use sisyphus_core::db;
use sisyphus_core::scheduler::{self, NewAction, ScheduledAction};

const TICK_SECS: u64 = 30;

/// 线程入口：独立连接（与 App 同库，WAL 并发），播种周期 job，循环 due-check。
pub fn run(db_path: PathBuf, vault_dir: PathBuf, app: AppHandle) {
    let conn = match db::open(db_path.to_str().unwrap_or_default()) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[scheduler] open db failed: {e}");
            return;
        }
    };
    let _ = conn.busy_timeout(Duration::from_secs(5));
    seed_daily_jobs(&conn);
    eprintln!("[scheduler] 主动触发 ticker 启动，tick={TICK_SECS}s");

    loop {
        let now = chrono::Utc::now().timestamp_millis();
        match scheduler::due_actions(&conn, now) {
            Ok(due) => {
                for a in &due {
                    dispatch(&conn, &vault_dir, &app, a, now);
                }
            }
            Err(e) => eprintln!("[scheduler] due_actions error: {e}"),
        }
        std::thread::sleep(Duration::from_secs(TICK_SECS));
    }
}

/// 幂等播种静态周期 job（dedup_key 保证重启不重复入队）。
fn seed_daily_jobs(conn: &rusqlite::Connection) {
    let now = chrono::Utc::now().timestamp_millis();
    let due = scheduler::next_due("daily@09:00", now).unwrap_or(now);
    let payload = r#"{"mode":"introspect","topic":"知识库自省：去碎片化 + 找薄弱点 + 提议学习方向"}"#;
    match scheduler::enqueue_action(
        conn,
        &NewAction {
            kind: "agent_run",
            payload_json: payload,
            due_at_ms: due,
            recurrence: Some("daily@09:00"),
            dedup_key: Some("daily-kb-introspect"),
            origin_event_id: None,
            created_by: "scheduler",
        },
    ) {
        Ok(Some(id)) => eprintln!("[scheduler] 播种每日9点知识自省 job: {id}"),
        Ok(None) => {} // 已存在 pending，幂等跳过
        Err(e) => eprintln!("[scheduler] seed job failed: {e}"),
    }
}

/// 按 kind 派发。周期动作无论成败先排下一次（保证不断链），再执行本次副作用。
fn dispatch(
    conn: &rusqlite::Connection,
    vault_dir: &PathBuf,
    app: &AppHandle,
    a: &ScheduledAction,
    now: i64,
) {
    if a.recurrence.is_some() {
        if let Err(e) = scheduler::reschedule(conn, a, now) {
            eprintln!("[scheduler] reschedule {} failed: {e}", a.id);
        }
    }

    let ok = match a.kind.as_str() {
        "notify" => do_notify(app, &a.payload_json),
        "agent_run" => do_agent_run(vault_dir, &a.payload_json),
        other => {
            eprintln!("[scheduler] 未知 kind「{other}」跳过（执行器待实现）");
            false
        }
    };

    let _ = if ok {
        scheduler::mark_done(conn, &a.id)
    } else {
        scheduler::mark_failed(conn, &a.id)
    };
}

fn do_notify(app: &AppHandle, payload_json: &str) -> bool {
    let v: Value = serde_json::from_str(payload_json).unwrap_or(Value::Null);
    let title = v.get("title").and_then(|x| x.as_str()).unwrap_or("西西弗斯");
    let body = v.get("body").and_then(|x| x.as_str()).unwrap_or("");
    match app.notification().builder().title(title).body(body).show() {
        Ok(_) => true,
        Err(e) => {
            eprintln!("[scheduler] notify failed: {e}");
            false
        }
    }
}

/// 拉起反思平面 agent（fire-and-forget）。命令来自 `SISYPHUS_AGENT_CMD`；
/// 把 topic 作为参数、`SISYPHUS_VAULT` 作为环境传入。未配置则降级返回 false。
fn do_agent_run(vault_dir: &PathBuf, payload_json: &str) -> bool {
    let cmd_line = match std::env::var("SISYPHUS_AGENT_CMD") {
        Ok(s) if !s.trim().is_empty() => s,
        _ => {
            eprintln!("[scheduler] agent_run 跳过：未设 SISYPHUS_AGENT_CMD（codex 派发未接入，暂降级）");
            return false;
        }
    };
    let v: Value = serde_json::from_str(payload_json).unwrap_or(Value::Null);
    let topic = v.get("topic").and_then(|x| x.as_str()).unwrap_or("");

    let mut parts = cmd_line.split_whitespace();
    let program = match parts.next() {
        Some(p) => p,
        None => return false,
    };
    let mut cmd = Command::new(program);
    cmd.args(parts);
    if !topic.is_empty() {
        cmd.arg(topic);
    }
    cmd.env("SISYPHUS_VAULT", vault_dir);

    match cmd.spawn() {
        Ok(_child) => {
            eprintln!("[scheduler] agent_run 已拉起：{program} …「{topic}」");
            true // fire-and-forget：拉起成功即记 done；agent 经 MCP 自行写回
        }
        Err(e) => {
            eprintln!("[scheduler] agent_run spawn failed: {e}");
            false
        }
    }
}
