use super::*;

use crate::command_admission::HostSlot;
use crate::gm_action::{
    GmAction, GmActionGrant, GmActionId, GmActionJournal, GmActionOrder, SimulationPaused,
};

fn grant(
    slot: u32,
    sequence: u64,
    apply_tick: u64,
    correlation: &str,
    active: bool,
) -> GmActionGrant {
    let origin = HostSlot(slot);
    GmActionGrant {
        from: origin,
        sequenced_by: origin,
        operator_id: format!("gm-{slot}"),
        correlation: GmActionId::new(correlation).unwrap(),
        apply_tick,
        order: GmActionOrder::new(origin, sequence),
        action: GmAction::SetSessionPaused { active },
    }
}

#[test]
fn paused_state_and_the_future_gm_frontier_round_trip_together() {
    let mut live = App::new();
    live.add_plugins(MinimalPlugins);
    live.world_mut().insert_resource(SimTick(10));
    live.world_mut().insert_resource(SimulationPaused(true));
    let mut journal = GmActionJournal::default();
    journal.insert(grant(1, 1, 7, "pause", true)).unwrap();
    journal
        .insert(grant(2, 2, 15, "future-resume", false))
        .unwrap();
    journal.restore_applied_frontier(1).unwrap();
    live.world_mut().insert_resource(journal.clone());

    let payload = capture(live.world());
    assert!(payload.paused);
    assert_eq!(payload.gm_actions, journal);
    // Exercise the same restore walk on the source before comparing whole-world
    // digests. In a deliberately bare fixture, initializing restore's systems
    // registers otherwise-absent query component types; that registry shape
    // changes absent-vs-empty markers even though it is not simulation state.
    let source_report = restore(live.world_mut(), &payload);
    assert!(
        source_report.is_complete(),
        "source gaps: {:?}",
        source_report.gaps
    );
    let captured_digest = crate::sim_digest::world_digest(live.world());

    let mut resumed = App::new();
    resumed.add_plugins(MinimalPlugins);
    resumed.world_mut().insert_resource(SimTick(999));
    resumed.world_mut().insert_resource(SimulationPaused(false));
    resumed
        .world_mut()
        .insert_resource(GmActionJournal::default());

    let report = restore(resumed.world_mut(), &payload);
    assert!(report.is_complete(), "gaps: {:?}", report.gaps);
    assert_eq!(
        resumed.world().resource::<GmActionJournal>(),
        &journal,
        "the future resume remains part of the restored idempotency frontier"
    );
    assert!(resumed.world().resource::<SimulationPaused>().0);
    assert!(
        resumed
            .world()
            .resource::<Time<bevy::time::Virtual>>()
            .is_paused(),
        "a restored pause must starve FixedUpdate before this frame can tick"
    );
    assert_eq!(
        resumed
            .world()
            .resource::<crate::gm_action::GmActionLog>()
            .entries()
            .len(),
        1,
        "only the due pause is terminal at the captured boundary"
    );
    assert_eq!(
        crate::sim_digest::world_digest(resumed.world()),
        captured_digest
    );
}

#[test]
fn snapshot_at_an_exact_gm_boundary_preserves_the_still_unapplied_frontier() {
    let mut live = App::new();
    live.add_plugins(MinimalPlugins);
    live.world_mut().insert_resource(SimTick(20));
    live.world_mut().insert_resource(SimulationPaused(false));
    live.world_mut()
        .insert_resource(crate::gm_action::GmActionLog::default());
    let mut journal = GmActionJournal::default();
    journal
        .insert(grant(1, 1, 20, "exact-boundary-pause", true))
        .unwrap();
    live.world_mut().insert_resource(journal);

    let payload = capture(live.world());
    assert_eq!(payload.gm_actions.applied_grants(), 0);

    let mut resumed = App::new();
    resumed.add_plugins(MinimalPlugins);
    let report = restore(resumed.world_mut(), &payload);
    assert!(report.is_complete(), "gaps: {:?}", report.gaps);
    assert_eq!(
        resumed
            .world()
            .resource::<crate::gm_action::GmActionLog>()
            .entries()
            .len(),
        0,
        "FixedLast's newly advanced tick must not make the boundary terminal"
    );
    assert!(!resumed.world().resource::<SimulationPaused>().0);

    resumed.add_systems(PreUpdate, crate::gm_action::apply_due_actions);
    resumed.update();
    assert_eq!(
        resumed
            .world()
            .resource::<GmActionJournal>()
            .applied_grants(),
        1
    );
    assert!(resumed.world().resource::<SimulationPaused>().0);
}

#[test]
fn restoring_running_state_does_not_release_an_unrelated_virtual_time_hold() {
    let payload = PhoenixSnapshot {
        tick: 4,
        paused: false,
        gm_actions: GmActionJournal::default(),
        ..Default::default()
    };
    let mut resumed = App::new();
    resumed.add_plugins(MinimalPlugins);
    resumed
        .world_mut()
        .resource_mut::<Time<bevy::time::Virtual>>()
        .pause();

    let report = restore(resumed.world_mut(), &payload);
    assert!(report.is_complete(), "gaps: {:?}", report.gaps);
    assert!(!resumed.world().resource::<SimulationPaused>().0);
    assert!(
        resumed
            .world()
            .resource::<Time<bevy::time::Virtual>>()
            .is_paused(),
        "snapshot restore must not release a recovery/model/peer hold owned by the next gate"
    );
}
