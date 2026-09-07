//! Complete NPC Station capability and ordinary consumer continuation (#1314).
#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]
use bevy::{ecs::system::RunSystemOnce, prelude::*};
use phoenix::command_admission::{log::ShipKey, HostSlot};
use phoenix::core::messages::{StationId, SystemControlPayload, SystemId};
use phoenix::entities::spawner::EntityUuid;
use phoenix::gm_action::*;
use phoenix::gm_puppet::{StationPuppetTarget, StationPuppets};
use project_phoenix as phoenix;

fn args() -> phoenix::headless::HeadlessArgs {
    phoenix::headless::HeadlessArgs {
        world_path: "assets/worlds/probe_gm_npc_puppet.toml".into(),
        ship_path: "assets/entities/alliance_cruiser.toml".into(),
        seed: Some(1314),
        deterministic: true,
        max_ticks: 260,
        ..Default::default()
    }
}
fn seeded() -> App {
    let mut app = phoenix::headless::build_headless_app(&args()).unwrap();
    for _ in 0..180 {
        app.update();
    }
    app
}
fn npc(app: &mut App) -> (Entity, String) {
    let mut q = app.world_mut().query_filtered::<(Entity, &EntityUuid), (
        With<phoenix::server_app::Ship>,
        Without<phoenix::lockstep::FleetSlotOf>,
    )>();
    let (entity, uuid) = q.single(app.world()).unwrap();
    (entity, uuid.0.clone())
}
fn member(ship: &str, station: &str, active: bool) -> GmAction {
    GmAction::SetStationPuppet {
        ship: ShipKey(ship.into()),
        station: StationId(station.into()),
        active,
    }
}
fn thrust(ship: &str, value: f32) -> GmAction {
    GmAction::IssueStationCommand {
        ship: ShipKey(ship.into()),
        station: StationId("helm".into()),
        target: SystemId("helm-thrust".into()),
        payload: phoenix::core::codec::canonical_system_command(&SystemControlPayload::SetThrust {
            value,
        })
        .unwrap(),
    }
}
fn grant(sequence: u64, tick: u64, slot: u32, action: GmAction) -> GmActionGrant {
    GmActionGrant {
        from: HostSlot(slot),
        sequenced_by: HostSlot(1),
        operator_id: format!("gm-{slot}"),
        correlation: GmActionId::new(format!("npc-{sequence}")).unwrap(),
        recovery_generation: 0,
        apply_tick: tick,
        order: GmActionOrder::new(HostSlot(slot), sequence),
        action,
    }
}
fn enqueue(app: &mut App, sequence: u64, slot: u32, action: GmAction) {
    let tick = app.world().resource::<phoenix::sim_tick::SimTick>().0;
    app.world_mut()
        .resource_mut::<GmActionJournal>()
        .insert(grant(sequence, tick, slot, action))
        .unwrap();
}
fn advance(app: &mut App, count: usize) {
    for _ in 0..count {
        app.update();
    }
}
fn digest(app: &App) -> u64 {
    phoenix::sim_digest::world_digest(app.world())
}

