//! 技能树读模型：把 LifeDB 的图投影成一张**人生能力地图**（背景扇区 + 技能点 + 前置边）。
//!
//! 概念到图元的映射是这个模块的全部要点。四个概念在 LifeDB 里各有其位，
//! **图元角色必须不同**，否则地图会退化成思维导图：
//!
//! | 概念 | 存储 | 图元 | 为什么 |
//! |---|---|---|---|
//! | 领域 | `life_areas` | **背景扇区**，永不是节点 | 领域没有完成态，只需维持标准（GTD Horizon 3）。画成节点它永远填不满，进度就失去意义 |
//! | 技能 | `kind=skill` | 技能点（大节点） | 唯一"拥有/未拥有"可判定的能力单位 |
//! | 里程碑 | `kind=milestone` | 父技能环上的刻度 | 它是等级刻度不是并列节点；`contains` 是组成关系，画成"属于"而不是画成边 |
//! | 想法 | `kind=idea` | 环外星尘（无边） | 还没决定要不要变成能力；看它向内迁移＝树在生长 |
//! | 事实进展 | `progress`/度量/子 action | **不是图元**：是填充、刻度亮数、度量读数与时间播放 | 把进展画成节点＝地图变成任务清单 |
//!
//! 三条不变量（与无极时间轴同源，见 `docs/spec/lifeindex-mind-system.md`）：
//!
//! 1. **几何算术归 Core**。扇区角区间、依赖深度、节点状态、领域掌握度都在这里算完；
//!    前端只做投影（极坐标 → 屏幕）与绘制，不做任何依赖 DB 语义的算术。
//! 2. **雷达 = 换投影**。同一份 `(sector_angle, depth, mastery)` 既能画成树（半径=依赖深度）
//!    也能画成雷达（半径=掌握度），前端在两者之间插值，不是两条渲染路径。
//! 3. **位置 = 结构维度，颜色 = 属性**。扇区只能是 `area`（用户显式声明的责任结构）；
//!    `track`（主线/支线）与 `status` 是属性，只能给颜色/亮度，**不许各占一个扇区**。
//!
//! 进度只有一个来源：[`crate::lifetree::progress_index`]。本模块不重新实现任何进度计算。

use rusqlite::Connection;
use serde::Serialize;
use std::collections::{BTreeMap, HashMap, HashSet};

use crate::clock;
use crate::lifedb::{self, LifeItem};
use crate::lifetree::{self, MAX_DEPTH};

/// 扇区起始角。`-π/2` 让第一个领域落在正上方（12 点方向），且完全确定。
const START_ANGLE: f64 = -std::f64::consts::FRAC_PI_2;
const TAU: f64 = std::f64::consts::TAU;

/// 节点四态。**不引入任何阈值魔数**——全部由进度与前置是否达成推出。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeState {
    /// 已掌握：进度满（含 status=done）。
    Attained,
    /// 在进展：0 < 进度 < 1。
    InProgress,
    /// 可解锁：还没开始，但所有前置已达成。
    Available,
    /// 锁定：还没开始，且有前置没达成（`blocked_by` 说得出是谁）。
    Locked,
}

/// 背景扇区 = 一个责任领域。`area_id=None` 是"未归属"兜底扇区。
#[derive(Debug, Clone, Serialize)]
pub struct SkillSector {
    pub index: usize,
    pub area_id: Option<String>,
    pub name: String,
    pub focus: bool,
    /// 扇区角区间（弧度，顺时针递增）。前端不算角度。
    pub start_angle: f64,
    pub end_angle: f64,
    /// 该领域**技能**进度的等权平均（雷达顶点半径）。里程碑不重复计入。
    pub mastery: f64,
    pub attained: usize,
    pub total: usize,
    /// 本扇区占了几个角度槽位（= 环上节点数）。前端按 `slot/slots` 分角，不自己数。
    pub slots: usize,
}

