use super::application::apply_repair;
use super::capture::{capture_entities, repair_state};
use super::*;
use crate::sim_rng::InstallSimRng;

use crate::command_admission::HostSlot;
use crate::gm::actions::action::{
    GmAction, GmActionGrant, GmActionId, GmActionJournal, GmActionOrder, SimulationPaused,
};

fn resource_row(system: &TorpedoSystemResource) -> EntityState {
    EntityState {
        uuid: "resource-ship".into(),
        weapons: Some(super::capture::weapon_state(
            None,
            None,
            Some(system),
            None,
            None,
            None,
        )),
        ..Default::default()
    }
}

#[test]
fn reserved_ammunition_and_fractional_flow_resume_from_ron_without_reset() {
    let config = crate::weapons::torpedo::TorpedoConfig {
        count: 4,
        load_time: 2.0,
        ..Default::default()
    };
    let mut original =
        TorpedoSystemResource(crate::weapons::torpedo::TorpedoSystem::new(config.clone()));
    assert!(original.0.start_load("fore_port"));
    original
        .0
        .tick(0.5, &std::collections::HashMap::new(), &mut || {
            "unused".into()
        });
    let stored: EntityState =
        ron::from_str(&ron::to_string(&resource_row(&original)).unwrap()).unwrap();
    let mut world = World::new();
    let entity = world
        .spawn(TorpedoSystemResource(
            crate::weapons::torpedo::TorpedoSystem::new(config),
        ))
        .id();
    super::preflight::validate_entity_resources(&world, entity, &stored).unwrap();
    super::application::apply_weapons(
        &mut world.entity_mut(entity),
        stored.weapons.as_ref().unwrap(),
    );
    let mut resumed = world.get::<TorpedoSystemResource>(entity).unwrap().clone();
    assert_eq!(original.0.magazine, resumed.0.magazine);
    assert_eq!(resumed.0.torpedoes_remaining(), 3);
    for _ in 0..4 {
        original
            .0
            .tick(0.5, &std::collections::HashMap::new(), &mut || {
                "unused".into()
            });
        resumed
            .0
            .tick(0.5, &std::collections::HashMap::new(), &mut || {
                "unused".into()
            });
        assert_eq!(
            resource_row(&original).weapons,
            resource_row(&resumed).weapons
        );
    }
    let flow = crate::umbilical::UmbilicalSaveState {
        running: true,
        carry: 0.75,
    };
    let flow: crate::umbilical::UmbilicalSaveState =
        ron::from_str(&ron::to_string(&flow).unwrap()).unwrap();
    let mut restored = crate::umbilical::TransferUmbilical::new(
        crate::umbilical::UmbilicalConfig {
            capacity: "torpedoes".into(),
            link: Default::default(),
            rate: 1.0,
            direction: crate::umbilical::UmbilicalDirection::Collect,
            min_power_level: 1,
        },
        crate::core::messages::PowerGroupId("umbilical".into()),
    );
    restored.restore(&flow);
    assert_eq!(restored.save_state(), flow);
}

#[test]
fn resource_preflight_refuses_orphan_overfilled_and_missing_claim_state_before_writes() {
    let system = TorpedoSystemResource(crate::weapons::torpedo::TorpedoSystem::new(
        crate::weapons::torpedo::TorpedoConfig {
            count: 10,
            ..Default::default()
        },
    ));
    let mut world = World::new();
    let entity = world.spawn((system.clone(), ShipRedAlert(false))).id();
    for failure in 0..5 {
        let mut row = resource_row(&system);
        row.red_alert = Some(true);
        let weapons = row.weapons.as_mut().unwrap();
        match failure {
            0 => {
                weapons.magazine.as_mut().unwrap().level = 5;
                weapons.tubes[0].loaded_count = 5;
            }
            1 => weapons.tubes[0].target_count = 5,
            2 => {
                weapons
                    .magazine
                    .as_mut()
                    .unwrap()
                    .reserved
                    .insert("orphan".into(), 1);
            }
            3 => weapons.magazine = None,
            _ => weapons.tubes[0].load_timer = [2.0, 1.0],
        }
        assert!(super::preflight::validate_entity_resources(&world, entity, &row).is_err());
        apply_entity_state(&mut world, entity, &row, 0);
        assert!(!world.get::<ShipRedAlert>(entity).unwrap().0);
        assert_eq!(
            world
                .get::<TorpedoSystemResource>(entity)
                .unwrap()
                .0
                .magazine,
            system.0.magazine
        );
    }
}

