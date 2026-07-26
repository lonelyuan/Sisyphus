//! 动态检测规则（[docs/spec/rule-engine.md](../../../../../docs/spec/rule-engine.md)）。
//!
//! 用户/智能体用一句话描述"什么情况该提醒我"，落成一条 `detection_rules`：声明式
//! [`RuleTrigger`]（泛化内置 `EntertainmentSessionRule`）+ [`ResponsePolicy`]。
//! [`RuleEngine`](crate::rule_engine::RuleEngine) 每次评估热加载启用规则，改规则无需重编。
//!
//! 纯 rusqlite、无副作用，安卓可编（副作用仍在 app 层按 kind 派发）。

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::db;
use crate::rule_engine::{Finding, ResponsePolicy, Rule, RuleContext};

const TERMINAL_STATUSES: &[&str] = &["completed", "skipped", "abandoned"];

/// 声明式触发条件。所有维度都是"与"关系；分类维度内 prefix / categories 是"或"。
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct RuleTrigger {
    /// 当前 app 分类前缀，如 "entertainment" / "entertainment.game"。
    #[serde(default)]
    pub category_prefix: Option<String>,
    /// 精确分类白名单（与 prefix 或关系）。
    #[serde(default)]
    pub category_in: Vec<String>,
    /// 目标 app 包名 / bundle id 白名单。
    #[serde(default)]
    pub app_in: Vec<String>,
    /// 统计窗口（分钟），默认 30。
    #[serde(default)]
    pub window_minutes: Option<i64>,
    /// 触发阈值：窗口内匹配时长 ≥ 此分钟数则命中。
    pub min_minutes_in_window: i64,
    /// 是否要求今日有未完成目标才触发，默认 true。
    #[serde(default)]
    pub requires_active_goal: Option<bool>,
    /// 可选生效时段（本地时间，支持跨午夜，如 20:00–02:00）。
    #[serde(default)]
    pub time_of_day: Option<TimeWindow>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TimeWindow {
    /// "HH:MM"
    pub from: String,
    /// "HH:MM"
    pub to: String,
}

impl RuleTrigger {
    fn window_minutes(&self) -> i64 {
        self.window_minutes.filter(|m| *m > 0).unwrap_or(30)
    }
    fn requires_active_goal(&self) -> bool {
        self.requires_active_goal.unwrap_or(true)
    }
    /// 至少要有一个匹配维度（分类或 app），否则规则会命中一切——校验时拒绝。
    fn has_scope(&self) -> bool {
        self.category_prefix.as_deref().is_some_and(|p| !p.is_empty())
            || !self.category_in.is_empty()
            || !self.app_in.is_empty()
    }
}

/// 一条持久化的检测规则（DB 行的读模型，供 MCP / Tauri 列出）。
#[derive(Debug, Clone, Serialize)]
pub struct DetectionRule {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub trigger_json: String,
    pub response_json: String,
    pub severity: String,
    pub cooldown_minutes: i64,
    pub created_by: String,
    pub origin_capture_id: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// 可执行的动态规则：DB 行 + 解析好的 trigger / response。
pub struct DynamicRule {
    rule: DetectionRule,
    trigger: RuleTrigger,
    response: ResponsePolicy,
}

impl DynamicRule {
    fn matches_current(&self, ctx: &RuleContext) -> bool {
        let cat = ctx.current_category.as_deref();
        let category_ok = {
            let mut any_filter = false;
            let mut matched = false;
            if let Some(prefix) = self.trigger.category_prefix.as_deref().filter(|p| !p.is_empty()) {
                any_filter = true;
                matched |= cat.map(|c| c.starts_with(prefix)).unwrap_or(false);
            }
            if !self.trigger.category_in.is_empty() {
                any_filter = true;
                matched |= cat.map(|c| self.trigger.category_in.iter().any(|x| x == c)).unwrap_or(false);
            }
            !any_filter || matched
        };
        let app_ok = if self.trigger.app_in.is_empty() {
            true
        } else {
            ctx.current_app
                .as_deref()
                .map(|a| self.trigger.app_in.iter().any(|x| x == a))
                .unwrap_or(false)
        };
        category_ok && app_ok
    }
}

impl Rule for DynamicRule {
    fn id(&self) -> &str {
        &self.rule.id
    }
    fn version(&self) -> u32 {
        1
    }
    fn evaluate(&self, ctx: &RuleContext, conn: &Connection) -> rusqlite::Result<Option<Finding>> {
        // 1. 当前前台必须命中本规则的分类 / app。
        if !self.matches_current(ctx) {
            return Ok(None);
        }
        // 2. 生效时段。
        if let Some(win) = &self.trigger.time_of_day {
            if !in_time_window(ctx.now_ms, win) {
                return Ok(None);
            }
        }
        // 3. 目标要求。
        if self.trigger.requires_active_goal() {
            match &ctx.today_goal {
                Some(g) if !TERMINAL_STATUSES.contains(&g.status.as_str()) => {}
                _ => return Ok(None),
            }
        }
        // 4. 冷却。
        let cooldown_ms = self.rule.cooldown_minutes.max(0) * 60_000;
        if !db::is_cooldown_ready(conn, &self.rule.id, ctx.now_ms, cooldown_ms)? {
            return Ok(None);
        }
        // 5. 窗口内匹配时长 ≥ 阈值（闭合会话 + 进行中会话补入）。
        let window_ms = self.trigger.window_minutes() * 60_000;
        let since = ctx.now_ms - window_ms;
        let closed = db::sum_foreground_ms(
            conn,
            &ctx.user_id,
            since,
            self.trigger.category_prefix.as_deref(),
            &self.trigger.category_in,
            &self.trigger.app_in,
        )?;
        // 进行中会话（未闭合，不在 DB）：当前既已命中过滤，补上其时长。
        let active = if ctx.active_session_ms > 0 {
            ctx.active_session_ms
        } else {
            ctx.active_entertainment_ms
        };
        let total_min = (closed + active) as f64 / 60_000.0;
        if total_min < self.trigger.min_minutes_in_window as f64 {
            return Ok(None);
        }

        let app_label = ctx.current_app.as_deref().unwrap_or("这个应用");
        let goal_hint = ctx
            .today_goal
            .as_ref()
            .map(|g| format!("\n今日目标：{}", g.raw_text))
            .unwrap_or_default();
        let message = format!(
            "「{}」触发：你在 {} 上已累计约 {:.0} 分钟。{}",
            self.rule.name, app_label, total_min, goal_hint
        );

        Ok(Some(Finding {
            rule_id: self.rule.id.clone(),
            rule_version: self.version(),
            severity: self.rule.severity.clone(),
            confidence: 1.0,
            context_snapshot: serde_json::json!({
                "current_app": ctx.current_app,
                "current_category": ctx.current_category,
                "total_minutes": total_min,
                "window_minutes": self.trigger.window_minutes(),
                "rule_name": self.rule.name,
            }),
            recommended_intervention_types: vec!["notification".to_string()],
            parent_event_ids: vec![],
            response: self.response.clone(),
            message: Some(message),
        }))
    }
}

/// 解析 "HH:MM"（本地时间）→ 当日分钟数。
fn parse_hm(s: &str) -> Option<i64> {
    let (h, m) = s.trim().split_once(':')?;
    let hh: i64 = h.trim().parse().ok()?;
    let mm: i64 = m.trim().parse().ok()?;
    if !(0..=23).contains(&hh) || !(0..=59).contains(&mm) {
        return None;
    }
    Some(hh * 60 + mm)
}

/// now 是否落在本地时段窗口内（from > to 视为跨午夜）。解析失败则视为始终生效。
fn in_time_window(now_ms: i64, win: &TimeWindow) -> bool {
    use chrono::{Local, TimeZone, Timelike};
    let (Some(from), Some(to)) = (parse_hm(&win.from), parse_hm(&win.to)) else {
        return true;
    };
    let now = match Local.timestamp_millis_opt(now_ms).single() {
        Some(t) => t,
        None => return true,
    };
    let cur = now.hour() as i64 * 60 + now.minute() as i64;
    if from <= to {
        cur >= from && cur < to
    } else {
        cur >= from || cur < to
    }
}

// ── CRUD ────────────────────────────────────────────────────────────────────

/// 校验 trigger_json / response_json，落一条 detection_rules，返回 id。
#[allow(clippy::too_many_arguments)]
pub fn create_rule(
    conn: &Connection,
    name: &str,
    trigger_json: &str,
    response_json: Option<&str>,
    severity: &str,
    cooldown_minutes: i64,
    created_by: &str,
    origin_capture_id: Option<&str>,
) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("规则名不能为空".to_string());
    }
    let trigger: RuleTrigger =
        serde_json::from_str(trigger_json).map_err(|e| format!("trigger_json 解析失败: {e}"))?;
    if trigger.min_minutes_in_window <= 0 {
        return Err("min_minutes_in_window 必须为正".to_string());
    }
    if !trigger.has_scope() {
        return Err(
            "规则至少要指定一个 category_prefix / category_in / app_in，否则会命中一切".to_string(),
        );
    }
    let response = response_json.unwrap_or(r#"{"policy":"immediate","kind":"notify"}"#);
    let _: ResponsePolicy =
        serde_json::from_str(response).map_err(|e| format!("response_json 解析失败: {e}"))?;
    let severity = match severity {
        "high" => "high",
        _ => "medium",
    };

    let id = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().timestamp_millis();
    conn.execute(
        "INSERT INTO detection_rules
           (id,name,enabled,trigger_json,response_json,severity,cooldown_minutes,created_by,origin_capture_id,created_at,updated_at)
         VALUES (?1,?2,1,?3,?4,?5,?6,?7,?8,?9,?9)",
        params![
            id,
            name,
            trigger_json,
            response,
            severity,
            cooldown_minutes.max(0),
            created_by,
            origin_capture_id,
            now
        ],
    )
    .map_err(|e| format!("写入规则失败: {e}"))?;
    Ok(id)
}

fn row_to_rule(r: &rusqlite::Row) -> rusqlite::Result<DetectionRule> {
    Ok(DetectionRule {
        id: r.get(0)?,
        name: r.get(1)?,
        enabled: r.get::<_, i64>(2)? != 0,
        trigger_json: r.get(3)?,
        response_json: r.get(4)?,
        severity: r.get(5)?,
        cooldown_minutes: r.get(6)?,
        created_by: r.get(7)?,
        origin_capture_id: r.get(8)?,
        created_at: r.get(9)?,
        updated_at: r.get(10)?,
    })
}

const SELECT_COLS: &str =
    "id,name,enabled,trigger_json,response_json,severity,cooldown_minutes,created_by,origin_capture_id,created_at,updated_at";

pub fn list_rules(conn: &Connection) -> rusqlite::Result<Vec<DetectionRule>> {
    let sql = format!("SELECT {SELECT_COLS} FROM detection_rules ORDER BY created_at DESC");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map([], row_to_rule)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

pub fn set_rule_enabled(conn: &Connection, id: &str, enabled: bool) -> rusqlite::Result<()> {
    let now = chrono::Utc::now().timestamp_millis();
    conn.execute(
        "UPDATE detection_rules SET enabled=?2, updated_at=?3 WHERE id=?1",
        params![id, enabled as i64, now],
    )?;
    Ok(())
}

pub fn delete_rule(conn: &Connection, id: &str) -> rusqlite::Result<()> {
    conn.execute("DELETE FROM detection_rules WHERE id=?1", params![id])?;
    Ok(())
}

/// 加载所有启用规则为可执行 [`DynamicRule`]。trigger/response 解析失败的行跳过（不拖垮评估）。
pub fn load_enabled_rules(conn: &Connection) -> rusqlite::Result<Vec<DynamicRule>> {
    let sql = format!("SELECT {SELECT_COLS} FROM detection_rules WHERE enabled=1");
    let mut stmt = conn.prepare(&sql)?;
    let rules = stmt
        .query_map([], row_to_rule)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rules
        .into_iter()
        .filter_map(|rule| {
            let trigger: RuleTrigger = serde_json::from_str(&rule.trigger_json).ok()?;
            let response: ResponsePolicy = serde_json::from_str(&rule.response_json).ok()?;
            Some(DynamicRule {
                rule,
                trigger,
                response,
            })
        })
        .collect())
}