/// 技能点（`skill`）或它环上的刻度（`milestone`）。
#[derive(Debug, Clone, Serialize)]
pub struct SkillNode {
    pub id: String,
    /// skill | milestone
    pub kind: String,
    pub title: String,
    pub area_id: Option<String>,
    pub track: String,
    pub status: String,
    /// 层级父节点（`contains`/`supports`）。里程碑的父技能。
    pub parent_id: Option<String>,
    /// 前置节点 id（`depends_on`，已把 `blocks` 规范化进来）。
    pub depends_on: Vec<String>,
    /// 其中尚未达成的那些——UI 要说得出"需先完成 X"。
    pub blocked_by: Vec<String>,
    pub state: NodeState,
    /// 0–1，来自 `lifetree`，不是估的。
    pub progress: f64,
    pub done_leaves: usize,
    pub total_leaves: usize,
    /// 依赖深度：无前置=0，否则 1+max(前置深度)。里程碑继承父技能深度。
    pub depth: usize,
    pub sector: usize,
    /// 角度槽位。有父技能的里程碑是**父节点周围的卫星序号**，其余节点是**扇区环上的序号**。
    /// 一律按 `(created_at, id)` 排——**不能用 `updated_at`**，否则每次编辑就换位置，
    /// 用户的空间记忆会被毁掉。
    pub slot: usize,
    /// 同一环（扇区或父节点）上的槽位总数，前端用 `slot/slot_count` 分角。
    pub slot_count: usize,
    pub created_at: i64,
    pub due_at_ms: Option<i64>,
    pub success_criteria: Option<String>,
    pub target_value: Option<f64>,
    pub current_value: Option<f64>,
    pub unit: Option<String>,
    /// 挂在这个技能上的目标/项目数（它们是产出不是能力，不进画布，只作角标）。
    pub goal_count: usize,
}

/// 前置边。层级边不进这里（它由 `parent_id` 表达）。
#[derive(Debug, Clone, Serialize)]
pub struct SkillEdge {
    /// 依赖者
    pub from: String,
    /// 前置
    pub to: String,
    pub satisfied: bool,
}

