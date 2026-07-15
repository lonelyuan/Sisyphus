//! 反思平面读模型：今日上下文、今日行动、目标与反馈写入。
//! App 命令与 MCP 工具都调用这里，保证逻辑单一来源。

use rusqlite::{params, Connection};
use serde::Serialize;
use uuid::Uuid;
use crate::db;
use crate::rule_engine::DailyGoal;

fn today_str() -> String {
    chrono::Utc::now().format("%Y-%m-%d").to_string()
}

fn today_start_ms() -> i64 {
    let now = chrono::Utc::now();
    now.date_naive()
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_utc()
        .timestamp_millis()
}

#[derive(Debug, Serialize)]
pub struct RecentIntervention {
    pub shown_at: i64,
    pub response: Option<String>,
}

/// 今日上下文（只含 L0–L1 数据），供 Agent `query_context` 构建最小必要上下文。
#[derive(Debug, Serialize)]
pub struct TodayContext {
    pub date: String,
    pub goal: Option<DailyGoal>,
    pub entertainment_minutes: f64,
    pub intervention_count: i64,
    pub recent_interventions: Vec<RecentIntervention>,
}

pub fn today_context(conn: &Connection, user_id: &str) -> rusqlite::Result<TodayContext> {
    let today = today_str();
    let date_start = today_start_ms();

    let goal = db::get_today_goal(conn, &today)?;
    let entertainment_ms = db::today_entertainment_ms(conn, user_id, date_start)?;
    let intervention_count = db::today_intervention_count(conn, date_start)?;

    let mut stmt = conn.prepare(
        "SELECT shown_at, user_response FROM interventions
         WHERE shown_at >= ?1 ORDER BY shown_at DESC LIMIT 5",
    )?;
    let recent = stmt
        .query_map(params![date_start], |r| {
            Ok(RecentIntervention {
                shown_at: r.get(0)?,
                response: r.get(1)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    Ok(TodayContext {
        date: today,
        goal,
        entertainment_minutes: entertainment_ms as f64 / 60_000.0,
        intervention_count,
        recent_interventions: recent,
    })
}

/// 今日最小行动（MVP）：有未完成的今日目标则返回它，否则为空（提示用户先设目标）。
/// 后续升级为 select_today_actions（多目标里选 1–3 个）。
pub fn today_actions(conn: &Connection) -> rusqlite::Result<Vec<String>> {
    let today = today_str();
    match db::get_today_goal(conn, &today)? {
        Some(g) => Ok(vec![g.raw_text]),
        None => Ok(vec![]),
    }
}

/// 设置今日目标（同一天复用 id，重复调用视为修改文本并重置为 planned）。返回 goal id。
pub fn set_goal(conn: &Connection, text: &str) -> rusqlite::Result<String> {
    let today = today_str();
    let existing = db::get_today_goal(conn, &today)?;
    let id = existing
        .map(|g| g.id)
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let now_ms = chrono::Utc::now().timestamp_millis();
    conn.execute(
        "INSERT INTO daily_goals (id, date, raw_text, status, created_at)
         VALUES (?1, ?2, ?3, 'planned', ?4)
         ON CONFLICT(id) DO UPDATE SET raw_text = excluded.raw_text, status = 'planned'",
        params![id, today, text, now_ms],
    )?;
    Ok(id)
}

/// 记录用户对干预的响应；start_task / abandon_today 同步更新今日目标状态。
pub fn record_feedback(
    conn: &Connection,
    intervention_id: &str,
    action: &str,
) -> rusqlite::Result<()> {
    db::update_intervention_response(conn, intervention_id, action)?;
    if action == "start_task" || action == "abandon_today" {
        let today = today_str();
        if let Some(goal) = db::get_today_goal(conn, &today)? {
            let new_status = if action == "start_task" {
                "started"
            } else {
                "abandoned"
            };
            let _ = db::update_goal_status(conn, &goal.id, new_status);
        }
    }
    Ok(())
}
