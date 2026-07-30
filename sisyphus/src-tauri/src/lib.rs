mod agent_runtime;
#[cfg(target_os = "android")]
mod android_jni;
#[cfg(target_os = "macos")]
mod collector;
mod commands;
#[cfg(desktop)]
mod scheduler_runner;

use commands::AppState;
use sisyphus_core::db;
use sisyphus_core::rule_engine::config::RuleConfig;
use sisyphus_core::rule_engine::RuleEngine;
use std::sync::Mutex;
#[cfg(target_os = "android")]
use tauri::plugin::{PluginHandle, TauriPlugin};
use tauri::{Manager, Runtime};
use uuid::Uuid;

#[tauri::command]
fn ping() -> String {
    "pong".to_string()
}

// ── Android usage 插件桥 ─────────────────────────────────────────────────────
// 直接从 JS invoke `plugin:usage|...` 会被 ACL 拒（自建移动插件无权限清单 → "not allowed /
// Plugin not found"）。改用「App 命令包一层，Rust 侧经 run_mobile_plugin 调 Kotlin」——
// App 自有命令不需要 ACL 授权（与 set_goal 等一致），是 Tauri v2 移动插件的惯用法。

#[cfg(target_os = "android")]
struct UsageBridge<R: Runtime>(PluginHandle<R>);

#[cfg(target_os = "android")]
#[derive(serde::Deserialize)]
struct PermResp {
    granted: bool,
}

/// 使用情况访问权限是否已授予。
#[tauri::command]
fn check_usage_permission<R: Runtime>(app: tauri::AppHandle<R>) -> Result<bool, String> {
    #[cfg(target_os = "android")]
    {
        let bridge = app.state::<UsageBridge<R>>();
        let r: PermResp = bridge
            .0
            .run_mobile_plugin("checkPermission", ())
            .map_err(|e| e.to_string())?;
        Ok(r.granted)
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = app;
        Ok(false)
    }
}

/// 跳转系统「使用情况访问」设置页（特殊权限，不在普通权限列表里）。
#[tauri::command]
fn request_usage_permission<R: Runtime>(app: tauri::AppHandle<R>) -> Result<(), String> {
    #[cfg(target_os = "android")]
    {
        let bridge = app.state::<UsageBridge<R>>();
        bridge
            .0
            .run_mobile_plugin::<serde_json::Value>("requestPermission", ())
            .map_err(|e| e.to_string())?;
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = app;
    }
    Ok(())
}

/// 启动前台采集服务。
#[tauri::command]
fn start_collector<R: Runtime>(app: tauri::AppHandle<R>) -> Result<(), String> {
    #[cfg(target_os = "android")]
    {
        let bridge = app.state::<UsageBridge<R>>();
        bridge
            .0
            .run_mobile_plugin::<serde_json::Value>("startCollector", ())
            .map_err(|e| e.to_string())?;
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = app;
    }
    Ok(())
}

/// 停止前台采集服务。
#[tauri::command]
fn stop_collector<R: Runtime>(app: tauri::AppHandle<R>) -> Result<(), String> {
    #[cfg(target_os = "android")]
    {
        let bridge = app.state::<UsageBridge<R>>();
        bridge
            .0
            .run_mobile_plugin::<serde_json::Value>("stopCollector", ())
            .map_err(|e| e.to_string())?;
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = app;
    }
    Ok(())
}

/// 桥接 Kotlin UsagePlugin（Android）。存 PluginHandle 供上面的 App 命令经 run_mobile_plugin 调用。
#[cfg(target_os = "android")]
fn usage_plugin<R: Runtime>() -> TauriPlugin<R> {
    tauri::plugin::Builder::<R>::new("usage")
        .setup(|app, api| {
            let handle = api.register_android_plugin("com.sisyphus", "UsagePlugin")?;
            app.manage(UsageBridge(handle));
            Ok(())
        })
        .build()
}

/// 桥接 Kotlin NotificationPlugin（Android）。
/// 注意：桌面端不注册它——名字与 `tauri_plugin_notification`（同名 "notification"）冲突，
/// 且桌面通知走 collector 里的 Rust `app.notification()`，不需要这个 Kotlin 桥。
#[cfg(target_os = "android")]
fn notification_plugin<R: Runtime>() -> TauriPlugin<R> {
    tauri::plugin::Builder::<R>::new("notification")
        .setup(|_app, api| {
            api.register_android_plugin("com.sisyphus", "NotificationPlugin")?;
            Ok(())
        })
        .build()
}

