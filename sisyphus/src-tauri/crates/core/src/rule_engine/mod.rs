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
    /// 当前前台会话已持续时长（ms，不限分类）。动态规则用它补齐进行中的未闭合会话。
    /// 0 = 未知。调用方（采集器）用 `now - session.start` 注入。
    #[serde(default)]
    pub active_session_ms: i64,
    /// Layer 2：媒体通知开始时间（epoch ms）。0 = Layer 2 未启用或未播放。
    pub media_playing_since_ms: i64,
    /// Layer 3：过去 10min scroll_burst 总次数。0 = Layer 3 未启用。
    pub recent_scroll_count: i64,
    pub today_goal: Option<DailyGoal>,
}

/// 命中后的响应策略（proactive-triggers.md §3 的可拓展 seam）。
/// 规则只表达"检出了什么 + 该怎么回应"，不直接产生副作用；由 [`crate::intervention`]
/// 翻译成"立即派发"或"入队 `scheduled_actions`"。可从动态规则的 `response_json` 反序列化。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "policy", rename_all = "snake_case")]
pub enum ResponsePolicy {
    /// 立即：由调用方当拍派发（保实时）。kind = notify | pet_message。
    Immediate {
        #[serde(default = "default_action_kind")]
        kind: String,
    },
    /// 延后：now+after_ms 入队，由 app ticker 到点派发。
    Deferred {
        #[serde(default = "default_action_kind")]
        kind: String,
        after_ms: i64,
    },
    /// 防打扰：带 dedup_key 入队，窗口内同 key 只提醒一次。
    Debounce {
        #[serde(default = "default_action_kind")]
        kind: String,
        window_ms: i64,
        dedup_key: String,
    },
    /// 不打扰（冷却期 / 夜间免打扰等）。
    Suppress,
}

fn default_action_kind() -> String {
    "notify".to_string()
}

impl Default for ResponsePolicy {
    fn default() -> Self {
        ResponsePolicy::Immediate {
            kind: default_action_kind(),
        }
    }
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
    /// 命中后的响应策略（默认 Immediate→notify，保持既有实时行为）。
    #[serde(default)]
    pub response: ResponsePolicy,
    /// 可选的定制干预文案。None 时 [`crate::intervention`] 用默认娱乐模板。
    #[serde(default)]
    pub message: Option<String>,
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

    /// 评估所有规则，返回**最该处理的那一条** Finding（每拍最多干预一次）。
    ///
    /// 顺序不是"谁先注册谁赢"：先跑用户/智能体建的动态规则，再跑内置规则，然后按
    /// severity 取最高（同级时动态规则优先）。此前是"第一个命中即返回、内置永远排在前面"，
    /// 于是用户精心建的「夜间游戏 20 分钟」会被内置的通用娱乐规则抢先，永远不触发。
    pub fn evaluate(
        &self,
        ctx: &RuleContext,
        conn: &Connection,
    ) -> rusqlite::Result<Option<Finding>> {
        let mut hits: Vec<Finding> = Vec::new();
        for rule in crate::rules::load_enabled_rules(conn)? {
            if let Some(finding) = rule.evaluate(ctx, conn)? {
                hits.push(finding);
            }
        }
        for rule in &self.rules {
            if let Some(finding) = rule.evaluate(ctx, conn)? {
                hits.push(finding);
            }
        }
        // stable sort：同 severity 时保持"动态规则在前"的次序。
        hits.sort_by_key(|f| match f.severity.as_str() {
            "high" => 0,
            _ => 1,
        });
        Ok(hits.into_iter().next())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_response_policy_json() {
        // 动态规则的 response_json 就用这个形状（批次 C 依赖）。
        let imm: ResponsePolicy =
            serde_json::from_str(r#"{"policy":"immediate","kind":"pet_message"}"#).unwrap();
        assert!(matches!(imm, ResponsePolicy::Immediate { kind } if kind == "pet_message"));

        // kind 省略时默认 notify。
        let def: ResponsePolicy =
            serde_json::from_str(r#"{"policy":"deferred","after_ms":600000}"#).unwrap();
        assert!(matches!(def, ResponsePolicy::Deferred { kind, after_ms } if kind == "notify" && after_ms == 600_000));

        let sup: ResponsePolicy = serde_json::from_str(r#"{"policy":"suppress"}"#).unwrap();
        assert!(matches!(sup, ResponsePolicy::Suppress));

        // 默认策略是立即 notify（保持既有实时行为）。
        assert!(matches!(ResponsePolicy::default(), ResponsePolicy::Immediate { kind } if kind == "notify"));
    }
}
