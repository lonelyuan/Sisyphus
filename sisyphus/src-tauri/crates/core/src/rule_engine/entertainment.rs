use std::collections::HashMap;
use rusqlite::Connection;
use serde_json::json;
use crate::rule_engine::{Finding, ResponsePolicy, Rule, RuleContext};
use crate::rule_engine::config::EntertainmentRuleConfig;
use crate::db;

/// MVP 娱乐应用包名 → 分类 映射。
/// 修改此列表只需在此处增删，不影响规则逻辑。
/// 暂未接线（Android 采集器落地后使用），先保留。
#[allow(dead_code)]
pub fn entertainment_packages() -> HashMap<&'static str, &'static str> {
    let mut m = HashMap::new();
    m.insert("tv.danmaku.bili",               "entertainment.video");  // B 站
    m.insert("com.ss.android.ugc.aweme",      "entertainment.video");  // 抖音
    m.insert("com.kuaishou.nebula",           "entertainment.video");  // 快手
    m.insert("com.zhiliaoapp.musically",      "entertainment.video");  // TikTok
    m.insert("com.google.android.youtube",    "entertainment.video");
    m.insert("com.netflix.mediaclient",       "entertainment.video");
    m.insert("com.tencent.qqlive",            "entertainment.video");  // 腾讯视频
    m.insert("com.qiyi.video",               "entertainment.video");  // 爱奇艺
    m.insert("com.youku.phone",              "entertainment.video");  // 优酷
    m.insert("com.ss.android.article.news",  "entertainment.news");   // 今日头条
    m.insert("com.tencent.news",             "entertainment.news");   // 腾讯新闻
    m.insert("com.sina.weibo",               "entertainment.social"); // 微博
    m.insert("com.reddit.frontpage",         "entertainment.social");
    m.insert("com.instagram.android",        "entertainment.social");
    m
}

pub struct EntertainmentSessionRule {
    config: EntertainmentRuleConfig,
}

impl EntertainmentSessionRule {
    pub fn new(config: EntertainmentRuleConfig) -> Self {
        Self { config }
    }
}

const TERMINAL_STATUSES: &[&str] = &["completed", "skipped", "abandoned"];

impl Rule for EntertainmentSessionRule {
    fn id(&self) -> &str {
        "entertainment_session_v1"
    }

    fn version(&self) -> u32 {
        1
    }

    fn evaluate(&self, ctx: &RuleContext, conn: &Connection) -> rusqlite::Result<Option<Finding>> {
        // 1. 当前 app 必须是娱乐类
        let is_entertainment = ctx
            .current_category
            .as_deref()
            .map(|c| c.starts_with("entertainment"))
            .unwrap_or(false);
        if !is_entertainment {
            return Ok(None);
        }

        // 2. 今日目标必须存在且未完成
        let goal = match &ctx.today_goal {
            Some(g) if !TERMINAL_STATUSES.contains(&g.status.as_str()) => g,
            _ => return Ok(None),
        };

        // 3. 冷却检查
        let cooldown_ms = self.config.cooldown_minutes * 60_000;
        if !db::is_cooldown_ready(conn, self.id(), ctx.now_ms, cooldown_ms)? {
            return Ok(None);
        }

        // 4. 窗口内娱乐总时长 >= 阈值
        let window_ms = self.config.window_minutes * 60_000;
        let since_ms = ctx.now_ms - window_ms;
        let closed_ms = db::sum_entertainment_ms(conn, &ctx.user_id, since_ms)?;
        let total_ms = closed_ms + ctx.active_entertainment_ms;
        let total_minutes = total_ms as f64 / 60_000.0;
        let threshold = self.config.threshold_minutes as f64;

        if total_minutes < threshold {
            return Ok(None);
        }

        // 5. 误报抑制（Layer 2 / Layer 3 均启用时才检查）
        if ctx.media_playing_since_ms > 0 && ctx.recent_scroll_count > 0 {
            let media_stable_ms = self.config.media_stable_minutes * 60_000;
            let media_stable = ctx.now_ms - ctx.media_playing_since_ms >= media_stable_ms;
            let scroll_active = ctx.recent_scroll_count >= self.config.scroll_active_threshold;
            if media_stable && !scroll_active {
                return Ok(None); // 被动看视频，跳过
            }
        }

        // 命中
        let severity = if total_minutes >= (self.config.threshold_minutes * 2) as f64 {
            "high"
        } else {
            "medium"
        };

        Ok(Some(Finding {
            rule_id: self.id().to_string(),
            rule_version: self.version(),
            severity: severity.to_string(),
            confidence: 1.0,
            context_snapshot: json!({
                "current_app": ctx.current_app,
                "current_category": ctx.current_category,
                "total_entertainment_minutes": total_minutes,
                "goal_text": goal.raw_text,
                "window_minutes": self.config.window_minutes,
            }),
            recommended_intervention_types: vec!["notification".to_string()],
            parent_event_ids: vec![],
            response: ResponsePolicy::Immediate {
                kind: "notify".to_string(),
            },
            message: None,
        }))
    }
}