#[test]
fn npc_capability_is_shared_fail_closed_and_resolves_instance_overrides() {
    let mut app = seeded();
    let (entity, uuid) = npc(&mut app);
    app.insert_resource(phoenix::gm_projection::BrowserGameMaster)
        .add_plugins(phoenix::gm_projection::GmProjectionPlugin);
    app.world_mut().run_schedule(FixedLast);
    let projections: Vec<_> = app
        .world_mut()
        .resource_mut::<Messages<phoenix::console_bridge::GmStationProjectionChanged>>()
        .drain()
        .map(|message| message.payload)
        .collect();
    let row = projections
        .last()
        .unwrap()
        .ships
        .iter()
        .find(|ship| ship.ship_id == uuid)
        .unwrap();
    assert_eq!(
        row.stations
            .iter()
            .map(|station| station.station_id.0.as_str())
            .collect::<Vec<_>>(),
        ["helm"]
    );
    assert_eq!(row.ship_config.helm_radar_range, 173.0);
    assert!(!app
        .world()
        .entity(entity)
        .contains::<phoenix::ai::server::AiHighFidelity>());
    let replica = &app
        .world()
        .get::<phoenix::gm_puppet::capability::NpcStationConfig>(entity)
        .unwrap()
        .0;
    assert_eq!(
        replica.helm_radar_range, 173.0,
        "resolved instance override, not template or local hull"
    );
    assert_eq!(
        phoenix::gm_puppet::validate_station_action_in_world(
            app.world_mut(),
            &member(&uuid, "helm", true),
            "gm-2"
        ),
        Ok(())
    );
    for station in ["captain", "engineering", "tactical", "missing"] {
        assert!(
            phoenix::gm_puppet::validate_station_action_in_world(
                app.world_mut(),
                &member(&uuid, station, true),
                "gm-2"
            )
            .is_err(),
            "{station}"
        );
    }
    let original = app
        .world()
        .get::<phoenix::ship::components::ShipConfigComponent>(entity)
        .unwrap()
        .clone();
    for missing in ["interface", "commandable", "mixed-family", "ownerless"] {
        let mut config = original.clone();
        match missing {
            "interface" => {
                config
                    .0
                    .stations
                    .iter_mut()
                    .find(|s| s.id.0 == "helm")
                    .unwrap()
                    .console = None
            }
            "commandable" => config
                .0
                .systems
                .retain(|s| s.station.as_ref().is_none_or(|s| s.0 != "helm")),
            "mixed-family" => {
                config
                    .0
                    .systems
                    .iter_mut()
                    .find(|s| s.id.0 == "helm-thrust")
                    .unwrap()
                    .kind = "power_reactor".into()
            }
            "ownerless" => {
                config
                    .0
                    .systems
                    .iter_mut()
                    .find(|s| s.id.0 == "helm-thrust")
                    .unwrap()
                    .ai_only = true
            }
            _ => unreachable!(),
        }
        app.world_mut().entity_mut(entity).insert(config);
        assert_eq!(
            phoenix::gm_puppet::validate_station_action_in_world(
                app.world_mut(),
                &member(&uuid, "helm", true),
                "gm-2"
            ),
            Err(GmActionRefusalReason::SystemUnavailable),
            "{missing}"
        );
    }
    app.world_mut().entity_mut(entity).insert(original);
    use phoenix::command_admission::router::{AdmittedConsumerRegistry, ConsumerMatcher};
    let registered = app
        .world_mut()
        .remove_resource::<AdmittedConsumerRegistry>()
        .unwrap();
    let mut wrong_address = AdmittedConsumerRegistry::default();
    wrong_address.register(ConsumerMatcher::exact("helm_thrust", "not-this-ships-axis"));
    app.insert_resource(wrong_address);
    assert_eq!(
        phoenix::gm_puppet::validate_station_action_in_world(
            app.world_mut(),
            &member(&uuid, "helm", true),
            "gm-2"
        ),
        Err(GmActionRefusalReason::SystemUnavailable)
    );
    app.insert_resource(registered);
    let duplicate = app
        .world_mut()
        .spawn((phoenix::server_app::Ship, EntityUuid(uuid.clone())))
        .id();
    assert_eq!(
        phoenix::gm_puppet::validate_station_action_in_world(
            app.world_mut(),
            &member(&uuid, "helm", true),
            "gm-2"
        ),
        Err(GmActionRefusalReason::UnknownStation)
    );
    app.world_mut().despawn(duplicate);
    // The real canonical reducer refuses an unsupported picker bypass.
    enqueue(&mut app, 1, 2, member(&uuid, "engineering", true));
    advance(&mut app, 2);
    assert_eq!(
        app.world().resource::<GmActionLog>().entries()[0].outcome,
        GmActionOutcome::Refused
    );
    // Global registration alone cannot provide the per-NPC physics consumer.
    app.world_mut()
        .entity_mut(entity)
        .remove::<phoenix::ship_plugin::ShipPhysicsConfigResource>();
    assert_eq!(
        phoenix::gm_puppet::validate_station_action_in_world(
            app.world_mut(),
            &member(&uuid, "helm", true),
            "gm-2"
        ),
        Err(GmActionRefusalReason::SystemUnavailable)
    );
    enqueue(&mut app, 2, 2, member(&uuid, "helm", true));
    advance(&mut app, 1);
    assert_eq!(
        app.world()
            .resource::<GmActionLog>()
            .entries()
            .last()
            .unwrap()
            .outcome,
        GmActionOutcome::Refused
    );
    assert_eq!(
        phoenix::gm_puppet::validate_station_action_in_world(
            app.world_mut(),
            &member("removed", "helm", false),
            "gm-2"
        ),
        Ok(()),
        "release is never stranded by missing capability"
    );
}

