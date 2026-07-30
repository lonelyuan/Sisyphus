//! 无极时间线读模型：行为带 + 里程碑点 + 长期计划跨度 + 干预历史。
//!
//! 三条设计约束（决定 UX 能做多深）：
//!
//! 1. **代价与缩放解耦**：粗尺度不再扫 `raw_events`，走 [`crate::rollups`] 的预聚合桶。
//!    年尺度的代价是 O(可见桶数)，不是 O(事件数)。
//! 2. **不同尺度显示不同抽象层次**：每个点事件带一个显著性等级 [`lod_level`]，
//!    尺度越粗只保留越重要的层。此前所有 artifact 事件在所有尺度全量返回、超限就按
//!    表的顺序截断——于是年视图里留下的是"恰好排在前面的"，不是"重要的"。
//! 3. **长期计划必须在时间线上**：LifeDB 的目标/项目/里程碑有 `start_at` / `due_at`，
//!    它们是"人生尺度"上唯一有意义的图层。此前 `has_long_term_source` 直接硬编码为
//!    `false`，这一层完全没画。

use rusqlite::{params, Connection};
use serde::Serialize;

use crate::clock;
use crate::rollups::{self, Bucket};

/// 一条干预记录。
#[derive(Debug, Serialize)]
pub struct InterventionRow {
    pub id: String,
    pub rule_id: String,
    pub shown_at: i64,
    pub severity: String,
    pub message: String,
    pub user_response: Option<String>,
    pub responded_at: Option<i64>,
    pub outcome: Option<String>,
}

/// 无极时间轴上的统一可视事件。所有坐标都用 epoch ms，前端只做投影和绘制。
#[derive(Debug, Clone, Serialize)]
pub struct TimelineEvent {
    pub id: String,
    /// behavior | capture | intervention | goal | task | reminder | knowledge | rule
    /// | life_goal | life_project | life_milestone | life_skill
    pub kind: String,
    pub start_ms: i64,
    pub end_ms: i64,
    pub title: String,
    pub category: Option<String>,
    pub detail: Option<String>,
    pub severity: Option<String>,
    /// 显著性等级：0 最重要（人生尺度也显示），3 最细（只在分钟/日尺度显示）。
    pub level: u8,
}

/// 事件在无极缩放下的显著性等级。**这就是"不同尺度不同抽象层次"的实现**。
///
/// 0 = 人生尺度仍可见（人生级里程碑：目标、技能、里程碑达成）
/// 1 = 月尺度可见（项目、知识结晶、检测规则这类结构性变化）
/// 2 = 周尺度可见（具体事项、提醒、干预）
/// 3 = 日/分钟尺度可见（原始行为会话、零散 capture）
pub fn lod_level(kind: &str) -> u8 {
    match kind {
        "life_goal" | "life_skill" | "life_milestone" => 0,
        "goal" | "life_project" | "knowledge" | "rule" => 1,
        "task" | "reminder" | "intervention" => 2,
        _ => 3,
    }
}

/// 当前尺度允许的最大等级。跨度越大，保留的层越少。
fn max_level_for(detail: &str, span_ms: i64) -> u8 {
    const DAY: i64 = 86_400_000;
    match detail {
        "minute" => 3,
        "day" => 3,
        "week" => 2,
        "month" => 1,
        "life" => 0,
        // detail 由前端连续 zoom 推导，兜底按跨度判断。
        _ => {
            if span_ms <= 2 * DAY {
                3
            } else if span_ms <= 60 * DAY {
                2
            } else if span_ms <= 550 * DAY {
                1
            } else {
                0
            }
        }
    }
}

/// 周尺度及以上使用的每日聚合。原始区间仍留在 Event log，不在前端重复聚合。
#[derive(Debug, Clone, Serialize)]
pub struct DaySummary {
    pub date: String,
    pub start_ms: i64,
    pub observed_ms: i64,
    pub focus_ms: i64,
    pub entertainment_ms: i64,
    pub neutral_ms: i64,
    pub intervention_count: i64,
    /// 0–100 的可解释启发式状态分；无观测时为 50，不伪装成健康结论。
    pub state_score: i64,
}

