mod commands;
#[cfg(target_os = "macos")]
mod collector;

use commands::AppState;
use sisyphus_core::db;
use sisyphus_core::rule_engine::RuleEngine;
use sisyphus_core::rule_engine::config::RuleConfig;
use std::sync::Mutex;
use tauri::{Manager, Runtime};
use tauri::plugin::TauriPlugin;
use uuid::Uuid;

#[tauri::command]
fn ping() -> String {
    "pong".to_string()
}

/// 桥接 Kotlin UsagePlugin（Android）/ 桌面端空实现
fn usage_plugin<R: Runtime>() -> TauriPlugin<R> {
    tauri::plugin::Builder::<R>::new("usage")
        .setup(|_app, api| {
            #[cfg(target_os = "android")]
            api.register_android_plugin("com.sisyphus", "UsagePlugin")?;
            Ok(())
        })
        .build()
}

/// 桥接 Kotlin NotificationPlugin（Android）/ 桌面端空实现
fn notification_plugin<R: Runtime>() -> TauriPlugin<R> {
    tauri::plugin::Builder::<R>::new("notification")
        .setup(|_app, api| {
            #[cfg(target_os = "android")]
            api.register_android_plugin("com.sisyphus", "NotificationPlugin")?;
            Ok(())
        })
        .build()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_os::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(usage_plugin())
        .plugin(notification_plugin())
        .setup(|app| {
            let data_dir = app
                .path()
                .app_data_dir()
                .expect("failed to get app data dir");
            std::fs::create_dir_all(&data_dir)?;
            let db_path = data_dir.join("sisyphus.db");
            let conn = db::open(db_path.to_str().unwrap())
                .expect("failed to open database");

            // 持久化 device_id（借鉴旧原生工程 AppPreferences）：生成一次存盘复用，
            // 避免每次启动都换 id 导致 seq_no 断裂 / 跨端聚合错乱。
            let device_id = {
                let p = data_dir.join("device_id");
                match std::fs::read_to_string(&p) {
                    Ok(s) if !s.trim().is_empty() => s.trim().to_string(),
                    _ => {
                        let new = format!("device-{}", &Uuid::new_v4().to_string()[..8]);
                        let _ = std::fs::write(&p, &new);
                        new
                    }
                }
            };
            let state = AppState {
                conn: Mutex::new(conn),
                rule_engine: RuleEngine::new(RuleConfig::default()),
                user_id: "local-user".to_string(),
                device_id,
            };
            app.manage(state);

            // 感知平面：桌面前台采集器后台线程（独立连接，与 App 同库）。
            #[cfg(target_os = "macos")]
            {
                let handle = app.handle().clone();
                std::thread::spawn(move || collector::run(db_path, handle));
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            ping,
            commands::ingest_event,
            commands::evaluate_rules,
            commands::set_goal,
            commands::update_goal_status,
            commands::record_feedback,
            commands::get_today_context,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