#[test]
fn npc_equal_gms_drive_real_consumers_and_restore_authored_ai_after_last_release() {
    let mut app = seeded();
    let (entity, uuid) = npc(&mut app);
    let target = StationPuppetTarget::new(ShipKey(uuid.clone()), StationId("helm".into()));
    let before = *app
        .world()
        .get::<phoenix::ship::state::ShipPhysics>(entity)
        .unwrap();
    // Takeover AND first input share one boundary; the low-LOD consumer cannot
    // swallow that first command or continue steering under the GM.
    enqueue(&mut app, 1, 2, member(&uuid, "helm", true));
    enqueue(&mut app, 2, 3, member(&uuid, "helm", true));
    enqueue(&mut app, 3, 2, thrust(&uuid, -0.7));
    enqueue(&mut app, 4, 3, thrust(&uuid, 0.9));
    advance(&mut app, 8);
    assert!(app
        .world()
        .entity(entity)
        .contains::<phoenix::ai::server::AiHighFidelity>());
    assert_eq!(
        app.world()
            .get::<phoenix::ship::helm::ThrustInput>(entity)
            .unwrap()
            .0,
        0.9
    );
    let after = *app
        .world()
        .get::<phoenix::ship::state::ShipPhysics>(entity)
        .unwrap();
    assert!(
        after.forward_speed > before.forward_speed,
        "winning canonical input must accelerate the actual NPC"
    );
    assert_eq!(
        app.world().resource::<StationPuppets>().operators(&target),
        ["gm-2", "gm-3"]
    );
    let sources = app
        .world()
        .get::<phoenix::ship::components::ShipSystemControlSources>(entity)
        .unwrap();
    assert!(
        !sources
            .0
            .policy_for(&SystemId("helm-thrust".into()))
            .operate_ai
    );
    assert!(
        sources
            .0
            .policy_for(&SystemId("phaser-control".into()))
            .operate_ai,
        "unrelated Station retains authored AI"
    );
    let activity = app
        .world()
        .resource::<phoenix::gm_puppet::StationPuppetActivity>()
        .entries();
    assert_eq!(
        activity
            .iter()
            .map(|e| e.operator_id.as_str())
            .collect::<Vec<_>>(),
        ["gm-2", "gm-3"]
    );
    enqueue(&mut app, 5, 2, member(&uuid, "helm", false));
    advance(&mut app, 3);
    assert_eq!(
        app.world()
            .get::<phoenix::ship::helm::ThrustInput>(entity)
            .unwrap()
            .0,
        0.9
    );
    assert_eq!(
        app.world().resource::<StationPuppets>().operators(&target),
        ["gm-3"]
    );
    enqueue(&mut app, 6, 3, member(&uuid, "helm", false));
    advance(&mut app, 240);
    assert!(!app.world().resource::<StationPuppets>().is_active(&target));
    assert!(
        app.world()
            .get::<phoenix::ship::components::ShipSystemControlSources>(entity)
            .unwrap()
            .0
            .policy_for(&SystemId("helm-thrust".into()))
            .operate_ai
    );
    assert!(
        !app.world()
            .entity(entity)
            .contains::<phoenix::ai::server::AiHighFidelity>(),
        "ordinary distance/dwell policy resumes after last release"
    );
    assert_ne!(
        *app.world()
            .get::<phoenix::ship::state::ShipPhysics>(entity)
            .unwrap(),
        after,
        "authored low-LOD patrol resumes motion"
    );
}

