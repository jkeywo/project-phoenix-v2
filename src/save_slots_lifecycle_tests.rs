use super::*;
use bevy::state::app::StatesPlugin;
use bevy::time::{TimePlugin, TimeUpdateStrategy};

const TEST_TICK_HZ: f32 = 4.0;
const PERIOD: std::time::Duration = std::time::Duration::from_millis(10);

#[derive(Resource)]
struct FixedPhaseBoundary(Option<GamePhase>);

fn apply_fixed_phase_boundary(
    mut boundary: ResMut<FixedPhaseBoundary>,
    mut next_phase: ResMut<NextState<GamePhase>>,
) {
    if let Some(phase) = boundary.0.take() {
        next_phase.set(phase);
    }
}

fn test_app_with_consumer(interval_ticks: u64, consumer: bool) -> App {
    let mut app = App::new();
    app.add_plugins(TimePlugin)
        .add_plugins(StatesPlugin)
        .insert_state(GamePhase::InProgress)
        .insert_resource(SaveScenario("assets/worlds/test.toml".into()));
    let mut config = crate::world::config::WorldConfig::default();
    config.global.sim_tick_hz = TEST_TICK_HZ;
    config.global.autosave_interval_secs = interval_ticks as f32 / TEST_TICK_HZ;
    app.insert_resource(config);
    register(&mut app);
    if consumer {
        app.insert_resource(SaveCaptureConsumer);
    }
    app.world_mut()
        .resource_mut::<Time<Fixed>>()
        .set_timestep(PERIOD);
    app
}

fn test_app(interval_ticks: u64) -> App {
    test_app_with_consumer(interval_ticks, true)
}

fn run_ticks(app: &mut App, ticks_per_frame: u32, total_ticks: u64) {
    app.insert_resource(TimeUpdateStrategy::ManualDuration(PERIOD * ticks_per_frame));
    app.update(); // establishes the zero-delta baseline
    for _ in 0..(total_ticks / u64::from(ticks_per_frame)) {
        app.update();
    }
    assert_eq!(
        app.world().resource::<crate::sim_tick::SimTick>().0,
        total_ticks
    );
}

fn captured_ticks(app: &App) -> Vec<(u64, crate::save_slots::CaptureReason)> {
    app.world()
        .resource::<PendingStoredRuns>()
        .iter()
        .map(|pending| {
            assert_eq!(
                pending.run.ledger.final_tick,
                pending.decision.tick.wrapping_add(1)
            );
            assert_eq!(
                pending.run.snapshot.as_ref().map(|snapshot| snapshot.tick),
                Some(pending.decision.tick.wrapping_add(1))
            );
            assert_eq!(
                pending
                    .run
                    .snapshot
                    .as_ref()
                    .and_then(|snapshot| snapshot.state.fixed_overstep_nanos),
                Some(0)
            );
            (pending.decision.tick, pending.decision.reason)
        })
        .collect()
}

fn captured_artifacts(app: &App) -> Vec<String> {
    app.world()
        .resource::<PendingStoredRuns>()
        .iter()
        .map(|pending| crate::snapshot::export_artifact(&pending.run).unwrap())
        .collect()
}

