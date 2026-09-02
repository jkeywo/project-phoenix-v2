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
        recovery_generation: 0,
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
fn station_puppet_membership_round_trips_at_the_authoritative_boundary() {
    let target = crate::gm_puppet::StationPuppetTarget::new(
        crate::command_admission::log::ShipKey("ship-player-1".into()),
        crate::core::messages::StationId("captain".into()),
    );
    let mut puppets = crate::gm_puppet::StationPuppets::default();
    puppets.set_operator(target.clone(), "gm-b".into(), true);
    puppets.set_operator(target.clone(), "gm-a".into(), true);

    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.world_mut().insert_resource(SimTick(71));
    app.world_mut().insert_resource(SimulationPaused(false));
    app.world_mut().insert_resource(GmActionJournal::default());
    app.world_mut().insert_resource(puppets.clone());

    let payload = capture(app.world());
    assert_eq!(payload.gm_puppets, puppets);
    app.world_mut()
        .insert_resource(crate::gm_puppet::StationPuppets::default());
    let order = GmActionOrder::new(HostSlot(9), 99);
    let mut pending = crate::gm_puppet::PendingGmStationCommands::default();
    pending.push(crate::gm_puppet::PendingGmStationCommand {
        tick: 999,
        order,
        operator_id: "stale-gm".into(),
        correlation: GmActionId::new("stale-command").unwrap(),
        ship: target.ship.clone(),
        station: target.station.clone(),
        target: crate::core::messages::SystemId("red-alert".into()),
        payload: crate::core::messages::SystemControlPayload::SetRedAlert { active: true },
    });
    app.world_mut().insert_resource(pending);
    let mut activity = crate::gm_puppet::StationPuppetActivity::default();
    activity.push(crate::gm_puppet::StationPuppetActivityEntry {
        tick: 999,
        order,
        operator_id: "stale-gm".into(),
        ship: target.ship,
        station: target.station,
        target: crate::core::messages::SystemId("red-alert".into()),
        action: "SetRedAlert".into(),
    });
    app.world_mut().insert_resource(activity);
    let report = restore(app.world_mut(), &payload);
    assert!(report.is_complete(), "gaps: {:?}", report.gaps);
    assert_eq!(
        app.world().resource::<crate::gm_puppet::StationPuppets>(),
        &puppets
    );
    assert_eq!(capture(app.world()).gm_puppets, puppets);
    assert!(
        app.world()
            .resource::<crate::gm_puppet::PendingGmStationCommands>()
            .entries()
            .is_empty(),
        "commands queued after the saved boundary must not leak through restore"
    );
    assert!(
        app.world()
            .resource::<crate::gm_puppet::StationPuppetActivity>()
            .entries()
            .is_empty(),
        "presentation activity is rebuilt from the restored timeline"
    );
    assert_eq!(
        app.world()
            .resource::<crate::gm_puppet::PreviousStationPuppetTargets>(),
        &crate::gm_puppet::PreviousStationPuppetTargets::from_puppets(&puppets)
    );
}

#[test]
fn a_technical_pause_does_not_rewrite_an_empty_gm_history_during_restore() {
    let mut live = App::new();
    live.add_plugins(MinimalPlugins);
    live.world_mut().insert_resource(SimTick(10));
    live.world_mut().insert_resource(SimulationPaused(true));
    live.world_mut().insert_resource(GmActionJournal::default());

    let payload = capture(live.world());
    assert!(!payload.gm_actions.initial_paused());
    let source_report = restore(live.world_mut(), &payload);
    assert!(
        source_report.is_complete(),
        "source gaps: {:?}",
        source_report.gaps
    );
    let live_digest = crate::sim_digest::world_digest(live.world());

    let mut resumed = App::new();
    resumed.add_plugins(MinimalPlugins);
    resumed.world_mut().insert_resource(SimTick(2));
    resumed.world_mut().insert_resource(SimulationPaused(false));
    resumed
        .world_mut()
        .insert_resource(GmActionJournal::default());
    let report = restore(resumed.world_mut(), &payload);

    assert!(report.is_complete(), "gaps: {:?}", report.gaps);
    assert!(!resumed
        .world()
        .resource::<GmActionJournal>()
        .initial_paused());
    assert_eq!(
        crate::sim_digest::world_digest(resumed.world()),
        live_digest
    );
}

