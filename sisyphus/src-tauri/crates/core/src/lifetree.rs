//! LifeDB 的树/图读模型：**技能树**、确定性的**下一步选择**、**周回顾队列**。
//!
//! 三件事都刻意放在 Core 而不是交给 agent 每次自由发挥：
//! - 进度、下一步、该回顾什么，必须**可复现、可解释、可测试**；agent 每次算一遍既不稳定也无法调试。
//! - agent 的活是"生成里程碑文本"和"措辞"，不是"算 76 个节点的进度"。
//!
//! 技能树的形状完全由现有结构承载，不引入新表：
//! ```text
//! skill（能力节点）
//!   ├─ contains  → milestone（等级/阶段，可判定）
//!   │                └─ contains → action（具体要做的事）
//!   └─ depends_on → skill（前置能力，构成 DAG）
//! ```

use rusqlite::{params, Connection};
use serde::Serialize;

use crate::clock;
use crate::lifedb::{self, LifeItem};

/// 树上一个节点（含确定性算出的进度）。
#[derive(Debug, Clone, Serialize)]
pub struct TreeNode {
    pub item: LifeItem,
    pub depth: usize,
    /// 0.0–1.0。叶子看 status/度量，内部节点是子节点的等权平均。
    pub progress: f64,
    /// 已完成的叶子数 / 叶子总数（给 UI 显示 "3/7"）。
    pub done_leaves: usize,
    pub total_leaves: usize,
    /// 前置节点 id（`depends_on` 边）。UI 画 DAG 用。
    pub depends_on: Vec<String>,
    pub children: Vec<TreeNode>,
}

const HIERARCHY: &[&str] = &["contains", "supports"];
pub const MAX_DEPTH: usize = 8;

fn is_closed(status: &str) -> bool {
    matches!(status, "done" | "archived")
}

/// 叶子进度：完成=1；有度量则按 current/target；否则 0（不给"在做"送分，保持诚实）。
fn leaf_progress(item: &LifeItem) -> f64 {
    if item.status == "done" {
        return 1.0;
    }
    match (item.target_value, item.current_value) {
        (Some(t), Some(c)) if t > 0.0 => (c / t).clamp(0.0, 1.0),
        _ => 0.0,
    }
}

/// 取所有非归档 item + 层级/依赖边，在内存里建树（个人库规模下代价可忽略）。
fn load_graph(
    conn: &Connection,
    include_archived: bool,
) -> Result<(Vec<LifeItem>, Vec<(String, String, String)>), String> {
    let items = lifedb::list_items(conn, include_archived)?;
    let mut stmt = conn
        .prepare(
            "SELECT from_item_id,to_item_id,relation FROM life_item_edges
             ORDER BY relation, sort_order, created_at",
        )
        .map_err(|e| e.to_string())?;
    let edges = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    Ok((items, edges))
}

struct Graph {
    items: std::collections::HashMap<String, LifeItem>,
    children: std::collections::HashMap<String, Vec<String>>,
    parents: std::collections::HashMap<String, Vec<String>>,
    depends: std::collections::HashMap<String, Vec<String>>,
    order: Vec<String>,
}

impl Graph {
    fn build(items: Vec<LifeItem>, edges: Vec<(String, String, String)>) -> Self {
        let order: Vec<String> = items.iter().map(|i| i.id.clone()).collect();
        let items: std::collections::HashMap<String, LifeItem> =
            items.into_iter().map(|i| (i.id.clone(), i)).collect();
        let mut children: std::collections::HashMap<String, Vec<String>> = Default::default();
        let mut parents: std::collections::HashMap<String, Vec<String>> = Default::default();
        let mut depends: std::collections::HashMap<String, Vec<String>> = Default::default();
        for (from, to, relation) in edges {
            if !items.contains_key(&from) || !items.contains_key(&to) {
                continue;
            }
            if HIERARCHY.contains(&relation.as_str()) {
                children.entry(from.clone()).or_default().push(to.clone());
                parents.entry(to).or_default().push(from);
            } else if relation == "depends_on" {
                depends.entry(from).or_default().push(to);
            }
        }
        Graph {
            items,
            children,
            parents,
            depends,
            order,
        }
    }

