use crate::core::balance::{BalanceEvent, VictimKind, WEAPON_KIND_COLLISION};
use crate::core::collision_history::*;
use crate::core::messages::GamePhase;
use bevy::prelude::*;
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

#[cfg(all(feature = "headless", not(target_arch = "wasm32")))]
#[test]
fn collision_history_restore_skips_bootstrap_backlog_but_keeps_later_entry_events() {
    fn entry_event(mut events: MessageWriter<BalanceEvent>) {
        events.write(collision("after-restored-entry"));
    }
    let mut app = app();
    app.insert_resource(crate::core::telemetry::RunTelemetry::default())
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
        .resource::<crate::core::telemetry::RunTelemetry>();
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
