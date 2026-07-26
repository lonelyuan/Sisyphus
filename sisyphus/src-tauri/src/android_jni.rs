//! Android JNI 桥：让 Kotlin 前台服务 / 通知按钮接收器**绕过 WebView** 直调 Rust core。
//!
//! 为什么需要：Android 上 App 退到后台时 WebView 被挂起，`invoke()` 路径失效——
//! 恰好是用户刷短视频、最该抓现行的时刻。前台服务经这里的 JNI 直接评估+记录，
//! 命中就由 Kotlin 直接弹通知，全程不依赖 WebView。逻辑仍全在 `sisyphus-core`（单一来源）。
//!
//! 对应 Kotlin：`object com.sisyphus.collector.NativeBridge`（见 NativeBridge.kt）。
//! JNI 命名 `Java_<pkg>_<Class>_<method>`；Kotlin object 的方法为实例方法 → 第二参数是 `JObject`。

use std::time::Duration;

use jni::objects::{JObject, JString};
use jni::sys::{jlong, jstring};
use jni::JNIEnv;
use rusqlite::Connection;

use sisyphus_core::intervention::evaluate_and_record;
use sisyphus_core::rule_engine::config::RuleConfig;
use sisyphus_core::rule_engine::{RuleContext, RuleEngine};
use sisyphus_core::{context, db, ingest};

const USER_ID: &str = "local-user";
const DEVICE_ID: &str = "android-usage";

fn open_conn(path: &str) -> Option<Connection> {
    let conn = db::open(path).ok()?;
    let _ = conn.busy_timeout(Duration::from_secs(5));
    Some(conn)
}

fn jstr(env: &mut JNIEnv, s: &JString) -> Option<String> {
    env.get_string(s).ok().map(|v| v.to_string_lossy().into_owned())
}

fn opt(s: String) -> Option<String> {
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

/// 评估当前前台 app 是否命中娱乐规则；命中则写 intervention 并返回 JSON
/// `{"message":..,"interventionId":..}`，否则返回空串。绝不 panic（越 JNI 边界 unwind 是 UB）。
#[no_mangle]
pub extern "system" fn Java_com_sisyphus_collector_NativeBridge_evaluate<'local>(
    mut env: JNIEnv<'local>,
    _this: JObject<'local>,
    db_path: JString<'local>,
    pkg: JString<'local>,
    category: JString<'local>,
    active_ms: jlong,
) -> jstring {
    let json = (|| -> Option<String> {
        let db_path = jstr(&mut env, &db_path)?;
        let pkg = jstr(&mut env, &pkg)?;
        let category = jstr(&mut env, &category)?;
        let conn = open_conn(&db_path)?;

        let now = chrono::Utc::now().timestamp_millis();
        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
        let goal = db::get_today_goal(&conn, &today).ok()?;
        // 权威分类：用户表 + 内置白名单（App 里加的 app 立即生效）；Kotlin 传的作后备。
        let category = sisyphus_core::category::categorize(&conn, &pkg)
            .ok()
            .flatten()
            .or_else(|| opt(category));

        let ctx = RuleContext {
            now_ms: now,
            user_id: USER_ID.to_string(),
            device_id: DEVICE_ID.to_string(),
            current_app: opt(pkg),
            current_category: category,
            active_entertainment_ms: active_ms,
            active_session_ms: active_ms,
            media_playing_since_ms: 0,
            recent_scroll_count: 0,
            today_goal: goal,
        };
        let engine = RuleEngine::new(RuleConfig::default());
        let out = evaluate_and_record(&conn, &engine, &ctx).ok()??;
        Some(
            serde_json::json!({
                "message": out.message,
                "interventionId": out.intervention_id,
            })
            .to_string(),
        )
    })()
    .unwrap_or_default();

    env.new_string(json)
        .map(|j| j.into_raw())
        .unwrap_or(std::ptr::null_mut())
}

/// 记录用户对干预通知的响应（按钮点击），后台也可写。
#[no_mangle]
pub extern "system" fn Java_com_sisyphus_collector_NativeBridge_recordFeedback<'local>(
    mut env: JNIEnv<'local>,
    _this: JObject<'local>,
    db_path: JString<'local>,
    intervention_id: JString<'local>,
    action: JString<'local>,
) {
    let _ = (|| -> Option<()> {
        let db_path = jstr(&mut env, &db_path)?;
        let id = jstr(&mut env, &intervention_id)?;
        let action = jstr(&mut env, &action)?;
        let conn = open_conn(&db_path)?;
        context::record_feedback(&conn, &id, &action).ok()?;
        Some(())
    })();
}