#[test]
fn automatic_capture_ticks_ignore_rendered_frame_batching() {
    let mut per_tick = test_app(5);
    let mut per_four = test_app(5);

    run_ticks(&mut per_tick, 1, 12);
    run_ticks(&mut per_four, 4, 12);

    assert_eq!(captured_ticks(&per_tick), captured_ticks(&per_four));
    assert_eq!(
        captured_artifacts(&per_tick),
        captured_artifacts(&per_four),
        "frame batching must not enter the stored artifact"
    );
    assert_eq!(
        captured_ticks(&per_tick),
        vec![
            (0, crate::save_slots::CaptureReason::RunStarted),
            (5, crate::save_slots::CaptureReason::Periodic),
            (10, crate::save_slots::CaptureReason::Periodic),
        ]
    );

    let saved = per_tick
        .world()
        .resource::<PendingStoredRuns>()
        .iter()
        .find(|pending| pending.decision.tick == 10)
        .unwrap()
        .run
        .clone();
    let saved_snapshot = saved.snapshot.as_ref().unwrap();
    let mut boundary = test_app(5);
    run_ticks(&mut boundary, 1, 11);
    assert_eq!(
        crate::sim_digest::world_digest(boundary.world()),
        saved_snapshot.digest,
        "the capture fold must equal an independently paced T+1 boundary: {:?}",
        crate::sim_digest::digest_stages(boundary.world())
    );
    let mut resumed = test_app(5);
    resumed.insert_resource(TimeUpdateStrategy::ManualDuration(PERIOD));
    resumed.update(); // establish the zero-delta baseline before restore
    let report = crate::snapshot::restore(resumed.world_mut(), &saved_snapshot.state);
    assert!(report.is_complete());
    assert_eq!(
        resumed.world().resource::<crate::sim_tick::SimTick>().0,
        11,
        "restore starts at T+1"
    );
    resumed.update(); // execute tick 11 once, reaching the source at 12
    assert_eq!(resumed.world().resource::<crate::sim_tick::SimTick>().0, 12);
    // This deliberately partial fixture lacks the empty asteroid/collision
    // resources a restore creates, and `world_digest` distinguishes absent
    // from present-empty. The full-world digest/continuation proof lives in
    // `tests/save_slots_persistence.rs`; here the exact tick boundary and
    // byte-identical artifacts are the useful unit-level assertions.
}

#[test]
fn no_consumer_advances_the_schedule_without_capturing_or_growing_a_queue() {
    let mut app = test_app_with_consumer(5, false);
    run_ticks(&mut app, 1, 1_000);
    assert!(app.world().resource::<PendingStoredRuns>().is_empty());

    // Installing a consumer later does not replay run-start or any missed
    // periodic captures. The already-advanced schedule emits only the next
    // decision that is genuinely due.
    app.insert_resource(SaveCaptureConsumer);
    app.update();
    let pending = app.world().resource::<PendingStoredRuns>();
    assert_eq!(pending.len(), 1);
    let captured = pending.iter().next().unwrap();
    assert_eq!(captured.decision.tick, 1_000);
    assert_eq!(
        captured.decision.reason,
        crate::save_slots::CaptureReason::Periodic
    );
    assert_eq!(captured.run.ledger.final_tick, 1_001);
}

#[test]
fn disposable_test_never_captures_even_with_an_accidental_live_consumer() {
    let mut app = test_app_with_consumer(5, true);
    app.insert_resource(crate::workshop::test_clock::DisposableTest);
    run_ticks(&mut app, 1, 1_000);
    assert!(app.world().resource::<PendingStoredRuns>().is_empty());
    assert!(app.world().contains_resource::<SaveCaptureConsumer>());
}

#[test]
fn manual_requests_are_peer_local_next_tick_and_duplicates_survive() {
    let mut requesting_peer = test_app(100);
    let mut other_peer = test_app(100);
    run_ticks(&mut requesting_peer, 1, 1);
    run_ticks(&mut other_peer, 1, 1);

    request_manual_save(requesting_peer.world_mut(), "manual-a");
    request_manual_save(requesting_peer.world_mut(), "manual-a");
    requesting_peer.update();
    other_peer.update();
    let requesting = requesting_peer.world().resource::<PendingStoredRuns>();
    assert_eq!(requesting.len(), 3);
    assert_eq!(
        requesting
            .iter()
            .skip(1)
            .map(|pending| (pending.decision.tick, &pending.decision.slot))
            .collect::<Vec<_>>(),
        vec![
            (
                1,
                &crate::save_slots::CaptureSlot::Manual("manual-a".into())
            ),
            (
                1,
                &crate::save_slots::CaptureSlot::Manual("manual-a".into())
            ),
        ]
    );
    assert_eq!(
        other_peer.world().resource::<PendingStoredRuns>().len(),
        1,
        "a local manual request must not enter another peer's queue"
    );
}

