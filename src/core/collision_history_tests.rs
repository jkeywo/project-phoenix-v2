use super::*;
use crate::{core::messages::GamePhase, sim_tick::SimTick};
use bevy::{
    state::app::StatesPlugin,
    time::{TimePlugin, TimeUpdateStrategy},
};
use std::time::Duration;

const STEP: Duration = Duration::from_millis(10);

fn app() -> App {
    let mut app = App::new();
    app.add_plugins((TimePlugin, StatesPlugin))
        .insert_state(GamePhase::InProgress)
        .insert_resource(TimeUpdateStrategy::ManualDuration(STEP));
    crate::sim_tick::register_sim_tick(&mut app);
    register(&mut app);
    register(&mut app); // ordinary shared composition must remain idempotent
    app.world_mut()
        .resource_mut::<Time<Fixed>>()
        .set_timestep(STEP);
    app.update(); // zero-delta baseline and ordinary OnEnter reset
    app
}

fn collision(victim: &str) -> BalanceEvent {
    BalanceEvent::DamageApplied {
        attacker: None,
        victim: victim.into(),
        victim_kind: VictimKind::Ship,
        weapon: WEAPON_KIND_COLLISION.into(),
        amount: 9.0,
        shield_absorbed: 2.0,
        hull_damage: 7.0,
        system_hit: None,
    }
}

fn emit_each_tick(tick: Res<SimTick>, mut events: MessageWriter<BalanceEvent>) {
    events.write(collision(&format!("ship-{}", tick.0)));
}

#[test]
fn collision_history_commits_each_fixed_tick_before_capture() {
    let mut app = app();
    let mut config = crate::world::config::WorldConfig::default();
    config.global.sim_tick_hz = 100.0;
    config.global.autosave_interval_secs = 0.01;
    app.insert_resource(config)
        .insert_resource(crate::save_slots_lifecycle::SaveCaptureConsumer)
        .insert_resource(crate::save_slots_lifecycle::SaveScenario(
            "collision-fixture".into(),
        ))
        .add_systems(FixedUpdate, emit_each_tick);
    crate::save_slots_lifecycle::register(&mut app);
    crate::save_slots_lifecycle::request_manual_save(app.world_mut(), "boundary");
    app.insert_resource(TimeUpdateStrategy::ManualDuration(STEP * 2));
    app.update(); // two committed logical ticks inside one rendered frame
    let history = app.world().resource::<CollisionHistory>().records();
    assert_eq!(
        history.iter().map(|row| row.tick).collect::<Vec<_>>(),
        vec![0, 1]
    );
    assert_eq!(
        history
            .iter()
            .map(|row| row.victim.as_str())
            .collect::<Vec<_>>(),
        vec!["ship-0", "ship-1"]
    );
    assert_eq!(app.world().resource::<SimTick>().0, 2);
    let mut count = 0;
    while let Some(saved) = app
        .world_mut()
        .resource_mut::<crate::save_slots_lifecycle::PendingStoredRuns>()
        .pop_front()
    {
        let snapshot = saved.run.snapshot.unwrap();
        assert_eq!(snapshot.state.collisions.len() as u64, snapshot.tick);
        assert_eq!(
            snapshot.state.collisions.last().unwrap().tick + 1,
            snapshot.tick
        );
        count += 1;
    }
    assert!(
        count >= 2,
        "both fixed boundaries must be captured, not only frame end"
    );
}

