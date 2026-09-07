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

/// A GM Fire that crossed its apply boundary but whose handler has not run yet
/// survives capture and restore (issue #1301).
///
/// This is the exact cross-schedule gap format 21 exists to close. The journal
/// already says the Fire was Applied, and its idempotency makes a second Fire
/// of a one-shot event a No-op — so a resume that forgot the arm would leave an
/// authored event reported as fired, never run, and permanently unreachable.
#[test]
fn an_armed_gm_event_fire_round_trips_with_its_authored_trigger_table() {
    fn gm_event(
        id: &str,
        condition: crate::world::config::TriggerCondition,
    ) -> crate::world::content::TriggerState {
        let mut trigger = crate::world::config::scripted_trigger(condition);
        trigger.id = Some(id.to_string());
        trigger.gm_controls = Some(crate::world::config::GmEventControls::fire_only(
            id.to_string(),
            format!("world.gm.event.{id}"),
        ));
        crate::world::content::TriggerState {
            trigger,
            fired: false,
            origin_layer: None,
            seen_destroyed: Default::default(),
            last_fired_elapsed: None,
        }
    }
    // One manual event (issue #1301) and one ORDINARY condition-bearing event
    // declaring the same control set (issue #1302), because the armed set is
    // keyed by qualified id and knows nothing about either condition — a
    // capture that started reading `Trigger::condition` would break here.
    fn table() -> crate::world::server::WorldContentRuntime {
        crate::world::server::WorldContentRuntime {
            trigger_states: vec![
                gm_event("breach", crate::world::config::TriggerCondition::Manual),
                gm_event(
                    "sweep",
                    crate::world::config::TriggerCondition::OnDestroyed {
                        entity_name: "courier".to_string(),
                    },
                ),
            ],
            ..Default::default()
        }
    }

    let mut live = App::new();
    live.add_plugins(MinimalPlugins);
    live.world_mut().insert_resource(SimTick(21));
    let mut runtime = table();
    runtime
        .pending_gm_event_fires
        .insert("base-world::sweep".into());
    runtime
        .pending_gm_event_fires
        .insert("base-world::breach".into());
    live.world_mut().insert_resource(runtime);

    let payload = capture(live.world());
    let scenario = payload.scenario.as_ref().expect("a world was loaded");
    assert_eq!(
        scenario.pending_gm_event_fires,
        vec![
            "base-world::breach".to_string(),
            "base-world::sweep".to_string()
        ],
        "each armed Fire is captured, sorted, by its layer-qualified id — the \
         manual event and the automatic one on identical terms"
    );

    let mut resumed = App::new();
    resumed.add_plugins(MinimalPlugins);
    resumed.world_mut().insert_resource(SimTick(999));
    // A freshly-loaded world rebuilds the same table with nothing armed, which
    // is what makes the restored reading unambiguous evidence.
    resumed.world_mut().insert_resource(table());
    assert!(resumed
        .world()
        .resource::<crate::world::server::WorldContentRuntime>()
        .pending_gm_event_fires
        .is_empty());

    let report = restore(resumed.world_mut(), &payload);
    assert!(report.is_complete(), "gaps: {:?}", report.gaps);
    assert_eq!(
        resumed
            .world()
            .resource::<crate::world::server::WorldContentRuntime>()
            .pending_gm_event_fires
            .iter()
            .cloned()
            .collect::<Vec<_>>(),
        vec![
            "base-world::breach".to_string(),
            "base-world::sweep".to_string()
        ],
    );

    // A world with no armed Fire writes the field out of the payload entirely,
    // which is what keeps every pre-#1301 scenario's capture byte-identical.
    let mut idle = App::new();
    idle.add_plugins(MinimalPlugins);
    idle.world_mut().insert_resource(SimTick(21));
    idle.world_mut().insert_resource(table());
    assert!(capture(idle.world())
        .scenario
        .expect("a world was loaded")
        .pending_gm_event_fires
        .is_empty());
}

