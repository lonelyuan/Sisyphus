//! 主动触发 ticker（app 层 · 感知平面常驻）。
//!
//! `sisyphus-core::scheduler` 只给"到期集合"（纯数据、安卓可编）；本模块负责**平台相关副作用**：
//! 到点弹通知 / 拉起反思平面 agent。守铁律：这些副作用绝不进 core。
//!
//! `agent_run` 与主对话共用可替换的只读 Agent runtime；默认自动发现 Pi / Codex，
//! 不再要求用户设置 `SISYPHUS_AGENT_CMD`。

use std::path::PathBuf;
use std::time::Duration;

use serde_json::Value;
use tauri::{AppHandle, Emitter};
use tauri_plugin_notification::NotificationExt;

use crate::agent_runtime;
use sisyphus_core::db;
use sisyphus_core::scheduler::{self, NewAction, ScheduledAction};

const TICK_SECS: u64 = 30;

/// 线程入口：独立连接（与 App 同库，WAL 并发），播种周期 job，循环 due-check。
pub fn run(db_path: PathBuf, _vault_dir: PathBuf, data_dir: PathBuf, app: AppHandle) {
    let conn = match db::open(db_path.to_str().unwrap_or_default()) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[scheduler] open db failed: {e}");
            return;
        }
    };
    seed_daily_jobs(&conn);
    eprintln!("[scheduler] 主动触发 ticker 启动，tick={TICK_SECS}s");

    loop {
        let now = chrono::Utc::now().timestamp_millis();
        // 无极时间线的预聚合桶：每拍追平（没有新行为事件时只花一次索引查询）。
        if let Err(e) = sisyphus_core::rollups::catch_up(&conn) {
            eprintln!("[scheduler] rollup catch_up error: {e}");
        }
        match scheduler::due_actions(&conn, now) {
            Ok(due) => {
                for a in &due {
                    dispatch(&conn, &db_path, &data_dir, &app, a, now);
                }
            }
            Err(e) => eprintln!("[scheduler] due_actions error: {e}"),
        }
        std::thread::sleep(Duration::from_secs(TICK_SECS));
    }
}

/// 幂等播种静态周期 job（dedup_key 保证重启不重复入队）。
fn seed_daily_jobs(conn: &rusqlite::Connection) {
    // 旧版“知识库自省”会让 agent 写知识库，和当前只读边界冲突；保留审计记录但停止待执行项。
    let _ = conn.execute(
        "UPDATE scheduled_actions SET status='cancelled' WHERE dedup_key='daily-kb-introspect' AND status='pending'",
        [],
    );
    let now = chrono::Utc::now().timestamp_millis();
    let due = scheduler::next_due("daily@09:00", now).unwrap_or(now);
    let payload = r#"{"mode":"proactive_recommendation","topic":"结合当前行为状态与用户只读信息源，判断此刻是否值得主动推荐一件事"}"#;
    match scheduler::enqueue_action(
        conn,
        &NewAction {
            kind: "agent_run",
            payload_json: payload,
            due_at_ms: due,
            recurrence: Some("daily@09:00"),
            dedup_key: Some("daily-proactive-recommendation"),
            origin_event_id: None,
            created_by: "scheduler",
        },
    ) {
        Ok(Some(id)) => eprintln!("[scheduler] 播种每日9点主动推荐 job: {id}"),
        Ok(None) => {} // 已存在 pending，幂等跳过
        Err(e) => eprintln!("[scheduler] seed job failed: {e}"),
    }

    // 周回顾（GTD weekly review）：LifeDB 的判断由 Core 算好，agent 只负责问与措辞。
    // 这是"看板不变成新焦虑源"的唯一已被反复验证的机制。
    let wr_due = scheduler::next_due("daily@20:10", now).unwrap_or(now);
    match scheduler::enqueue_action(
        conn,
        &NewAction {
            kind: "agent_run",
            payload_json: r#"{"mode":"weekly_review","topic":"周回顾：读 review_queue 与 next_actions，只在周日提问，其余日子返回 no_recommendation"}"#,
            due_at_ms: wr_due,
            recurrence: Some("daily@20:10"),
            dedup_key: Some("weekly-review"),
            origin_event_id: None,
            created_by: "scheduler",
        },
    ) {
        Ok(Some(id)) => eprintln!("[scheduler] 播种周回顾 job: {id}"),
        Ok(None) => {}
        Err(e) => eprintln!("[scheduler] seed weekly review failed: {e}"),
    }

    // 取消旧版单向只读刷新，迁移到每日双向 LifeIndexSync。
    let _ = conn.execute(
        "UPDATE scheduled_actions SET status='cancelled'
         WHERE dedup_key='daily-lifeindex-refresh' AND status='pending'",
        [],
    );
    let li_due = scheduler::next_due("daily@08:30", now).unwrap_or(now);
    let li_payload = r#"{"mode":"lifeindex_sync","topic":"三方语义合并 LifeDB、Notion 当前文本与上次成功快照，并把严格四视图投影写回 Notion"}"#;
    match scheduler::enqueue_action(
        conn,
        &NewAction {
            kind: "agent_run",
            payload_json: li_payload,
            due_at_ms: li_due,
            recurrence: Some("daily@08:30"),
            dedup_key: Some("daily-lifeindex-sync"),
            origin_event_id: None,
            created_by: "scheduler",
        },
    ) {
        Ok(Some(id)) => eprintln!("[scheduler] 播种每日8:30 LifeIndex 双向同步 job: {id}"),
        Ok(None) => {}
        Err(e) => eprintln!("[scheduler] seed lifeindex job failed: {e}"),
    }
}

