use sisyphus_core::db;
use sisyphus_core::ingest::{ingest_event, NewEvent};
use sisyphus_core::timeline;

#[test]
fn timeline_switches_from_events_to_daily_aggregation() {
    let conn = db::open(":memory:").unwrap();
    let start = chrono::Utc::now().timestamp_millis() - 60 * 60_000;
    ingest_event(
        &conn,
        "local-user",
        "test-device",
        NewEvent {
            event_id: Some("timeline-event".into()),
            source: "desktop_agent".into(),
            layer: "raw".into(),
            event_type: "app_foreground".into(),
            time_mode: "interval".into(),
            event_time: None,
            start_time: Some(start),
            end_time: Some(start + 30 * 60_000),
            entity: Some("com.example.editor".into()),
            category: Some("productivity.code".into()),
            payload: serde_json::json!({}),
            parent_event_ids: vec![],
            privacy_level: "L0".into(),
        },
    )
    .unwrap();
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

    let minute =
        timeline::query_timeline(&conn, start - 60_000, start + 60 * 60_000, "minute", 100)
            .unwrap();
    assert_eq!(minute.events.len(), 2);
    assert!(minute.events.iter().any(|event| event.kind == "behavior"));
    assert!(minute
        .events
        .iter()
        .any(|event| event.kind == "intervention"));

    let week = timeline::query_timeline(
        &conn,
        start - 7 * 86_400_000,
        start + 86_400_000,
        "week",
        100,
    )
    .unwrap();
    assert!(week.events.iter().all(|event| event.kind != "behavior"));
    assert_eq!(week.days.len(), 1);
    assert_eq!(week.days[0].focus_ms, 30 * 60_000);
    assert_eq!(week.days[0].intervention_count, 1);
}
