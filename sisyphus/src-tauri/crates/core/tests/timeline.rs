use sisyphus_core::clock;
use sisyphus_core::db;
use sisyphus_core::ingest::{capture_text, ingest_event, NewEvent};
use sisyphus_core::timeline;

const DAY: i64 = 86_400_000;
const HOUR: i64 = 3_600_000;

fn session(conn: &rusqlite::Connection, id: &str, start: i64, end: i64, category: &str, app: &str) {
    ingest_event(
        conn,
        "local-user",
        "test-device",
        NewEvent {
            event_id: Some(id.into()),
            source: "desktop_agent".into(),
            layer: "raw".into(),
            event_type: "app_foreground".into(),
            time_mode: "interval".into(),
            event_time: None,
            start_time: Some(start),
            end_time: Some(end),
            entity: Some(app.into()),
            category: Some(category.into()),
            payload: serde_json::json!({}),
            parent_event_ids: vec![],
            privacy_level: "L0".into(),
        },
    )
    .unwrap();
}

#[test]
fn timeline_switches_from_events_to_daily_aggregation() {
    let conn = db::open(":memory:").unwrap();
    let start = chrono::Utc::now().timestamp_millis() - 60 * 60_000;
    session(
        &conn,
        "timeline-event",
        start,
        start + 30 * 60_000,
        "productivity.code",
        "com.example.editor",
    );
    db::insert_intervention(
        &conn,
        "timeline-intervention",
        "test-rule",
        start + 10 * 60_000,
        "low",
        "回来看看现在最重要的事",
        "[]",
    )
    .unwrap();

    let minute = timeline::query_timeline(
        &conn,
        start - 60_000,
        start + 60 * 60_000,
        "minute",
        100,
        "none",
    )
    .unwrap();
    assert_eq!(minute.events.len(), 2);
    assert!(minute.events.iter().any(|event| event.kind == "behavior"));
    assert!(minute
        .events
        .iter()
        .any(|event| event.kind == "intervention"));

    let week = timeline::query_timeline(
        &conn,
        start - 7 * DAY,
        start + DAY,
        "week",
        100,
        "none",
    )
    .unwrap();
    assert!(week.events.iter().all(|event| event.kind != "behavior"));
    assert_eq!(week.days.len(), 1);
    assert_eq!(week.days[0].focus_ms, 30 * 60_000);
    assert_eq!(week.days[0].intervention_count, 1);
}

#[test]
fn ticks_come_from_core_and_align_with_logical_days() {
    let conn = db::open(":memory:").unwrap();
    let now = clock::now_ms();
    let response =
        timeline::query_timeline(&conn, now - 3 * DAY, now, "day", 100, "none").unwrap();
    assert!(!response.ticks.is_empty(), "刻度必须由后端给出");
    assert!(response.ticks.iter().any(|t| t.tier == 0 && !t.label.is_empty()));
    let boundary = response.boundary_hour;
    for tick in response.ticks.iter().filter(|t| t.day_start) {
        assert_eq!(
            tick.ms,
            clock::day_start_at(tick.ms, boundary),
            "日界刻度必须与日桶起点一致（前端用 Date 会偏到 UTC 午夜）"
        );
    }
    assert_eq!(response.fold, "none");
    assert_eq!(response.cell_kind, "none");
    assert!(response.grid.rows.is_empty());
}

#[test]
fn day_fold_uses_sessions_at_short_span() {
    let conn = db::open(":memory:").unwrap();
    let now = clock::now_ms();
    let today = clock::day_start_at(now, 0);
    // 昨天与今天各一段：折叠后应各占一行。
    session(&conn, "s1", today + 10 * HOUR, today + 11 * HOUR, "work", "term");
    session(
        &conn,
        "s2",
        today - DAY + 22 * HOUR,
        today - DAY + 23 * HOUR,
        "entertainment.video",
        "bili",
    );

    let response = timeline::query_timeline(
        &conn,
        today - 3 * DAY,
        today + DAY,
        // 4 天跨度下 LOD 本会把原始会话过滤掉；折叠成日时必须放开，否则格子是空的。
        "week",
        2_000,
        "day",
    )
    .unwrap();
    assert_eq!(response.fold, "day");
    assert_eq!(response.cell_kind, "session");
    assert_eq!(response.grid.cols, 24);
    assert!(response.grid.rows.len() >= 4);
    let behaviors = response
        .events
        .iter()
        .filter(|e| e.kind == "behavior")
        .count();
    assert_eq!(behaviors, 2, "折叠成日必须拿到原始会话");
}

#[test]
fn day_fold_falls_back_to_hour_cells_at_long_span() {
    let conn = db::open(":memory:").unwrap();
    let now = clock::now_ms();
    let today = clock::day_start_at(now, 0);
    session(&conn, "s1", today + 9 * HOUR, today + 10 * HOUR + 1_800_000, "work", "term");

    let response =
        timeline::query_timeline(&conn, today - 200 * DAY, today + DAY, "life", 2_000, "day")
            .unwrap();
    assert_eq!(response.cell_kind, "hour", "长跨度折叠必须走小时预聚合");
    let cells: Vec<_> = response
        .cells
        .iter()
        .filter(|c| c.observed_ms > 0)
        .collect();
    assert_eq!(cells.len(), 2, "09:00 与 10:00 两格");
    assert!(cells.iter().all(|c| c.col == 9 || c.col == 10));
    // 同一天 → 同一行。
    assert_eq!(cells[0].row, cells[1].row);
    assert_eq!(cells.iter().map(|c| c.observed_ms).sum::<i64>(), 5_400_000);
}