    fn node(&self, id: &str, depth: usize, seen: &mut Vec<String>) -> Option<TreeNode> {
        let item = self.items.get(id)?.clone();
        if depth >= MAX_DEPTH || seen.contains(&item.id) {
            return Some(TreeNode {
                progress: leaf_progress(&item),
                done_leaves: usize::from(item.status == "done"),
                total_leaves: 1,
                depends_on: self.depends.get(id).cloned().unwrap_or_default(),
                depth,
                item,
                children: Vec::new(),
            });
        }
        seen.push(item.id.clone());
        let mut children = Vec::new();
        for child_id in self.children.get(id).map(|v| v.as_slice()).unwrap_or(&[]) {
            if let Some(child) = self.node(child_id, depth + 1, seen) {
                children.push(child);
            }
        }
        seen.pop();

        let (progress, done_leaves, total_leaves) = if children.is_empty() {
            (
                leaf_progress(&item),
                usize::from(item.status == "done"),
                1usize,
            )
        } else {
            let sum: f64 = children.iter().map(|c| c.progress).sum();
            let done = children.iter().map(|c| c.done_leaves).sum();
            let total = children.iter().map(|c| c.total_leaves).sum();
            (sum / children.len() as f64, done, total)
        };
        Some(TreeNode {
            progress,
            done_leaves,
            total_leaves,
            depends_on: self.depends.get(id).cloned().unwrap_or_default(),
            depth,
            item,
            children,
        })
    }

    fn is_root(&self, id: &str) -> bool {
        self.parents.get(id).map(|p| p.is_empty()).unwrap_or(true)
    }
}

/// 某个节点的子树（技能树的一根分支 / 一个目标的分解）。
pub fn subtree(conn: &Connection, root_id: &str) -> Result<TreeNode, String> {
    let (items, edges) = load_graph(conn, false)?;
    let graph = Graph::build(items, edges);
    graph
        .node(root_id, 0, &mut Vec::new())
        .ok_or_else(|| format!("LifeItem 不存在或已归档: {root_id}"))
}

/// 森林：指定 kind 的所有根节点及其子树。`kinds` 为空表示不过滤 kind。
///
/// 技能树视图 = `forest(conn, &["skill"])`；目标分解视图 = `forest(conn, &["goal"])`。
pub fn forest(conn: &Connection, kinds: &[&str]) -> Result<Vec<TreeNode>, String> {
    let (items, edges) = load_graph(conn, false)?;
    let graph = Graph::build(items, edges);
    let mut out = Vec::new();
    for id in &graph.order {
        let Some(item) = graph.items.get(id) else {
            continue;
        };
        if !kinds.is_empty() && !kinds.contains(&item.kind.as_str()) {
            continue;
        }
        if !graph.is_root(id) {
            continue;
        }
        if let Some(node) = graph.node(id, 0, &mut Vec::new()) {
            out.push(node);
        }
    }
    Ok(out)
}

/// 一个节点的确定性进度。
#[derive(Debug, Clone, Copy, Serialize)]
pub struct Progress {
    pub progress: f64,
    pub done_leaves: usize,
    pub total_leaves: usize,
}