#[test]
fn npc_snapshot_preserves_active_and_queued_control_then_removal_retires_memberships() {
    let mut live = seeded();
    let (entity, uuid) = npc(&mut live);
    enqueue(&mut live, 1, 2, member(&uuid, "helm", true));
    enqueue(&mut live, 2, 2, thrust(&uuid, 0.65));
    // The canonical reducer ran, but no ordinary consumer has received input.
    live.world_mut().run_system_once(apply_due_actions).unwrap();
    assert_eq!(
        live.world()
            .resource::<phoenix::gm_puppet::PendingGmStationCommands>()
            .entries()
            .len(),
        1
    );
    let snapshot = phoenix::snapshot::capture(live.world());
    let mut restored = seeded();
    let report = phoenix::snapshot::restore(restored.world_mut(), &snapshot);
    assert!(report.is_complete(), "{:?}", report.gaps);
    assert_eq!(digest(&live), digest(&restored));
    advance(&mut live, 8);
    advance(&mut restored, 8);
    assert_eq!(digest(&live), digest(&restored));
    assert_eq!(
        live.world()
            .get::<phoenix::ship::helm::ThrustInput>(entity)
            .unwrap()
            .0,
        0.65
    );
    let held = phoenix::snapshot::capture(live.world());
    let report = phoenix::snapshot::restore(restored.world_mut(), &held);
    assert!(report.is_complete(), "{:?}", report.gaps);
    advance(&mut live, 4);
    advance(&mut restored, 4);
    assert_eq!(digest(&live), digest(&restored));
    let remove = GmAction::DespawnEntity {
        target: uuid.clone(),
    };
    enqueue(&mut live, 3, 2, remove.clone());
    enqueue(&mut restored, 3, 2, remove);
    advance(&mut live, 4);
    advance(&mut restored, 4);
    assert!(live.world().get_entity(entity).is_err());
    assert!(!live
        .world()
        .resource::<StationPuppets>()
        .operates_ship(&uuid));
    assert_eq!(digest(&live), digest(&restored));
    let removed = phoenix::snapshot::capture(live.world());
    let mut fresh = seeded();
    let report = phoenix::snapshot::restore(fresh.world_mut(), &removed);
    assert!(report.is_complete(), "{:?}", report.gaps);
    assert!(!fresh
        .world()
        .resource::<StationPuppets>()
        .operates_ship(&uuid));
    assert_eq!(
        fresh.world().resource::<GmActionLog>().entries(),
        live.world().resource::<GmActionLog>().entries()
    );
}

#[test]
fn npc_commands_replay_through_the_portable_artifact() {
    use phoenix::headless::replay::{drive_run, drive_run_with_gm_actions};
    let mut discovery = drive_run(&args(), &[], 0).unwrap();
    let (_, uuid) = npc(discovery.app_mut());
    let mut planned = GmActionJournal::default();
    planned.record_slot_recovery(HostSlot(2), 170).unwrap();
    let recovered = |sequence, tick, action| {
        let mut result = grant(sequence, tick, 2, action);
        result.recovery_generation = 1;
        result
    };
    for g in [
        grant(1, 120, 2, member(&uuid, "helm", true)),
        grant(2, 120, 3, member(&uuid, "helm", true)),
        grant(3, 120, 2, thrust(&uuid, -0.4)),
        grant(4, 120, 3, thrust(&uuid, 0.8)),
        grant(5, 160, 2, member(&uuid, "helm", false)),
        grant(6, 175, 2, member(&uuid, "helm", true)),
        recovered(7, 176, member(&uuid, "helm", true)),
        recovered(8, 177, thrust(&uuid, 0.45)),
        recovered(9, 178, member(&uuid, "helm", false)),
        grant(10, 190, 3, member(&uuid, "helm", false)),
        recovered(11, 210, member(&uuid, "engineering", true)),
        grant(
            12,
            230,
            3,
            GmAction::DespawnEntity {
                target: uuid.clone(),
            },
        ),
        recovered(13, 245, member(&uuid, "helm", true)),
    ] {
        planned.insert(g).unwrap();
    }
    planned.restore_applied_frontier(planned.len()).unwrap();
    let mut recorded = drive_run_with_gm_actions(&args(), &[], &planned, 25).unwrap();
    let actions = recorded.recorded_gm_actions();
    assert_eq!(
        actions.applied_results()[5].reason,
        Some(GmActionRefusalReason::NotGameMaster),
        "pre-recovery authority cannot return with its old generation"
    );
    assert_eq!(
        actions.applied_results().last().unwrap().reason,
        Some(GmActionRefusalReason::UnknownStation),
        "fresh authority still cannot command a removed NPC"
    );
    assert_eq!(
        actions
            .applied_results()
            .iter()
            .map(|r| r.outcome)
            .collect::<Vec<_>>(),
        [
            GmActionOutcome::Applied,
            GmActionOutcome::Applied,
            GmActionOutcome::Applied,
            GmActionOutcome::Applied,
            GmActionOutcome::Applied,
            GmActionOutcome::Refused,
            GmActionOutcome::Applied,
            GmActionOutcome::Applied,
            GmActionOutcome::Applied,
            GmActionOutcome::Applied,
            GmActionOutcome::Refused,
            GmActionOutcome::Applied,
            GmActionOutcome::Refused,
        ]
    );
    let artifact = phoenix::headless::ReplayArtifact::capture(
        &args(),
        recorded.recorded_log(),
        actions,
        recorded.tick(),
        recorded.seal(),
    )
    .unwrap();
    assert_eq!(phoenix::headless::verify_artifact(&artifact).unwrap(), None);
}