#[test]
fn repair_jobs_resume_return_queue_and_external_commitment_from_ron() {
    use crate::modifiers::repair_teams::{RepairTeams, RepairTimings};
    let timings = RepairTimings {
        travel_duration: 2.0,
        repair_rate_hp_per_sec: 1.0,
    };
    let mut live = RepairTeams::new_with_timings(2, timings);
    let mut hull = crate::ship::damage::SystemHull::from_config(&[
        (SystemId("first".into()), 10.0),
        (SystemId("second".into()), 10.0),
    ]);
    hull.set_hp(&SystemId("first".into()), 1.0);
    hull.set_hp(&SystemId("second".into()), 2.0);
    live.dispatch(0, SystemId("first".into()), "First".into());
    live.tick(1.0, &mut hull, None);
    live.dispatch(0, SystemId("second".into()), "Second".into());
    assert!(live.claim_external(1, "ally".into()));
    assert!(live.step_external());
    let stored = repair_state(Some(&ShipRepairTeams(live.clone())), None, None);
    let stored: RepairState = ron::from_str(&ron::to_string(&stored).unwrap()).unwrap();
    let mut target = World::new();
    let entity = target
        .spawn(ShipRepairTeams(RepairTeams::new_with_timings(2, timings)))
        .id();
    apply_repair(&mut target.entity_mut(entity), &stored);
    let mut resumed = target.get::<ShipRepairTeams>(entity).unwrap().0.clone();
    assert_eq!(resumed.save_state(), live.save_state());
    assert!(resumed.free_team_indices(None).is_empty());
    let mut resumed_hull = hull.clone();
    for _ in 0..4 {
        live.tick(1.0, &mut hull, None);
        resumed.tick(1.0, &mut resumed_hull, None);
        assert_eq!(live.step_external(), resumed.step_external());
        assert_eq!(live.save_state(), resumed.save_state());
        assert_eq!(
            hull.current_for(&SystemId("second".into())),
            resumed_hull.current_for(&SystemId("second".into()))
        );
    }
    assert_eq!(resumed.external_target(), Some("ally"));
}

#[test]
fn repair_job_preflight_refuses_invalid_identity_before_entity_writes() {
    use crate::modifiers::repair_teams::RepairTeams;
    let mut teams = RepairTeams::new(1);
    teams.dispatch(0, SystemId("engine".into()), "Engine".into());
    let mut world = sparse_capture_world(None);
    world.register_component::<ActiveBeam>();
    world.register_component::<PhaserCooldown>();
    world.register_component::<TorpedoSystemResource>();
    world.register_component::<EntityShipArcHull>();
    world.register_component::<RepairRequestQueue>();
    world.register_component::<RepairHumanAlerted>();
    world.register_component::<crate::server_app::ShipSystemBlackboards>();
    world.register_component::<crate::ai::server::ObjectiveCursors>();
    world.register_component::<crate::console::weapons::blaster::BlasterSystemResource>();
    world.register_component::<crate::ship::shields::ShipShields>();
    let entity = world
        .spawn((
            EntityUuid("repairer".into()),
            ShipRepairTeams(teams),
            ShipRedAlert(false),
        ))
        .id();
    let before = world.get::<ShipRepairTeams>(entity).unwrap().0.save_state();
    let mut row = capture_entities(&world).remove(0);
    row.repair.as_mut().unwrap().jobs.as_mut().unwrap().teams[0]
        .as_mut()
        .unwrap()
        .id
        .0 = 0;
    assert!(validate_entity_repair_jobs(&world, entity, &row).is_err());
    apply_entity_state(&mut world, entity, &row, 0);
    assert_eq!(
        world.get::<ShipRepairTeams>(entity).unwrap().0.save_state(),
        before
    );
    assert!(!world.get::<ShipRedAlert>(entity).unwrap().0);
}

fn sparse_capture_world(missing: Option<usize>) -> World {
    let mut world = World::new();
    if missing != Some(0) {
        world.register_component::<ShipPhysics>();
    }
    if missing != Some(1) {
        world.register_component::<EntitySystemHull>();
    }
    if missing != Some(2) {
        world.register_component::<ShipRedAlert>();
    }
    if missing != Some(3) {
        world.register_component::<crate::console::command::server::ShipStationStances>();
    }
    if missing != Some(4) {
        world.register_component::<crate::ship_plugin::ShipSystemControlSources>();
    }
    world
}

