pub mod config;
pub mod entertainment;

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use crate::rule_engine::config::RuleConfig;

// ── 共享数据结构 ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DailyGoal {
    pub id: String,
    pub date: String,
    pub raw_text: String,
    pub status: String, // planned | started | completed | skipped | abandoned
}

/// 规则评估的输入上下文，由 ForegroundService（Android）或定时器（Desktop）每 10s 组装后传入。
#[derive(Debug, Clone, Deserialize)]
pub struct RuleContext {
    pub now_ms: i64,
    pub user_id: String,
    pub device_id: String,
    /// 当前前台应用包名（Android）或进程名（Desktop）
    pub current_app: Option<String>,
    /// 当前 app 分类，如 "entertainment.video"
    pub current_category: Option<String>,
    /// 当前正在进行的娱乐会话时长（ms）。防漏算：未切走的 session 不在 DB 中，由调用方注入。
    pub active_entertainment_ms: i64,
    /// Layer 2：媒体通知开始时间（epoch ms）。0 = Layer 2 未启用或未播放。
    pub media_playing_since_ms: i64,
    /// Layer 3：过去 10min scroll_burst 总次数。0 = Layer 3 未启用。
    pub recent_scroll_count: i64,
    pub today_goal: Option<DailyGoal>,
}

/// 规则命中结果，传给 InterventionDecider 生成干预消息。
#[derive(Debug, Clone, Serialize)]
pub struct Finding {
    pub rule_id: String,
    pub rule_version: u32,
    pub severity: String, // "medium" | "high"
    pub confidence: f64,
    pub context_snapshot: serde_json::Value,
    pub recommended_intervention_types: Vec<String>,
    pub parent_event_ids: Vec<String>,
}

// ── Rule trait ────────────────────────────────────────────────────────────────

pub trait Rule: Send + Sync {
    fn id(&self) -> &str;
    fn version(&self) -> u32;
    fn evaluate(&self, ctx: &RuleContext, conn: &Connection) -> rusqlite::Result<Option<Finding>>;
}

// ── RuleEngine ────────────────────────────────────────────────────────────────

pub struct RuleEngine {
    rules: Vec<Box<dyn Rule>>,
}

impl RuleEngine {
    pub fn new(config: RuleConfig) -> Self {
        Self {
            rules: vec![
                Box::new(entertainment::EntertainmentSessionRule::new(config.entertainment)),
            ],
        }
    }

    /// 评估所有规则，返回第一个命中的 Finding（每次最多触发一条）。
    pub fn evaluate(
        &self,
        ctx: &RuleContext,
        conn: &Connection,
    ) -> rusqlite::Result<Option<Finding>> {
        for rule in &self.rules {
            if let Some(finding) = rule.evaluate(ctx, conn)? {
                return Ok(Some(finding));
            }
        }
        Ok(None)
    }
}