/// 全库进度索引：id → 进度。**与 [`forest`] 完全同源**（两者都走 [`Graph::node`]），
/// 所以技能树、时间线的 `plans` 图层与时间播放不可能给出互相矛盾的数字。
///
/// 传入的 `items` / `edges` 可以是"某个历史时刻的样子"（由 `skillmap` 按进度账本覆盖过），
/// 于是回放与"现在"共用同一套算法，而不是第二套估算。
pub fn progress_index(
    items: Vec<LifeItem>,
    edges: Vec<(String, String, String)>,
) -> std::collections::HashMap<String, Progress> {
    fn collect(node: &TreeNode, out: &mut std::collections::HashMap<String, Progress>) {
        out.insert(
            node.item.id.clone(),
            Progress {
                progress: node.progress,
                done_leaves: node.done_leaves,
                total_leaves: node.total_leaves,
            },
        );
        for child in &node.children {
            collect(child, out);
        }
    }
    let graph = Graph::build(items, edges);
    let mut out = std::collections::HashMap::new();
    for id in &graph.order {
        if !graph.is_root(id) {
            continue;
        }
        if let Some(node) = graph.node(id, 0, &mut Vec::new()) {
            collect(&node, &mut out);
        }
    }
    // 纯环（每个节点都有父）没有根，兜底单独算一遍——保证每个 item 都有数，不会静默缺项。
    for id in &graph.order {
        if out.contains_key(id) {
            continue;
        }
        if let Some(node) = graph.node(id, 0, &mut Vec::new()) {
            collect(&node, &mut out);
        }
    }
    out
}

// ── 下一步选择（确定性 + 可解释）────────────────────────────────────────────
#[derive(Debug, Clone, Serialize)]
pub struct NextAction {
    pub item_id: String,
    pub title: String,
    pub kind: String,
    pub track: String,
    pub due_at_ms: Option<i64>,
    /// 为什么是它——UI/agent 必须能把理由讲给用户听。
    pub reason: String,
}

/// 今日最小行动（1–3 条）。**规则化**，不是"取前 N 个未完成任务"。
///
/// 优先级从上到下，命中即取，去重后截断：
/// 1. 已逾期的事项/里程碑
/// 2. 已排在"当下"（horizon=now）且在做
/// 3. 三天内到期
/// 4. 重点领域或主线目标下最浅的未完成事项
/// 5. 今日日常
/// 6. horizon=next 里最久没动的
pub fn next_actions(conn: &Connection, limit: usize) -> Result<Vec<NextAction>, String> {
    let now = clock::now_ms();
    let (items, edges) = load_graph(conn, false)?;
    let graph = Graph::build(items, edges);
    let focus_areas: Vec<String> = lifedb::list_areas(conn)?
        .into_iter()
        .filter(|a| a.focus)
        .map(|a| a.id)
        .collect();

    let mut picked: Vec<NextAction> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    let push = |picked: &mut Vec<NextAction>,
                    seen: &mut Vec<String>,
                    item: &LifeItem,
                    reason: &str| {
        if seen.contains(&item.id) || picked.len() >= limit {
            return;
        }
        seen.push(item.id.clone());
        picked.push(NextAction {
            item_id: item.id.clone(),
            title: item.title.clone(),
            kind: item.kind.clone(),
            track: item.track.clone(),
            due_at_ms: item.due_at_ms,
            reason: reason.to_string(),
        });
    };

    let open: Vec<&LifeItem> = graph
        .order
        .iter()
        .filter_map(|id| graph.items.get(id))
        .filter(|i| !is_closed(&i.status))
        .collect();
    let actionable = |i: &LifeItem| matches!(i.kind.as_str(), "action" | "milestone" | "routine");

    // 1. 逾期
    let mut overdue: Vec<&&LifeItem> = open
        .iter()
        .filter(|i| actionable(i) && i.due_at_ms.is_some_and(|d| d < now))
        .collect();
    overdue.sort_by_key(|i| i.due_at_ms);
    for i in overdue {
        push(&mut picked, &mut seen, i, "已逾期");
    }

    // 2. 已排在当下且在做
    for i in open
        .iter()
        .filter(|i| actionable(i) && i.horizon == "now" && i.status == "active")
    {
        push(&mut picked, &mut seen, i, "已排在当下");
    }

    // 3. 三天内到期
    let soon = now + 3 * 86_400_000;
    let mut upcoming: Vec<&&LifeItem> = open
        .iter()
        .filter(|i| actionable(i) && i.due_at_ms.is_some_and(|d| d >= now && d <= soon))
        .collect();
    upcoming.sort_by_key(|i| i.due_at_ms);
    for i in upcoming {
        push(&mut picked, &mut seen, i, "临近截止");
    }

    // 4. 重点领域 / 主线目标下最浅的未完成事项（广度优先 → "最浅"）
    let roots: Vec<&&LifeItem> = open
        .iter()
        .filter(|i| {
            matches!(i.kind.as_str(), "goal" | "project" | "skill")
                && (i.track == "main"
                    || i.area_id
                        .as_ref()
                        .is_some_and(|a| focus_areas.contains(a)))
        })
        .collect();
    for root in roots {
        let mut queue: Vec<(String, usize)> = vec![(root.id.clone(), 0)];
        let mut visited: Vec<String> = Vec::new();
        while let Some((id, depth)) = queue.first().cloned() {
            queue.remove(0);
            if depth > MAX_DEPTH || visited.contains(&id) {
                continue;
            }
            visited.push(id.clone());
            if let Some(item) = graph.items.get(&id) {
                if actionable(item) && !is_closed(&item.status) {
                    let reason = if root.track == "main" {
                        format!("主线「{}」的下一步", root.title)
                    } else {
                        format!("重点领域下「{}」的下一步", root.title)
                    };
                    push(&mut picked, &mut seen, item, &reason);
                }
            }
            for child in graph.children.get(&id).map(|v| v.as_slice()).unwrap_or(&[]) {
                queue.push((child.clone(), depth + 1));
            }
        }
    }

    // 5. 今日日常
    for i in open.iter().filter(|i| {
        i.kind == "routine"
            && (i.horizon == "now"
                || i.recurrence
                    .as_deref()
                    .is_some_and(|r| r.contains("daily") || r.contains("每天")))
    }) {
        push(&mut picked, &mut seen, i, "今日日常");
    }

    // 6. 兜底：horizon=next 里最久没动的
    let mut next_up: Vec<&&LifeItem> = open
        .iter()
        .filter(|i| actionable(i) && i.horizon == "next")
        .collect();
    next_up.sort_by_key(|i| i.updated_at);
    for i in next_up {
        push(&mut picked, &mut seen, i, "候选下一步");
    }

    // 7. 最终兜底：任何未完成的可执行事项（刚记下、还没排期的也必须能被看见）。
    // 呼应"不得因过滤而丢失"——没有元数据的新事项不该在今日视图里凭空消失。
    let mut rest: Vec<&&LifeItem> = open.iter().filter(|i| actionable(i)).collect();
    rest.sort_by_key(|i| i.created_at);
    for i in rest {
        push(&mut picked, &mut seen, i, "待安排");
    }

    Ok(picked)
}