/// An armed GM PLACEMENT survives the boundary between its `PreUpdate` apply
/// and the `FixedUpdate` spawn that performs it (issue #1305), in the order
/// that decides which placement draws which `WorldIdMint` id.
///
/// Losing it is worse than losing an armed Fire: each press is its own
/// correlation rather than an idempotent latch, so a GM trying again after a
/// resume would risk TWO hulls rather than recovering the one they were told
/// they placed.
#[test]
fn armed_gm_placements_round_trip_in_their_canonical_order() {
    fn palette() -> crate::world::server::WorldContentRuntime {
        crate::world::server::WorldContentRuntime {
            gm_palette: vec![crate::world::config::GmPaletteEntry {
                id: "raider".into(),
                label: "world.gm.palette.raider.label".into(),
                template_path: "assets/entities/ship_harrow_destroyer.toml".into(),
                ..Default::default()
            }],
            ..Default::default()
        }
    }
    fn armed(name: &str, x_mm: i64) -> crate::gm_spawn::PendingGmSpawn {
        crate::gm_spawn::PendingGmSpawn {
            palette: "raider".into(),
            variant: None,
            name: name.into(),
            position_mm: [x_mm, 0, -40_000],
            heading_mdeg: 90_000,
        }
    }

    let mut live = App::new();
    live.add_plugins(MinimalPlugins);
    live.world_mut().insert_resource(SimTick(33));
    let mut runtime = palette();
    runtime.pending_gm_spawns = vec![armed("raider_2", 200_000), armed("raider_1", 100_000)];
    live.world_mut().insert_resource(runtime);

    let payload = capture(live.world());
    let scenario = payload.scenario.as_ref().expect("a world was loaded");
    assert_eq!(
        scenario
            .pending_gm_spawns
            .iter()
            .map(|pending| pending.name.as_str())
            .collect::<Vec<_>>(),
        vec!["raider_2", "raider_1"],
        "captured in the runtime's own order, never sorted: the order is the identity"
    );

    let mut resumed = App::new();
    resumed.add_plugins(MinimalPlugins);
    resumed.world_mut().insert_resource(SimTick(999));
    // A freshly-loaded world rebuilds the same palette with nothing armed,
    // which is what makes the restored reading unambiguous evidence.
    resumed.world_mut().insert_resource(palette());
    assert!(resumed
        .world()
        .resource::<crate::world::server::WorldContentRuntime>()
        .pending_gm_spawns
        .is_empty());

    let report = restore(resumed.world_mut(), &payload);
    assert!(report.is_complete(), "gaps: {:?}", report.gaps);
    assert_eq!(
        resumed
            .world()
            .resource::<crate::world::server::WorldContentRuntime>()
            .pending_gm_spawns,
        vec![armed("raider_2", 200_000), armed("raider_1", 100_000)],
    );

    // A world with nothing armed writes the field out of the payload entirely,
    // which is what keeps every pre-#1305 scenario's capture byte-identical.
    let mut idle = App::new();
    idle.add_plugins(MinimalPlugins);
    idle.world_mut().insert_resource(SimTick(33));
    idle.world_mut().insert_resource(palette());
    assert!(capture(idle.world())
        .scenario
        .expect("a world was loaded")
        .pending_gm_spawns
        .is_empty());
}

