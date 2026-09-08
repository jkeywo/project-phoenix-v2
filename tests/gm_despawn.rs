//! Safe removal's authoritative boundary and host-neutral lifecycle (#1306).
#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]
use bevy::{ecs::system::RunSystemOnce, prelude::*};
use phoenix::command_admission::HostSlot;
use phoenix::entities::spawner::{EntityTagsSection, EntityUuid};
use phoenix::gm_action::*;
use phoenix::sim_tick::SimTick;
use phoenix::world::server::WorldContentRuntime;
use project_phoenix as phoenix;

fn grant(slot: u32, sequence: u64, tick: u64, target: &str) -> GmActionGrant {
    GmActionGrant {
        from: HostSlot(slot),
        sequenced_by: HostSlot(1),
        operator_id: format!("gm-{slot}"),
        correlation: GmActionId::new(format!("remove-{sequence}")).unwrap(),
        recovery_generation: 0,
        apply_tick: tick,
        order: GmActionOrder::new(HostSlot(slot), sequence),
        action: GmAction::DespawnEntity {
            target: target.into(),
        },
    }
}
fn bare() -> App {
    let mut app = App::new();
    app.insert_resource(SimTick(42))
        .init_resource::<SimulationPaused>()
        .init_resource::<GmActionJournal>()
        .init_resource::<GmActionLog>()
        .init_resource::<WorldContentRuntime>()
        .init_resource::<phoenix::comms::server::CommsRuntime>();
    app
}
fn apply(app: &mut App, request: GmActionGrant) {
    app.world_mut()
        .resource_mut::<GmActionJournal>()
        .insert(request)
        .unwrap();
    app.world_mut().run_system_once(apply_due_actions).unwrap();
}
fn references(app: &mut App, target: &str) -> Entity {
    let observer = app
        .world_mut()
        .spawn((
            phoenix::console::weapons::TacticalRadarSelection(Some(target.into())),
            phoenix::ship::sensors::SensorRadarSelection(Some(target.into())),
        ))
        .id();
    let mut comms = app
        .world_mut()
        .resource_mut::<phoenix::comms::server::CommsRuntime>();
    comms.contacts.push(phoenix::core::messages::CommsContact {
        uuid: target.into(),
        name: "Historical identity".into(),
        in_range: true,
        is_urgent: false,
    });
    comms.open_hails.insert(target.into());
    comms.range_flags.insert(target.into(), true);
    comms
        .fleet_range_flags
        .entry(HostSlot(2))
        .or_default()
        .insert(target.into(), true);
    observer
}

#[test]
fn protected_classes_and_unknown_peers_refuse_without_touching_live_references() {
    for tag in [
        "player",
        "star",
        "planet",
        "moon",
        "asteroid_field",
        "trigger_volume",
        "objective_marker",
        "region",
        "unclassified",
    ] {
        let mut app = bare();
        // Permission cannot override a foundational class. Unclassified has no permission.
        let tags = if tag == "unclassified" {
            vec!["structure".into()]
        } else {
            vec![tag.into(), "gm_removable".into(), "structure".into()]
        };
        let target = app
            .world_mut()
            .spawn((EntityUuid("protected".into()), EntityTagsSection(tags)))
            .id();
        let observer = references(&mut app, "protected");
        apply(&mut app, grant(1, 1, 42, "protected"));
        let result = &app.world().resource::<GmActionLog>().entries()[0];
        assert_eq!(
            result.reason,
            Some(GmActionRefusalReason::ProtectedEntity),
            "{tag}"
        );
        assert!(app.world().get_entity(target).is_ok());
        assert_eq!(
            app.world()
                .get::<phoenix::console::weapons::TacticalRadarSelection>(observer)
                .unwrap()
                .0
                .as_deref(),
            Some("protected")
        );
        assert!(app
            .world()
            .resource::<phoenix::comms::server::CommsRuntime>()
            .open_hails
            .contains("protected"));
        assert!(app
            .world()
            .resource::<WorldContentRuntime>()
            .pending_gm_despawns
            .is_empty());
    }
    let mut app = bare();
    app.world_mut().spawn((
        EntityUuid("fleet".into()),
        EntityTagsSection(vec!["gm_removable".into()]),
        phoenix::server_app::Ship,
        phoenix::lockstep::FleetSlotOf(HostSlot(2)),
    ));
    for (seq, target, reason) in [
        (1, "fleet", GmActionRefusalReason::ProtectedEntity),
        (2, "gm-2", GmActionRefusalReason::UnknownEntity),
        (3, "stale-uuid", GmActionRefusalReason::UnknownEntity),
    ] {
        apply(&mut app, grant(1, seq, 42, target));
        assert_eq!(
            app.world()
                .resource::<GmActionLog>()
                .entries()
                .last()
                .unwrap()
                .reason,
            Some(reason)
        );
    }
}

