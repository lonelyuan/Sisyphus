//! 应用分类（bundle id / 包名 → category）。MVP 用硬编码白名单 + 用户可配置覆盖，与规则逻辑分离。
//! Android 娱乐包名见 [`crate::rule_engine::entertainment::entertainment_packages`]。

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::collections::HashMap;
use std::path::Path;

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// 桌面（macOS）内置娱乐白名单（可枚举，供「监控名单」展示）。
pub const DESKTOP_ENTERTAINMENT: &[(&str, &str)] = &[
    ("com.apple.TV", "entertainment.video"),
    ("com.netflix.Netflix", "entertainment.video"),
    ("com.colliderli.iina", "entertainment.video"), // IINA 播放器
    ("tv.plex.plex-desktop", "entertainment.video"),
    ("com.valvesoftware.steam", "entertainment.game"),
];

/// 桌面（macOS）娱乐应用 bundle id → 分类（内置默认白名单）。
///
/// **注意**：桌面上真正的刷视频/信息流多发生在浏览器内，准确信号需要浏览器插件（延后的采集源）。
/// 这里只覆盖原生娱乐应用。刻意不含浏览器/音乐，避免误报。
/// 想纳入自己的"时间黑洞"app，用 `desktop_categories.json` 覆盖，无需改代码。
pub fn categorize_desktop(bundle_id: &str) -> Option<&'static str> {
    DESKTOP_ENTERTAINMENT
        .iter()
        .find(|(id, _)| *id == bundle_id)
        .map(|(_, c)| *c)
}

/// 一条被监控的娱乐 app（供「监控名单」UI 展示）。
#[derive(Debug, Serialize)]
pub struct MonitoredApp {
    pub id: String,
    pub category: String,
    pub platform: String, // desktop | android | custom（用户自定义，跨端生效）
    pub source: String,   // builtin | user
}

/// 统一分类判定：用户表(monitored_apps) > 内置桌面白名单 > 内置 Android 白名单。
/// 桌面采集器与 Android JNI 都调它——加/删监控 app 跨端立即生效。
pub fn categorize(conn: &Connection, id: &str) -> rusqlite::Result<Option<String>> {
    let user: Option<String> = conn
        .query_row(
            "SELECT category FROM monitored_apps WHERE bundle_id = ?1",
            params![id],
            |r| r.get(0),
        )
        .optional()?;
    if user.is_some() {
        return Ok(user);
    }
    if let Some(c) = categorize_desktop(id) {
        return Ok(Some(c.to_string()));
    }
    Ok(crate::rule_engine::entertainment::entertainment_packages()
        .get(id)
        .map(|c| c.to_string()))
}

/// 加入 / 更新一个用户监控 app（增改）。
pub fn add_monitored_app(conn: &Connection, id: &str, category: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO monitored_apps (bundle_id, category, created_at) VALUES (?1, ?2, ?3)
         ON CONFLICT(bundle_id) DO UPDATE SET category = excluded.category",
        params![id, category, now_ms()],
    )?;
    Ok(())
}

/// 移除一个用户监控 app（删）。内置项无法删除，只能被同 id 的用户项覆盖。
pub fn remove_monitored_app(conn: &Connection, id: &str) -> rusqlite::Result<()> {
    conn.execute("DELETE FROM monitored_apps WHERE bundle_id = ?1", params![id])?;
    Ok(())
}

/// 查：用户自定义 + 内置桌面 + 内置 Android（用户项覆盖同 id 内置项）。
pub fn list_monitored(conn: &Connection) -> rusqlite::Result<Vec<MonitoredApp>> {
    let mut out = Vec::new();
    let mut stmt =
        conn.prepare("SELECT bundle_id, category FROM monitored_apps ORDER BY created_at DESC")?;
    let users: Vec<(String, String)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let user_ids: std::collections::HashSet<&str> = users.iter().map(|(id, _)| id.as_str()).collect();
    for (id, cat) in &users {
        out.push(MonitoredApp {
            id: id.clone(),
            category: cat.clone(),
            platform: "custom".into(),
            source: "user".into(),
        });
    }
    for (id, cat) in DESKTOP_ENTERTAINMENT {
        if !user_ids.contains(id) {
            out.push(MonitoredApp {
                id: id.to_string(),
                category: cat.to_string(),
                platform: "desktop".into(),
                source: "builtin".into(),
            });
        }
    }
    let mut android: Vec<_> = crate::rule_engine::entertainment::entertainment_packages()
        .into_iter()
        .collect();
    android.sort();
    for (id, cat) in android {
        if !user_ids.contains(id) {
            out.push(MonitoredApp {
                id: id.to_string(),
                category: cat.to_string(),
                platform: "android".into(),
                source: "builtin".into(),
            });
        }
    }
    Ok(out)
}

/// 一次性迁移：把旧的 `desktop_categories.json` 覆盖项导入 monitored_apps 表（INSERT OR IGNORE），
/// 让用户此前手动加的（如 HoYowave）平滑进入表。启动时调用一次。
pub fn import_overrides_to_table(conn: &Connection, dir: &Path) -> rusqlite::Result<()> {
    for (id, cat) in load_desktop_overrides(dir) {
        conn.execute(
            "INSERT OR IGNORE INTO monitored_apps (bundle_id, category, created_at) VALUES (?1, ?2, ?3)",
            params![id, cat, now_ms()],
        )?;
    }
    Ok(())
}

/// 从 `{data_dir}/desktop_categories.json` 读用户自定义分类覆盖（bundle_id → category）。
/// 文件缺失/非法 → 空表。让用户无需改代码/重编译即可把自己的娱乐 app 纳入规则。
pub fn load_desktop_overrides(dir: &Path) -> HashMap<String, String> {
    match std::fs::read_to_string(dir.join("desktop_categories.json")) {
        Ok(s) => serde_json::from_str::<HashMap<String, String>>(&s)
            .map(|m| {
                // 过滤掉以 `_` 开头的说明性键（如 _comment），它们不会匹配任何 bundle id。
                m.into_iter().filter(|(k, _)| !k.starts_with('_')).collect()
            })
            .unwrap_or_default(),
        Err(_) => HashMap::new(),
    }
}

/// 首次运行时写一个带说明 + 示例的 `desktop_categories.json` 模板（已存在则不动）。
/// 让用户一眼看到格式，并把常见娱乐 app 直接生效。
pub fn ensure_starter_categories(dir: &Path) {
    let p = dir.join("desktop_categories.json");
    if p.exists() {
        return;
    }
    let starter = r#"{
  "_comment": "把你的娱乐/摸鱼 app 的 bundle id 映射到 entertainment.video|game|social|news。改完重启 App 生效。查看当前前台 app 的 bundle id：终端跑 osascript -e 'tell application \"System Events\" to get bundle identifier of first application process whose frontmost is true'",
  "com.miHoYo.HoYowave": "entertainment.game"
}
"#;
    let _ = std::fs::write(&p, starter);
}