/// 一个聚合桶（无极缩放的主数据；桶粒度随跨度自动变粗）。
#[derive(Debug, Clone, Serialize)]
pub struct TimeBand {
    pub bucket_start_ms: i64,
    pub bucket_end_ms: i64,
    pub observed_ms: i64,
    pub focus_ms: i64,
    pub entertainment_ms: i64,
    pub neutral_ms: i64,
    /// 该桶内时长最高的分类（给条带上色）。
    pub top_category: Option<String>,
}

/// 长期计划跨度（LifeDB 图层）：目标/项目/技能在时间轴上的区间，里程碑是点。
#[derive(Debug, Clone, Serialize)]
pub struct PlanSpan {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub track: String,
    pub status: String,
    pub start_ms: i64,
    pub end_ms: i64,
    /// 由 Core 确定性算出的进度（0–1），不是估的。
    pub progress: f64,
    pub level: u8,
}

#[derive(Debug, Serialize)]
pub struct TimelineResponse {
    pub start_ms: i64,
    pub end_ms: i64,
    pub detail: String,
    /// 本次实际使用的桶粒度：day | week | month | none。
    pub bucket: String,
    pub events: Vec<TimelineEvent>,
    pub days: Vec<DaySummary>,
    /// 预聚合条带（粗尺度的主数据）。
    pub bands: Vec<TimeBand>,
    /// 长期计划图层（LifeDB）。
    pub plans: Vec<PlanSpan>,
    pub truncated: bool,
    /// 长期计划来自 LifeDB；没有 LifeItem 时前端展示真实空态。
    pub has_long_term_source: bool,
}

/// 按可见窗口查询时间轴。`detail` 由前端根据连续 zoom 推导，而不是切换页面。
pub fn query_timeline(
    conn: &Connection,
    start_ms: i64,
    end_ms: i64,
    detail: &str,
    max_items: i64,
) -> Result<TimelineResponse, String> {
    if end_ms <= start_ms {
        return Err("时间窗口无效".to_string());
    }
    let span = end_ms - start_ms;
    let limit = max_items.clamp(50, 5_000);
    let max_level = max_level_for(detail, span);
    let boundary = clock::boundary_hour(conn);
    // 读路径自愈：有新行为事件就先追平预聚合桶（无新事件时只花一次索引查询）。
    let _ = rollups::catch_up(conn);

    let mut events: Vec<TimelineEvent> = Vec::new();
    let mut truncated = false;

    // 原始行为会话只在最细的两档出现（等级 3）。
    if max_level >= 3 {
        let mut raw = query_behavior_events(conn, start_ms, end_ms, detail, limit + 1)
            .map_err(|e| e.to_string())?;
        if raw.len() as i64 > limit {
            raw.truncate(limit as usize);
            truncated = true;
        }
        events.extend(raw);
    }

    // 干预（等级 2）与 artifact 里程碑（等级 1–2）按等级过滤，而不是按表顺序截断。
    if max_level >= 2 {
        let remaining = (limit as usize).saturating_sub(events.len()).max(20) as i64;
        let mut interaction = query_intervention_events(conn, start_ms, end_ms, remaining + 1)
            .map_err(|e| e.to_string())?;
        if interaction.len() as i64 > remaining {
            interaction.truncate(remaining as usize);
            truncated = true;
        }
        events.extend(interaction);
    }

    let artifact_cap = (limit / 4).clamp(40, 400);
    let mut artifacts = query_artifact_events(conn, start_ms, end_ms, artifact_cap + 1, max_level)
        .map_err(|e| e.to_string())?;
    if artifacts.len() as i64 > artifact_cap {
        artifacts.truncate(artifact_cap as usize);
        truncated = true;
    }
    events.extend(artifacts);

    events.retain(|e| e.level <= max_level);
    events.sort_by_key(|e| e.start_ms);

    // 聚合条带：跨度决定桶粒度；细尺度不需要条带。
    let (bucket, bands) = if span >= 2 * 86_400_000 {
        let b = Bucket::from_span(span);
        let aligned_start = b.start_of(start_ms, boundary);
        (b.as_str().to_string(), bands(conn, b, aligned_start, end_ms, boundary)?)
    } else {
        ("none".to_string(), Vec::new())
    };

    // days 仍保留（前端已有 UI 依赖），但现在由 rollup 桶推导，不再全表 strftime。
    let days = if span >= 86_400_000 {
        day_summaries(conn, start_ms, end_ms, boundary)?
    } else {
        Vec::new()
    };

    let plans = plan_spans(conn, start_ms, end_ms, max_level)?;
    let has_long_term_source = has_life_items(conn)?;

    Ok(TimelineResponse {
        start_ms,
        end_ms,
        detail: detail.to_string(),
        bucket,
        events,
        days,
        bands,
        plans,
        truncated,
        has_long_term_source,
    })
}