#[test]
fn canonical_boundary_attributes_once_and_shared_cleanup_needs_no_local_ship() {
    let mut app = bare();
    let entity = app
        .world_mut()
        .spawn((
            EntityUuid("npc".into()),
            phoenix::server_app::Ship,
            EntityTagsSection(vec!["gm_removable".into()]),
        ))
        .id();
    let observer = references(&mut app, "npc");
    app.world_mut()
        .resource_mut::<WorldContentRuntime>()
        .name_to_uuid
        .insert("old-name".into(), "npc".into());
    let first = grant(1, 1, 43, "npc");
    apply(&mut app, first.clone());
    assert!(
        app.world().resource::<GmActionLog>().entries().is_empty(),
        "future grant cannot affect current state"
    );
    // Two operators share the owner sequence; retransmission keeps the same identity.
    app.world_mut()
        .resource_mut::<GmActionJournal>()
        .insert(grant(2, 2, 43, "npc"))
        .unwrap();
    app.world_mut()
        .resource_mut::<GmActionJournal>()
        .insert(first)
        .unwrap();
    app.world_mut().resource_mut::<SimTick>().0 = 43;
    app.world_mut().run_system_once(apply_due_actions).unwrap();
    let entries = app.world().resource::<GmActionLog>().entries();
    assert_eq!(entries.len(), 2);
    assert_eq!(
        (
            entries[0].operator_id.as_str(),
            entries[0].tick,
            entries[0].action_kind,
            entries[0].outcome
        ),
        (
            "gm-1",
            43,
            GmActionKind::WorldDespawn,
            GmActionOutcome::Applied
        )
    );
    assert_eq!(entries[1].outcome, GmActionOutcome::NoOp);
    assert_eq!(
        app.world()
            .resource::<WorldContentRuntime>()
            .pending_gm_despawns,
        ["npc"]
    );
    // Exercise the public lifecycle independently of any renderer/local-ship systems.
    phoenix::gm_despawn::remove_entity(app.world_mut(), entity);
    assert!(app.world().get_entity(entity).is_err());
    assert_eq!(
        app.world()
            .get::<phoenix::console::weapons::TacticalRadarSelection>(observer)
            .unwrap()
            .0,
        None
    );
    assert_eq!(
        app.world()
            .get::<phoenix::ship::sensors::SensorRadarSelection>(observer)
            .unwrap()
            .0,
        None
    );
    let comms = app
        .world()
        .resource::<phoenix::comms::server::CommsRuntime>();
    assert!(
        comms.contacts.is_empty() && comms.open_hails.is_empty() && comms.range_flags.is_empty()
    );
    assert!(comms
        .fleet_range_flags
        .values()
        .all(|flags| flags.is_empty()));
    assert_eq!(
        app.world().resource::<WorldContentRuntime>().name_to_uuid["old-name"],
        "npc",
        "destruction predicates keep their historical name identity"
    );
    assert_eq!(
        app.world().resource::<GmActionLog>().entries()[0]
            .target
            .as_deref(),
        Some("npc"),
        "attributed history survives entity removal"
    );
}

fn seeded() -> App {
    let args = phoenix::headless::HeadlessArgs {
        world_path: "assets/worlds/patrol.toml".into(),
        ship_path: "assets/entities/alliance_cruiser.toml".into(),
        seed: Some(1306),
        deterministic: true,
        max_ticks: 260,
        ..Default::default()
    };
    let mut app = phoenix::headless::build_headless_app(&args).unwrap();
    app.finish();
    app.cleanup();
    for _ in 0..180 {
        app.update();
    }
    app
}