/// 按 kind 派发。周期动作无论成败先排下一次（保证不断链），再执行本次副作用。
/// `agent_run` 会拉起 LLM（10–60s），放到 worker 线程执行，避免阻塞 30s due-check 主循环
/// 里同一批的 `notify`（本该即时、确定性）。notify / pet_message 便宜，仍同步。
fn dispatch(
    conn: &rusqlite::Connection,
    db_path: &PathBuf,
    data_dir: &PathBuf,
    app: &AppHandle,
    a: &ScheduledAction,
    now: i64,
) {
    if a.recurrence.is_some() {
        if let Err(e) = scheduler::reschedule(conn, a, now) {
            eprintln!("[scheduler] reschedule {} failed: {e}", a.id);
        }
    }

    match a.kind.as_str() {
        "notify" => finish(conn, &a.id, do_notify(app, &a.payload_json)),
        "pet_message" => finish(conn, &a.id, do_pet_message(app, &a.payload_json)),
        // 近端结果观察：纯数据回填，不打扰用户。
        "observe_outcome" => finish(conn, &a.id, do_observe_outcome(conn, &a.payload_json)),
        "agent_run" => spawn_agent_run(db_path, data_dir, app, a),
        other => {
            eprintln!("[scheduler] 未知 kind「{other}」跳过（执行器待实现）");
            finish(conn, &a.id, false);
        }
    }
}

/// 干预后 10/30 分钟回看：这段时间里用户在干什么，回填 `interventions.outcome`。
/// 这是 1.1 唯一的学习信号——把"感觉有没有用"变成"数据说话"。
fn do_observe_outcome(conn: &rusqlite::Connection, payload_json: &str) -> bool {
    let v: Value = serde_json::from_str(payload_json).unwrap_or(Value::Null);
    let Some(id) = v.get("intervention_id").and_then(|x| x.as_str()) else {
        return false;
    };
    let minutes = v
        .get("after_minutes")
        .and_then(|x| x.as_i64())
        .unwrap_or(10);
    match sisyphus_core::intervention::observe_outcome(conn, id, minutes) {
        Ok(Some(outcome)) => {
            eprintln!("[scheduler] 近端结果 {id} +{minutes}min → {outcome}");
            true
        }
        Ok(None) => true, // 已回填过 / 干预不存在：不是错误
        Err(e) => {
            eprintln!("[scheduler] observe_outcome failed: {e}");
            false
        }
    }
}