#[test]
fn week_fold_puts_days_in_calendar_columns() {
    let conn = db::open(":memory:").unwrap();
    let now = clock::now_ms();
    let today = clock::day_start_at(now, 0);
    session(&conn, "s1", today + 9 * HOUR, today + 10 * HOUR, "work", "term");

    let response =
        timeline::query_timeline(&conn, today - 20 * DAY, today + DAY, "week", 500, "week")
            .unwrap();
    assert_eq!(response.cell_kind, "day");
    assert_eq!(response.grid.cols, 7);
    let cell = response
        .cells
        .iter()
        .find(|c| c.observed_ms > 0)
        .expect("今天应有一个单元格");
    assert_eq!(
        cell.col,
        clock::logical_weekday(today, 0) as i32,
        "列号 = 星期几"
    );
    assert_eq!(cell.focus_ms, HOUR);

    // 网格必须补齐：没观测的日子也要有格子，否则列选区会漏掉整行，
    // 而且"没数据"会和"有数据但没娱乐"长得一样。
    for row in &response.grid.rows {
        let count = response.cells.iter().filter(|c| c.row == row.index).count();
        assert_eq!(count, 7, "每一周都应有 7 个格子");
    }
    assert!(response.cells.iter().any(|c| c.observed_ms == 0));
}

#[test]
fn month_fold_grid_is_complete_per_calendar_month() {
    let conn = db::open(":memory:").unwrap();
    let now = clock::now_ms();
    let today = clock::day_start_at(now, 0);
    let response =
        timeline::query_timeline(&conn, today - 60 * DAY, today + DAY, "life", 500, "month")
            .unwrap();
    assert_eq!(response.grid.cols, 31);
    for row in &response.grid.rows {
        let count = response.cells.iter().filter(|c| c.row == row.index).count();
        let days = ((row.end_ms - row.start_ms) as f64 / DAY as f64).round() as usize;
        assert_eq!(count, days, "格子数 = 该月的实际天数（短月右侧留空）");
    }
}

#[test]
fn range_stats_are_exact_at_window_edges() {
    let conn = db::open(":memory:").unwrap();
    let now = clock::now_ms();
    let today = clock::day_start_at(now, 0);
    // 09:00–11:00 工作；选区只取 10:00–11:00 → 只应计 1 小时。
    session(&conn, "s1", today + 9 * HOUR, today + 11 * HOUR, "work", "term");
    let stats = timeline::range_stats(&conn, &[(today + 10 * HOUR, today + 11 * HOUR)]).unwrap();
    assert_eq!(stats.observed_ms, HOUR, "必须按交集算，不是整段算");
    assert_eq!(stats.focus_ms, HOUR);
    assert_eq!(stats.covered_ms, HOUR);
    assert_eq!(stats.top_apps[0].key, "term");
}

#[test]
fn range_stats_sum_phase_windows_across_days() {
    let conn = db::open(":memory:").unwrap();
    let now = clock::now_ms();
    let today = clock::day_start_at(now, 0);
    let yesterday = today - DAY;
    // 连续两天的 22:00–23:00 都在刷视频：相位选区应把两天加在一起。
    session(
        &conn,
        "s1",
        today + 22 * HOUR,
        today + 23 * HOUR,
        "entertainment.video",
        "bili",
    );
    session(
        &conn,
        "s2",
        yesterday + 22 * HOUR,
        yesterday + 23 * HOUR,
        "entertainment.video",
        "bili",
    );
    // 白天的工作不该被算进夜间相位。
    session(&conn, "s3", today + 10 * HOUR, today + 12 * HOUR, "work", "term");

    let windows = [
        (yesterday + 22 * HOUR, yesterday + 24 * HOUR),
        (today + 22 * HOUR, today + 24 * HOUR),
    ];
    let stats = timeline::range_stats(&conn, &windows).unwrap();
    assert_eq!(stats.windows, 2);
    assert_eq!(stats.entertainment_ms, 2 * HOUR);
    assert_eq!(stats.focus_ms, 0, "白天的工作不在夜间相位里");
    assert_eq!(stats.covered_ms, 4 * HOUR);
    assert_eq!(stats.top_categories[0].key, "entertainment.video");
}

#[test]
fn range_stats_count_interactions_and_artifacts() {
    let conn = db::open(":memory:").unwrap();
    let now = clock::now_ms();
    let today = clock::day_start_at(now, 0);
    db::insert_intervention(
        &conn,
        "i1",
        "rule",
        today + 3 * HOUR,
        "low",
        "歇会儿",
        "[]",
    )
    .unwrap();
    db::record_intervention_outcome(
        &conn,
        "i1",
        sisyphus_core::intervention::OUTCOME_SWITCHED,
        Some("测试"),
        today + 4 * HOUR,
    )
    .unwrap();
    capture_text(&conn, "local-user", "test-device", "记一句").unwrap();

    let stats = timeline::range_stats(&conn, &[(today, today + DAY)]).unwrap();
    assert_eq!(stats.intervention_count, 1);
    assert_eq!(stats.intervention_switched, 1);
    assert_eq!(stats.capture_count, 1);
    assert_eq!(stats.observed_ms, 0, "没有前台会话时不许凭空造观测");
}

#[test]
fn range_stats_reject_empty_and_inverted_windows() {
    let conn = db::open(":memory:").unwrap();
    let now = clock::now_ms();
    let stats = timeline::range_stats(&conn, &[(now, now), (now + 1000, now)]).unwrap();
    assert_eq!(stats.windows, 0);
    assert_eq!(stats.covered_ms, 0);
    assert!(!stats.truncated);
}