// ── 周回顾队列 ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct ReviewItem {
    pub item_id: String,
    pub title: String,
    pub kind: String,
    pub question: String,
}

/// 周回顾要问的问题（GTD weekly review 的确定性部分）。
///
/// 让"手动维护"变成"每周回答几个二选一"：这些判断由 SQL/Rust 算出来交给 agent 提问，
/// **不是**让 agent 自己在上下文里数一遍。
#[derive(Debug, Clone, Serialize)]
pub struct ReviewQueue {
    /// review_at 已到期的（idea 的毕业审查）。
    pub due_review: Vec<ReviewItem>,
    /// 有未完成子项但自己长期没动的目标/项目。
    pub stalled: Vec<ReviewItem>,
    /// 完全没有下层行动的目标/技能（"想了但没拆"）。
    pub undecomposed: Vec<ReviewItem>,
    /// 缺可判定完成条件的目标/里程碑（永远无法收敛）。
    pub no_success_criteria: Vec<ReviewItem>,
    /// 长期停留在 inbox 的想法。
    pub stale_inbox: Vec<ReviewItem>,
}

pub fn review_queue(conn: &Connection, idle_days: i64) -> Result<ReviewQueue, String> {
    let now = clock::now_ms();
    let idle_before = now - idle_days.max(1) * 86_400_000;
    let (items, edges) = load_graph(conn, false)?;
    let graph = Graph::build(items, edges);

    let mut q = ReviewQueue {
        due_review: Vec::new(),
        stalled: Vec::new(),
        undecomposed: Vec::new(),
        no_success_criteria: Vec::new(),
        stale_inbox: Vec::new(),
    };
    let mk = |item: &LifeItem, question: &str| ReviewItem {
        item_id: item.id.clone(),
        title: item.title.clone(),
        kind: item.kind.clone(),
        question: question.to_string(),
    };

    for id in &graph.order {
        let Some(item) = graph.items.get(id) else {
            continue;
        };
        if is_closed(&item.status) {
            continue;
        }
        if item.review_at_ms.is_some_and(|r| r <= now) {
            q.due_review
                .push(mk(item, "该升级成项目、降为someday，还是归档？"));
        }
        let has_children = graph
            .children
            .get(id)
            .map(|c| !c.is_empty())
            .unwrap_or(false);
        let long_term = matches!(item.kind.as_str(), "goal" | "project" | "skill");
        if long_term && !has_children {
            q.undecomposed
                .push(mk(item, "拆一个最小的下一步出来？"));
        }
        if long_term && has_children && item.updated_at < idle_before {
            q.stalled.push(mk(
                item,
                "这周没有任何推进——还要它吗，还是先放回 someday？",
            ));
        }
        if matches!(item.kind.as_str(), "goal" | "milestone")
            && item
                .success_criteria
                .as_deref()
                .map(|s| s.trim().is_empty())
                .unwrap_or(true)
        {
            q.no_success_criteria
                .push(mk(item, "怎样算完成？给一句可判定的标准。"));
        }
        if item.kind == "idea" && item.status == "inbox" && item.created_at < idle_before {
            q.stale_inbox.push(mk(item, "还想做吗？升级 / someday / 归档"));
        }
    }
    Ok(q)
}