/// 同步动作执行后落状态。
fn finish(conn: &rusqlite::Connection, id: &str, ok: bool) {
    let _ = if ok {
        scheduler::mark_done(conn, id)
    } else {
        scheduler::mark_failed(conn, id)
    };
}

fn do_notify(app: &AppHandle, payload_json: &str) -> bool {
    let v: Value = serde_json::from_str(payload_json).unwrap_or(Value::Null);
    let title = v
        .get("title")
        .and_then(|x| x.as_str())
        .unwrap_or("西西弗斯");
    let body = v.get("body").and_then(|x| x.as_str()).unwrap_or("");
    match app.notification().builder().title(title).body(body).show() {
        Ok(_) => true,
        Err(e) => {
            eprintln!("[scheduler] notify failed: {e}");
            false
        }
    }
}

/// 宠物气泡：把结构化文案 emit 给宠物窗口（Pet.tsx 监听 `pet-message`）。
fn do_pet_message(app: &AppHandle, payload_json: &str) -> bool {
    let v: Value = serde_json::from_str(payload_json).unwrap_or(Value::Null);
    let body = v
        .get("body")
        .and_then(|x| x.as_str())
        .or_else(|| v.as_str())
        .unwrap_or("");
    if body.is_empty() {
        return false;
    }
    match app.emit("pet-message", body.to_string()) {
        Ok(_) => true,
        Err(e) => {
            eprintln!("[scheduler] pet_message emit failed: {e}");
            false
        }
    }
}

/// 拉起只读 Agent 拿一条建议——放到独立 worker 线程（自开 db 连接），跑完再落状态。
/// 动作已被 `due_actions` 原子置 `fired`；周期动作已在 dispatch 排好下一次。
fn spawn_agent_run(db_path: &PathBuf, data_dir: &PathBuf, app: &AppHandle, a: &ScheduledAction) {
    let db_path = db_path.clone();
    let data_dir = data_dir.clone();
    let app = app.clone();
    let id = a.id.clone();
    let payload = a.payload_json.clone();
    std::thread::spawn(move || {
        let conn = match db::open(db_path.to_str().unwrap_or_default()) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("[scheduler] agent worker open db failed: {e}");
                return;
            }
        };
        let ok = do_agent_run(&conn, &data_dir, &app, &payload);
        finish(&conn, &id, ok);
    });
}