fn bucket_next(bucket: Bucket, start: i64, boundary: u32) -> i64 {
    match bucket {
        Bucket::Day => clock::day_end_at(start, boundary),
        Bucket::Week => clock::week_start_at(start + 8 * 86_400_000, boundary),
        Bucket::Month => clock::month_start_at(start + 32 * 86_400_000, boundary),
    }
}

/// 从 `time_rollups` 组装条带（分类维度）。
fn bands(
    conn: &Connection,
    bucket: Bucket,
    start_ms: i64,
    end_ms: i64,
    boundary: u32,
) -> Result<Vec<TimeBand>, String> {
    let slices = rollups::slices(conn, bucket, "category", start_ms, end_ms)
        .map_err(|e| e.to_string())?;
    let mut out: Vec<TimeBand> = Vec::new();
    for slice in slices {
        let band = match out.last_mut() {
            Some(b) if b.bucket_start_ms == slice.bucket_start_ms => b,
            _ => {
                out.push(TimeBand {
                    bucket_start_ms: slice.bucket_start_ms,
                    bucket_end_ms: bucket_next(bucket, slice.bucket_start_ms, boundary),
                    observed_ms: 0,
                    focus_ms: 0,
                    entertainment_ms: 0,
                    neutral_ms: 0,
                    top_category: None,
                });
                out.last_mut().unwrap()
            }
        };
        band.observed_ms += slice.duration_ms;
        if slice.key.starts_with("entertainment") {
            band.entertainment_ms += slice.duration_ms;
        } else if slice.key == "(unknown)" {
            band.neutral_ms += slice.duration_ms;
        } else {
            band.focus_ms += slice.duration_ms;
        }
        // slices 按 duration DESC 排在同桶内，第一条即最大。
        if band.top_category.is_none() {
            band.top_category = Some(slice.key.clone());
        }
    }
    Ok(out)
}

/// 日聚合：由日桶 + 当日干预数推导（口径与条带一致）。
fn day_summaries(
    conn: &Connection,
    start_ms: i64,
    end_ms: i64,
    boundary: u32,
) -> Result<Vec<DaySummary>, String> {
    let aligned = clock::day_start_at(start_ms, boundary);
    let slices = rollups::slices(conn, Bucket::Day, "category", aligned, end_ms)
        .map_err(|e| e.to_string())?;
    let mut counts = std::collections::HashMap::<i64, i64>::new();
    {
        let mut stmt = conn
            .prepare("SELECT shown_at FROM interventions WHERE shown_at BETWEEN ?1 AND ?2")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![aligned, end_ms], |r| r.get::<_, i64>(0))
            .map_err(|e| e.to_string())?;
        for row in rows {
            let at = row.map_err(|e| e.to_string())?;
            *counts.entry(clock::day_start_at(at, boundary)).or_default() += 1;
        }
    }

    let mut out: Vec<DaySummary> = Vec::new();
    for slice in slices {
        let day = match out.last_mut() {
            Some(d) if d.start_ms == slice.bucket_start_ms => d,
            _ => {
                let start = slice.bucket_start_ms;
                out.push(DaySummary {
                    date: clock::day_str_at(start, boundary),
                    start_ms: start,
                    observed_ms: 0,
                    focus_ms: 0,
                    entertainment_ms: 0,
                    neutral_ms: 0,
                    intervention_count: counts.get(&start).copied().unwrap_or(0),
                    state_score: 50,
                });
                out.last_mut().unwrap()
            }
        };
        day.observed_ms += slice.duration_ms;
        if slice.key.starts_with("entertainment") {
            day.entertainment_ms += slice.duration_ms;
        } else if slice.key == "(unknown)" {
            day.neutral_ms += slice.duration_ms;
        } else {
            day.focus_ms += slice.duration_ms;
        }
    }
    for day in out.iter_mut() {
        day.state_score = state_score(
            day.observed_ms,
            day.focus_ms,
            day.entertainment_ms,
            day.intervention_count,
        );
    }
    Ok(out)
}