#[test]
fn fixed_phase_boundary_refuses_manual_capture_before_snapshotting() {
    for boundary in [GamePhase::GameOver, GamePhase::Lobby] {
        let mut app = test_app(100);
        app.insert_resource(FixedPhaseBoundary(None))
            .add_systems(FixedUpdate, apply_fixed_phase_boundary);
        run_ticks(&mut app, 1, 1);
        while app
            .world_mut()
            .resource_mut::<PendingStoredRuns>()
            .pop_front()
            .is_some()
        {}

        // Ingress observes a live run. During that request's fixed step,
        // the production-shaped fixed StateTransition applies the terminal
        // or ReturnToLobby boundary before FixedLast capture.
        request_manual_save(app.world_mut(), "manual-at-boundary");
        app.world_mut().resource_mut::<FixedPhaseBoundary>().0 = Some(boundary.clone());
        app.update();

        let pending = app.world().resource::<PendingStoredRuns>();
        assert!(pending
            .iter()
            .all(|capture| capture.decision.reason != CaptureReason::Manual));
        assert!(pending.iter().all(|capture| {
            capture
                .run
                .snapshot
                .as_ref()
                .is_some_and(|snapshot| snapshot.state.phase != Some(GamePhase::Lobby))
        }));
        match boundary {
            GamePhase::GameOver => {
                let final_capture = pending.iter().collect::<Vec<_>>();
                assert_eq!(final_capture.len(), 1);
                assert_eq!(final_capture[0].decision.reason, CaptureReason::GameOver);
                assert_eq!(
                    final_capture[0]
                        .run
                        .snapshot
                        .as_ref()
                        .and_then(|snapshot| snapshot.state.phase.clone()),
                    Some(GamePhase::GameOver)
                );
            }
            GamePhase::Lobby => assert!(pending.is_empty()),
            GamePhase::Loading | GamePhase::InProgress => unreachable!(),
        }
        let refused = app
            .world()
            .resource::<RefusedManualSaves>()
            .iter()
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(
            refused,
            vec![RefusedManualSave {
                tick: 1,
                slot_id: "manual-at-boundary".into(),
                reason: ManualSaveRefusalReason::PhaseChanged {
                    phase: match boundary {
                        GamePhase::GameOver => SavePhase::GameOver,
                        GamePhase::Lobby => SavePhase::BeforeRun,
                        GamePhase::Loading | GamePhase::InProgress => unreachable!(),
                    },
                },
            }]
        );
    }
}

#[test]
fn startup_restore_gate_discards_bootstrap_and_rebases_cadence() {
    let mut app = test_app(5);
    begin_startup_restore(app.world_mut());
    request_manual_save(app.world_mut(), "manual-during-restore");

    run_ticks(&mut app, 1, 12);
    assert!(startup_restore_pending(app.world()));
    assert!(app.world().resource::<PendingStoredRuns>().is_empty());
    assert_eq!(
        app.world()
            .resource::<RefusedManualSaves>()
            .iter()
            .cloned()
            .collect::<Vec<_>>(),
        vec![RefusedManualSave {
            tick: 0,
            slot_id: "manual-during-restore".into(),
            reason: ManualSaveRefusalReason::StartupRestorePending,
        }]
    );

    // snapshot::restore writes SimTick before this API is called in
    // production. Reproduce that continuation boundary directly here.
    app.insert_resource(crate::sim_tick::SimTick(20));
    complete_startup_restore(app.world_mut(), 20);
    assert!(!startup_restore_pending(app.world()));
    for _ in 0..5 {
        app.update();
    }
    assert!(app.world().resource::<PendingStoredRuns>().is_empty());
    app.update();
    let pending = app.world().resource::<PendingStoredRuns>();
    assert_eq!(pending.len(), 1);
    let first = pending.iter().next().unwrap();
    assert_eq!(first.decision.tick, 25);
    assert_eq!(first.decision.reason, CaptureReason::Periodic);
    assert_eq!(first.run.ledger.final_tick, 26);
}

#[test]
fn draining_or_failing_storage_cannot_change_simulation_state() {
    let mut app = test_app(100);
    run_ticks(&mut app, 1, 1);
    let digest_before = crate::sim_digest::world_digest(app.world());
    let tick_before = app.world().resource::<crate::sim_tick::SimTick>().0;

    let pending = app
        .world_mut()
        .resource_mut::<PendingStoredRuns>()
        .pop_front()
        .expect("run-start capture should be waiting");
    let fake_storage_result: Result<(), &str> = Err("quota exceeded");
    drop((pending, fake_storage_result));

    assert_eq!(crate::sim_digest::world_digest(app.world()), digest_before);
    assert_eq!(
        app.world().resource::<crate::sim_tick::SimTick>().0,
        tick_before
    );
    assert!(app.world().resource::<PendingStoredRuns>().is_empty());
}
