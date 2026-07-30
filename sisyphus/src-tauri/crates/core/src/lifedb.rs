//! LifeDB：LifeIndex 的结构化事实来源。
//!
//! `life_items` 保存想法/目标/项目/事项/日常的共同字段，`life_item_edges` 保存关系；
//! 本地 UI 与 Notion 都只是同一份数据的投影。Notion 同步由 Agent 做语义合并，但所有
//! 本地写入、状态约束、脏标记和审计基线都由这里确定性执行。

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use uuid::Uuid;

use crate::scheduler::{self, NewAction};

/// LifeItem 的五种基本形态 + 技能树两种：
/// - `skill`：能力节点。前置关系用 `depends_on` 边，等级/阶段用 `contains` 边挂 `milestone`。
/// - `milestone`：可判定的检查点（目标拆解的产物），同时是无极时间线上的抽象层标记。
const KINDS: &[&str] = &[
    "idea",
    "goal",
    "project",
    "action",
    "routine",
    "skill",
    "milestone",
];
const TRACKS: &[&str] = &["main", "side", "neutral", "undecided"];
const HORIZONS: &[&str] = &["now", "next", "later", "someday", "unscheduled"];
const STATUSES: &[&str] = &["inbox", "active", "waiting", "done", "archived"];
const ORIGINS: &[&str] = &["app", "agent", "notion", "import"];
const RELATIONS: &[&str] = &[
    "contains",
    "supports",
    "depends_on",
    "blocks",
    "derived_from",
    "related",
];

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

#[derive(Debug, Clone, Serialize)]
pub struct LifeItem {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub body: String,
    pub track: String,
    pub horizon: String,
    pub status: String,
    /// 责任领域（GTD Horizon 3）。→ `life_areas.id`。
    pub area_id: Option<String>,
    /// 可判定的完成条件（OKR 的 key result）。缺它的 goal 永远无法收敛。
    pub success_criteria: Option<String>,
    /// 度量：目标值 / 当前值 / 单位。技能树的进度由它确定性算出。
    pub target_value: Option<f64>,
    pub current_value: Option<f64>,
    pub unit: Option<String>,
    pub start_at_ms: Option<i64>,
    pub due_at_ms: Option<i64>,
    pub review_at_ms: Option<i64>,
    pub recurrence: Option<String>,
    pub source_event_id: Option<String>,
    pub intent_id: Option<String>,
    pub sync_status: String,
    pub revision: i64,
    pub created_at: i64,
    pub updated_at: i64,
    pub external_refs: Vec<LifeExternalRef>,
}