#[test]
fn seeded_canonical_removal_replays_and_restore_does_not_resurrect_authored_npc() {
    fn target(app: &mut App) -> (Entity, String) {
        let uuid = app.world().resource::<WorldContentRuntime>().name_to_uuid
            ["world.entity.raider_alpha.name"]
            .clone();
        let entity = app
            .world_mut()
            .query::<(Entity, &EntityUuid)>()
            .iter(app.world())
            .find(|(_, id)| id.0 == uuid)
            .unwrap()
            .0;
        app.world_mut()
            .get_mut::<EntityTagsSection>(entity)
            .unwrap()
            .0
            .push("gm_removable".into());
        (entity, uuid)
    }
    let mut live = seeded();
    let (entity, uuid) = target(&mut live);
    let tick = live.world().resource::<SimTick>().0 + 2;
    let script = vec![
        grant(2, 1, tick, &uuid),
        grant(1, 2, tick, &uuid),
        grant(1, 3, tick + 3, &uuid),
    ];
    // Portable grant bytes, reinserted through the canonical production journal.
    let bytes = serde_json::to_string(&script).unwrap();
    for request in script {
        live.world_mut()
            .resource_mut::<GmActionJournal>()
            .insert(request)
            .unwrap();
    }
    for _ in 0..10 {
        live.update();
    }
    assert!(
        live.world().get_entity(entity).is_err(),
        "ordinary FixedUpdate must consume the GM removal"
    );
    assert!(live
        .world()
        .resource::<WorldContentRuntime>()
        .pending_gm_despawns
        .is_empty());
    assert!(
        live.world()
            .resource::<phoenix::world::server::ObjectiveManagerRes>()
            .0
            .sorted_snapshots()
            .iter()
            .any(|objective| objective.id == "obj-raider-destroyed"),
        "the ordinary on_destroyed cascade must observe the GM removal"
    );
    let facts = live.world().resource::<GmActionLog>().entries().to_vec();
    assert_eq!(
        facts.iter().map(|r| r.outcome).collect::<Vec<_>>(),
        [
            GmActionOutcome::Applied,
            GmActionOutcome::NoOp,
            GmActionOutcome::Refused
        ]
    );
    assert_eq!(facts[2].reason, Some(GmActionRefusalReason::UnknownEntity));
    let mut replay = seeded();
    assert_eq!(target(&mut replay).1, uuid);
    for request in serde_json::from_str::<Vec<GmActionGrant>>(&bytes).unwrap() {
        replay
            .world_mut()
            .resource_mut::<GmActionJournal>()
            .insert(request)
            .unwrap();
    }
    for _ in 0..10 {
        replay.update();
    }
    assert_eq!(replay.world().resource::<GmActionLog>().entries(), facts);
    assert_eq!(
        phoenix::sim_digest::world_digest(live.world()),
        phoenix::sim_digest::world_digest(replay.world())
    );
    let saved = phoenix::snapshot::capture(live.world());
    let mut restored = seeded();
    let (original, _) = target(&mut restored);
    let report = phoenix::snapshot::restore(restored.world_mut(), &saved);
    assert!(report.is_complete(), "{:?}", report.gaps);
    assert!(
        restored.world().get_entity(original).is_err(),
        "restore removes boot-authored NPCs absent from the save"
    );
    assert!(!restored
        .world_mut()
        .query::<&EntityUuid>()
        .iter(restored.world())
        .any(|id| id.0 == uuid));
    assert_eq!(restored.world().resource::<GmActionLog>().entries(), facts);
}

#[test]
fn structures_and_only_provenance_bearing_hazards_are_removable() {
    use phoenix::entities::spawner::{EntitySpawnOrigin, RegionEffectsSection, RegionShapeSection};
    use phoenix::regions::{effects::RegionEffectKind, shape::RegionShape};
    for kind in [
        "structure",
        "authored-hazard",
        "spawned-hazard",
        "inert-region",
    ]
    .iter()
    {
        let mut app = bare();
        let entity = app
            .world_mut()
            .spawn((
                EntityUuid("target".into()),
                EntityTagsSection(vec!["gm_removable".into(), "structure".into()]),
            ))
            .id();
        if *kind != "structure" {
            app.world_mut()
                .entity_mut(entity)
                .insert(RegionShapeSection(RegionShape::Sphere { radius: 20.0 }));
            if *kind != "inert-region" {
                app.world_mut()
                    .entity_mut(entity)
                    .insert(RegionEffectsSection(vec![RegionEffectKind::DamageZone {
                        dps: 1.0,
                        shield_pierce: 0.0,
                    }]));
            }
            if *kind == "spawned-hazard" || *kind == "inert-region" {
                app.world_mut().entity_mut(entity).insert(EntitySpawnOrigin(
                    phoenix::world::spawn_origin::SpawnOrigin::default(),
                ));
            }
        }
        apply(&mut app, grant(1, 1, 42, "target"));
        let result = &app.world().resource::<GmActionLog>().entries()[0];
        let allowed = *kind == "structure" || *kind == "spawned-hazard";
        assert_eq!(
            result.outcome,
            if allowed {
                GmActionOutcome::Applied
            } else {
                GmActionOutcome::Refused
            },
            "{kind}"
        );
        if !allowed {
            assert_eq!(result.reason, Some(GmActionRefusalReason::ProtectedEntity));
        }
    }
}