/// 写一段已结束的前台会话到 Event log（app 切换时调用），供时长统计/query_context。
#[no_mangle]
pub extern "system" fn Java_com_sisyphus_collector_NativeBridge_ingestForeground<'local>(
    mut env: JNIEnv<'local>,
    _this: JObject<'local>,
    db_path: JString<'local>,
    pkg: JString<'local>,
    category: JString<'local>,
    start_ms: jlong,
    end_ms: jlong,
) {
    let _ = (|| -> Option<()> {
        let db_path = jstr(&mut env, &db_path)?;
        let pkg = jstr(&mut env, &pkg)?;
        let category = jstr(&mut env, &category)?;
        let conn = open_conn(&db_path)?;
        let category = sisyphus_core::category::categorize(&conn, &pkg)
            .ok()
            .flatten()
            .or_else(|| opt(category));
        let ev = ingest::NewEvent {
            event_id: None,
            source: "android_usage".to_string(),
            layer: "raw".to_string(),
            event_type: "app_foreground".to_string(),
            time_mode: "interval".to_string(),
            event_time: None,
            start_time: Some(start_ms),
            end_time: Some(end_ms),
            entity: opt(pkg),
            category,
            payload: serde_json::json!({}),
            parent_event_ids: vec![],
            privacy_level: "L0".to_string(),
        };
        ingest::ingest_event(&conn, USER_ID, DEVICE_ID, ev).ok()?;
        Some(())
    })();
}

/// 诊断：返回 DB 路径/是否存在/今日目标/前台事件数/干预数 的 JSON，供 logcat 排查
/// 「手机 0 记录」。绝不 panic。
#[no_mangle]
pub extern "system" fn Java_com_sisyphus_collector_NativeBridge_debugState<'local>(
    mut env: JNIEnv<'local>,
    _this: JObject<'local>,
    db_path: JString<'local>,
) -> jstring {
    let json = (|| -> Option<String> {
        let db_path = jstr(&mut env, &db_path)?;
        let exists = std::path::Path::new(&db_path).exists();
        let conn = open_conn(&db_path)?;
        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
        let goal = db::get_today_goal(&conn, &today).ok().flatten();
        let fg: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM raw_events WHERE type='app_foreground'",
                [],
                |r| r.get(0),
            )
            .unwrap_or(-1);
        let interventions: i64 = conn
            .query_row("SELECT COUNT(*) FROM interventions", [], |r| r.get(0))
            .unwrap_or(-1);
        Some(
            serde_json::json!({
                "dbPath": db_path,
                "dbExists": exists,
                "hasGoal": goal.is_some(),
                "goal": goal.map(|g| g.raw_text),
                "foregroundEvents": fg,
                "interventions": interventions,
            })
            .to_string(),
        )
    })()
    .unwrap_or_else(|| "{\"error\":\"debugState failed\"}".to_string());

    env.new_string(json)
        .map(|j| j.into_raw())
        .unwrap_or(std::ptr::null_mut())
}

/// 取出并标记到期提醒，返回 JSON 数组 `[{"id","text"}]` 供 Kotlin 弹通知。绝不 panic。
#[no_mangle]
pub extern "system" fn Java_com_sisyphus_collector_NativeBridge_fireDueReminders<'local>(
    mut env: JNIEnv<'local>,
    _this: JObject<'local>,
    db_path: JString<'local>,
) -> jstring {
    let json = (|| -> Option<String> {
        let db_path = jstr(&mut env, &db_path)?;
        let conn = open_conn(&db_path)?;
        let now = chrono::Utc::now().timestamp_millis();
        let due = sisyphus_core::artifacts::take_due_reminders(&conn, now).ok()?;
        let arr: Vec<serde_json::Value> = due
            .iter()
            .map(|r| serde_json::json!({ "id": r.id, "text": r.text }))
            .collect();
        serde_json::to_string(&arr).ok()
    })()
    .unwrap_or_else(|| "[]".to_string());

    env.new_string(json)
        .map(|j| j.into_raw())
        .unwrap_or(std::ptr::null_mut())
}
