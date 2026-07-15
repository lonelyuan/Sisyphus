# Spec: 规则引擎

规则引擎运行在 **Rust 侧**（`sisyphus/src-tauri/src/rule_engine/`），由 ForegroundService（Android）或定时器（Desktop）每 10s 调用一次。它是**感知平面**的一部分，本地常驻、确定性、实时（见 [architecture.md](architecture.md) §1.1）。

值钱逻辑只写一份，两端共用同一 Rust 代码。

---

## 接口定义

```rust
// rule_engine/mod.rs

pub trait Rule: Send + Sync {
    fn id(&self) -> &str;
    fn version(&self) -> u32;
    fn evaluate(&self, ctx: &RuleContext, conn: &Connection) -> Result<Option<Finding>>;
}

pub struct RuleContext {
    pub now_ms: i64,
    pub user_id: String,
    pub device_id: String,
    pub current_app: Option<String>,        // 当前前台包名
    pub current_category: Option<String>,   // 当前 app 分类
    pub active_entertainment_ms: i64,       // 正在进行的娱乐会话时长（防漏算，由 Service 注入）
    pub media_playing_since_ms: i64,        // 媒体通知开始时间，0 = 未播放（Layer 2）
    pub recent_scroll_count: i64,           // 过去 10min scroll_burst 总次数，0 = 未开启（Layer 3）
    pub today_goal: Option<DailyGoal>,
}

pub struct Finding {
    pub rule_id: String,
    pub rule_version: u32,
    pub severity: String,       // "medium" | "high"
    pub confidence: f64,
    pub context_snapshot: serde_json::Value,
    pub recommended_intervention_types: Vec<String>,
    pub parent_event_ids: Vec<String>,
}
```

### RuleEngine 调度

```rust
pub struct RuleEngine {
    rules: Vec<Box<dyn Rule>>,
}

impl RuleEngine {
    pub async fn evaluate(&self, ctx: RuleContext, conn: &Connection) {
        for rule in &self.rules {
            if let Some(finding) = rule.evaluate(&ctx, conn)? {
                let decision = InterventionDecider::decide(&finding, &ctx);
                if decision.should_show {
                    // 写 finding/decision 事件到本地 SQLite
                    // 触发通知（Android: Kotlin plugin；Desktop: tauri notification）
                    // 更新冷却时间
                }
            }
        }
    }
}
```

---

## CooldownStore

用 SQLite `interventions` 表查询上次触发时间，避免额外状态存储：

```rust
fn is_ready(rule_id: &str, now_ms: i64, cooldown_ms: i64, conn: &Connection) -> bool {
    let last_shown: Option<i64> = conn.query_row(
        "SELECT MAX(shown_at) FROM interventions WHERE rule_id = ?",
        [rule_id], |r| r.get(0)
    ).ok().flatten();
    last_shown.map_or(true, |t| now_ms - t > cooldown_ms)
}
```

---

## MVP 规则：EntertainmentSessionRule

### 触发条件（全部满足）

| 条件 | 实现 |
|---|---|
| 当前前台 app 是娱乐类 | `current_category.starts_with("entertainment")` |
| 窗口内娱乐总时长 ≥ 阈值 | SQL 查询闭合区间 + `active_entertainment_ms`（防漏算） |
| 今日目标存在且未完成 | `today_goal.status in ["planned", "started"]` |
| 冷却时间已过 | `CooldownStore::is_ready()` |

**误报抑制**：`media_playing_since > 0` 且稳定播放 ≥5min 且 `recent_scroll_count < 30` → 认为被动看视频 → 跳过。

### 默认参数

```rust
const WINDOW_MINUTES: i64 = 30;
const THRESHOLD_MINUTES: i64 = 15;
const COOLDOWN_MINUTES: i64 = 30;
const SCROLL_ACTIVE_THRESHOLD: i64 = 30;  // 过去 10min 内
```

### 防漏算说明

`UsageStatsManager.queryEvents()` 只返回已有 BACKGROUND 事件的闭合区间。若娱乐 app 仍在前台（未切走），此段时长不在 DB 中。

解决方案：ForegroundService 在**内存**中维护 `foreground_start_ms`，每次 evaluate 时通过 `active_entertainment_ms` 字段注入当前进行中的时长：

```
total_ms = closed_ms_from_db + active_entertainment_ms
```

---

## 新增规则

1. 在 `app/src-tauri/src/rule_engine/rules/` 添加新 `.rs` 文件，实现 `Rule` trait
2. 在 `RuleEngine::new()` 的 `rules` 列表中注册
3. 在本文档登记规则 ID、版本、触发条件

### 预留规则（v1.1）

`ScrollBurstRule`（`scroll_burst_v1`）：过去 10min 内 scroll_burst 总次数 ≥ 50 → 命中。依赖 AccessibilityService（Layer 3，用户可选）。
