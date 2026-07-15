/// 规则引擎策略配置 —— 所有规则参数的统一入口，与规则逻辑代码分离。
/// 修改规则参数只需改此文件，不触碰规则逻辑。

#[derive(Debug, Clone)]
pub struct EntertainmentRuleConfig {
    /// 时间窗口（分钟）：在此窗口内统计娱乐 app 总时长
    pub window_minutes: i64,
    /// 触发阈值（分钟）：窗口内娱乐总时长超过此值则命中
    pub threshold_minutes: i64,
    /// 冷却时间（分钟）：上次触发后多久才能再次触发
    pub cooldown_minutes: i64,
    /// 滚动活跃阈值：10min 内 scroll_burst 总次数超过此值认为在主动刷（Layer 3 启用时生效）
    pub scroll_active_threshold: i64,
    /// 媒体稳定播放阈值（分钟）：连续播放超过此值认为在被动看视频（Layer 2 启用时生效）
    pub media_stable_minutes: i64,
}

impl Default for EntertainmentRuleConfig {
    fn default() -> Self {
        Self {
            window_minutes: 30,
            threshold_minutes: 15,
            cooldown_minutes: 30,
            scroll_active_threshold: 30,
            media_stable_minutes: 5,
        }
    }
}

#[cfg(debug_assertions)]
impl EntertainmentRuleConfig {
    /// Debug 构建下使用小阈值，方便快速验证整条链路。
    pub fn debug_fast() -> Self {
        Self {
            window_minutes: 2,
            threshold_minutes: 1,
            cooldown_minutes: 2,
            scroll_active_threshold: 30,
            media_stable_minutes: 1,
        }
    }
}

/// 全局规则配置，RuleEngine 初始化时注入。
#[derive(Debug, Clone)]
pub struct RuleConfig {
    pub entertainment: EntertainmentRuleConfig,
}

impl Default for RuleConfig {
    fn default() -> Self {
        Self {
            #[cfg(debug_assertions)]
            entertainment: EntertainmentRuleConfig::debug_fast(),
            #[cfg(not(debug_assertions))]
            entertainment: EntertainmentRuleConfig::default(),
        }
    }
}