/// 运行只读/同步 Agent。proactive_recommendation → 投递一条建议；
/// lifeindex_sync → Agent 在受限网关内双向合并 LifeDB 与唯一 Notion 页面。
fn do_agent_run(
    conn: &rusqlite::Connection,
    data_dir: &PathBuf,
    app: &AppHandle,
    payload_json: &str,
) -> bool {
    let v: Value = serde_json::from_str(payload_json).unwrap_or(Value::Null);
    let topic = v.get("topic").and_then(|x| x.as_str()).unwrap_or("");
    let mode_str = v.get("mode").and_then(|x| x.as_str()).unwrap_or("");
    let local = sisyphus_core::context::today_context(conn, "local-user")
        .ok()
        .and_then(|ctx| serde_json::to_string(&ctx).ok())
        .unwrap_or_else(|| "{}".to_string());

    if matches!(mode_str, "lifeindex_sync" | "lifeindex_refresh") {
        let Some(target_id) = agent_runtime::notion_sync_target(data_dir) else {
            // 未启用同步是合法状态：每日 job 保留，等用户配置后自然生效。
            eprintln!("[scheduler] LifeIndex 同步未配置，本轮跳过");
            return true;
        };
        let started_at = chrono::Utc::now().timestamp_millis();
        let prompt = format!(
            "定时 LifeIndex 双向同步。{topic}\n目标 page id：{target_id}\n本地今日快照：{local}\n\
             严格执行三方合并；保留 Notion 中全部用户事项；写回成功后必须 complete_lifeindex_sync。"
        );
        return match agent_runtime::run_agent(
            data_dir,
            &prompt,
            None,
            None,
            agent_runtime::RunMode::LifeIndexSync,
        ) {
            Ok(out) => {
                let completed = sisyphus_core::lifedb::get_sync_state(conn, "notion", &target_id)
                    .ok()
                    .flatten()
                    .and_then(|state| state.last_success_at_ms)
                    .is_some_and(|at| at >= started_at);
                if completed {
                    let _ = app.emit("lifeindex-updated", ());
                    eprintln!("[scheduler] {} LifeIndex 双向同步完成", out.runtime);
                    true
                } else {
                    let error = "Agent 已结束，但没有完成 Notion 写回确认";
                    let _ = sisyphus_core::lifedb::fail_sync(conn, &target_id, error);
                    eprintln!("[scheduler] lifeindex_sync incomplete: {error}");
                    false
                }
            }
            Err(e) => {
                let _ = sisyphus_core::lifedb::fail_sync(conn, &target_id, &e);
                eprintln!("[scheduler] lifeindex_sync failed: {e}");
                false
            }
        };
    }

    if mode_str == "weekly_review" {
        use chrono::Datelike;
        let weekday = chrono::Local::now().weekday();
        if weekday != chrono::Weekday::Sun {
            return true; // 只在周日回顾，其余天静默
        }
        let queue = sisyphus_core::lifetree::review_queue(conn, 7)
            .ok()
            .and_then(|q| serde_json::to_string(&q).ok())
            .unwrap_or_else(|| "{}".to_string());
        let prompt = format!(
            "周回顾。{topic}\n本地今日快照：{local}\n回顾队列（Core 已算好，别自己数）：{queue}\n\
             用关心不评判的语气总结这一周，并只挑最重要的 1–2 条提问（升级 / someday / 归档 三选一）。\n\
             不得调用任何写工具；若这周没什么值得问的，只返回 no_recommendation。"
        );
        return match agent_runtime::run_agent(
            data_dir,
            &prompt,
            None,
            None,
            agent_runtime::RunMode::Proactive,
        ) {
            Ok(out) => {
                let text = out.text.trim();
                if text.eq_ignore_ascii_case("no_recommendation") || text.is_empty() {
                    return true;
                }
                let _ = app.emit("agent-recommendation", text.to_string());
                let body: String = text.chars().take(240).collect();
                app.notification()
                    .builder()
                    .title("西西弗斯 · 周回顾")
                    .body(body)
                    .show()
                    .is_ok()
            }
            Err(e) => {
                eprintln!("[scheduler] weekly_review failed: {e}");
                false
            }
        };
    }

    let prompt = format!(
        "这是定时主动任务。主题：{topic}\n本地状态快照：{local}\n\
         如已配置 Notion 只读源，请先读取用户最近更新；不得调用任何写工具。\n\
         只返回一条适合现在的简短建议；若不应打扰，只返回 no_recommendation。"
    );

    match agent_runtime::run_agent(
        data_dir,
        &prompt,
        None,
        None,
        agent_runtime::RunMode::Proactive,
    ) {
        Ok(out) => {
            let text = out.text.trim();
            if text.eq_ignore_ascii_case("no_recommendation") || text.is_empty() {
                return true;
            }
            let _ = app.emit("agent-recommendation", text.to_string());
            let body: String = text.chars().take(240).collect();
            match app
                .notification()
                .builder()
                .title("西西弗斯 · 此刻")
                .body(body)
                .show()
            {
                Ok(_) => {
                    eprintln!("[scheduler] {} 主动建议已投递", out.runtime);
                    true
                }
                Err(e) => {
                    eprintln!("[scheduler] agent recommendation notify failed: {e}");
                    false
                }
            }
        }
        Err(e) => {
            eprintln!("[scheduler] agent_run failed: {e}");
            false
        }
    }
}