#[test]
fn runtime_palette_npc_restore_rebuilds_its_resolved_station_config() {
    let mut live = seeded();
    enqueue(
        &mut live,
        1,
        2,
        GmAction::SpawnPaletteEntity {
            palette: "npc_helm".into(),
            variant: Some("short_range".into()),
            position_mm: [7_000_000, 0, -6_000_000],
            heading_mdeg: 90_000,
        },
    );
    advance(&mut live, 3);
    let find_spawned = |app: &mut App| {
        let mut q = app.world_mut().query::<(
            &EntityUuid,
            &phoenix::gm_puppet::capability::NpcStationConfig,
        )>();
        q.iter(app.world())
            .find(|(_, config)| config.0.helm_radar_range == 119.0)
            .map(|(uuid, config)| (uuid.0.clone(), config.0.clone()))
            .expect("palette variant replica")
    };
    let (uuid, config) = find_spawned(&mut live);
    enqueue(&mut live, 2, 2, member(&uuid, "helm", true));
    enqueue(&mut live, 3, 2, thrust(&uuid, 0.55));
    advance(&mut live, 3);
    let snapshot = phoenix::snapshot::capture(live.world());
    let mut restored = seeded();
    let report = phoenix::snapshot::restore(restored.world_mut(), &snapshot);
    assert!(report.is_complete(), "{:?}", report.gaps);
    assert_eq!(find_spawned(&mut restored), (uuid.clone(), config));
    assert_eq!(
        phoenix::gm_puppet::validate_station_action_in_world(
            restored.world_mut(),
            &thrust(&uuid, 0.6),
            "gm-2"
        ),
        Ok(())
    );
    advance(&mut live, 4);
    advance(&mut restored, 4);
    assert_eq!(digest(&live), digest(&restored));
}

#[test]
fn a_delivered_command_whose_npc_dies_before_its_consumer_receives_a_terminal_refusal() {
    let mut app = seeded();
    let (entity, uuid) = npc(&mut app);
    enqueue(&mut app, 1, 2, member(&uuid, "helm", true));
    enqueue(
        &mut app,
        2,
        2,
        GmAction::IssueStationCommand {
            ship: ShipKey(uuid.clone()),
            station: StationId("helm".into()),
            target: SystemId("helm-impulse".into()),
            payload: phoenix::core::codec::canonical_system_command(
                &SystemControlPayload::StartImpulseCharge,
            )
            .unwrap(),
        },
    );
    app.world_mut().run_system_once(apply_due_actions).unwrap();
    app.world_mut()
        .run_system_once(phoenix::gm_puppet::admit_station_puppet_commands)
        .unwrap();
    assert_eq!(
        app.world().resource::<GmActionJournal>().applied_results()[1].outcome,
        GmActionOutcome::Pending
    );
    // Models a destruction boundary between delivery and the consumer; this
    // route has already left PendingGmStationCommands and cannot be repaired
    // by the ordinary missing-destination check there.
    app.world_mut().despawn(entity);
    app.world_mut()
        .run_system_once(phoenix::gm_puppet::settle_station_puppet_feedback)
        .unwrap();
    app.world_mut()
        .run_system_once(phoenix::gm_puppet::settle_removed_station_feedback)
        .unwrap();
    let result = app.world().resource::<GmActionJournal>().applied_results()[1].clone();
    assert_eq!(result.outcome, GmActionOutcome::Refused);
    assert_eq!(
        result.reason,
        Some(GmActionRefusalReason::SystemUnavailable)
    );
    assert_eq!(result.operator_id, "gm-2");
    app.world_mut()
        .run_system_once(phoenix::gm_puppet::settle_removed_station_feedback)
        .unwrap();
    assert_eq!(app.world().resource::<GmActionLog>().entries()[1], result);
}