/// Issue #1303: which events a GM has PAUSED crosses a save, and a world that
/// has paused nothing writes the field out of the payload entirely.
///
/// It has to travel for the armed-Fire test's reason with the sign reversed. A
/// paused event's condition is not evaluated at all, so a resume that forgot
/// this field silently starts evaluating conditions the GM stopped — while the
/// journal in the same payload still says the Pause was Applied, which makes a
/// second Pause a No-op and leaves the GM no lever that can change either fact.
#[test]
fn paused_gm_events_round_trip_with_their_authored_trigger_table() {
    fn pausable(
        id: &str,
        condition: crate::world::config::TriggerCondition,
    ) -> crate::world::content::TriggerState {
        let mut trigger = crate::world::config::scripted_trigger(condition);
        trigger.id = Some(id.to_string());
        let mut controls = crate::world::config::GmEventControls::fire_only(
            id.to_string(),
            format!("world.gm.event.{id}"),
        );
        controls.pause = true;
        trigger.gm_controls = Some(controls);
        crate::world::content::TriggerState {
            trigger,
            fired: false,
            origin_layer: None,
            seen_destroyed: Default::default(),
            last_fired_elapsed: None,
        }
    }
    fn table() -> crate::world::server::WorldContentRuntime {
        crate::world::server::WorldContentRuntime {
            trigger_states: vec![
                pausable("breach", crate::world::config::TriggerCondition::Manual),
                pausable(
                    "sweep",
                    crate::world::config::TriggerCondition::OnDestroyed {
                        entity_name: "courier".to_string(),
                    },
                ),
            ],
            ..Default::default()
        }
    }

    let mut live = App::new();
    live.add_plugins(MinimalPlugins);
    live.world_mut().insert_resource(SimTick(21));
    let mut runtime = table();
    runtime.paused_gm_events.insert("base-world::sweep".into());
    runtime.paused_gm_events.insert("base-world::breach".into());
    live.world_mut().insert_resource(runtime);

    let payload = capture(live.world());
    let scenario = payload.scenario.as_ref().expect("a world was loaded");
    assert_eq!(
        scenario.paused_gm_events,
        vec![
            "base-world::breach".to_string(),
            "base-world::sweep".to_string()
        ],
    );

    let mut resumed = App::new();
    resumed.add_plugins(MinimalPlugins);
    resumed.world_mut().insert_resource(SimTick(999));
    // A freshly-loaded world pauses nothing, which is what makes the restored
    // reading unambiguous evidence rather than a value that was already there.
    resumed.world_mut().insert_resource(table());
    assert!(resumed
        .world()
        .resource::<crate::world::server::WorldContentRuntime>()
        .paused_gm_events
        .is_empty());

    let report = restore(resumed.world_mut(), &payload);
    assert!(report.is_complete(), "gaps: {:?}", report.gaps);
    assert_eq!(
        resumed
            .world()
            .resource::<crate::world::server::WorldContentRuntime>()
            .paused_gm_events
            .iter()
            .cloned()
            .collect::<Vec<_>>(),
        vec![
            "base-world::breach".to_string(),
            "base-world::sweep".to_string()
        ],
    );

    let mut idle = App::new();
    idle.add_plugins(MinimalPlugins);
    idle.world_mut().insert_resource(SimTick(21));
    idle.world_mut().insert_resource(table());
    assert!(capture(idle.world())
        .scenario
        .expect("a world was loaded")
        .paused_gm_events
        .is_empty());
}