fn state_score(observed: i64, focus: i64, entertainment: i64, interventions: i64) -> i64 {
    if observed <= 0 {
        return 50;
    }
    let focus_share = focus as f64 / observed as f64;
    let entertainment_share = entertainment as f64 / observed as f64;
    (50.0 + focus_share * 40.0 - entertainment_share * 30.0 - interventions as f64 * 3.0)
        .round()
        .clamp(0.0, 100.0) as i64
}

/// 长期计划图层：LifeDB 的目标/项目/技能跨度 + 里程碑点。
fn plan_spans(
    conn: &Connection,
    start_ms: i64,
    end_ms: i64,
    max_level: u8,
) -> Result<Vec<PlanSpan>, String> {
    let items = crate::lifedb::list_items(conn, false)?;
    // 进度用同一套确定性算法（技能树用的那套），避免两处口径不一致。
    let trees = crate::lifetree::forest(conn, &[])?;
    let mut progress = std::collections::HashMap::<String, f64>::new();
    fn collect(node: &crate::lifetree::TreeNode, out: &mut std::collections::HashMap<String, f64>) {
        out.insert(node.item.id.clone(), node.progress);
        for child in &node.children {
            collect(child, out);
        }
    }
    for t in &trees {
        collect(t, &mut progress);
    }

    let mut out = Vec::new();
    for item in items {
        let kind = match item.kind.as_str() {
            "goal" => "life_goal",
            "project" => "life_project",
            "milestone" => "life_milestone",
            "skill" => "life_skill",
            _ => continue,
        };
        let level = lod_level(kind);
        if level > max_level {
            continue;
        }
        let span_start = item.start_at_ms.unwrap_or(item.created_at);
        let span_end = item
            .due_at_ms
            .or(item.review_at_ms)
            .unwrap_or_else(|| span_start.max(item.updated_at));
        let (span_start, span_end) = if span_end < span_start {
            (span_end, span_start)
        } else {
            (span_start, span_end)
        };
        if span_start > end_ms || span_end < start_ms {
            continue;
        }
        out.push(PlanSpan {
            progress: progress.get(&item.id).copied().unwrap_or(0.0),
            id: item.id,
            kind: kind.to_string(),
            title: item.title,
            track: item.track,
            status: item.status,
            start_ms: span_start,
            end_ms: span_end,
            level,
        });
    }
    out.sort_by_key(|p| (p.level, p.start_ms));
    Ok(out)
}

fn has_life_items(conn: &Connection) -> Result<bool, String> {
    let n: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM life_items WHERE status != 'archived'",
            [],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    Ok(n > 0)
}