#[test]
fn recovery_immediately_after_a_due_gm_command_preserves_its_effect_and_digest() {
    use crate::core::messages::{AdmittedCommands, StationId, SystemControlPayload, SystemId};
    use crate::ship::control_source::ControlSource;
    use bevy::ecs::system::RunSystemOnce;

    fn bootstrap() -> App {
        let config = crate::ship::config::ShipConfig::from_toml(
            r#"
[[station]]
id = "captain"
name = "Captain"
description = ""
rank = ""
console = "captain.html"

[[station.rating]]
name = "Manual"
automated_systems = []

[[system]]
id = "red-alert"
kind = "red_alert"
station = "captain"
"#,
            &["red_alert"],
        )
        .unwrap();
        let station = StationId("captain".into());
        let mut ratings = crate::ship::components::ActiveStationRatings::default();
        ratings
            .0
            .insert(station, crate::ship::rating::BACKFILL_RATING.into());
        let mut sources = crate::ship::components::ShipSystemControlSources::default();
        sources
            .0
            .set(SystemId("red-alert".into()), ControlSource::Ai);

        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_message::<crate::lobby::server::OutboundMessage>();
        // `capture_entities` deliberately refuses a partial Option-query when
        // any production component type is unregistered. A real boot registers
        // these through its plugins/spawner; this narrow fixture does so
        // explicitly without inventing component values.
        app.world_mut()
            .register_component::<crate::ship::state::ShipPhysics>();
        app.world_mut()
            .register_component::<crate::entities::spawner::EntitySystemHull>();
        app.world_mut()
            .register_component::<crate::ship::state::ShipWeaponsHold>();
        app.world_mut()
            .register_component::<crate::console::command::server::ShipStationStances>();
        app.world_mut().insert_resource(SimTick(42));
        app.world_mut().insert_resource(SimulationPaused(false));
        app.world_mut().insert_resource(GmActionJournal::default());
        app.world_mut()
            .insert_resource(crate::gm_action::GmActionLog::default());
        app.world_mut()
            .insert_resource(crate::gm_puppet::StationPuppets::default());
        app.world_mut()
            .insert_resource(crate::gm_puppet::PendingGmStationCommands::default());
        app.world_mut()
            .insert_resource(crate::gm_puppet::PendingGmStationFeedbackRoutes::default());
        app.world_mut()
            .insert_resource(crate::gm_puppet::StationPuppetActivity::default());
        app.world_mut().spawn((
            crate::server_app::Ship,
            crate::entities::spawner::EntityUuid("player-1".into()),
            crate::ship::components::ShipConfigComponent(config),
            ratings,
            sources,
            AdmittedCommands::default(),
            crate::ship::state::ShipRedAlert(false),
        ));
        app
    }

    let station = StationId("captain".into());
    let ship = crate::command_admission::ShipKey("player-1".into());
    let puppet_target = crate::gm_puppet::StationPuppetTarget::new(ship.clone(), station.clone());
    let mut live = bootstrap();
    live.world_mut()
        .resource_mut::<crate::gm_puppet::StationPuppets>()
        .set_operator(puppet_target, "gm-1".into(), true);
    let grant = crate::gm_action::GmActionGrant {
        from: HostSlot(1),
        sequenced_by: HostSlot(1),
        operator_id: "gm-1".into(),
        correlation: GmActionId::new("recover-due-command").unwrap(),
        recovery_generation: 0,
        apply_tick: 42,
        order: GmActionOrder::new(HostSlot(1), 1),
        action: GmAction::IssueStationCommand {
            ship,
            station,
            target: SystemId("red-alert".into()),
            payload: crate::core::codec::canonical_system_command(
                &SystemControlPayload::SetRedAlert { active: true },
            )
            .unwrap(),
        },
    };
    live.world_mut()
        .resource_mut::<GmActionJournal>()
        .insert(grant)
        .unwrap();

    live.world_mut()
        .run_system_once(crate::gm_action::apply_due_actions)
        .unwrap();
    assert_eq!(
        live.world()
            .resource::<crate::gm_puppet::PendingGmStationCommands>()
            .entries()
            .len(),
        1,
        "the due action is accepted before FixedUpdate delivery",
    );
    assert_eq!(
        live.world()
            .resource::<crate::gm_action::GmActionLog>()
            .entries()[0]
            .outcome,
        crate::gm_action::GmActionOutcome::Pending,
    );
    let boundary = capture(live.world());
    assert!(boundary
        .entities
        .iter()
        .any(|entity| entity.uuid == "player-1"));
    assert_eq!(boundary.gm_station_commands.entries().len(), 1);

    // Restore both sides through the same production walk so bare-App
    // component registration cannot masquerade as a digest difference.
    let report = restore(live.world_mut(), &boundary);
    assert!(report.is_complete(), "source gaps: {:?}", report.gaps);
    let mut recovered = bootstrap();
    let report = restore(recovered.world_mut(), &boundary);
    assert!(report.is_complete(), "recovered gaps: {:?}", report.gaps);
    assert_eq!(
        crate::sim_digest::world_digest(live.world()),
        crate::sim_digest::world_digest(recovered.world()),
        "the accepted-but-undelivered command participates in the recovery digest",
    );

    for app in [&mut live, &mut recovered] {
        app.world_mut()
            .run_system_once(crate::gm_puppet::admit_station_puppet_commands)
            .unwrap();
        app.world_mut()
            .run_system_once(crate::console::captain::handle_set_red_alert)
            .unwrap();
        app.world_mut()
            .run_system_once(crate::gm_puppet::settle_station_puppet_feedback)
            .unwrap();
        app.world_mut().resource_mut::<SimTick>().0 += 1;
    }
    for app in [&mut live, &mut recovered] {
        let mut query = app.world_mut().query::<&crate::ship::state::ShipRedAlert>();
        let red_alert = query.single(app.world()).unwrap();
        assert!(
            red_alert.0,
            "the restored production consumer applies the command"
        );
        assert!(app
            .world()
            .resource::<crate::gm_puppet::PendingGmStationCommands>()
            .entries()
            .is_empty());
    }
    assert_eq!(
        crate::sim_digest::world_digest(live.world()),
        crate::sim_digest::world_digest(recovered.world()),
        "both continuations retain identical effects and digest",
    );
}

#[test]
fn format_17_without_apply_outcomes_or_pending_station_commands_is_refused() {
    let previous = vellum_save::Versions::new(17, SIMULATION_RULES, 0);
    let current = vellum_save::Versions::new(SNAPSHOT_FORMAT, SIMULATION_RULES, 0);
    assert!(matches!(
        previous
            .check(&current)
            .expect_err("format 17 cannot distinguish Applied from an undelivered payload"),
        vellum_save::Moved::Format { .. },
    ));
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