/// 环外星尘：还没决定要不要变成能力的想法。
#[derive(Debug, Clone, Serialize)]
pub struct IdeaMote {
    pub id: String,
    pub title: String,
    pub area_id: Option<String>,
    pub sector: usize,
    pub created_at: i64,
    pub review_at_ms: Option<i64>,
    /// 毕业审查已到期（周回顾会问它）。
    pub due_review: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct SkillMap {
    /// 本次快照对应的时刻（`at_ms=None` 时是 `now`）。
    pub at_ms: i64,
    pub sectors: Vec<SkillSector>,
    pub nodes: Vec<SkillNode>,
    pub edges: Vec<SkillEdge>,
    pub ideas: Vec<IdeaMote>,
    pub max_depth: usize,
    /// 已掌握技能数 / 技能总数（全局 "Lv"）。
    pub attained: usize,
    pub total: usize,
}

/// 一次进度变化（时间播放用，delta 编码：只在进度真的变了时才有一条）。
#[derive(Debug, Clone, Serialize)]
pub struct ProgressChange {
    pub item_id: String,
    pub at_ms: i64,
    pub progress: f64,
    pub done_leaves: usize,
    pub total_leaves: usize,
    pub state: NodeState,
}

/// 一次领域掌握度变化（雷达多边形的顶点动画）。
#[derive(Debug, Clone, Serialize)]
pub struct SectorChange {
    pub sector: usize,
    pub at_ms: i64,
    pub mastery: f64,
}

/// 生长史。舞台（节点身份与位置）来自 `skill_map(None)`，**播放期间位置绝不移动**，
/// 只有填充、刻度与状态在变——所以已归档的旧节点不出现在回放里，
/// 回放回答的是"**现在这棵树**是怎么长成的"。
#[derive(Debug, Clone, Serialize)]
pub struct Growth {
    pub from_ms: i64,
    pub to_ms: i64,
    /// 所有发生过变化的时刻（升序、去重）。
    pub instants: Vec<i64>,
    pub changes: Vec<ProgressChange>,
    pub sectors: Vec<SectorChange>,
}

/// 某个时刻的可变字段快照（进度账本折叠出来的）。
#[derive(Debug, Clone)]
struct Effective {
    status: String,
    current_value: Option<f64>,
    target_value: Option<f64>,
}

/// 把账本折叠成 "T 时刻每个 item 的状态"。没有账本行的 item 保留库里的当前值
/// （迁移已为老库回填两个锚点，所以这条兜底正常走不到；走到了也不假装知道变更时刻）。
fn effective_at(conn: &Connection, at_ms: i64) -> Result<HashMap<String, Effective>, String> {
    let mut out: HashMap<String, Effective> = HashMap::new();
    for point in lifedb::list_progress(conn, Some(at_ms))? {
        out.insert(
            point.item_id,
            Effective {
                status: point.status,
                current_value: point.current_value,
                target_value: point.target_value,
            },
        );
    }
    Ok(out)
}

/// 载入 T 时刻的 items 与边。`at_ms=None` 表示"现在"，此时直接用库里的值、不读账本。
fn load_at(
    conn: &Connection,
    at_ms: Option<i64>,
) -> Result<(Vec<LifeItem>, Vec<(String, String, String)>), String> {
    // 一律带上 archived：历史时刻里它们可能还活着，由下面的有效状态决定去留。
    let mut items = lifedb::list_items(conn, true)?;
    if let Some(t) = at_ms {
        let effective = effective_at(conn, t)?;
        items.retain(|item| item.created_at <= t);
        for item in items.iter_mut() {
            if let Some(e) = effective.get(&item.id) {
                item.status = e.status.clone();
                item.current_value = e.current_value;
                item.target_value = e.target_value;
            }
        }
    }
    items.retain(|item| item.status != "archived");
    let alive: HashSet<String> = items.iter().map(|i| i.id.clone()).collect();
    let cutoff = at_ms.unwrap_or(i64::MAX);
    let edges = lifedb::list_edges(conn)?
        .into_iter()
        .filter(|e| e.created_at <= cutoff)
        .filter(|e| alive.contains(&e.from_item_id) && alive.contains(&e.to_item_id))
        .map(|e| (e.from_item_id, e.to_item_id, e.relation))
        .collect();
    Ok((items, edges))
}

/// 依赖关系（前置）。`A blocks B` ≡ `B depends_on A`，在这里规范化，UI 与 agent 不必两套逻辑。
fn dependency_map(edges: &[(String, String, String)]) -> HashMap<String, Vec<String>> {
    let mut deps: HashMap<String, Vec<String>> = HashMap::new();
    for (from, to, relation) in edges {
        match relation.as_str() {
            "depends_on" => deps.entry(from.clone()).or_default().push(to.clone()),
            "blocks" => deps.entry(to.clone()).or_default().push(from.clone()),
            _ => {}
        }
    }
    for list in deps.values_mut() {
        list.sort();
        list.dedup();
    }
    deps
}

/// 层级父节点（`contains`/`supports`）：`to` 的父是 `from`，附带边上的 `sort_order`。
fn parent_map(edges: &[(String, String, String)]) -> HashMap<String, String> {
    let mut parents = HashMap::new();
    for (from, to, relation) in edges {
        if matches!(relation.as_str(), "contains" | "supports") {
            parents.entry(to.clone()).or_insert_with(|| from.clone());
        }
    }
    parents
}

/// 依赖深度。环安全：递归带 `seen` 栈与 `MAX_DEPTH`（与 `lifetree` 同一个上限）。
fn depth_of(
    id: &str,
    deps: &HashMap<String, Vec<String>>,
    known: &HashSet<String>,
    memo: &mut HashMap<String, usize>,
    seen: &mut Vec<String>,
) -> usize {
    if let Some(d) = memo.get(id) {
        return *d;
    }
    if seen.len() >= MAX_DEPTH || seen.iter().any(|s| s == id) {
        return 0;
    }
    seen.push(id.to_string());
    let depth = deps
        .get(id)
        .map(|list| {
            list.iter()
                .filter(|p| known.contains(p.as_str()))
                .map(|p| 1 + depth_of(p, deps, known, memo, seen))
                .max()
                .unwrap_or(0)
        })
        .unwrap_or(0);
    seen.pop();
    memo.insert(id.to_string(), depth);
    depth
}

fn state_of(progress: f64, blocked: bool) -> NodeState {
    if progress >= 1.0 {
        NodeState::Attained
    } else if progress > 0.0 {
        NodeState::InProgress
    } else if blocked {
        NodeState::Locked
    } else {
        NodeState::Available
    }
}

/// 技能地图。`at_ms=None` 是"现在"，`Some(T)` 是 T 时刻的样子（走进度账本）。
pub fn skill_map(conn: &Connection, at_ms: Option<i64>) -> Result<SkillMap, String> {
    let now = clock::now_ms();
    let stamp = at_ms.unwrap_or(now);
    let (items, edges) = load_at(conn, at_ms)?;
    let progress = lifetree::progress_index(items.clone(), edges.clone());
    let deps = dependency_map(&edges);
    let parents = parent_map(&edges);

    // ── 扇区：按 (sort_order, name) 排，**不按 focus 排**（focus 一变就换位置会毁掉空间记忆）。
    let mut areas = lifedb::list_areas(conn)?;
    areas.sort_by(|a, b| {
        a.sort_order
            .cmp(&b.sort_order)
            .then_with(|| a.name.cmp(&b.name))
    });

    let is_skill = |item: &LifeItem| item.kind == "skill";
    let on_map = |item: &LifeItem| matches!(item.kind.as_str(), "skill" | "milestone");
    let skills: Vec<&LifeItem> = items.iter().filter(|i| is_skill(i)).collect();
    let needs_unassigned = items
        .iter()
        .any(|i| (on_map(i) || i.kind == "idea") && i.area_id.is_none());

    let mut sector_of_area: HashMap<String, usize> = HashMap::new();
    let mut sectors: Vec<SkillSector> = Vec::new();
    for area in &areas {
        sector_of_area.insert(area.id.clone(), sectors.len());
        sectors.push(SkillSector {
            index: sectors.len(),
            area_id: Some(area.id.clone()),
            name: area.name.clone(),
            focus: area.focus,
            start_angle: 0.0,
            end_angle: 0.0,
            mastery: 0.0,
            attained: 0,
            total: 0,
            slots: 1,
        });
    }
    let unassigned = if needs_unassigned {
        let index = sectors.len();
        sectors.push(SkillSector {
            index,
            area_id: None,
            name: "未归属".to_string(),
            focus: false,
            start_angle: 0.0,
            end_angle: 0.0,
            mastery: 0.0,
            attained: 0,
            total: 0,
            slots: 1,
        });
        Some(index)
    } else {
        None
    };
    if sectors.is_empty() {
        // 一个领域都没有：给一个整圆的兜底扇区，画布不至于无处落笔。
        sectors.push(SkillSector {
            index: 0,
            area_id: None,
            name: "未归属".to_string(),
            focus: false,
            start_angle: START_ANGLE,
            end_angle: START_ANGLE + TAU,
            mastery: 0.0,
            attained: 0,
            total: 0,
            slots: 1,
        });
    }
    let sector_for = |area_id: &Option<String>| -> usize {
        area_id
            .as_ref()
            .and_then(|id| sector_of_area.get(id).copied())
            .or(unassigned)
            .unwrap_or(0)
    };

    // ── 扇区角宽 ∝ (1 + 技能数)：空领域也留一条细缝，不会凭空消失。
    let mut weights = vec![1.0_f64; sectors.len()];
    for skill in &skills {
        weights[sector_for(&skill.area_id)] += 1.0;
    }
    let total_weight: f64 = weights.iter().sum();
    let mut cursor = START_ANGLE;
    for (index, sector) in sectors.iter_mut().enumerate() {
        let width = TAU * weights[index] / total_weight;
        sector.start_angle = cursor;
        sector.end_angle = cursor + width;
        cursor += width;
    }

    // ── 依赖深度只在技能之间算；里程碑继承父技能深度（它们是刻度，不占独立环）。
    let skill_ids: HashSet<String> = skills.iter().map(|s| s.id.clone()).collect();
    let mut memo: HashMap<String, usize> = HashMap::new();
    let mut depth_by_id: HashMap<String, usize> = HashMap::new();
    for skill in &skills {
        let depth = depth_of(
            &skill.id,
            &deps,
            &skill_ids,
            &mut memo,
            &mut Vec::new(),
        );
        depth_by_id.insert(skill.id.clone(), depth);
    }

    let progress_of = |id: &str| progress.get(id).map(|p| p.progress).unwrap_or(0.0);
    let attained_id = |id: &str| progress_of(id) >= 1.0;

    // ── 挂在技能上的目标/项目数（角标，不进画布）。
    let mut goal_count: HashMap<String, usize> = HashMap::new();
    {
        let kind_of: HashMap<&str, &str> = items
            .iter()
            .map(|i| (i.id.as_str(), i.kind.as_str()))
            .collect();
        for (from, to, relation) in &edges {
            if !matches!(relation.as_str(), "contains" | "supports" | "related") {
                continue;
            }
            for (a, b) in [(from, to), (to, from)] {
                if kind_of.get(a.as_str()) == Some(&"skill")
                    && matches!(kind_of.get(b.as_str()), Some(&"goal") | Some(&"project"))
                {
                    *goal_count.entry(a.clone()).or_default() += 1;
                }
            }
        }
    }

    // ── 节点：环上槽位按 (created_at, id)，稳定到跨会话。
    // 有父技能的里程碑不占扇区角度——它是父节点周围的卫星（低缩放时画成环上刻度）。
    let mut ordered: Vec<&LifeItem> = items.iter().filter(|i| on_map(i)).collect();
    ordered.sort_by(|a, b| a.created_at.cmp(&b.created_at).then_with(|| a.id.cmp(&b.id)));
    let satellite_of = |item: &LifeItem| -> Option<String> {
        if item.kind != "milestone" {
            return None;
        }
        parents
            .get(&item.id)
            .filter(|parent| skill_ids.contains(*parent))
            .cloned()
        };
    // 第一遍：定环（扇区环 or 某个父节点的卫星环）并数出每环的槽位总数。
    let mut ring_of: HashMap<String, (Option<String>, usize)> = HashMap::new();
    let mut sector_by_id: HashMap<String, usize> = HashMap::new();
    let mut ring_size: HashMap<Option<String>, usize> = HashMap::new();
    let mut ring_size_by_sector: BTreeMap<usize, usize> = BTreeMap::new();
    for item in &ordered {
        let satellite = satellite_of(item);
        // 里程碑跟着父技能落在同一扇区，视觉上才是"父技能的刻度"。
        let effective_area = match &satellite {
            Some(parent) => items
                .iter()
                .find(|i| &i.id == parent)
                .and_then(|p| p.area_id.clone())
                .or_else(|| item.area_id.clone()),
            None => item.area_id.clone(),
        };
        let sector = sector_for(&effective_area);
        sector_by_id.insert(item.id.clone(), sector);
        let slot = match &satellite {
            Some(parent) => {
                let counter = ring_size.entry(Some(parent.clone())).or_insert(0);
                let value = *counter;
                *counter += 1;
                value
            }
            None => {
                let counter = ring_size_by_sector.entry(sector).or_insert(0);
                let value = *counter;
                *counter += 1;
                value
            }
        };
        ring_of.insert(item.id.clone(), (satellite, slot));
    }

    let mut nodes: Vec<SkillNode> = Vec::new();
    for item in ordered {
        let parent_id = parents.get(&item.id).cloned();
        let (satellite, slot) = ring_of.get(&item.id).cloned().unwrap_or((None, 0));
        let sector = sector_by_id.get(&item.id).copied().unwrap_or(0);
        let slot_count = match &satellite {
            Some(parent) => ring_size.get(&Some(parent.clone())).copied().unwrap_or(1),
            None => ring_size_by_sector.get(&sector).copied().unwrap_or(1),
        }
        .max(1);
        let depth = match &satellite {
            Some(parent) => depth_by_id.get(parent).copied().unwrap_or(0),
            None => depth_by_id.get(&item.id).copied().unwrap_or(0),
        };
        let node_deps: Vec<String> = deps.get(&item.id).cloned().unwrap_or_default();
        let blocked_by: Vec<String> = node_deps
            .iter()
            .filter(|p| !attained_id(p))
            .cloned()
            .collect();
        let p = progress.get(&item.id).copied().unwrap_or(lifetree::Progress {
            progress: 0.0,
            done_leaves: 0,
            total_leaves: 1,
        });
        nodes.push(SkillNode {
            id: item.id.clone(),
            kind: item.kind.clone(),
            title: item.title.clone(),
            area_id: item.area_id.clone(),
            track: item.track.clone(),
            status: item.status.clone(),
            parent_id,
            state: state_of(p.progress, !blocked_by.is_empty()),
            depends_on: node_deps,
            blocked_by,
            progress: p.progress,
            done_leaves: p.done_leaves,
            total_leaves: p.total_leaves,
            depth,
            sector,
            slot,
            slot_count,
            created_at: item.created_at,
            due_at_ms: item.due_at_ms,
            success_criteria: item.success_criteria.clone(),
            target_value: item.target_value,
            current_value: item.current_value,
            unit: item.unit.clone(),
            goal_count: goal_count.get(&item.id).copied().unwrap_or(0),
        });
    }

    // ── 领域掌握度：只算技能，避免里程碑被计两次（它们已在所属技能的进度里）。
    for sector in sectors.iter_mut() {
        let members: Vec<&SkillNode> = nodes
            .iter()
            .filter(|n| n.sector == sector.index && n.kind == "skill")
            .collect();
        sector.total = members.len();
        sector.attained = members
            .iter()
            .filter(|n| n.state == NodeState::Attained)
            .count();
        sector.mastery = if members.is_empty() {
            0.0
        } else {
            members.iter().map(|n| n.progress).sum::<f64>() / members.len() as f64
        };
        sector.slots = ring_size_by_sector
            .get(&sector.index)
            .copied()
            .unwrap_or(0)
            .max(1);
    }

    let on_map_ids: HashSet<&String> = nodes.iter().map(|n| &n.id).collect();
    let mut out_edges: Vec<SkillEdge> = Vec::new();
    for (id, prerequisites) in &deps {
        if !on_map_ids.contains(id) {
            continue;
        }
        for prerequisite in prerequisites {
            if !on_map_ids.contains(prerequisite) {
                continue;
            }
            out_edges.push(SkillEdge {
                from: id.clone(),
                to: prerequisite.clone(),
                satisfied: attained_id(prerequisite),
            });
        }
    }
    out_edges.sort_by(|a, b| a.from.cmp(&b.from).then_with(|| a.to.cmp(&b.to)));

    let mut ideas: Vec<IdeaMote> = items
        .iter()
        .filter(|i| i.kind == "idea")
        .map(|item| IdeaMote {
            id: item.id.clone(),
            title: item.title.clone(),
            sector: sector_for(&item.area_id),
            area_id: item.area_id.clone(),
            created_at: item.created_at,
            review_at_ms: item.review_at_ms,
            due_review: item.review_at_ms.is_some_and(|r| r <= stamp),
        })
        .collect();
    ideas.sort_by(|a, b| a.created_at.cmp(&b.created_at).then_with(|| a.id.cmp(&b.id)));

    let skill_nodes: Vec<&SkillNode> = nodes.iter().filter(|n| n.kind == "skill").collect();
    Ok(SkillMap {
        at_ms: stamp,
        max_depth: nodes.iter().map(|n| n.depth).max().unwrap_or(0),
        attained: skill_nodes
            .iter()
            .filter(|n| n.state == NodeState::Attained)
            .count(),
        total: skill_nodes.len(),
        sectors,
        nodes,
        edges: out_edges,
        ideas,
    })
}

/// 所有"可能发生变化"的时刻：item 出生、账本变更、边建立。
fn instants(conn: &Connection, from_ms: i64, to_ms: i64) -> Result<Vec<i64>, String> {
    let mut set: Vec<i64> = Vec::new();
    for item in lifedb::list_items(conn, true)? {
        set.push(item.created_at);
    }
    for point in lifedb::list_progress(conn, Some(to_ms))? {
        set.push(point.at_ms);
    }
    for edge in lifedb::list_edges(conn)? {
        set.push(edge.created_at);
    }
    set.retain(|t| *t >= from_ms && *t <= to_ms);
    set.push(from_ms);
    set.push(to_ms);
    set.sort_unstable();
    set.dedup();
    Ok(set)
}

/// 生长史：在每个变化时刻跑一遍同一套算法，**只输出进度真的变了的那些**（delta 编码）。
///
/// 前端按时刻取"最后一次变化"即可播放，不需要重算任何聚合。
pub fn growth(conn: &Connection, from_ms: i64, to_ms: i64) -> Result<Growth, String> {
    if to_ms <= from_ms {
        return Err("生长史时间窗口无效".to_string());
    }
    let times = instants(conn, from_ms, to_ms)?;
    let mut changes: Vec<ProgressChange> = Vec::new();
    let mut sector_changes: Vec<SectorChange> = Vec::new();
    let mut last_progress: HashMap<String, f64> = HashMap::new();
    let mut last_mastery: HashMap<usize, f64> = HashMap::new();
    for at in &times {
        let snapshot = skill_map(conn, Some(*at))?;
        for node in &snapshot.nodes {
            let previous = last_progress.get(&node.id).copied();
            if previous.is_some_and(|p| (p - node.progress).abs() < 1e-9) {
                continue;
            }
            last_progress.insert(node.id.clone(), node.progress);
            changes.push(ProgressChange {
                item_id: node.id.clone(),
                at_ms: *at,
                progress: node.progress,
                done_leaves: node.done_leaves,
                total_leaves: node.total_leaves,
                state: node.state,
            });
        }
        for sector in &snapshot.sectors {
            let previous = last_mastery.get(&sector.index).copied();
            if previous.is_some_and(|m| (m - sector.mastery).abs() < 1e-9) {
                continue;
            }
            last_mastery.insert(sector.index, sector.mastery);
            sector_changes.push(SectorChange {
                sector: sector.index,
                at_ms: *at,
                mastery: sector.mastery,
            });
        }
    }
    Ok(Growth {
        from_ms,
        to_ms,
        instants: times,
        changes,
        sectors: sector_changes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use crate::lifedb::LifeItemInput;
    use rusqlite::params;

    fn item(conn: &Connection, kind: &str, title: &str) -> String {
        lifedb::upsert_item(
            conn,
            LifeItemInput {
                kind: kind.into(),
                title: title.into(),
                origin: "app".into(),
                ..Default::default()
            },
        )
        .unwrap()
    }

    fn set(conn: &Connection, id: &str, fragment: &str) {
        conn.execute(
            &format!("UPDATE life_items SET {fragment} WHERE id=?1"),
            params![id],
        )
        .unwrap();
    }

    fn node<'a>(map: &'a SkillMap, title: &str) -> &'a SkillNode {
        map.nodes.iter().find(|n| n.title == title).unwrap()
    }

    #[test]
    fn locked_until_prerequisite_is_attained() {
        let conn = db::open(":memory:").unwrap();
        let base = item(&conn, "skill", "所有权");
        let advanced = item(&conn, "skill", "async");
        lifedb::link_items(&conn, &advanced, &base, "depends_on", 0, "app").unwrap();

        let map = skill_map(&conn, None).unwrap();
        assert_eq!(node(&map, "async").state, NodeState::Locked);
        assert_eq!(node(&map, "async").blocked_by, vec![base.clone()]);
        assert_eq!(node(&map, "所有权").state, NodeState::Available);
        assert_eq!(map.edges.len(), 1);
        assert!(!map.edges[0].satisfied);

        set(&conn, &base, "status='done'");
        let map = skill_map(&conn, None).unwrap();
        assert_eq!(node(&map, "所有权").state, NodeState::Attained);
        assert_eq!(node(&map, "async").state, NodeState::Available);
        assert!(node(&map, "async").blocked_by.is_empty());
        assert!(map.edges[0].satisfied);
        assert_eq!((map.attained, map.total), (1, 2));
    }

    #[test]
    fn depth_follows_prerequisite_chain_and_survives_cycles() {
        let conn = db::open(":memory:").unwrap();
        let a = item(&conn, "skill", "A");
        let b = item(&conn, "skill", "B");
        let c = item(&conn, "skill", "C");
        lifedb::link_items(&conn, &b, &a, "depends_on", 0, "app").unwrap();
        lifedb::link_items(&conn, &c, &b, "depends_on", 0, "app").unwrap();
        let map = skill_map(&conn, None).unwrap();
        assert_eq!(node(&map, "A").depth, 0);
        assert_eq!(node(&map, "B").depth, 1);
        assert_eq!(node(&map, "C").depth, 2);
        assert_eq!(map.max_depth, 2);

        // 环形依赖不许死循环。
        lifedb::link_items(&conn, &a, &c, "depends_on", 0, "app").unwrap();
        let map = skill_map(&conn, None).unwrap();
        assert!(map.max_depth <= MAX_DEPTH);
    }

    #[test]
    fn blocks_is_normalised_into_depends_on() {
        let conn = db::open(":memory:").unwrap();
        let base = item(&conn, "skill", "基础");
        let next = item(&conn, "skill", "进阶");
        // "基础 blocks 进阶" 与 "进阶 depends_on 基础" 必须给出同一张图。
        lifedb::link_items(&conn, &base, &next, "blocks", 0, "app").unwrap();
        let map = skill_map(&conn, None).unwrap();
        assert_eq!(node(&map, "进阶").depends_on, vec![base]);
        assert_eq!(node(&map, "进阶").depth, 1);
        assert_eq!(node(&map, "进阶").state, NodeState::Locked);
    }

    #[test]
    fn milestones_are_ticks_on_their_skill_not_separate_rings() {
        let conn = db::open(":memory:").unwrap();
        let base = item(&conn, "skill", "基础");
        let skill = item(&conn, "skill", "Rust");
        lifedb::link_items(&conn, &skill, &base, "depends_on", 0, "app").unwrap();
        let m1 = item(&conn, "milestone", "会写 trait");
        let m2 = item(&conn, "milestone", "会写 async");
        lifedb::link_items(&conn, &skill, &m1, "contains", 0, "app").unwrap();
        lifedb::link_items(&conn, &skill, &m2, "contains", 1, "app").unwrap();
        set(&conn, &m1, "status='done'");

        let map = skill_map(&conn, None).unwrap();
        let rust = node(&map, "Rust");
        assert!((rust.progress - 0.5).abs() < 1e-9, "两个里程碑完成一个 = 50%");
        assert_eq!((rust.done_leaves, rust.total_leaves), (1, 2));
        // 里程碑继承父技能的深度与扇区：它是刻度不是并列节点。
        assert_eq!(node(&map, "会写 trait").depth, rust.depth);
        assert_eq!(node(&map, "会写 async").sector, rust.sector);
        assert_eq!(node(&map, "会写 trait").parent_id.as_deref(), Some(skill.as_str()));
        // 卫星槽位在**父节点**上分，不占扇区角度：两个里程碑 = 父节点周围 2 个槽。
        assert_eq!(node(&map, "会写 trait").slot_count, 2);
        assert_eq!(node(&map, "会写 async").slot_count, 2);
        assert_ne!(node(&map, "会写 trait").slot, node(&map, "会写 async").slot);
        // 扇区环上只有两个技能，里程碑不参与。
        let sector = &map.sectors[node(&map, "Rust").sector];
        assert_eq!(sector.slots, 2);
        // 前置边只有技能之间那一条，层级边不进图。
        assert_eq!(map.edges.len(), 1);
    }

    #[test]
    fn sectors_span_the_whole_circle_and_keep_empty_areas() {
        let conn = db::open(":memory:").unwrap();
        let area_a = lifedb::upsert_area(&conn, "能力", None, Some(true)).unwrap();
        lifedb::upsert_area(&conn, "健康", None, None).unwrap();
        let skill = item(&conn, "skill", "Rust");
        set(&conn, &skill, &format!("area_id='{area_a}'"));

        let map = skill_map(&conn, None).unwrap();
        let span: f64 = map
            .sectors
            .iter()
            .map(|s| s.end_angle - s.start_angle)
            .sum();
        assert!((span - TAU).abs() < 1e-9, "扇区必须铺满整圆");
        assert!(map.sectors.iter().all(|s| s.end_angle > s.start_angle), "空领域也要留缝");
        assert!(map.sectors[0].start_angle == START_ANGLE);
        // focus 不参与排序：改 focus 不能让扇区换位置。
        let before: Vec<String> = map.sectors.iter().map(|s| s.name.clone()).collect();
        lifedb::upsert_area(&conn, "健康", None, Some(true)).unwrap();
        let after: Vec<String> = skill_map(&conn, None)
            .unwrap()
            .sectors
            .iter()
            .map(|s| s.name.clone())
            .collect();
        assert_eq!(before, after);
    }

    #[test]
    fn mastery_averages_skills_only() {
        let conn = db::open(":memory:").unwrap();
        let area = lifedb::upsert_area(&conn, "能力", None, None).unwrap();
        let done = item(&conn, "skill", "会了的");
        let todo = item(&conn, "skill", "没会的");
        set(&conn, &done, &format!("area_id='{area}', status='done'"));
        set(&conn, &todo, &format!("area_id='{area}'"));
        // 给"会了的"挂一个已完成里程碑：它不该让领域掌握度被计两次。
        let m = item(&conn, "milestone", "刻度");
        set(&conn, &m, "status='done'");
        lifedb::link_items(&conn, &done, &m, "contains", 0, "app").unwrap();

        let map = skill_map(&conn, None).unwrap();
        let sector = map
            .sectors
            .iter()
            .find(|s| s.area_id.as_deref() == Some(area.as_str()))
            .unwrap();
        assert!((sector.mastery - 0.5).abs() < 1e-9);
        assert_eq!((sector.attained, sector.total), (1, 2));
    }

    #[test]
    fn slot_is_stable_when_only_updated_at_changes() {
        let conn = db::open(":memory:").unwrap();
        let first = item(&conn, "skill", "先建的");
        let second = item(&conn, "skill", "后建的");
        set(&conn, &first, "created_at=1000");
        set(&conn, &second, "created_at=2000");
        let before: Vec<(String, usize)> = skill_map(&conn, None)
            .unwrap()
            .nodes
            .iter()
            .map(|n| (n.id.clone(), n.slot))
            .collect();
        // 只碰 updated_at（list_items 默认按它排序）——位置不许动。
        set(&conn, &first, "updated_at=9999999999999");
        let after: Vec<(String, usize)> = skill_map(&conn, None)
            .unwrap()
            .nodes
            .iter()
            .map(|n| (n.id.clone(), n.slot))
            .collect();
        assert_eq!(before, after);
    }

    #[test]
    fn playback_reads_the_ledger_not_updated_at() {
        let conn = db::open(":memory:").unwrap();
        let skill = item(&conn, "skill", "英语");
        let milestone = item(&conn, "milestone", "雅思 7 分");
        lifedb::link_items(&conn, &skill, &milestone, "contains", 0, "app").unwrap();
        // 把出生时间与边推到过去，模拟"上个月建的树"。
        let base = clock::now_ms() - 30 * 86_400_000;
        set(&conn, &skill, &format!("created_at={base}"));
        set(&conn, &milestone, &format!("created_at={base}"));
        conn.execute("UPDATE life_item_edges SET created_at=?1", params![base])
            .unwrap();
        conn.execute(
            "UPDATE life_item_progress SET at_ms=?1",
            params![base],
        )
        .unwrap();
        // 十天前完成里程碑。
        let done_at = clock::now_ms() - 10 * 86_400_000;
        set(&conn, &milestone, "status='done'");
        lifedb::record_progress(&conn, &milestone, done_at, "done", None, None, "app").unwrap();

        let before = skill_map(&conn, Some(done_at - 1)).unwrap();
        assert_eq!(node(&before, "英语").progress, 0.0);
        let after = skill_map(&conn, Some(done_at + 1)).unwrap();
        assert!((node(&after, "英语").progress - 1.0).abs() < 1e-9);
        // 出生之前，节点还不该存在。
        let empty = skill_map(&conn, Some(base - 1)).unwrap();
        assert!(empty.nodes.is_empty());

        // 生长史：delta 编码，只在真的变了时才有一条。
        let g = growth(&conn, base - 86_400_000, clock::now_ms()).unwrap();
        let english: Vec<&ProgressChange> = g
            .changes
            .iter()
            .filter(|c| c.item_id == skill)
            .collect();
        assert_eq!(english.len(), 2, "从 0 到 1 只有两条：出生与达成");
        assert_eq!(english[0].progress, 0.0);
        assert!((english[1].progress - 1.0).abs() < 1e-9);
        assert_eq!(english[1].state, NodeState::Attained);
        assert!(g.sectors.iter().any(|s| s.mastery > 0.0));
    }

    #[test]
    fn ideas_stay_outside_the_ring() {
        let conn = db::open(":memory:").unwrap();
        let idea = item(&conn, "idea", "也许学吉他");
        set(&conn, &idea, "review_at_ms=1");
        let map = skill_map(&conn, None).unwrap();
        assert!(map.nodes.is_empty(), "想法不是技能点");
        assert_eq!(map.ideas.len(), 1);
        assert!(map.ideas[0].due_review, "review_at 已过 → 该毕业审查了");
    }
}