#[test]
fn collision_history_is_independent_of_reports_and_preserves_every_folded_field() {
    let mut app = app();
    app.world_mut()
        .resource_mut::<Messages<BalanceEvent>>()
        .write(collision("captured-victim"));
    app.update();
    let rows = app
        .world()
        .resource::<CollisionHistory>()
        .records()
        .to_vec();
    let digest = crate::sim_digest::world_digest(app.world());
    app.insert_resource(super::super::telemetry::RunTelemetry::default());
    assert_eq!(crate::sim_digest::world_digest(app.world()), digest);
    app.world_mut()
        .resource_mut::<super::super::telemetry::RunTelemetry>()
        .balance_events
        .push(rows[0].stamped_event());
    assert_eq!(
        crate::sim_digest::world_digest(app.world()),
        digest,
        "a report cannot double-fold collisions"
    );
    for field in 0..4 {
        let mut changed = rows.clone();
        match field {
            0 => changed[0].victim = "other-victim".into(),
            1 => changed[0].amount += 1.0,
            2 => changed[0].shield_absorbed += 1.0,
            _ => changed[0].hull_damage += 1.0,
        }
        restore(app.world_mut(), &changed);
        assert_ne!(
            crate::sim_digest::world_digest(app.world()),
            digest,
            "field {field} must remain authoritative"
        );
    }
    restore(app.world_mut(), &rows);
    assert_eq!(crate::sim_digest::world_digest(app.world()), digest);
    assert_eq!(crate::snapshot::capture(app.world()).collisions, rows);
}

#[test]
fn collision_history_new_mission_discards_prior_history_and_pending_input() {
    let mut app = app();
    app.world_mut()
        .resource_mut::<Messages<BalanceEvent>>()
        .write(collision("round-one"));
    app.update();
    assert_eq!(
        app.world().resource::<CollisionHistory>().records().len(),
        1
    );
    app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::ZERO));
    app.world_mut()
        .resource_mut::<NextState<GamePhase>>()
        .set(GamePhase::Lobby);
    app.update();
    app.world_mut()
        .resource_mut::<Messages<BalanceEvent>>()
        .write(collision("old-unread"));
    app.world_mut()
        .resource_mut::<NextState<GamePhase>>()
        .set(GamePhase::InProgress);
    app.update();
    assert!(app
        .world()
        .resource::<CollisionHistory>()
        .records()
        .is_empty());
    app.world_mut()
        .resource_mut::<Messages<BalanceEvent>>()
        .write(collision("round-two"));
    app.insert_resource(TimeUpdateStrategy::ManualDuration(STEP));
    app.update();
    assert_eq!(
        app.world()
            .resource::<CollisionHistory>()
            .records()
            .iter()
            .map(|row| row.victim.as_str())
            .collect::<Vec<_>>(),
        vec!["round-two"]
    );
}

#[cfg(all(feature = "headless", not(target_arch = "wasm32")))]
#[test]
fn collision_history_restore_skips_bootstrap_backlog_but_keeps_later_entry_events() {
    fn entry_event(mut events: MessageWriter<BalanceEvent>) {
        events.write(collision("after-restored-entry"));
    }
    let mut app = app();
    app.insert_resource(super::super::telemetry::RunTelemetry::default())
        .add_systems(Last, crate::headless::report::collect_balance_events)
        .add_systems(OnEnter(GamePhase::GameOver), entry_event);
    app.world_mut()
        .resource_mut::<Messages<BalanceEvent>>()
        .write(collision("captured"));
    app.update();
    let mut snapshot = crate::snapshot::capture(app.world());
    assert_eq!(snapshot.collisions.len(), 1);
    app.world_mut()
        .resource_mut::<Messages<BalanceEvent>>()
        .write(collision("bootstrap-unread"));
    // Full ordinary restore replaces both histories and rebases independent
    // cursors. A subsequent GameOver entry event is beyond that frontier.
    snapshot.phase = Some(GamePhase::GameOver);
    let report = crate::snapshot::restore(app.world_mut(), &snapshot);
    assert!(report.is_complete(), "{:?}", report.gaps);
    app.update();
    let history = app.world().resource::<CollisionHistory>().records();
    assert_eq!(
        history
            .iter()
            .map(|row| row.victim.as_str())
            .collect::<Vec<_>>(),
        vec!["captured", "after-restored-entry"]
    );
    let report = app
        .world()
        .resource::<super::super::telemetry::RunTelemetry>();
    let victims: Vec<_> = report
        .balance_events
        .iter()
        .filter_map(|row| match &row.event {
            BalanceEvent::DamageApplied { victim, .. } => Some(victim.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(victims, vec!["captured", "after-restored-entry"]);
}