#[test]
fn multi_ship_frozen_backfill_and_absent_survive_snapshot_replay_without_reevaluation() {
    use crate::ship_slots::{FrozenShipSlots, LaunchSource, LaunchedSlot};

    // `reserve` was authored Absent and is therefore deliberately not in the
    // launch artifact. `wing` was Backfill and must remain present even if a
    // continuation's live authoring/configuration would now say otherwise.
    let frozen = FrozenShipSlots(vec![LaunchedSlot {
        slot_id: "wing".into(),
        hull: "destroyer".into(),
        claimant: None,
        source: LaunchSource::Backfill,
    }]);
    let mut source = App::new();
    source.insert_resource(frozen.clone());
    let encoded = serde_json::to_vec(&capture(source.world())).unwrap();
    let saved: PhoenixSnapshot = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(saved.frozen_ship_slots, Some(frozen.clone()));

    let mut resumed = App::new();
    resumed.insert_resource(FrozenShipSlots(vec![LaunchedSlot {
        slot_id: "reserve".into(),
        hull: "scout".into(),
        claimant: None,
        source: LaunchSource::Backfill,
    }]));
    let _ = restore(resumed.world_mut(), &saved);
    assert_eq!(resumed.world().resource::<FrozenShipSlots>(), &frozen);
    assert!(resumed
        .world()
        .resource::<FrozenShipSlots>()
        .0
        .iter()
        .all(|row| row.slot_id != "reserve"));
}

#[test]
fn restored_objective_history_is_silent_on_continuation_but_new_transitions_remain_visible() {
    use crate::core::narrative::{NarrativeEvent, NarrativeKind};
    use crate::world::server::ObjectiveManagerRes;
    let mut source = App::new();
    source
        .add_plugins(MinimalPlugins)
        .init_resource::<ObjectiveManagerRes>();
    {
        let mut objectives = source.world_mut().resource_mut::<ObjectiveManagerRes>();
        for id in ["completed", "failed", "active"] {
            objectives.0.add(id, "objective.text", false, vec![]);
        }
        objectives.0.complete("completed");
        objectives.0.fail("failed");
        objectives.0.set_recipients("active", vec!["ship-a".into()]);
    }
    let saved = capture(source.world());
    for warm_observer in [false, true] {
        let mut target = App::new();
        target
            .add_plugins(MinimalPlugins)
            .init_resource::<ObjectiveManagerRes>()
            .add_message::<NarrativeEvent>()
            .add_systems(Update, crate::narrative::emit_scenario_narrative);
        if warm_observer {
            target
                .world_mut()
                .resource_mut::<ObjectiveManagerRes>()
                .0
                .add("completed", "old.text", false, vec![]);
            target.update();
            target
                .world_mut()
                .resource_mut::<Messages<NarrativeEvent>>()
                .clear();
        }
        restore(target.world_mut(), &saved);
        assert_eq!(
            target.world().resource::<ObjectiveManagerRes>().0.records(),
            source.world().resource::<ObjectiveManagerRes>().0.records()
        );
        target.update();
        assert!(
            target
                .world_mut()
                .resource_mut::<Messages<NarrativeEvent>>()
                .drain()
                .next()
                .is_none(),
            "restored historical states must not become new events; warm={warm_observer}"
        );
        // A real change after restoration is still reported once.
        target
            .world_mut()
            .resource_mut::<ObjectiveManagerRes>()
            .0
            .complete("active");
        target.update();
        let events: Vec<_> = target
            .world_mut()
            .resource_mut::<Messages<NarrativeEvent>>()
            .drain()
            .collect();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, NarrativeKind::ObjectiveCompleted);
        assert_eq!(events[0].id, "active");
        target.update();
        assert!(target
            .world_mut()
            .resource_mut::<Messages<NarrativeEvent>>()
            .drain()
            .next()
            .is_none());
        // A real transition before the first continuation also survives rebasing.
        restore(target.world_mut(), &saved);
        target
            .world_mut()
            .resource_mut::<ObjectiveManagerRes>()
            .0
            .fail("active");
        target.update();
        let events: Vec<_> = target
            .world_mut()
            .resource_mut::<Messages<NarrativeEvent>>()
            .drain()
            .collect();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, NarrativeKind::ObjectiveFailed);
        assert_eq!(events[0].id, "active");
    }
}

