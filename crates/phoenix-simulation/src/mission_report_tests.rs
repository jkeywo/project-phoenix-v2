use super::*;
use crate::core::report::ReportRowState;
use bevy::ecs::message::Messages;

fn report_app() -> App {
    let mut app = App::new();
    app.add_message::<NarrativeEvent>()
        .init_resource::<EffectQueue<ReportRow>>()
        .init_resource::<MissionReport>()
        .add_systems(Update, apply_report_rows);
    app
}

fn row(id: &str, state: ReportRowState, score: i32) -> ReportRow {
    ReportRow {
        id: id.to_string(),
        heading_id: format!("world.probe.report.{id}.heading"),
        outcome_id: format!("world.probe.report.{id}.{}", state.as_str()),
        state,
        score,
    }
}

fn push(app: &mut App, row: ReportRow) {
    app.world_mut()
        .resource_mut::<EffectQueue<ReportRow>>()
        .0
        .push(row);
}

fn drain(app: &mut App) -> Vec<NarrativeEvent> {
    let mut messages = app.world_mut().resource_mut::<Messages<NarrativeEvent>>();
    messages.drain().collect()
}

#[test]
fn a_queued_row_lands_on_the_report_and_beats_the_timeline() {
    let mut app = report_app();
    push(&mut app, row("lyra", ReportRowState::Saved, 6));
    app.update();

    let report = app.world().resource::<MissionReport>();
    assert_eq!(report.rows().len(), 1);
    assert_eq!(report.rows()[0].state, ReportRowState::Saved);
    assert_eq!(report.total(), 6);

    let events = drain(&mut app);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].kind, NarrativeKind::ReportRowUpdated);
    assert_eq!(events[0].id, "lyra");
    assert_eq!(
        events[0].detail.get("state"),
        Some(&NarrativeValue::Text("saved".into()))
    );
    // The timeline is a diagnostic surface, so the hidden score is on it.
    assert_eq!(events[0].detail.get("score"), Some(&NarrativeValue::Int(6)));
}

/// The queue is drained in order, and the report keeps that order.
#[test]
fn rows_land_in_queue_order() {
    let mut app = report_app();
    push(&mut app, row("lyra", ReportRowState::Saved, 6));
    push(&mut app, row("traffic", ReportRowState::Partial, 2));
    app.update();

    let ids: Vec<String> = app
        .world()
        .resource::<MissionReport>()
        .rows()
        .iter()
        .map(|r| r.id.clone())
        .collect();
    assert_eq!(ids, vec!["lyra".to_string(), "traffic".to_string()]);
}

/// Two writes to one row in one tick settle to the last, and beat once.
#[test]
fn two_writes_to_one_row_in_a_tick_settle_to_the_last() {
    let mut app = report_app();
    push(&mut app, row("lyra", ReportRowState::Lost, -6));
    push(&mut app, row("lyra", ReportRowState::Saved, 6));
    app.update();

    let report = app.world().resource::<MissionReport>();
    assert_eq!(report.rows().len(), 1);
    assert_eq!(report.rows()[0].state, ReportRowState::Saved);
    assert_eq!(drain(&mut app).len(), 2, "each real move is one beat");
}

/// The rate guard: re-stating an unchanged row writes no beat at all.
#[test]
fn restating_an_unchanged_row_produces_no_beat() {
    let mut app = report_app();
    push(&mut app, row("lyra", ReportRowState::Saved, 6));
    app.update();
    assert_eq!(drain(&mut app).len(), 1);

    push(&mut app, row("lyra", ReportRowState::Saved, 6));
    app.update();
    assert!(drain(&mut app).is_empty());
    assert_eq!(app.world().resource::<MissionReport>().rows().len(), 1);
}

/// The run boundary (AC5). A second round must not inherit round one's
/// rows: the report is per-run state, and a scenario that authored nothing
/// has to reach its ending with an empty report even when the round before
/// it filled one.
#[test]
fn a_new_run_starts_with_no_rows() {
    use bevy::ecs::system::RunSystemOnce;

    let mut app = report_app();
    push(&mut app, row("lyra", ReportRowState::Saved, 6));
    app.update();
    assert_eq!(app.world().resource::<MissionReport>().rows().len(), 1);
    let _ = drain(&mut app);

    // Round two starts. `OnEnter(GamePhase::InProgress)` runs this on every
    // start, including the one a `ReturnToLobby` leads back to.
    app.world_mut()
        .run_system_once(reset_mission_report)
        .expect("reset_mission_report should run");

    let report = app.world().resource::<MissionReport>();
    assert!(
        report.is_empty(),
        "round two inherited round one's rows: {:?}",
        report.rows()
    );
    assert_eq!(report.total(), 0);

    // And the round that follows still accumulates normally.
    push(&mut app, row("traffic", ReportRowState::Partial, 2));
    app.update();
    let ids: Vec<String> = app
        .world()
        .resource::<MissionReport>()
        .rows()
        .iter()
        .map(|r| r.id.clone())
        .collect();
    assert_eq!(ids, vec!["traffic".to_string()]);
}

/// A row still sitting in the queue when a run starts belongs to the run
/// that queued it, not to this one.
#[test]
fn a_new_run_starts_with_an_empty_queue() {
    use bevy::ecs::system::RunSystemOnce;

    let mut app = report_app();
    push(&mut app, row("lyra", ReportRowState::Saved, 6));
    app.world_mut()
        .run_system_once(reset_mission_report)
        .expect("reset_mission_report should run");
    app.update();

    assert!(app.world().resource::<MissionReport>().is_empty());
    assert!(drain(&mut app).is_empty());
}

/// A run that authored nothing leaves the report empty — which is what
/// keeps every other scenario's ending exactly as it was (AC5).
#[test]
fn an_unwritten_report_stays_empty() {
    let mut app = report_app();
    app.update();
    assert!(app.world().resource::<MissionReport>().is_empty());
    assert!(drain(&mut app).is_empty());
}

#[test]
fn the_finalized_beat_carries_the_row_count_and_the_hidden_total() {
    let mut report = MissionReport::default();
    report.set_row(row("lyra", ReportRowState::Saved, 6));
    report.set_row(row("traffic", ReportRowState::Lost, -4));

    let event = finalized_event("world.probe.game_over.done", &report);
    assert_eq!(event.kind, NarrativeKind::ReportFinalized);
    assert_eq!(event.id, "world.probe.game_over.done");
    assert_eq!(event.detail.get("rows"), Some(&NarrativeValue::Int(2)));
    assert_eq!(event.detail.get("total"), Some(&NarrativeValue::Int(2)));
}
