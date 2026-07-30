//! 反思平面读模型：今日上下文、今日行动、目标与反馈写入。
//! App 命令与 MCP 工具都调用这里，保证逻辑单一来源。
//!
//! 两处已修正的口径问题：
//! 1. **"今天"的定义**统一走 [`crate::clock`]（本地时区 + 换日点），不再用 UTC 日期
//!    ——UTC+8 用户此前的日界落在早上 8 点。
//! 2. **未完成事项的来源**统一为 `life_items`（LifeDB 是事实源），不再读 `tasks`。
//!    此前看板写 `life_items`、今日上下文读 `tasks`，在看板里做完一件事，
//!    第二天早上仍会被提醒做它。

use rusqlite::{params, Connection};
use serde::Serialize;
use uuid::Uuid;

use crate::clock;
use crate::db;
use crate::lifedb::LifeItem;
use crate::lifetree::{self, NextAction};
use crate::rule_engine::DailyGoal;

#[derive(Debug, Serialize)]
pub struct RecentIntervention {
    pub shown_at: i64,
    pub response: Option<String>,
    pub outcome: Option<String>,
}

/// 今日上下文（只含 L0–L1 数据），供 Agent `query_context` 构建最小必要上下文。
#[derive(Debug, Serialize)]
pub struct TodayContext {
    pub date: String,
    pub goal: Option<DailyGoal>,
    pub entertainment_minutes: f64,
    pub intervention_count: i64,
    pub recent_interventions: Vec<RecentIntervention>,
    /// 未完成的人生事项（LifeDB 事实源：action / milestone / routine）。
    pub open_items: Vec<LifeItem>,
    /// 确定性选出的今日最小行动，**带理由**（见 [`lifetree::next_actions`]）。
    pub next_actions: Vec<NextAction>,
    /// 已到期未处理的提醒（被动：由 Agent 在规划时提及）。
    pub due_reminders: Vec<crate::artifacts::Reminder>,
}

pub fn today_context(conn: &Connection, user_id: &str) -> rusqlite::Result<TodayContext> {
    let boundary = clock::boundary_hour(conn);
    let now = clock::now_ms();
    let today = clock::day_str_at(now, boundary);
    let date_start = clock::day_start_at(now, boundary);

    let goal = db::get_today_goal(conn, &today)?;
    let entertainment_ms = db::today_entertainment_ms(conn, user_id, date_start)?;
    let intervention_count = db::today_intervention_count(conn, date_start)?;

    let mut stmt = conn.prepare(
        "SELECT shown_at, user_response, outcome FROM interventions
         WHERE shown_at >= ?1 ORDER BY shown_at DESC LIMIT 5",
    )?;
    let recent = stmt
        .query_map(params![date_start], |r| {
            Ok(RecentIntervention {
                shown_at: r.get(0)?,
                response: r.get(1)?,
                outcome: r.get(2)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let open_items = list_open_items(conn).unwrap_or_default();
    let next_actions = lifetree::next_actions(conn, 3).unwrap_or_default();
    let due_reminders = crate::artifacts::list_due_reminders(conn, now)?;

    Ok(TodayContext {
        date: today,
        goal,
        entertainment_minutes: entertainment_ms as f64 / 60_000.0,
        intervention_count,
        recent_interventions: recent,
        open_items,
        next_actions,
        due_reminders,
    })
}

/// 未完成的可执行事项（LifeDB）：action / milestone / routine，未 done/archived。
pub fn list_open_items(conn: &Connection) -> Result<Vec<LifeItem>, String> {
    Ok(crate::lifedb::list_items(conn, false)?
        .into_iter()
        .filter(|i| {
            matches!(i.kind.as_str(), "action" | "milestone" | "routine")
                && !matches!(i.status.as_str(), "done" | "archived")
        })
        .collect())
}

/// 今日最小行动（1–3 条文本，向后兼容的窄接口）。
/// 需要理由/结构时用 [`lifetree::next_actions`] 或 `today_context().next_actions`。
pub fn today_actions(conn: &Connection) -> rusqlite::Result<Vec<String>> {
    let today = clock::today_str(conn);
    let mut actions: Vec<String> = Vec::new();
    if let Some(g) = db::get_today_goal(conn, &today)? {
        actions.push(g.raw_text);
    }
    for a in lifetree::next_actions(conn, 3).unwrap_or_default() {
        if actions.len() >= 3 {
            break;
        }
        if !actions.contains(&a.title) {
            actions.push(a.title);
        }
    }
    Ok(actions)
}

/// 设置今日目标（同一天复用 id，重复调用视为修改文本并重置为 planned）。返回 goal id。
pub fn set_goal(conn: &Connection, text: &str) -> rusqlite::Result<String> {
    let today = clock::today_str(conn);
    let existing = db::get_today_goal(conn, &today)?;
    let id = existing
        .map(|g| g.id)
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    conn.execute(
        "INSERT INTO daily_goals (id, date, raw_text, status, created_at)
         VALUES (?1, ?2, ?3, 'planned', ?4)
         ON CONFLICT(id) DO UPDATE SET raw_text = excluded.raw_text, status = 'planned'",
        params![id, today, text, clock::now_ms()],
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
        let today = clock::today_str(conn);
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