/// 给 `idea` 排一个毕业审查时间（默认 7 天后）。到期由周回顾提出三选一。
pub fn schedule_review(conn: &Connection, item_id: &str, after_days: i64) -> Result<(), String> {
    let at = clock::now_ms() + after_days.max(1) * 86_400_000;
    conn.execute(
        "UPDATE life_items SET review_at_ms=?2, updated_at=?3 WHERE id=?1",
        params![item_id, at, clock::now_ms()],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use crate::lifedb::LifeItemInput;

    fn item(conn: &Connection, kind: &str, title: &str) -> String {
        lifedb::upsert_item(
            conn,
            LifeItemInput {
                id: None,
                expected_revision: None,
                kind: kind.into(),
                title: title.into(),
                body: String::new(),
                track: "undecided".into(),
                horizon: "unscheduled".into(),
                status: "inbox".into(),
                area_id: None,
                success_criteria: None,
                target_value: None,
                current_value: None,
                unit: None,
                start_at_ms: None,
                due_at_ms: None,
                review_at_ms: None,
                recurrence: None,
                source_event_id: None,
                intent_id: None,
                origin: "app".into(),
                external_ref: None,
            },
        )
        .unwrap()
    }

    fn set(conn: &Connection, id: &str, sql_fragment: &str) {
        conn.execute(
            &format!("UPDATE life_items SET {sql_fragment} WHERE id=?1"),
            params![id],
        )
        .unwrap();
    }

    #[test]
    fn skill_tree_progress_rolls_up_from_milestones() {
        let conn = db::open(":memory:").unwrap();
        let skill = item(&conn, "skill", "Rust");
        let m1 = item(&conn, "milestone", "会写 trait");
        let m2 = item(&conn, "milestone", "会写 async");
        lifedb::link_items(&conn, &skill, &m1, "contains", 0, "app").unwrap();
        lifedb::link_items(&conn, &skill, &m2, "contains", 1, "app").unwrap();
        set(&conn, &m1, "status='done'");

        let trees = forest(&conn, &["skill"]).unwrap();
        assert_eq!(trees.len(), 1);
        let root = &trees[0];
        assert_eq!(root.item.title, "Rust");
        assert_eq!(root.children.len(), 2);
        assert!((root.progress - 0.5).abs() < 1e-9, "两个里程碑完成一个 = 50%");
        assert_eq!((root.done_leaves, root.total_leaves), (1, 2));
    }

    #[test]
    fn metric_milestone_contributes_partial_progress() {
        let conn = db::open(":memory:").unwrap();
        let skill = item(&conn, "skill", "英语");
        let m = item(&conn, "milestone", "雅思 7 分");
        lifedb::link_items(&conn, &skill, &m, "contains", 0, "app").unwrap();
        set(&conn, &m, "target_value=7.0, current_value=5.25");
        let trees = forest(&conn, &["skill"]).unwrap();
        assert!((trees[0].progress - 0.75).abs() < 1e-9);
    }

    #[test]
    fn depends_on_is_exposed_but_not_treated_as_hierarchy() {
        let conn = db::open(":memory:").unwrap();
        let a = item(&conn, "skill", "所有权");
        let b = item(&conn, "skill", "async");
        lifedb::link_items(&conn, &b, &a, "depends_on", 0, "app").unwrap();
        let trees = forest(&conn, &["skill"]).unwrap();
        // 前置边不构成父子，两者都是根。
        assert_eq!(trees.len(), 2);
        let async_node = trees.iter().find(|t| t.item.title == "async").unwrap();
        assert_eq!(async_node.depends_on, vec![a]);
    }

    #[test]
    fn cycle_in_edges_does_not_hang() {
        let conn = db::open(":memory:").unwrap();
        let a = item(&conn, "goal", "A");
        let b = item(&conn, "project", "B");
        lifedb::link_items(&conn, &a, &b, "contains", 0, "app").unwrap();
        lifedb::link_items(&conn, &b, &a, "contains", 0, "app").unwrap();
        let node = subtree(&conn, &a).unwrap();
        assert_eq!(node.item.title, "A");
    }

    #[test]
    fn next_actions_prioritises_overdue_then_main_line() {
        let conn = db::open(":memory:").unwrap();
        let now = clock::now_ms();
        let late = item(&conn, "action", "交报告");
        set(
            &conn,
            &late,
            &format!("due_at_ms={}, status='active'", now - 86_400_000),
        );
        let goal = item(&conn, "goal", "上线 LifeIndex");
        set(&conn, &goal, "track='main', status='active'");
        let step = item(&conn, "action", "写迁移");
        set(&conn, &step, "status='active'");
        lifedb::link_items(&conn, &goal, &step, "contains", 0, "app").unwrap();
        let noise = item(&conn, "action", "无关杂事");
        set(&conn, &noise, "horizon='next'");

        let picks = next_actions(&conn, 3).unwrap();
        assert_eq!(picks[0].title, "交报告");
        assert_eq!(picks[0].reason, "已逾期");
        assert_eq!(picks[1].title, "写迁移");
        assert!(picks[1].reason.contains("主线"));
        assert!(picks.len() <= 3);
    }

    #[test]
    fn review_queue_flags_undecomposed_and_missing_criteria() {
        let conn = db::open(":memory:").unwrap();
        let goal = item(&conn, "goal", "学好英语");
        set(&conn, &goal, "status='active'");
        let q = review_queue(&conn, 7).unwrap();
        assert!(q.undecomposed.iter().any(|r| r.item_id == goal));
        assert!(q.no_success_criteria.iter().any(|r| r.item_id == goal));
    }

    #[test]
    fn review_queue_flags_due_review_ideas() {
        let conn = db::open(":memory:").unwrap();
        let idea = item(&conn, "idea", "也许学吉他");
        schedule_review(&conn, &idea, 1).unwrap();
        set(
            &conn,
            &idea,
            &format!("review_at_ms={}", clock::now_ms() - 1000),
        );
        let q = review_queue(&conn, 7).unwrap();
        assert!(q.due_review.iter().any(|r| r.item_id == idea));
    }
}