#[test]
fn shared_removal_releases_support_partners_without_a_local_ship() {
    use phoenix::core::messages::{PowerGroupId, SystemId};
    let mut world = World::new();
    let target = world.spawn(EntityUuid("partner".into())).id();
    let mut tractor = phoenix::tractor::TractorBeam::new(
        serde_json::from_value(serde_json::json!({
            "range": 100.0, "coupling_offset": [0.0,0.0,-10.0], "min_power_level": 1,
            "tow_load": {"max_penalty": 0.75, "half_penalty_mass": 100.0}
        }))
        .unwrap(),
        PowerGroupId("utility".into()),
    );
    tractor.engaged = true;
    tractor.coupled_target = Some("partner".into());
    let mut dock = phoenix::dock::DockControl::new(
        SystemId("dock".into()),
        serde_json::from_value(serde_json::json!({
            "range": 100.0, "engage_distance": 100.0, "approach_speed": 1.0,
            "mate_tolerance": 1.0, "undock_clear_distance": 10.0, "min_power_level": 1
        }))
        .unwrap(),
        PowerGroupId("utility".into()),
    );
    dock.engaged = true;
    dock.docked = true;
    dock.docking_target = Some("partner".into());
    dock.available_target = Some("partner".into());
    let mut umbilical = phoenix::umbilical::TransferUmbilical::new(
        serde_json::from_value(serde_json::json!({
            "capacity": "fuel", "rate": 1.0, "direction": "deliver", "min_power_level": 1
        }))
        .unwrap(),
        PowerGroupId("utility".into()),
    );
    umbilical.running = true;
    umbilical.activation_target = Some("partner".into());
    umbilical.partner_level = Some(90);
    let observer = world.spawn((tractor, dock, umbilical)).id();
    phoenix::gm_despawn::remove_entity(&mut world, target);
    let tractor = world
        .get::<phoenix::tractor::TractorBeam>(observer)
        .unwrap();
    assert!(!tractor.engaged && tractor.coupled_target.is_none());
    let dock = world.get::<phoenix::dock::DockControl>(observer).unwrap();
    assert!(
        !dock.engaged
            && !dock.docked
            && dock.docking_target.is_none()
            && dock.available_target.is_none()
    );
    let umbilical = world
        .get::<phoenix::umbilical::TransferUmbilical>(observer)
        .unwrap();
    assert!(
        !umbilical.running
            && umbilical.activation_target.is_none()
            && umbilical.partner_level.is_none()
    );
}

#[test]
fn removing_an_actor_ends_active_and_queued_work_without_claiming_its_subject_died() {
    use phoenix::core::task_lifecycle::{
        TaskLifecycleRequest, TaskLifecycles, TaskSlot, TaskTerminalReason,
    };
    use phoenix::effect_queue::EffectQueue;
    let mut world = World::new();
    let actor = world.spawn(EntityUuid("actor".into())).id();
    let subject = world.spawn(EntityUuid("survivor".into())).id();
    let active = TaskSlot::new("actor", "sensors", "scan");
    let queued = TaskSlot::new("actor", "tractor", "tractor_hold");
    let mut tasks = TaskLifecycles::default();
    tasks.begin(active.clone(), Some("survivor".into()), None, 41);
    world.insert_resource(tasks);
    world.insert_resource(EffectQueue(vec![TaskLifecycleRequest::Start {
        slot: queued.clone(),
        target: Some("survivor".into()),
    }]));
    phoenix::gm_despawn::remove_entity(&mut world, actor);
    assert!(world.get_entity(subject).is_ok());
    let requests = &world.resource::<EffectQueue<TaskLifecycleRequest>>().0;
    for slot in [active, queued] {
        assert_eq!(
            requests
                .iter()
                .filter(|request| matches!(request,
            TaskLifecycleRequest::End { slot: ended, reason: TaskTerminalReason::OrderWithdrawn }
            if *ended == slot))
                .count(),
            1,
            "each actor-owned task must close exactly once"
        );
    }
    assert!(
        !requests.iter().any(|request| matches!(
            request,
            TaskLifecycleRequest::End {
                reason: TaskTerminalReason::TargetDestroyed,
                ..
            }
        )),
        "the surviving subject must not be reported destroyed"
    );
}