/// 显示并聚焦主窗口（dock 点击 / tray 打开时唤回）。
#[cfg(desktop)]
fn show_main(app: &tauri::AppHandle) {
    // 恢复 Dock 图标：挂后台时切到 Accessory 会把它从程序坞移除，唤回时切回 Regular。
    #[cfg(target_os = "macos")]
    let _ = app.set_activation_policy(tauri::ActivationPolicy::Regular);
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

/// 显示/隐藏桌面宠物窗口（tray 菜单「宠物」切换）。
#[cfg(desktop)]
fn toggle_pet(app: &tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("pet") {
        if w.is_visible().unwrap_or(false) {
            let _ = w.hide();
        } else {
            let _ = w.show();
            let _ = w.set_focus();
        }
    }
}

/// 在桌面端安装菜单栏（tray）图标：左键唤回窗口，右键菜单「打开 / 退出」。
#[cfg(desktop)]
fn setup_tray(app: &tauri::App) -> tauri::Result<()> {
    use tauri::menu::{Menu, MenuItem};
    use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

    let open_i = MenuItem::with_id(app, "open", "打开西西弗斯", true, None::<&str>)?;
    let pet_i = MenuItem::with_id(app, "pet", "宠物 显示/隐藏", true, None::<&str>)?;
    let quit_i = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open_i, &pet_i, &quit_i])?;

    let mut builder = TrayIconBuilder::with_id("main")
        .tooltip("西西弗斯 · 感知平面常驻中")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => show_main(app),
            "pet" => toggle_pet(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main(tray.app_handle());
            }
        });

    // 菜单栏 / 系统托盘小图标：专用剪影（透明底）。icon_as_template=true → macOS 用其 alpha 形状
    // 做遮罩，随亮/暗菜单栏自动反色（浅色栏显黑、深色栏显白），不再受"恒白在浅色栏看不清"之苦。
    // Windows/Linux 无 template 概念，此标记被忽略，按白色剪影原样显示（深色任务栏正好显白）。
    builder = builder
        .icon(tauri::include_image!("icons/tray/tray-64.png"))
        .icon_as_template(true);
    builder.build(app)?;
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let mut builder = tauri::Builder::default()
        .plugin(
            tauri_plugin_log::Builder::new()
                .level(log::LevelFilter::Info)
                .targets([
                    tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::Stdout),
                    tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::LogDir {
                        file_name: None,
                    }),
                ])
                .build(),
        )
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_os::init())
        .plugin(tauri_plugin_notification::init());

    // 开机自启（仅桌面）：LaunchAgent 方式，默认不自动启用，由 Settings 开关控制。
    #[cfg(desktop)]
    {
        builder = builder.plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ));
    }

    // Android 原生采集/通知桥（桌面不注册，避免与 tauri-plugin-notification 重名）。
    #[cfg(target_os = "android")]
    {
        builder = builder.plugin(usage_plugin()).plugin(notification_plugin());
    }

    let app = builder
        .setup(|app| {
            let data_dir = app
                .path()
                .app_data_dir()
                .expect("failed to get app data dir");
            std::fs::create_dir_all(&data_dir)?;

            // 修编译期路径依赖：用运行期 resource_dir 注入 skills / pi runner / mcp 二进制路径，
            // release 构建不再指向编译期 CARGO_MANIFEST_DIR（打包 / 移动仓库后即失效）。
            #[cfg(desktop)]
            agent_runtime::init_paths(app.path().resource_dir().ok().as_deref(), None);

            let db_path = data_dir.join("sisyphus.db");
            let conn = db::open(db_path.to_str().unwrap()).expect("failed to open database");

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
            // 第二大脑知识库 vault：默认 data_dir/vault，可用 SISYPHUS_VAULT 覆盖。
            let vault_dir = match std::env::var("SISYPHUS_VAULT") {
                Ok(p) if !p.trim().is_empty() => std::path::PathBuf::from(p),
                _ => data_dir.join("vault"),
            };
            let _ = std::fs::create_dir_all(&vault_dir);
            // vault 交给 git：整卡覆盖的风险从"不可逆"降为"可 diff 可回滚"，
            // 并顺带得到版本历史与 blame（维基百科质量的支柱之一）。
            sisyphus_core::vault::git_init_if_needed(&vault_dir);
            // 索引是可重建的投影，vault 的 .md 才是本体：启动时追平一次
            // （回填老库缺的正文/领域/链接边，也吸收用户在 Obsidian 里的手改）。
            match sisyphus_core::knowledge::reindex_vault(&conn, &vault_dir) {
                Ok(r) => eprintln!(
                    "[startup] 知识库索引追平：扫 {} 张，新增 {}，更新 {}，链接 {}",
                    r.scanned, r.inserted, r.updated, r.links
                ),
                Err(e) => eprintln!("[startup] 知识库索引追平失败: {e}"),
            }

            // 监控名单：新装写 json 模板 + 把旧 json 覆盖项迁入 monitored_apps 表（一次性、幂等）。
            sisyphus_core::category::ensure_starter_categories(&data_dir);
            let _ = sisyphus_core::category::import_overrides_to_table(&conn, &data_dir);

            // 为主动触发 ticker 预留副本（vault_dir/db_path 稍后被 state/collector 移走）。
            #[cfg(desktop)]
            let sched_db = db_path.clone();
            #[cfg(desktop)]
            let sched_vault = vault_dir.clone();
            #[cfg(desktop)]
            let sched_data = data_dir.clone();

            let state = AppState {
                conn: Mutex::new(conn),
                rule_engine: RuleEngine::new(RuleConfig::default()),
                user_id: "local-user".to_string(),
                device_id,
                vault_dir,
                data_dir: data_dir.clone(),
            };
            app.manage(state);

            // 反思平面主动触发：调度器 ticker 后台线程（proactive-triggers.md）。
            #[cfg(desktop)]
            {
                let handle = app.handle().clone();
                std::thread::spawn(move || {
                    scheduler_runner::run(sched_db, sched_vault, sched_data, handle)
                });
            }

            // 感知平面：桌面前台采集器后台线程（独立连接，与 App 同库）。
            #[cfg(target_os = "macos")]
            {
                let handle = app.handle().clone();
                std::thread::spawn(move || collector::run(db_path, handle));
            }

            // 菜单栏常驻图标（桌面）。
            #[cfg(desktop)]
            setup_tray(app)?;

            Ok(())
        })
        // 关窗即隐藏（仅 macOS）：红叉不退出，进程与采集器线程存活；tray 可唤回。
        // 只在 macOS 启用——那里 dock 图标 + RunEvent::Reopen 保证一定能把窗口唤回；
        // 其它桌面若无 tray 宿主（如缺 AppIndicator 的 Linux）隐藏窗口会变成无法唤回/退出的僵尸。
        .on_window_event(|_window, _event| {
            #[cfg(target_os = "macos")]
            {
                if let tauri::WindowEvent::CloseRequested { api, .. } = _event {
                    let _ = _window.hide();
                    // 挂后台时从程序坞移除（Accessory）：只留菜单栏图标，点 tray 图标唤回。
                    let _ = _window
                        .app_handle()
                        .set_activation_policy(tauri::ActivationPolicy::Accessory);
                    api.prevent_close();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            ping,
            commands::ingest_event,
            commands::evaluate_rules,
            commands::set_goal,
            commands::update_goal_status,
            commands::record_feedback,
            commands::get_today_context,
            commands::get_vault_path,
            commands::get_agent_runtime_status,
            commands::set_agent_runtime,
            commands::run_agent,
            commands::cancel_agent_run,
            commands::get_llm_config,
            commands::set_llm_config,
            commands::get_notion_config,
            commands::set_notion_config,
            commands::clear_notion_config,
            commands::run_knowledge_agent,
            commands::list_tasks,
            commands::create_task,
            commands::set_task_status,
            commands::delete_task,
            commands::list_reminders,
            commands::set_reminder_status,
            commands::list_interventions,
            commands::query_timeline,
            commands::list_knowledge,
            commands::list_scheduled_actions,
            commands::list_monitored_apps,
            commands::add_monitored_app,
            commands::remove_monitored_app,
            commands::list_detection_rules,
            commands::set_detection_rule_enabled,
            commands::delete_detection_rule,
            commands::list_lifeindex,
            commands::list_life_items,
            commands::upsert_life_item,
            commands::archive_life_item,
            commands::link_life_items,
            commands::list_life_item_edges,
            commands::get_lifeindex_sync_overview,
            commands::run_lifeindex_sync,
            commands::list_lifeindex_runs,
            commands::kb_doctor,
            commands::kb_wanted,
            commands::kb_reindex,
            commands::refresh_mocs,
            commands::life_tree,
            commands::skill_map,
            commands::skill_tree_growth,
            commands::next_actions,
            commands::review_queue,
            commands::list_life_areas,
            commands::upsert_life_area,
            commands::intervention_outcomes,
            commands::rebuild_rollups,
            commands::set_day_boundary_hour,
            commands::get_day_boundary_hour,
            check_usage_permission,
            request_usage_permission,
            start_collector,
            stop_collector,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    app.run(|_app_handle, _event| {
        // 常驻保活仅 macOS（见 on_window_event 注释）：关窗/隐式退出时保活，dock 唤回。
        #[cfg(target_os = "macos")]
        {
            match _event {
                // 关窗/隐式退出时保活；显式 app.exit(code)（tray「退出」）带 Some(code)，放行。
                tauri::RunEvent::ExitRequested { code, api, .. } => {
                    if code.is_none() {
                        api.prevent_exit();
                    }
                }
                // 点击 dock 图标唤回窗口。
                tauri::RunEvent::Reopen { .. } => show_main(_app_handle),
                _ => {}
            }
        }
    });
}
