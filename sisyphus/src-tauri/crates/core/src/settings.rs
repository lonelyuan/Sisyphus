//! 本地配置 KV（`settings` 表）。
//!
//! 只放**影响 Core 行为**的少量开关（换日点、rollup 保留窗口等）。
//! 凭据、模型配置仍在 app 侧的 json 文件里（见 `app_config`），不进 SQLite。

use rusqlite::{params, Connection};

pub fn get(conn: &Connection, key: &str) -> Option<String> {
    conn.query_row(
        "SELECT value FROM settings WHERE key = ?1",
        params![key],
        |r| r.get::<_, String>(0),
    )
    .ok()
}

pub fn set(conn: &Connection, key: &str, value: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO settings (key, value, updated_at) VALUES (?1, ?2, ?3)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
        params![key, value, crate::clock::now_ms()],
    )?;
    Ok(())
}

pub fn get_i64(conn: &Connection, key: &str, default: i64) -> i64 {
    get(conn, key)
        .and_then(|v| v.trim().parse::<i64>().ok())
        .unwrap_or(default)
}