/// A GM Skip that crossed its apply boundary and is still waiting for the
/// occurrence it stands in front of survives capture and restore (issue #1304).
///
/// Format 25's reason, and the sharper half of format 21's: a Skip arm is
/// waiting on the WORLD, so it routinely outlives any number of saves, and a
/// resume that forgot one runs in full the occurrence the GM decided the crew
/// would not see -- with the journal still reporting the arm Applied and no
/// re-press able to undo a moment that has already passed.
///
/// It is captured beside the Fire set and never merged with it: the two levers
/// are opposite futures for one event, so the payload has to be able to say
/// that `evac` is armed for Skip while `breach` is armed for Fire.
#[test]
fn an_armed_gm_event_skip_round_trips_beside_an_armed_fire() {
    fn gm_event(id: &str) -> crate::world::content::TriggerState {
        let mut trigger = crate::world::config::scripted_trigger(
            crate::world::config::TriggerCondition::OnDestroyed {
                entity_name: "courier".to_string(),
            },
        );
        trigger.id = Some(id.to_string());
        let mut controls = crate::world::config::GmEventControls::fire_only(
            id.to_string(),
            format!("world.gm.event.{id}"),
        );
        controls.skip = true;
        trigger.gm_controls = Some(controls);
        crate::world::content::TriggerState {
            trigger,
            fired: false,
            origin_layer: None,
            seen_destroyed: Default::default(),
            last_fired_elapsed: None,
        }
    }
    fn table() -> crate::world::server::WorldContentRuntime {
        crate::world::server::WorldContentRuntime {
            trigger_states: vec![gm_event("breach"), gm_event("evac"), gm_event("sweep")],
            ..Default::default()
        }
    }

    let mut live = App::new();
    live.add_plugins(MinimalPlugins);
    live.world_mut().insert_resource(SimTick(21));
    let mut runtime = table();
    runtime
        .pending_gm_event_fires
        .insert("base-world::breach".into());
    runtime
        .pending_gm_event_skips
        .insert("base-world::sweep".into());
    runtime
        .pending_gm_event_skips
        .insert("base-world::evac".into());
    live.world_mut().insert_resource(runtime);

    let payload = capture(live.world());
    let scenario = payload.scenario.as_ref().expect("a world was loaded");
    assert_eq!(
        scenario.pending_gm_event_skips,
        vec![
            "base-world::evac".to_string(),
            "base-world::sweep".to_string()
        ],
        "each armed Skip is captured, sorted, by its layer-qualified id"
    );
    assert_eq!(
        scenario.pending_gm_event_fires,
        vec!["base-world::breach".to_string()],
        "and the Fire set is untouched by it"
    );

    let mut resumed = App::new();
    resumed.add_plugins(MinimalPlugins);
    resumed.world_mut().insert_resource(SimTick(999));
    resumed.world_mut().insert_resource(table());
    assert!(resumed
        .world()
        .resource::<crate::world::server::WorldContentRuntime>()
        .pending_gm_event_skips
        .is_empty());

    let report = restore(resumed.world_mut(), &payload);
    assert!(report.is_complete(), "gaps: {:?}", report.gaps);
    let restored = resumed
        .world()
        .resource::<crate::world::server::WorldContentRuntime>();
    assert_eq!(
        restored
            .pending_gm_event_skips
            .iter()
            .cloned()
            .collect::<Vec<_>>(),
        vec![
            "base-world::evac".to_string(),
            "base-world::sweep".to_string()
        ],
    );
    assert_eq!(
        restored
            .pending_gm_event_fires
            .iter()
            .cloned()
            .collect::<Vec<_>>(),
        vec!["base-world::breach".to_string()],
        "the two levers come back apart, not merged"
    );

    // A world with no armed Skip writes the field out of the payload entirely,
    // which is what keeps every pre-#1304 scenario's capture byte-identical.
    let mut idle = App::new();
    idle.add_plugins(MinimalPlugins);
    idle.world_mut().insert_resource(SimTick(21));
    idle.world_mut().insert_resource(table());
    assert!(capture(idle.world())
        .scenario
        .expect("a world was loaded")
        .pending_gm_event_skips
        .is_empty());
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
fn format_21_without_the_armed_direct_effects_is_refused() {
    let previous = vellum_save::Versions::new(21, SIMULATION_RULES, 0);
    let current = vellum_save::Versions::new(SNAPSHOT_FORMAT, SIMULATION_RULES, 0);
    assert!(matches!(
        previous
            .check(&current)
            .expect_err("format 21 records an Applied hit nothing will ever land"),
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

/// An armed GM direct effect crosses the capture boundary and lands EXACTLY
/// once, identically, on the live world and on a fresh restore (issue #1310).
///
/// This is the cross-schedule gap format 22 exists for: the grant is already
/// Applied in the journal — with an exact hull amount and a lethal flag on its
/// durable result — but `SimSet::Damage` has not run yet, so a capture that
/// dropped the arm would resume a world claiming damage nobody will ever apply.
#[test]
fn an_armed_direct_effect_survives_capture_and_lands_once_on_both_sides() {
    use bevy::ecs::system::RunSystemOnce;

    fn bootstrap() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_message::<crate::lobby::server::OutboundMessage>();
        app.world_mut()
            .register_component::<crate::ship::state::ShipPhysics>();
        app.world_mut()
            .register_component::<crate::entities::spawner::EntitySystemHull>();
        app.world_mut()
            .register_component::<crate::console::command::server::ShipStationStances>();
        app.world_mut()
            .register_component::<crate::ship_plugin::ShipSystemControlSources>();
        app.world_mut()
            .register_component::<crate::ship::state::ShipRedAlert>();
        app.world_mut().insert_resource(SimTick(42));
        app.world_mut().insert_resource(SimulationPaused(false));
        app.world_mut().insert_resource(GmActionJournal::default());
        app.world_mut()
            .insert_resource(crate::gm_action::GmActionLog::default());
        app.world_mut()
            .insert_resource(crate::gm_effect::PendingGmDirectEffects::default());
        app.world_mut().insert_resource(crate::sim_rng::SimRng::new(
            4242,
            crate::sim_rng::SeedSource::Cli,
        ));
        let system = SystemId("captain".into());
        app.world_mut().spawn((
            crate::entities::spawner::EntityUuid("npc-1".into()),
            crate::entities::spawner::EntitySystemHull(
                crate::ship::damage::SystemHull::from_config(&[(system, 100.0)]),
            ),
        ));
        app
    }

    let mut live = bootstrap();
    live.world_mut()
        .resource_mut::<GmActionJournal>()
        .insert(crate::gm_action::GmActionGrant {
            from: HostSlot(1),
            sequenced_by: HostSlot(1),
            operator_id: "gm-1".into(),
            correlation: GmActionId::new("recover-due-effect").unwrap(),
            recovery_generation: 0,
            apply_tick: 42,
            order: GmActionOrder::new(HostSlot(1), 1),
            action: GmAction::ApplyDirectEffect {
                target: "npc-1".into(),
                scope: crate::gm_effect::GmDirectEffectScope::Entity,
                effect: crate::gm_effect::GmDirectEffectKind::Damage,
                amount_milli_hp: 40_000,
            },
        })
        .unwrap();
    live.world_mut()
        .run_system_once(crate::gm_action::apply_due_actions)
        .unwrap();
    assert_eq!(
        live.world()
            .resource::<crate::gm_effect::PendingGmDirectEffects>()
            .entries()
            .len(),
        1,
        "the due effect is resolved before the damage phase runs",
    );

    let boundary = capture(live.world());
    assert!(
        boundary
            .entities
            .iter()
            .any(|entity| entity.uuid == "npc-1"),
        "captured entities: {:?}",
        boundary
            .entities
            .iter()
            .map(|entity| entity.uuid.clone())
            .collect::<Vec<_>>()
    );
    assert_eq!(boundary.gm_direct_effects.entries().len(), 1);

    let report = restore(live.world_mut(), &boundary);
    assert!(report.is_complete(), "source gaps: {:?}", report.gaps);
    let mut recovered = bootstrap();
    let report = restore(recovered.world_mut(), &boundary);
    assert!(report.is_complete(), "recovered gaps: {:?}", report.gaps);
    assert_eq!(
        crate::sim_digest::world_digest(live.world()),
        crate::sim_digest::world_digest(recovered.world()),
        "the resolved-but-unapplied effect participates in the recovery digest",
    );

    for app in [&mut live, &mut recovered] {
        app.world_mut()
            .run_system_once(crate::gm_effect::apply_gm_direct_effects)
            .unwrap();
    }
    for app in [&mut live, &mut recovered] {
        let mut query = app
            .world_mut()
            .query::<&crate::entities::spawner::EntitySystemHull>();
        let hull = query.single(app.world()).unwrap();
        assert!(
            (hull.0.total_current() - 60.0).abs() < 0.01,
            "the restored damage phase applies the resolved amount exactly once"
        );
        assert!(app
            .world()
            .resource::<crate::gm_effect::PendingGmDirectEffects>()
            .is_empty());
    }
    assert_eq!(
        crate::sim_digest::world_digest(live.world()),
        crate::sim_digest::world_digest(recovered.world()),
        "both continuations retain identical hulls and digest",
    );
}

/// The SCOPE of an armed effect crosses the capture boundary with it, so a
/// resumed peer damages the Station the grant named rather than the whole hull
/// (issue #1311, snapshot format 26).
///
/// This is the fact format 26 exists for. The amount alone survived a format-25
/// record; a restore that defaulted the scope back to `Entity` would land the
/// same 25 points across every System on the ship, and the live peer and the
/// resumed one would then disagree about a hull neither could explain.
#[test]
fn an_armed_scoped_effect_restores_against_the_same_station_it_named() {
    use bevy::ecs::system::RunSystemOnce;

    fn bootstrap() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_message::<crate::lobby::server::OutboundMessage>();
        app.world_mut()
            .register_component::<crate::ship::state::ShipPhysics>();
        app.world_mut()
            .register_component::<crate::entities::spawner::EntitySystemHull>();
        app.world_mut()
            .register_component::<crate::console::command::server::ShipStationStances>();
        app.world_mut()
            .register_component::<crate::ship_plugin::ShipSystemControlSources>();
        app.world_mut()
            .register_component::<crate::ship::state::ShipRedAlert>();
        app.world_mut().insert_resource(SimTick(42));
        app.world_mut().insert_resource(SimulationPaused(false));
        app.world_mut().insert_resource(GmActionJournal::default());
        app.world_mut()
            .insert_resource(crate::gm_action::GmActionLog::default());
        app.world_mut()
            .insert_resource(crate::gm_effect::PendingGmDirectEffects::default());
        app.world_mut().insert_resource(crate::sim_rng::SimRng::new(
            4242,
            crate::sim_rng::SeedSource::Cli,
        ));
        app.world_mut().spawn((
            crate::entities::spawner::EntityUuid("npc-1".into()),
            crate::entities::spawner::EntitySystemHull(
                crate::ship::damage::SystemHull::from_config(&[
                    (SystemId("impulse-drive".into()), 50.0),
                    (SystemId("phaser-bank".into()), 50.0),
                ]),
            ),
            crate::ship::components::ShipConfigComponent(
                toml::from_str(
                    r#"
[[station]]
id = "helm"
name = "station.helm.display_name"
description = "station.helm.description"
rank = "Lieutenant"

[[station]]
id = "tactical"
name = "station.tactical.display_name"
description = "station.tactical.description"
rank = "Lieutenant"

[[system]]
id = "impulse-drive"
kind = "impulse-drive"
station = "helm"

[[system]]
id = "phaser-bank"
kind = "phaser-bank"
station = "tactical"
"#,
                )
                .expect("a well-formed authoring fixture"),
            ),
        ));
        app
    }

    let mut live = bootstrap();
    live.world_mut()
        .resource_mut::<GmActionJournal>()
        .insert(crate::gm_action::GmActionGrant {
            from: HostSlot(1),
            sequenced_by: HostSlot(1),
            operator_id: "gm-1".into(),
            correlation: GmActionId::new("recover-scoped-effect").unwrap(),
            recovery_generation: 0,
            apply_tick: 42,
            order: GmActionOrder::new(HostSlot(1), 1),
            action: GmAction::ApplyDirectEffect {
                target: "npc-1".into(),
                scope: crate::gm_effect::GmDirectEffectScope::Station(
                    crate::core::messages::StationId("helm".into()),
                ),
                effect: crate::gm_effect::GmDirectEffectKind::Damage,
                amount_milli_hp: 25_000,
            },
        })
        .unwrap();
    live.world_mut()
        .run_system_once(crate::gm_action::apply_due_actions)
        .unwrap();

    let boundary = capture(live.world());
    assert_eq!(
        boundary
            .gm_direct_effects
            .entries()
            .iter()
            .map(|effect| effect.scope.clone())
            .collect::<Vec<_>>(),
        vec![crate::gm_effect::GmDirectEffectScope::Station(
            crate::core::messages::StationId("helm".into())
        )],
        "the capture carries WHERE the effect lands, not only how much",
    );
    assert_eq!(
        boundary
            .gm_actions
            .applied_results()
            .iter()
            .map(|result| result.effect_scope.clone())
            .collect::<Vec<_>>(),
        vec![Some(crate::gm_effect::GmDirectEffectScope::Station(
            crate::core::messages::StationId("helm".into())
        ))],
        "and so does the durable fact the resumed feed re-renders",
    );

    let report = restore(live.world_mut(), &boundary);
    assert!(report.is_complete(), "source gaps: {:?}", report.gaps);
    let mut recovered = bootstrap();
    let report = restore(recovered.world_mut(), &boundary);
    assert!(report.is_complete(), "recovered gaps: {:?}", report.gaps);

    for app in [&mut live, &mut recovered] {
        app.world_mut()
            .run_system_once(crate::gm_effect::apply_gm_direct_effects)
            .unwrap();
    }
    for app in [&mut live, &mut recovered] {
        let mut query = app
            .world_mut()
            .query::<&crate::entities::spawner::EntitySystemHull>();
        let hull = query.single(app.world()).unwrap();
        assert!(
            (hull
                .0
                .current_for(&SystemId("impulse-drive".into()))
                .unwrap()
                - 25.0)
                .abs()
                < 0.01,
            "the resumed damage phase restricts to the Station the grant named"
        );
        assert!(
            (hull.0.current_for(&SystemId("phaser-bank".into())).unwrap() - 50.0).abs() < 0.01,
            "a resumed scoped hit still cannot reach another Station's System"
        );
    }
    assert_eq!(
        crate::sim_digest::world_digest(live.world()),
        crate::sim_digest::world_digest(recovered.world()),
        "both continuations retain identical hulls and digest",
    );
}