/// 责任领域：无完成态，只需维持标准。`focus` 标记当前重点，用于推导主线/支线。
#[derive(Debug, Clone, Serialize)]
pub struct LifeArea {
    pub id: String,
    pub name: String,
    pub description: String,
    pub sort_order: i64,
    pub focus: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct LifeItemEdge {
    pub from_item_id: String,
    pub to_item_id: String,
    pub relation: String,
    pub sort_order: i64,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct LifeExternalRef {
    pub provider: String,
    pub external_id: String,
    pub external_url: Option<String>,
    pub external_updated_at_ms: Option<i64>,
    pub content_hash: Option<String>,
    pub last_pushed_revision: Option<i64>,
    pub observed_at_ms: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ExternalRefInput {
    pub provider: String,
    pub external_id: String,
    #[serde(default)]
    pub external_url: Option<String>,
    #[serde(default)]
    pub external_updated_at_ms: Option<i64>,
    #[serde(default)]
    pub content_hash: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LifeItemInput {
    #[serde(default)]
    pub id: Option<String>,
    /// 乐观并发控制。同步 Agent 更新已有项时应回传 list_life_items 看到的 revision。
    #[serde(default)]
    pub expected_revision: Option<i64>,
    pub kind: String,
    pub title: String,
    #[serde(default)]
    pub body: String,
    #[serde(default = "default_track")]
    pub track: String,
    #[serde(default = "default_horizon")]
    pub horizon: String,
    #[serde(default = "default_status")]
    pub status: String,
    /// 责任领域 id（可空；不猜）。
    #[serde(default)]
    pub area_id: Option<String>,
    /// 可判定完成条件。goal/milestone 建议必填，但 Core 不强制（宁缺毋滥优先）。
    #[serde(default)]
    pub success_criteria: Option<String>,
    #[serde(default)]
    pub target_value: Option<f64>,
    #[serde(default)]
    pub current_value: Option<f64>,
    #[serde(default)]
    pub unit: Option<String>,
    #[serde(default)]
    pub start_at_ms: Option<i64>,
    #[serde(default)]
    pub due_at_ms: Option<i64>,
    #[serde(default)]
    pub review_at_ms: Option<i64>,
    #[serde(default)]
    pub recurrence: Option<String>,
    #[serde(default)]
    pub source_event_id: Option<String>,
    #[serde(default)]
    pub intent_id: Option<String>,
    /// app | agent | notion | import。notion/import 写入视为已同步，其余会触发出站同步。
    #[serde(default = "default_origin")]
    pub origin: String,
    #[serde(default)]
    pub external_ref: Option<ExternalRefInput>,
}

impl Default for LifeItemInput {
    fn default() -> Self {
        Self {
            id: None,
            expected_revision: None,
            kind: "idea".to_string(),
            title: String::new(),
            body: String::new(),
            track: default_track(),
            horizon: default_horizon(),
            status: default_status(),
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
            origin: default_origin(),
            external_ref: None,
        }
    }
}

fn default_track() -> String {
    "undecided".to_string()
}
fn default_horizon() -> String {
    "unscheduled".to_string()
}
fn default_status() -> String {
    "inbox".to_string()
}
fn default_origin() -> String {
    "agent".to_string()
}

#[derive(Debug, Clone, Serialize)]
pub struct LifeSyncState {
    pub provider: String,
    pub target_id: String,
    pub last_snapshot_text: String,
    pub last_summary: String,
    pub last_success_at_ms: Option<i64>,
    pub last_attempt_at_ms: Option<i64>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LifeProjection {
    pub markdown: String,
    pub max_revision: i64,
    pub dirty_count: usize,
    pub item_count: usize,
    pub last_snapshot_text: String,
    /// 本次投影实际包含的逐项 revision。完成同步时原样回传，只清理未被并发修改的行。
    pub projected_revisions: BTreeMap<String, i64>,
}

pub fn upsert_item(conn: &Connection, mut input: LifeItemInput) -> Result<String, String> {
    normalize_and_validate(&mut input)?;
    let now = now_ms();
    let id = resolve_item_id(conn, &input)?.unwrap_or_else(|| Uuid::new_v4().to_string());
    let exists: bool = conn
        .query_row("SELECT 1 FROM life_items WHERE id=?1", params![id], |_| {
            Ok(true)
        })
        .optional()
        .map_err(|e| e.to_string())?
        .unwrap_or(false);
    let sync_status = match input.origin.as_str() {
        "import" => "clean",
        "notion" => "notion_dirty",
        _ => "local_dirty",
    };
    let archived_at = (input.status == "archived").then_some(now);

    if exists {
        let changed = conn
            .execute(
                "UPDATE life_items SET
                kind=?2,title=?3,body=?4,track=?5,horizon=?6,status=?7,
                start_at_ms=?8,due_at_ms=?9,review_at_ms=?10,recurrence=?11,
                source_event_id=COALESCE(?12,source_event_id),intent_id=COALESCE(?13,intent_id),
                sync_status=?14,revision=revision+1,updated_at=?15,archived_at=?16,
                area_id=?18,success_criteria=?19,target_value=?20,current_value=?21,unit=?22
             WHERE id=?1 AND (?17 IS NULL OR revision=?17)",
                params![
                    id,
                    input.kind,
                    input.title,
                    input.body,
                    input.track,
                    input.horizon,
                    input.status,
                    input.start_at_ms,
                    input.due_at_ms,
                    input.review_at_ms,
                    input.recurrence,
                    input.source_event_id,
                    input.intent_id,
                    sync_status,
                    now,
                    archived_at,
                    input.expected_revision,
                    input.area_id,
                    input.success_criteria,
                    input.target_value,
                    input.current_value,
                    input.unit,
                ],
            )
            .map_err(|e| format!("更新 LifeItem 失败: {e}"))?;
        if changed == 0 {
            return Err(format!(
                "LifeItem {id} 已在同步期间发生变化，请重新读取后做语义合并"
            ));
        }
    } else {
        conn.execute(
            "INSERT INTO life_items
               (id,kind,title,body,track,horizon,status,start_at_ms,due_at_ms,review_at_ms,
                recurrence,source_event_id,intent_id,sync_status,revision,created_at,updated_at,archived_at,
                area_id,success_criteria,target_value,current_value,unit)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,1,?15,?15,?16,?17,?18,?19,?20,?21)",
            params![
                id,
                input.kind,
                input.title,
                input.body,
                input.track,
                input.horizon,
                input.status,
                input.start_at_ms,
                input.due_at_ms,
                input.review_at_ms,
                input.recurrence,
                input.source_event_id,
                input.intent_id,
                sync_status,
                now,
                archived_at,
                input.area_id,
                input.success_criteria,
                input.target_value,
                input.current_value,
                input.unit,
            ],
        )
        .map_err(|e| format!("创建 LifeItem 失败: {e}"))?;
    }

    if let Some(ext) = input.external_ref {
        upsert_external_ref(conn, &id, &ext)?;
    }
    if !matches!(input.origin.as_str(), "notion" | "import") {
        request_sync(conn)?;
    }
    Ok(id)
}

pub const ITEM_COLUMNS: &str = "id,kind,title,body,track,horizon,status,start_at_ms,due_at_ms,\
     review_at_ms,recurrence,source_event_id,intent_id,sync_status,revision,created_at,updated_at,\
     area_id,success_criteria,target_value,current_value,unit";

pub fn list_items(conn: &Connection, include_archived: bool) -> Result<Vec<LifeItem>, String> {
    let sql = format!(
        "SELECT {ITEM_COLUMNS}
               FROM life_items
               WHERE (?1=1 OR status!='archived')
               ORDER BY
                 CASE track WHEN 'main' THEN 0 WHEN 'side' THEN 1 WHEN 'undecided' THEN 2 ELSE 3 END,
                 CASE horizon WHEN 'now' THEN 0 WHEN 'next' THEN 1 WHEN 'later' THEN 2
                              WHEN 'someday' THEN 3 ELSE 4 END,
                 CASE status WHEN 'active' THEN 0 WHEN 'inbox' THEN 1 WHEN 'waiting' THEN 2
                             WHEN 'done' THEN 3 ELSE 4 END,
                 updated_at DESC"
    );
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![include_archived as i64], row_to_item)
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    rows.into_iter()
        .map(|mut item| {
            item.external_refs = list_external_refs(conn, &item.id)?;
            Ok(item)
        })
        .collect()
}

pub fn list_dirty_items(conn: &Connection) -> Result<Vec<LifeItem>, String> {
    Ok(list_items(conn, true)?
        .into_iter()
        .filter(|item| item.sync_status != "clean")
        .collect())
}

pub fn archive_item(
    conn: &Connection,
    id: &str,
    origin: &str,
    expected_revision: Option<i64>,
) -> Result<(), String> {
    let now = now_ms();
    let sync_status = if origin == "notion" {
        "clean"
    } else {
        "local_dirty"
    };
    let changed = conn
        .execute(
            "UPDATE life_items SET status='archived',archived_at=?2,updated_at=?2,
                    revision=revision+1,sync_status=?3
             WHERE id=?1 AND (?4 IS NULL OR revision=?4)",
            params![id, now, sync_status, expected_revision],
        )
        .map_err(|e| e.to_string())?;
    if changed == 0 {
        return Err(format!("LifeItem 不存在或 revision 已变化: {id}"));
    }
    if origin != "notion" {
        request_sync(conn)?;
    }
    Ok(())
}

pub fn link_items(
    conn: &Connection,
    from_item_id: &str,
    to_item_id: &str,
    relation: &str,
    sort_order: i64,
    origin: &str,
) -> Result<(), String> {
    if !RELATIONS.contains(&relation) {
        return Err(format!("非法 LifeItem 关系: {relation}"));
    }
    if from_item_id == to_item_id {
        return Err("LifeItem 不能关联自身".to_string());
    }
    let now = now_ms();
    conn.execute(
        "INSERT INTO life_item_edges (from_item_id,to_item_id,relation,sort_order,created_at)
         VALUES (?1,?2,?3,?4,?5)
         ON CONFLICT(from_item_id,to_item_id,relation) DO UPDATE SET sort_order=excluded.sort_order",
        params![from_item_id, to_item_id, relation, sort_order, now],
    )
    .map_err(|e| format!("建立 LifeItem 关系失败: {e}"))?;
    let sync_status = if origin == "notion" {
        "clean"
    } else {
        "local_dirty"
    };
    conn.execute(
        "UPDATE life_items SET sync_status=?2,revision=revision+1,updated_at=?3
         WHERE id IN (?1,?4)",
        params![from_item_id, sync_status, now, to_item_id],
    )
    .map_err(|e| e.to_string())?;
    if origin != "notion" {
        request_sync(conn)?;
    }
    Ok(())
}

pub fn list_edges(conn: &Connection) -> Result<Vec<LifeItemEdge>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT from_item_id,to_item_id,relation,sort_order,created_at
             FROM life_item_edges ORDER BY relation,sort_order,created_at",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            Ok(LifeItemEdge {
                from_item_id: r.get(0)?,
                to_item_id: r.get(1)?,
                relation: r.get(2)?,
                sort_order: r.get(3)?,
                created_at: r.get(4)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    Ok(rows)
}

pub fn render_projection(conn: &Connection, target_id: &str) -> Result<LifeProjection, String> {
    // archived 不渲染到 Markdown，但必须进入 revision token，成功发布“删除”后才能清理 dirty。
    let items = list_items(conn, true)?;
    let dirty_count = items.iter().filter(|x| x.sync_status != "clean").count();
    let max_revision = items.iter().map(|x| x.revision).max().unwrap_or(0);
    let projected_revisions = items
        .iter()
        .map(|item| (item.id.clone(), item.revision))
        .collect();
    let mut markdown = format!(
        "# LifeIndex\n\n> 由 Sisyphus LifeDB 生成 · revision {max_revision} · 请在此页自由修改文本，Agent 会在下次同步时理解并合并。\n\n"
    );
    append_section(
        &mut markdown,
        "⏰ 事项",
        items.iter().filter(|x| x.kind == "action"),
    );
    append_section(
        &mut markdown,
        "♻️ 日常",
        items.iter().filter(|x| x.kind == "routine"),
    );
    append_section(
        &mut markdown,
        "🌳 技能与里程碑",
        items
            .iter()
            .filter(|x| x.kind == "skill" || x.kind == "milestone"),
    );
    append_section(
        &mut markdown,
        "🔑 主线",
        items.iter().filter(|x| x.track == "main"),
    );
    append_section(
        &mut markdown,
        "🔥 支线",
        items.iter().filter(|x| x.track == "side"),
    );
    append_section(
        &mut markdown,
        "💭 待整理",
        items
            .iter()
            .filter(|x| x.track == "undecided" || x.status == "inbox"),
    );
    let state = get_sync_state(conn, "notion", target_id)?.unwrap_or(LifeSyncState {
        provider: "notion".to_string(),
        target_id: target_id.to_string(),
        last_snapshot_text: String::new(),
        last_summary: String::new(),
        last_success_at_ms: None,
        last_attempt_at_ms: None,
        last_error: None,
    });
    Ok(LifeProjection {
        markdown,
        max_revision,
        dirty_count,
        item_count: items.iter().filter(|item| item.status != "archived").count(),
        last_snapshot_text: state.last_snapshot_text,
        projected_revisions,
    })
}

pub fn complete_sync(
    conn: &Connection,
    target_id: &str,
    remote_before_text: &str,
    snapshot_text: &str,
    summary: &str,
    projected_revisions: &BTreeMap<String, i64>,
) -> Result<(), String> {
    let now = now_ms();
    conn.execute(
        "INSERT INTO life_sync_runs
           (id,provider,target_id,remote_before_text,final_snapshot_text,summary,completed_at_ms)
         VALUES (?1,'notion',?2,?3,?4,?5,?6)",
        params![
            Uuid::new_v4().to_string(),
            target_id,
            remote_before_text,
            snapshot_text,
            summary,
            now
        ],
    )
    .map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT INTO life_sync_state
           (provider,target_id,last_snapshot_text,last_summary,last_success_at_ms,last_attempt_at_ms,last_error)
         VALUES ('notion',?1,?2,?3,?4,?4,NULL)
         ON CONFLICT(provider,target_id) DO UPDATE SET
           last_snapshot_text=excluded.last_snapshot_text,last_summary=excluded.last_summary,
           last_success_at_ms=excluded.last_success_at_ms,last_attempt_at_ms=excluded.last_attempt_at_ms,
           last_error=NULL",
        params![target_id, snapshot_text, summary, now],
    )
    .map_err(|e| e.to_string())?;
    for (id, revision) in projected_revisions {
        conn.execute(
            "UPDATE life_items SET sync_status='clean'
             WHERE id=?1 AND revision=?2 AND sync_status IN ('local_dirty','notion_dirty')",
            params![id, revision],
        )
        .map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE life_item_external_refs SET last_pushed_revision=?2
             WHERE item_id=?1 AND provider='notion'",
            params![id, revision],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub fn fail_sync(conn: &Connection, target_id: &str, error: &str) -> Result<(), String> {
    let now = now_ms();
    conn.execute(
        "INSERT INTO life_sync_state
           (provider,target_id,last_attempt_at_ms,last_error)
         VALUES ('notion',?1,?2,?3)
         ON CONFLICT(provider,target_id) DO UPDATE SET
           last_attempt_at_ms=excluded.last_attempt_at_ms,last_error=excluded.last_error",
        params![target_id, now, error],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn get_sync_state(
    conn: &Connection,
    provider: &str,
    target_id: &str,
) -> Result<Option<LifeSyncState>, String> {
    conn.query_row(
        "SELECT provider,target_id,last_snapshot_text,last_summary,last_success_at_ms,
                last_attempt_at_ms,last_error
         FROM life_sync_state WHERE provider=?1 AND target_id=?2",
        params![provider, target_id],
        |r| {
            Ok(LifeSyncState {
                provider: r.get(0)?,
                target_id: r.get(1)?,
                last_snapshot_text: r.get(2)?,
                last_summary: r.get(3)?,
                last_success_at_ms: r.get(4)?,
                last_attempt_at_ms: r.get(5)?,
                last_error: r.get(6)?,
            })
        },
    )
    .optional()
    .map_err(|e| e.to_string())
}

fn resolve_item_id(conn: &Connection, input: &LifeItemInput) -> Result<Option<String>, String> {
    if let Some(id) = input.id.as_ref().filter(|id| !id.trim().is_empty()) {
        return Ok(Some(id.trim().to_string()));
    }
    let Some(ext) = &input.external_ref else {
        return Ok(None);
    };
    conn.query_row(
        "SELECT item_id FROM life_item_external_refs WHERE provider=?1 AND external_id=?2",
        params![ext.provider.trim(), ext.external_id.trim()],
        |r| r.get(0),
    )
    .optional()
    .map_err(|e| e.to_string())
}

fn upsert_external_ref(
    conn: &Connection,
    item_id: &str,
    ext: &ExternalRefInput,
) -> Result<(), String> {
    let provider = ext.provider.trim();
    let external_id = ext.external_id.trim();
    if provider.is_empty() || external_id.is_empty() {
        return Err("external_ref.provider / external_id 不能为空".to_string());
    }
    conn.execute(
        "INSERT INTO life_item_external_refs
           (item_id,provider,external_id,external_url,external_updated_at_ms,content_hash,observed_at_ms)
         VALUES (?1,?2,?3,?4,?5,?6,?7)
         ON CONFLICT(provider,external_id) DO UPDATE SET
           item_id=excluded.item_id,external_url=excluded.external_url,
           external_updated_at_ms=excluded.external_updated_at_ms,content_hash=excluded.content_hash,
           observed_at_ms=excluded.observed_at_ms",
        params![
            item_id,
            provider,
            external_id,
            ext.external_url,
            ext.external_updated_at_ms,
            ext.content_hash,
            now_ms()
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn list_external_refs(conn: &Connection, item_id: &str) -> Result<Vec<LifeExternalRef>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT provider,external_id,external_url,external_updated_at_ms,content_hash,
                    last_pushed_revision,observed_at_ms
             FROM life_item_external_refs WHERE item_id=?1 ORDER BY provider,external_id",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![item_id], |r| {
            Ok(LifeExternalRef {
                provider: r.get(0)?,
                external_id: r.get(1)?,
                external_url: r.get(2)?,
                external_updated_at_ms: r.get(3)?,
                content_hash: r.get(4)?,
                last_pushed_revision: r.get(5)?,
                observed_at_ms: r.get(6)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    Ok(rows)
}

fn normalize_and_validate(input: &mut LifeItemInput) -> Result<(), String> {
    input.kind = input.kind.trim().to_lowercase();
    input.track = input.track.trim().to_lowercase();
    input.horizon = input.horizon.trim().to_lowercase();
    input.status = input.status.trim().to_lowercase();
    input.origin = input.origin.trim().to_lowercase();
    input.title = input.title.trim().to_string();
    input.body = input.body.trim().to_string();
    if input.title.is_empty() {
        return Err("LifeItem title 不能为空".to_string());
    }
    validate_value("kind", &input.kind, KINDS)?;
    validate_value("track", &input.track, TRACKS)?;
    validate_value("horizon", &input.horizon, HORIZONS)?;
    validate_value("status", &input.status, STATUSES)?;
    validate_value("origin", &input.origin, ORIGINS)?;
    if input.kind == "routine" && input.recurrence.as_deref() == Some("") {
        input.recurrence = None;
    }
    Ok(())
}

fn validate_value(name: &str, value: &str, allowed: &[&str]) -> Result<(), String> {
    if allowed.contains(&value) {
        Ok(())
    } else {
        Err(format!("非法 {name} '{value}'，应为 {allowed:?}"))
    }
}

pub fn row_to_item(r: &rusqlite::Row) -> rusqlite::Result<LifeItem> {
    Ok(LifeItem {
        id: r.get(0)?,
        kind: r.get(1)?,
        title: r.get(2)?,
        body: r.get(3)?,
        track: r.get(4)?,
        horizon: r.get(5)?,
        status: r.get(6)?,
        start_at_ms: r.get(7)?,
        due_at_ms: r.get(8)?,
        review_at_ms: r.get(9)?,
        recurrence: r.get(10)?,
        source_event_id: r.get(11)?,
        intent_id: r.get(12)?,
        sync_status: r.get(13)?,
        revision: r.get(14)?,
        created_at: r.get(15)?,
        updated_at: r.get(16)?,
        area_id: r.get(17)?,
        success_criteria: r.get(18)?,
        target_value: r.get(19)?,
        current_value: r.get(20)?,
        unit: r.get(21)?,
        external_refs: Vec::new(),
    })
}

fn append_section<'a, I>(out: &mut String, title: &str, items: I)
where
    I: Iterator<Item = &'a LifeItem>,
{
    out.push_str(&format!("## {title}\n\n"));
    let mut count = 0;
    for item in items.filter(|item| item.status != "archived") {
        count += 1;
        let state = match item.status.as_str() {
            "done" => "✓",
            "active" => "◐",
            "waiting" => "⏸",
            _ => "○",
        };
        let due = item
            .due_at_ms
            .and_then(chrono::DateTime::from_timestamp_millis)
            .map(|d| format!(" · 截止 {}", d.format("%Y-%m-%d")))
            .unwrap_or_default();
        out.push_str(&format!(
            "- {state} **{}** · {} / {} / {}{} <!-- lifeitem:{} -->\n",
            item.title, item.kind, item.track, item.horizon, due, item.id
        ));
        if !item.body.is_empty() {
            out.push_str(&format!("  - {}\n", item.body.replace('\n', " ")));
        }
    }
    if count == 0 {
        out.push_str("- 暂无\n");
    }
    out.push('\n');
}

/// 请求一次尽快同步。dedup_key 保证连续本地编辑只产生一个 pending job。
pub fn request_sync(conn: &Connection) -> Result<(), String> {
    let payload = r#"{"mode":"lifeindex_sync","topic":"合并 LifeDB 与指定 Notion LifeIndex 页面，并将最终普通 Markdown 看板投影回该页面"}"#;
    scheduler::enqueue_action(
        conn,
        &NewAction {
            kind: "agent_run",
            payload_json: payload,
            due_at_ms: now_ms(),
            recurrence: None,
            dedup_key: Some("lifeindex-outbound-sync"),
            origin_event_id: None,
            created_by: "lifedb",
        },
    )?;
    Ok(())
}

// ── 责任领域（GTD Horizon 3）────────────────────────────────────────────────

pub fn list_areas(conn: &Connection) -> Result<Vec<LifeArea>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id,name,description,sort_order,focus,created_at,updated_at
             FROM life_areas ORDER BY focus DESC, sort_order ASC, name ASC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            Ok(LifeArea {
                id: r.get(0)?,
                name: r.get(1)?,
                description: r.get(2)?,
                sort_order: r.get(3)?,
                focus: r.get::<_, i64>(4)? != 0,
                created_at: r.get(5)?,
                updated_at: r.get(6)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    Ok(rows)
}

/// 按名字幂等 upsert 一个领域，返回 id。`focus=Some(true)` 标为当前重点。
pub fn upsert_area(
    conn: &Connection,
    name: &str,
    description: Option<&str>,
    focus: Option<bool>,
) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("领域名不能为空".to_string());
    }
    let now = now_ms();
    let existing: Option<String> = conn
        .query_row(
            "SELECT id FROM life_areas WHERE name=?1",
            params![name],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let id = existing.unwrap_or_else(|| Uuid::new_v4().to_string());
    conn.execute(
        "INSERT INTO life_areas (id,name,description,sort_order,focus,created_at,updated_at)
         VALUES (?1,?2,COALESCE(?3,''),0,COALESCE(?4,0),?5,?5)
         ON CONFLICT(id) DO UPDATE SET
           description=COALESCE(?3, description),
           focus=COALESCE(?4, focus),
           updated_at=excluded.updated_at",
        params![id, name, description, focus.map(|f| f as i64), now],
    )
    .map_err(|e| format!("写入领域失败: {e}"))?;
    Ok(id)
}

// ── Notion 回滚（整页替换的安全网）──────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct LifeSyncRun {
    pub id: String,
    pub target_id: String,
    pub remote_before_text: String,
    pub final_snapshot_text: String,
    pub summary: String,
    pub completed_at_ms: i64,
}

/// 列出历史同步轮次（不含全文，避免上下文爆掉；预览截断到 200 字）。
pub fn list_sync_runs(
    conn: &Connection,
    target_id: &str,
    limit: i64,
) -> Result<Vec<LifeSyncRun>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id,target_id,SUBSTR(remote_before_text,1,200),SUBSTR(final_snapshot_text,1,200),
                    summary,completed_at_ms
             FROM life_sync_runs WHERE provider='notion' AND target_id=?1
             ORDER BY completed_at_ms DESC LIMIT ?2",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![target_id, limit], |r| {
            Ok(LifeSyncRun {
                id: r.get(0)?,
                target_id: r.get(1)?,
                remote_before_text: r.get(2)?,
                final_snapshot_text: r.get(3)?,
                summary: r.get(4)?,
                completed_at_ms: r.get(5)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    Ok(rows)
}

/// 取某一轮的**写回前全文**，用于把 Notion 页恢复成同步前的样子。
///
/// 整页替换是这条链路上唯一不可逆的一步：语义合并出错一次，用户手写内容整页被覆盖。
/// `life_sync_runs` 一直在存写前全文，但此前没有任何读取入口——补上它，同步才敢开。
pub fn sync_run_remote_before(conn: &Connection, run_id: &str) -> Result<String, String> {
    conn.query_row(
        "SELECT remote_before_text FROM life_sync_runs WHERE id=?1",
        params![run_id],
        |r| r.get(0),
    )
    .optional()
    .map_err(|e| e.to_string())?
    .ok_or_else(|| format!("同步轮次不存在: {run_id}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    fn conn() -> Connection {
        db::open(":memory:").unwrap()
    }

    #[test]
    fn upsert_marks_local_dirty_and_notion_dirty() {
        let conn = conn();
        let id = upsert_item(
            &conn,
            LifeItemInput {
                id: None,
                expected_revision: None,
                kind: "project".into(),
                title: "西西弗斯".into(),
                body: "做本地优先助手".into(),
                track: "main".into(),
                horizon: "now".into(),
                status: "active".into(),
                start_at_ms: None,
                due_at_ms: None,
                review_at_ms: None,
                recurrence: None,
                source_event_id: None,
                intent_id: None,
                origin: "app".into(),
                external_ref: None,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(list_dirty_items(&conn).unwrap().len(), 1);

        let original = list_items(&conn, false).unwrap().remove(0);
        upsert_item(
            &conn,
            LifeItemInput {
                id: Some(id),
                expected_revision: Some(original.revision),
                kind: original.kind,
                title: "西西弗斯计划".into(),
                body: original.body,
                track: original.track,
                horizon: original.horizon,
                status: original.status,
                start_at_ms: None,
                due_at_ms: None,
                review_at_ms: None,
                recurrence: None,
                source_event_id: None,
                intent_id: None,
                origin: "notion".into(),
                external_ref: None,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            list_dirty_items(&conn).unwrap()[0].sync_status,
            "notion_dirty"
        );
    }

    #[test]
    fn relations_and_projection_use_same_items() {
        let conn = conn();
        let make = |kind: &str, title: &str, track: &str| LifeItemInput {
            id: None,
            expected_revision: None,
            kind: kind.into(),
            title: title.into(),
            body: String::new(),
            track: track.into(),
            horizon: "next".into(),
            status: "active".into(),
            start_at_ms: None,
            due_at_ms: None,
            review_at_ms: None,
            recurrence: None,
            source_event_id: None,
            intent_id: None,
            origin: "import".into(),
            external_ref: None,
            ..Default::default()
        };
        let project = upsert_item(&conn, make("project", "学吉他", "side")).unwrap();
        let action = upsert_item(&conn, make("action", "买琴弦", "side")).unwrap();
        link_items(&conn, &project, &action, "contains", 0, "import").unwrap();
        assert_eq!(list_edges(&conn).unwrap().len(), 1);
        let projection = render_projection(&conn, "page").unwrap();
        assert!(projection.markdown.contains("⏰ 事项"));
        assert!(projection.markdown.contains("🔥 支线"));
        assert!(projection.markdown.matches("买琴弦").count() >= 2);
    }

    #[test]
    fn complete_sync_does_not_clean_a_concurrent_edit() {
        let conn = conn();
        let id = upsert_item(
            &conn,
            LifeItemInput {
                id: None,
                expected_revision: None,
                kind: "action".into(),
                title: "写方案".into(),
                body: String::new(),
                track: "main".into(),
                horizon: "now".into(),
                status: "active".into(),
                start_at_ms: None,
                due_at_ms: None,
                review_at_ms: None,
                recurrence: None,
                source_event_id: None,
                intent_id: None,
                origin: "app".into(),
                external_ref: None,
                ..Default::default()
            },
        )
        .unwrap();
        let projected = render_projection(&conn, "page").unwrap();

        let before = list_items(&conn, false).unwrap().remove(0);
        upsert_item(
            &conn,
            LifeItemInput {
                id: Some(id.clone()),
                expected_revision: Some(before.revision),
                kind: before.kind,
                title: "写完方案".into(),
                body: before.body,
                track: before.track,
                horizon: before.horizon,
                status: before.status,
                start_at_ms: before.start_at_ms,
                due_at_ms: before.due_at_ms,
                review_at_ms: before.review_at_ms,
                recurrence: before.recurrence,
                source_event_id: None,
                intent_id: None,
                origin: "app".into(),
                external_ref: None,
                ..Default::default()
            },
        )
        .unwrap();

        complete_sync(
            &conn,
            "page",
            &projected.markdown,
            &projected.markdown,
            "old projection",
            &projected.projected_revisions,
        )
        .unwrap();
        let current = list_items(&conn, false).unwrap().remove(0);
        assert_eq!(current.title, "写完方案");
        assert_eq!(current.sync_status, "local_dirty");
    }
}