fn query_behavior_events(
    conn: &Connection,
    start_ms: i64,
    end_ms: i64,
    detail: &str,
    limit: i64,
) -> rusqlite::Result<Vec<TimelineEvent>> {
    let mut stmt = conn.prepare(
        "SELECT event_id, type,
                COALESCE(start_time, event_time, produced_at),
                COALESCE(end_time, start_time, event_time, produced_at),
                entity, category, payload_json
         FROM raw_events
         WHERE COALESCE(start_time, event_time, produced_at) <= ?2
           AND COALESCE(end_time, start_time, event_time, produced_at) >= ?1
         ORDER BY COALESCE(start_time, event_time, produced_at) ASC
         LIMIT ?3",
    )?;
    let mut rows = stmt
        .query_map(params![start_ms, end_ms, limit], |r| {
            let event_type: String = r.get(1)?;
            let entity: Option<String> = r.get(4)?;
            let raw_detail: String = r.get(6)?;
            Ok(TimelineEvent {
                id: r.get(0)?,
                kind: "behavior".to_string(),
                start_ms: r.get(2)?,
                end_ms: r.get(3)?,
                title: entity.unwrap_or_else(|| event_type.clone()),
                category: r.get(5)?,
                detail: if detail == "minute" && raw_detail != "{}" {
                    Some(raw_detail)
                } else {
                    Some(event_type)
                },
                severity: None,
                level: 3,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    // note_text / material_text 单独标为 capture，便于时间轴区分图层。
    for ev in rows.iter_mut() {
        let is_text = matches!(
            ev.detail.as_deref(),
            Some("note_text") | Some("material_text")
        ) || ev.title == "note_text"
            || ev.title == "material_text";
        if is_text {
            ev.kind = "capture".to_string();
        }
    }
    Ok(rows)
}

/// artifact 里程碑（点事件）：目标 / 事项 / 提醒 / 知识卡片 / 检测规则的创建。
/// 按显著性等级过滤——粗尺度只留结构性变化，不再"按表顺序截断"。
fn query_artifact_events(
    conn: &Connection,
    start_ms: i64,
    end_ms: i64,
    limit: i64,
    max_level: u8,
) -> rusqlite::Result<Vec<TimelineEvent>> {
    let queries: [(&str, &str); 4] = [
        (
            "SELECT id, created_at, raw_text FROM daily_goals WHERE created_at BETWEEN ?1 AND ?2",
            "goal",
        ),
        (
            "SELECT id, created_at, text FROM reminders WHERE created_at BETWEEN ?1 AND ?2",
            "reminder",
        ),
        (
            "SELECT id, created_at, title FROM knowledge_notes WHERE created_at BETWEEN ?1 AND ?2",
            "knowledge",
        ),
        (
            "SELECT id, created_at, name FROM detection_rules WHERE created_at BETWEEN ?1 AND ?2",
            "rule",
        ),
    ];
    let mut out = Vec::new();
    for (sql, kind) in queries {
        let level = lod_level(kind);
        if level > max_level {
            continue;
        }
        let mut stmt = conn.prepare(sql)?;
        let rows = stmt.query_map(params![start_ms, end_ms], |r| {
            let at: i64 = r.get(1)?;
            Ok(TimelineEvent {
                id: r.get(0)?,
                kind: kind.to_string(),
                start_ms: at,
                end_ms: at,
                title: r.get(2)?,
                category: Some("milestone".to_string()),
                detail: Some(kind.to_string()),
                severity: None,
                level,
            })
        })?;
        for row in rows {
            out.push(row?);
            if out.len() as i64 >= limit {
                return Ok(out);
            }
        }
    }
    Ok(out)
}

fn query_intervention_events(
    conn: &Connection,
    start_ms: i64,
    end_ms: i64,
    limit: i64,
) -> rusqlite::Result<Vec<TimelineEvent>> {
    let mut stmt = conn.prepare(
        "SELECT id, shown_at, intensity, message, rule_id, outcome
         FROM interventions
         WHERE shown_at BETWEEN ?1 AND ?2
         ORDER BY shown_at ASC LIMIT ?3",
    )?;
    let rows = stmt
        .query_map(params![start_ms, end_ms, limit], |r| {
            let at: i64 = r.get(1)?;
            let rule_id: String = r.get(4)?;
            let outcome: Option<String> = r.get(5)?;
            Ok(TimelineEvent {
                id: r.get(0)?,
                kind: "intervention".to_string(),
                start_ms: at,
                end_ms: at,
                title: r.get(3)?,
                category: Some("interaction".to_string()),
                // 近端结果直接挂在事件上：时间线能看出"哪次提醒真的起作用了"。
                detail: Some(match outcome {
                    Some(o) => format!("{rule_id} · {o}"),
                    None => rule_id,
                }),
                severity: r.get(2)?,
                level: 2,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// 最近的干预记录（倒序）。
pub fn list_interventions(conn: &Connection, limit: i64) -> rusqlite::Result<Vec<InterventionRow>> {
    let mut stmt = conn.prepare(
        "SELECT id, rule_id, shown_at, intensity, message, user_response, responded_at, outcome
         FROM interventions ORDER BY shown_at DESC LIMIT ?1",
    )?;
    let rows = stmt
        .query_map(params![limit], |r| {
            Ok(InterventionRow {
                id: r.get(0)?,
                rule_id: r.get(1)?,
                shown_at: r.get(2)?,
                severity: r.get(3)?,
                message: r.get(4)?,
                user_response: r.get(5)?,
                responded_at: r.get(6)?,
                outcome: r.get(7)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}