#[test]
fn objective_records_round_trip_and_change_digest_without_journal_or_story_beats() {
    use crate::core::messages::{AiDirective, ObjectiveSource};
    use crate::objectives::{ObjectiveManager, UtilityConfig};
    use crate::world::server::ObjectiveManagerRes;
    let mut live = App::new();
    live.add_plugins(MinimalPlugins);
    let empty_digest = crate::sim_digest::world_digest(live.world());
    live.insert_resource(ObjectiveManagerRes::default());
    assert_eq!(
        empty_digest,
        crate::sim_digest::world_digest(live.world()),
        "empty manager preserves the objective-free fold"
    );
    let mut manager = ObjectiveManager::new();
    manager.add_full_with_params(
        "escort",
        "objective.escort",
        [("count".into(), "3".into())].into(),
        true,
        vec!["courier".into()],
        AiDirective::Destroy {
            target: "raider".into(),
        },
        UtilityConfig {
            base_priority: 12.0,
            ..Default::default()
        },
        ObjectiveSource::Mission,
        None,
    );
    manager.set_recipients("escort", vec!["ship-a".into()]);
    live.insert_resource(ObjectiveManagerRes(manager));
    let active_digest = crate::sim_digest::world_digest(live.world());
    assert_ne!(empty_digest, active_digest);
    live.world_mut()
        .resource_mut::<ObjectiveManagerRes>()
        .0
        .complete("escort");
    let complete_digest = crate::sim_digest::world_digest(live.world());
    assert_ne!(
        active_digest, complete_digest,
        "status itself is folded, with no GM journal"
    );
    live.world_mut()
        .resource_mut::<ObjectiveManagerRes>()
        .0
        .drain_transitions();
    live.world_mut()
        .resource_mut::<ObjectiveManagerRes>()
        .0
        .mark_clean();
    assert_eq!(
        complete_digest,
        crate::sim_digest::world_digest(live.world())
    );
    let payload: PhoenixSnapshot =
        serde_json::from_slice(&serde_json::to_vec(&capture(live.world())).unwrap()).unwrap();
    assert_eq!(
        payload.objective_records,
        live.world().resource::<ObjectiveManagerRes>().0.records()
    );
    assert!(restore(live.world_mut(), &payload).is_complete());
    let expected_digest = crate::sim_digest::world_digest(live.world());
    let mut resumed = App::new();
    resumed.add_plugins(MinimalPlugins);
    let mut bootstrap = ObjectiveManager::new();
    bootstrap.add("bootstrap", "Must disappear", false, vec![]);
    resumed.insert_resource(ObjectiveManagerRes(bootstrap));
    assert!(restore(resumed.world_mut(), &payload).is_complete());
    assert_eq!(
        crate::sim_digest::world_digest(resumed.world()),
        expected_digest
    );
    let mut restored = resumed.world_mut().resource_mut::<ObjectiveManagerRes>();
    assert_eq!(restored.0.records(), payload.objective_records);
    assert!(restored.0.drain_transitions().is_empty());
    assert!(!restored.0.add("escort", "Cannot reopen", false, vec![]));
    assert!(restored.0.snapshots_for("ship-b").is_empty());
}

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
            .resource::<crate::gm::actions::action::GmActionLog>()
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
            triggers: vec![
                gm_event("breach", crate::world::config::TriggerCondition::Manual),
                gm_event(
                    "sweep",
                    crate::world::config::TriggerCondition::OnDestroyed {
                        entity_name: "courier".to_string(),
                    },
                ),
            ]
            .into(),
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
    fn armed(name: &str, x_mm: i64) -> crate::gm::operations::spawn::PendingGmSpawn {
        crate::gm::operations::spawn::PendingGmSpawn {
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
            triggers: vec![
                pausable("breach", crate::world::config::TriggerCondition::Manual),
                pausable(
                    "sweep",
                    crate::world::config::TriggerCondition::OnDestroyed {
                        entity_name: "courier".to_string(),
                    },
                ),
            ]
            .into(),
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
            triggers: vec![gm_event("breach"), gm_event("evac"), gm_event("sweep")].into(),
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
    let target = crate::gm::operations::puppet::StationPuppetTarget::new(
        crate::command_admission::log::ShipKey("ship-player-1".into()),
        crate::core::messages::StationId("captain".into()),
    );
    let mut puppets = crate::gm::operations::puppet::StationPuppets::default();
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
        .insert_resource(crate::gm::operations::puppet::StationPuppets::default());
    let order = GmActionOrder::new(HostSlot(9), 99);
    let mut pending = crate::gm::operations::puppet::PendingGmStationCommands::default();
    pending.push(crate::gm::operations::puppet::PendingGmStationCommand {
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
    let mut activity = crate::gm::operations::puppet::StationPuppetActivity::default();
    activity.push(crate::gm::operations::puppet::StationPuppetActivityEntry {
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
        app.world()
            .resource::<crate::gm::operations::puppet::StationPuppets>(),
        &puppets
    );
    assert_eq!(capture(app.world()).gm_puppets, puppets);
    assert!(
        app.world()
            .resource::<crate::gm::operations::puppet::PendingGmStationCommands>()
            .entries()
            .is_empty(),
        "commands queued after the saved boundary must not leak through restore"
    );
    assert!(
        app.world()
            .resource::<crate::gm::operations::puppet::StationPuppetActivity>()
            .entries()
            .is_empty(),
        "presentation activity is rebuilt from the restored timeline"
    );
    assert_eq!(
        app.world()
            .resource::<crate::gm::operations::puppet::PreviousStationPuppetTargets>(),
        &crate::gm::operations::puppet::PreviousStationPuppetTargets::from_puppets(&puppets)
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

[[station.rating]]
name = "Manual"
automated_systems = []

[[console]]
id = "captain-console"
home_station = "captain"
screen = "captain.html"

[[system]]
id = "red-alert"
kind = "red_alert"
console = "captain-console"
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
        // Recovery replays the real Captain consumer, including its ownership promise.
        crate::console::captain::server::register_captain_consumers(&mut app);
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
            .insert_resource(crate::gm::actions::action::GmActionLog::default());
        app.world_mut()
            .insert_resource(crate::gm::operations::puppet::StationPuppets::default());
        app.world_mut()
            .insert_resource(crate::gm::operations::puppet::PendingGmStationCommands::default());
        app.world_mut().insert_resource(
            crate::gm::operations::puppet::PendingGmStationFeedbackRoutes::default(),
        );
        app.world_mut()
            .insert_resource(crate::gm::operations::puppet::StationPuppetActivity::default());
        app.world_mut().spawn((
            crate::server_app::Ship,
            // This fixture is a player ship. The canonical station capability
            // gate otherwise correctly treats an untagged partial hull as an
            // NPC whose console producers have not been installed.
            crate::lockstep::FleetSlotOf(HostSlot(1)),
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
    let puppet_target =
        crate::gm::operations::puppet::StationPuppetTarget::new(ship.clone(), station.clone());
    let mut live = bootstrap();
    live.world_mut()
        .resource_mut::<crate::gm::operations::puppet::StationPuppets>()
        .set_operator(puppet_target, "gm-1".into(), true);
    let grant = crate::gm::actions::action::GmActionGrant {
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
        .run_system_once(crate::gm::actions::action::apply_due_actions)
        .unwrap();
    assert_eq!(
        live.world()
            .resource::<crate::gm::operations::puppet::PendingGmStationCommands>()
            .entries()
            .len(),
        1,
        "the due action is accepted before FixedUpdate delivery",
    );
    assert_eq!(
        live.world()
            .resource::<crate::gm::actions::action::GmActionLog>()
            .entries()[0]
            .outcome,
        crate::gm::actions::action::GmActionOutcome::Pending,
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
            .run_system_once(crate::gm::operations::puppet::admit_station_puppet_commands)
            .unwrap();
        app.world_mut()
            .run_system_once(crate::console::captain::handle_set_red_alert)
            .unwrap();
        app.world_mut()
            .run_system_once(crate::gm::operations::puppet::settle_station_puppet_feedback)
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
            .resource::<crate::gm::operations::puppet::PendingGmStationCommands>()
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
fn snapshot_at_an_exact_gm_boundary_preserves_the_still_unapplied_frontier() {
    let mut live = App::new();
    live.add_plugins(MinimalPlugins);
    live.world_mut().insert_resource(SimTick(20));
    live.world_mut().insert_resource(SimulationPaused(false));
    live.world_mut()
        .insert_resource(crate::gm::actions::action::GmActionLog::default());
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
            .resource::<crate::gm::actions::action::GmActionLog>()
            .entries()
            .len(),
        0,
        "FixedLast's newly advanced tick must not make the boundary terminal"
    );
    assert!(!resumed.world().resource::<SimulationPaused>().0);

    resumed.add_systems(PreUpdate, crate::gm::actions::action::apply_due_actions);
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
            .insert_resource(crate::gm::actions::action::GmActionLog::default());
        app.world_mut()
            .insert_resource(crate::gm::operations::effect::PendingGmDirectEffects::default());
        app.world_mut().insert_sim_rng(crate::sim_rng::SimRng::new(
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
        .insert(crate::gm::actions::action::GmActionGrant {
            from: HostSlot(1),
            sequenced_by: HostSlot(1),
            operator_id: "gm-1".into(),
            correlation: GmActionId::new("recover-due-effect").unwrap(),
            recovery_generation: 0,
            apply_tick: 42,
            order: GmActionOrder::new(HostSlot(1), 1),
            action: GmAction::ApplyDirectEffect {
                target: "npc-1".into(),
                scope: crate::gm::operations::effect::GmDirectEffectScope::Entity,
                effect: crate::gm::operations::effect::GmDirectEffectKind::Damage,
                amount_milli_hp: 40_000,
            },
        })
        .unwrap();
    live.world_mut()
        .run_system_once(crate::gm::actions::action::apply_due_actions)
        .unwrap();
    assert_eq!(
        live.world()
            .resource::<crate::gm::operations::effect::PendingGmDirectEffects>()
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
            .run_system_once(crate::gm::operations::effect::apply_gm_direct_effects)
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
            .resource::<crate::gm::operations::effect::PendingGmDirectEffects>()
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
            .insert_resource(crate::gm::actions::action::GmActionLog::default());
        app.world_mut()
            .insert_resource(crate::gm::operations::effect::PendingGmDirectEffects::default());
        app.world_mut().insert_sim_rng(crate::sim_rng::SimRng::new(
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

[[console]]
id = "helm-console"
home_station = "helm"
screen = "fixture.html"

[[console]]
id = "tactical-console"
home_station = "tactical"
screen = "fixture.html"

[[system]]
id = "impulse-drive"
kind = "impulse-drive"
console = "helm-console"

[[system]]
id = "phaser-bank"
kind = "phaser-bank"
console = "tactical-console"
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
        .insert(crate::gm::actions::action::GmActionGrant {
            from: HostSlot(1),
            sequenced_by: HostSlot(1),
            operator_id: "gm-1".into(),
            correlation: GmActionId::new("recover-scoped-effect").unwrap(),
            recovery_generation: 0,
            apply_tick: 42,
            order: GmActionOrder::new(HostSlot(1), 1),
            action: GmAction::ApplyDirectEffect {
                target: "npc-1".into(),
                scope: crate::gm::operations::effect::GmDirectEffectScope::Station(
                    crate::core::messages::StationId("helm".into()),
                ),
                effect: crate::gm::operations::effect::GmDirectEffectKind::Damage,
                amount_milli_hp: 25_000,
            },
        })
        .unwrap();
    live.world_mut()
        .run_system_once(crate::gm::actions::action::apply_due_actions)
        .unwrap();

    let boundary = capture(live.world());
    assert_eq!(
        boundary
            .gm_direct_effects
            .entries()
            .iter()
            .map(|effect| effect.scope.clone())
            .collect::<Vec<_>>(),
        vec![crate::gm::operations::effect::GmDirectEffectScope::Station(
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
        vec![Some(
            crate::gm::operations::effect::GmDirectEffectScope::Station(
                crate::core::messages::StationId("helm".into())
            )
        )],
        "and so does the durable fact the resumed feed re-renders",
    );

    let report = restore(live.world_mut(), &boundary);
    assert!(report.is_complete(), "source gaps: {:?}", report.gaps);
    let mut recovered = bootstrap();
    let report = restore(recovered.world_mut(), &boundary);
    assert!(report.is_complete(), "recovered gaps: {:?}", report.gaps);

    for app in [&mut live, &mut recovered] {
        app.world_mut()
            .run_system_once(crate::gm::operations::effect::apply_gm_direct_effects)
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

/// The accepted removal survives the PreUpdate / FixedUpdate save boundary.
#[test]
fn armed_gm_despawns_round_trip_in_order_and_affect_the_digest() {
    fn app(queue: Vec<String>) -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(SimTick(33));
        app.insert_resource(crate::world::server::WorldContentRuntime {
            pending_gm_despawns: queue,
            ..Default::default()
        });
        app
    }
    let queue = vec!["target-z".to_string(), "target-a".to_string()];
    let mut live = app(queue.clone());
    let boundary = capture(live.world());
    assert_eq!(
        boundary.scenario.as_ref().unwrap().pending_gm_despawns,
        queue
    );
    let report = restore(live.world_mut(), &boundary);
    assert!(report.is_complete(), "{:?}", report.gaps);
    let mut recovered = app(Vec::new());
    let report = restore(recovered.world_mut(), &boundary);
    assert!(report.is_complete(), "{:?}", report.gaps);
    assert_eq!(
        recovered
            .world()
            .resource::<crate::world::server::WorldContentRuntime>()
            .pending_gm_despawns,
        queue
    );
    assert_eq!(
        crate::sim_digest::world_digest(live.world()),
        crate::sim_digest::world_digest(recovered.world())
    );
    recovered
        .world_mut()
        .resource_mut::<crate::world::server::WorldContentRuntime>()
        .pending_gm_despawns
        .reverse();
    assert_ne!(
        crate::sim_digest::world_digest(live.world()),
        crate::sim_digest::world_digest(recovered.world()),
        "the accepted order is authoritative"
    );
    recovered
        .world_mut()
        .resource_mut::<crate::world::server::WorldContentRuntime>()
        .pending_gm_despawns
        .clear();
    assert_ne!(
        crate::sim_digest::world_digest(live.world()),
        crate::sim_digest::world_digest(recovered.world()),
        "forgetting an accepted removal must diverge"
    );
}

#[test]
fn console_views_survive_capture_restore_without_becoming_device_preferences() {
    use crate::ship::components::ConsoleWorkingViews;
    use phoenix_model::{
        console::{ConsoleWorkingView, NavigationWorkingView},
        identity::ConsoleId,
    };
    let mut world = sparse_capture_world(None);
    let views = ConsoleWorkingViews(std::collections::BTreeMap::from([(
        ConsoleId("navigation".into()),
        ConsoleWorkingView {
            navigation: Some(NavigationWorkingView {
                zoom: 2.0,
                center_x: 100.0,
                center_y: -50.0,
                selected_uuid: Some(uuid::Uuid::nil()),
            }),
            ..Default::default()
        },
    )]));
    let entity = world.spawn((EntityUuid("ship".into()), views.clone())).id();
    let row = capture_entities(&world).remove(0);
    world
        .get_mut::<ConsoleWorkingViews>(entity)
        .unwrap()
        .0
        .clear();
    apply_entity_state(&mut world, entity, &row, 10);
    assert_eq!(world.get::<ConsoleWorkingViews>(entity), Some(&views));
    assert_eq!(capture_entities(&world)[0].console_views, row.console_views);
}

#[test]
fn gm_npc_console_assignment_applies_and_survives_capture_restore() {
    use crate::core::messages::{AdmittedCommands, StationId, SystemControlPayload, SystemId};
    use crate::ship::components::{
        ConsoleWorkingViews, ShipConfigComponent, ShipSystemControlSources,
    };
    use bevy::ecs::system::RunSystemOnce;
    use phoenix_model::{console::ConsoleWorkingView, identity::ConsoleId};
    let config: crate::ship::config::ShipConfig = toml::from_str(
        r#"
[[station]]
id = "helm"
name = "Helm"
description = ""
rank = ""
[[console]]
id = "npc-navigation"
home_station = "helm"
screen = "navigation.html"
"#,
    )
    .unwrap();
    let console = ConsoleId("npc-navigation".into());
    let view = ConsoleWorkingView {
        comms_thread: Some("fleet".into()),
        ..Default::default()
    };
    // An authenticated GM can operate an uncrewed NPC's authored home Console.
    let command = crate::command_admission::validate_station_command(
        &StationId("helm".into()),
        SystemId(crate::ship::console_views::ASSIGN_CONSOLE_VIEW.into()),
        SystemControlPayload::AssignConsoleView {
            console: console.clone(),
            view: view.clone(),
        },
        &ShipSystemControlSources::default(),
        &config,
        None,
        None,
    )
    .expect("valid GM Console assignment");
    let mut world = sparse_capture_world(None);
    let entity = world
        .spawn((
            EntityUuid("npc-console-ship".into()),
            ShipConfigComponent(config),
            AdmittedCommands(vec![command]),
        ))
        .id();
    assert!(!world
        .entity(entity)
        .contains::<crate::server_app::LocalShip>());
    assert!(world
        .get::<ConsoleWorkingViews>(entity)
        .unwrap()
        .0
        .is_empty());
    world
        .run_system_once(crate::ship::console_views::apply_console_views)
        .unwrap();
    assert_eq!(
        world
            .get::<ConsoleWorkingViews>(entity)
            .unwrap()
            .0
            .get(&console),
        Some(&view)
    );
    let row = capture_entities(&world).remove(0);
    assert_eq!(row.console_views.get(&console), Some(&view));
    world
        .get_mut::<ConsoleWorkingViews>(entity)
        .unwrap()
        .0
        .clear();
    apply_entity_state(&mut world, entity, &row, 10);
    assert_eq!(
        world
            .get::<ConsoleWorkingViews>(entity)
            .unwrap()
            .0
            .get(&console),
        Some(&view)
    );
}

#[test]
fn system_tree_snapshot_restores_private_nested_continuation() {
    use crate::ai::state_tree_runtime::{continued_tree, ShipAiTreeRuntime};
    use crate::core::messages::SystemId;
    let mut source = sparse_capture_world(None);
    let state = ShipAiTreeRuntime(std::collections::BTreeMap::from([
        (SystemId("dock-left".into()), continued_tree()),
        (SystemId("dock-right".into()), Default::default()),
    ]));
    source.spawn((EntityUuid("docker".into()), state.clone()));
    let rows = capture_entities(&source);
    let saved: EntityState =
        serde_json::from_slice(&serde_json::to_vec(&rows[0]).unwrap()).unwrap();
    assert_eq!(saved.ai_trees.as_ref(), Some(&state));
    let mut target = sparse_capture_world(None);
    let entity = target
        .spawn((EntityUuid("docker".into()), ShipAiTreeRuntime::default()))
        .id();
    let changed_at = target.change_tick();
    apply_entity_state_with(&mut target, entity, &saved, 8, 0.0, changed_at);
    assert_eq!(target.get::<ShipAiTreeRuntime>(entity), Some(&state));
    assert_eq!(capture_entities(&target)[0].ai_trees, saved.ai_trees);
}

#[test]
fn repair_report_sources_and_recovery_watermarks_survive_snapshot_restore() {
    use phoenix_model::observation::*;
    let observation = ObservationMetadata::at(
        42,
        ObservationSubject::System {
            id: crate::core::messages::SystemId("helm-radar".into()),
        },
        ObservationProvenance {
            kind: ObservationSource::SystemReport,
            system: Some(crate::core::messages::SystemId("helm-radar".into())),
            ..Default::default()
        },
        ObservationUncertainty::Qualitative,
    )
    .received(43);
    let mut queue = RepairRequestQueue::default();
    queue.entries.push(RepairQueueEntry {
        group: crate::core::messages::RepairGroup::Core,
        station_label: "Core".into(),
        tier: crate::ship::damage::DamageTier::Damaged,
        deficit: 3.0,
        observations: vec![observation.clone()],
    });
    queue.recoveries.push((
        crate::core::messages::RepairGroup::Station(crate::core::messages::StationId(
            "helm".into(),
        )),
        observation,
    ));
    let stored = repair_state(None, Some(&queue), None);
    let text = ron::to_string(&stored).unwrap();
    let restored: RepairState = ron::from_str(&text).unwrap();
    let mut world = World::new();
    let entity = world.spawn(RepairRequestQueue::default()).id();
    apply_repair(&mut world.entity_mut(entity), &restored);
    let actual = world.get::<RepairRequestQueue>(entity).unwrap();
    assert_eq!(actual.entries, queue.entries);
    assert_eq!(actual.recoveries, queue.recoveries);
}
