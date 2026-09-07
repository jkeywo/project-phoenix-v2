//! A real seeded host snapshot retains the independent latch across continuation.
#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]
use bevy::{ecs::system::RunSystemOnce, prelude::*};
use project_phoenix as phoenix;

fn boot() -> App {
    let mut app = phoenix::headless::build_headless_app(&phoenix::headless::HeadlessArgs {
        world_path: "tests/fixtures/worlds/layer_dynamic_resume.toml".into(),
        seed: Some(1312),
        deterministic: true,
        ..Default::default()
    })
    .unwrap();
    app.finish();
    app.cleanup();
    for _ in 0..400 {
        app.update();
    }
    app
}

#[test]
fn seeded_system_latch_snapshot_continuation_matches_uninterrupted_host() {
    use phoenix::{
        command_admission::HostSlot,
        entities::spawner::EntityUuid,
        gm_action::*,
        ship::components::{ShipConfigComponent, ShipSystemControlSources},
    };
    let mut live = boot();
    let mut query=live.world_mut().query_filtered::<(&EntityUuid,&ShipConfigComponent),With<phoenix::server_app::LocalShip>>();
    let (uuid, config) = query.single(live.world()).unwrap();
    let target = uuid.0.clone();
    let system = config
        .0
        .systems
        .iter()
        .find(|s| s.kind == "red_alert")
        .expect("host has red alert")
        .id
        .clone();
    let tick = live.world().resource::<phoenix::sim_tick::SimTick>().0;
    let slot = HostSlot(1);
    live.world_mut()
        .resource_mut::<GmActionJournal>()
        .insert(GmActionGrant {
            from: slot,
            sequenced_by: slot,
            operator_id: "gm-one".into(),
            correlation: GmActionId::new("disable-before-save").unwrap(),
            recovery_generation: 0,
            apply_tick: tick,
            order: GmActionOrder::new(slot, 1),
            action: GmAction::SetSystemDisabled {
                target: target.clone(),
                system: system.clone(),
                disabled: true,
            },
        })
        .unwrap();
    live.world_mut().run_system_once(apply_due_actions).unwrap();
    let saved = phoenix::snapshot::capture(live.world());
    assert!(saved
        .entities
        .iter()
        .find(|row| row.uuid == target)
        .unwrap()
        .gm_disabled_systems
        .contains(&system.0));
    let mut resumed = boot();
    let report = phoenix::snapshot::restore(resumed.world_mut(), &saved);
    assert!(report.is_complete(), "{:?}", report.gaps);
    assert_eq!(
        phoenix::sim_digest::world_digest(live.world()),
        phoenix::sim_digest::world_digest(resumed.world())
    );
    for app in [&mut live, &mut resumed] {
        let mut q = app
            .world_mut()
            .query::<(&EntityUuid, &ShipSystemControlSources)>();
        let (_, sources) = q.iter(app.world()).find(|(id, _)| id.0 == target).unwrap();
        assert!(sources.0.is_gm_disabled(&system));
        assert!(!sources.0.policy_for(&system).coordinate);
        app.update();
    }
    assert_eq!(
        phoenix::sim_digest::world_digest(live.world()),
        phoenix::sim_digest::world_digest(resumed.world())
    );
}

#[test]
fn seeded_system_disable_restore_replays_recorded_actions_and_preserves_hull() {
    use phoenix::{
        command_admission::HostSlot,
        entities::spawner::{EntitySystemHull, EntityUuid},
        gm_action::*,
        headless::{
            replay::{drive_run, drive_run_with_gm_actions, PhoenixSim},
            verify_artifact, HeadlessArgs, ReplayArtifact,
        },
        ship::components::ShipSystemControlSources,
    };
    let args = HeadlessArgs {
        world_path: "assets/worlds/patrol.toml".into(),
        ship_path: "assets/entities/alliance_cruiser.toml".into(),
        max_ticks: 260,
        seed: Some(1312),
        deterministic: true,
        ..Default::default()
    };
    let mut discovery = drive_run(&args, &[], 25).unwrap();
    let target = {
        let world = discovery.app_mut().world_mut();
        let mut q = world.query_filtered::<&EntityUuid, With<phoenix::server_app::LocalShip>>();
        q.single(world).unwrap().0.clone()
    };
    let system = phoenix::ship::system_registry::helm_radar_system_id();
    let inspect = |sim: &mut PhoenixSim| {
        let world = sim.app_mut().world_mut();
        let mut q = world.query::<(
            &EntityUuid,
            &EntitySystemHull,
            &ShipSystemControlSources,
            &phoenix::modifiers::ShipModifiers,
        )>();
        let (_, hull, sources, modifiers) =
            q.iter(world).find(|(id, _, _, _)| id.0 == target).unwrap();
        (
            hull.0.clone(),
            sources.0.is_gm_disabled(&system),
            modifiers.get(&phoenix::core::messages::ModifierSlot::HelmRadarRange),
            world.resource::<GmActionLog>().entries().to_vec(),
        )
    };
    let baseline = inspect(&mut discovery);
    let mut planned = GmActionJournal::default();
    for (index, disabled) in [true, true, false, true].into_iter().enumerate() {
        let from = HostSlot(2);
        planned
            .insert(GmActionGrant {
                from,
                sequenced_by: HostSlot(1),
                operator_id: "gm-replay".into(),
                correlation: GmActionId::new(format!("system-replay-{index}")).unwrap(),
                recovery_generation: 0,
                apply_tick: 120,
                order: GmActionOrder::new(from, index as u64 + 1),
                action: GmAction::SetSystemDisabled {
                    target: target.clone(),
                    system: system.clone(),
                    disabled,
                },
            })
            .unwrap();
    }
    let mut recorded = drive_run_with_gm_actions(&args, &[], &planned, 25).unwrap();
    let final_state = inspect(&mut recorded);
    assert_eq!(
        final_state.0, baseline.0,
        "GM latch never changes hull or damage configuration"
    );
    assert!(final_state.1);
    assert_eq!(
        final_state.2, 0.0,
        "ordinary radar capability is suppressed"
    );
    assert_eq!(
        final_state
            .3
            .iter()
            .map(|fact| fact.outcome)
            .collect::<Vec<_>>(),
        [
            GmActionOutcome::Applied,
            GmActionOutcome::NoOp,
            GmActionOutcome::Applied,
            GmActionOutcome::Applied
        ]
    );
    let actions = recorded.recorded_gm_actions();
    let tick = recorded.tick();
    let artifact = ReplayArtifact::capture(
        &args,
        recorded.recorded_log(),
        actions,
        tick,
        recorded.seal(),
    )
    .unwrap();
    let artifact = ReplayArtifact::from_ron(&artifact.to_ron().unwrap()).unwrap();
    assert_eq!(verify_artifact(&artifact).unwrap(), None);
    let mut replayed = drive_run_with_gm_actions(
        &artifact.replay_args(),
        artifact.log.entries(),
        &artifact.gm_actions,
        25,
    )
    .unwrap();
    assert_eq!(
        inspect(&mut replayed),
        final_state,
        "independent ordinary replay reproduces latch, HP, capability and durable results"
    );
    assert_eq!(replayed.seal().final_digest, artifact.ledger.final_digest);
    assert_ne!(discovery.seal().final_digest, artifact.ledger.final_digest);
}

#[test]
fn disabled_sensor_radar_is_rebuilt_before_the_first_restored_input_consumer() {
    use phoenix::{
        core::messages::ModifierSlot,
        entities::spawner::{EntityUuid, FactionComponent},
        modifiers::{coordination::apply_radar_damage_modifiers, ShipModifiers},
        ship::{
            components::ShipSystemControlSources,
            sensors::{tick_sensors_threat_warning, SensorsThreatState},
        },
    };
    fn host() -> App {
        let mut app = phoenix::headless::build_headless_app(&phoenix::headless::HeadlessArgs {
            world_path: "tests/fixtures/worlds/gm_system_radar_resume.toml".into(),
            ship_path: "assets/entities/alliance_cruiser.toml".into(),
            seed: Some(1312),
            deterministic: true,
            ..Default::default()
        })
        .unwrap();
        app.finish();
        app.cleanup();
        for _ in 0..400 {
            app.update();
        }
        app
    }
    fn local(app: &mut App) -> Entity {
        let mut q = app
            .world_mut()
            .query_filtered::<Entity, With<phoenix::server_app::LocalShip>>();
        q.single(app.world()).unwrap()
    }
    let mut live = host();
    let player = local(&mut live);
    let self_faction = live.world().get::<FactionComponent>(player).unwrap().0;
    let (enemy, enemy_uuid) = {
        let world = live.world_mut();
        let mut q = world.query::<(Entity, &EntityUuid, &FactionComponent)>();
        let registry = &world
            .resource::<phoenix::entities::config_cache::FactionRegistryResource>()
            .0;
        q.iter(world)
            .find(|(_, _, f)| {
                phoenix::ai::faction::is_enemy(Some(self_faction), Some(f.0), registry)
            })
            .map(|(e, id, _)| (e, id.0.clone()))
            .expect("the radar restore fixture has an explicitly hostile contact")
    };
    let player_position = *live
        .world()
        .get::<phoenix::ship::state::ShipPhysics>(player)
        .unwrap();
    if let Some(mut physics) = live
        .world_mut()
        .get_mut::<phoenix::ship::state::ShipPhysics>(enemy)
    {
        physics.x = player_position.x + 10.0;
        physics.z = player_position.z;
    }
    if let Some(mut transform) = live.world_mut().get_mut::<Transform>(enemy) {
        transform.translation.x = player_position.x + 10.0;
        transform.translation.z = player_position.z;
    }
    live.world_mut()
        .run_system_once(tick_sensors_threat_warning)
        .unwrap();
    assert_eq!(
        live.world()
            .get::<SensorsThreatState>(player)
            .unwrap()
            .last_threat_uuid
            .as_deref(),
        Some(enemy_uuid.as_str()),
        "enabled radar must actually detect this hostile"
    );
    live.world_mut()
        .entity_mut(player)
        .insert(SensorsThreatState::default());
    let enabled = phoenix::snapshot::capture(live.world());
    let sid = phoenix::ship::system_registry::sensor_radar_system_id();
    live.world_mut()
        .get_mut::<ShipSystemControlSources>(player)
        .unwrap()
        .0
        .set_gm_disabled(sid.clone(), true);
    live.world_mut()
        .run_system_once(apply_radar_damage_modifiers)
        .unwrap();
    let saved = phoenix::snapshot::capture(live.world());
    let mut resumed = host();
    let resumed_player = local(&mut resumed);
    let report = phoenix::snapshot::restore(resumed.world_mut(), &saved);
    assert!(report.is_complete(), "{:?}", report.gaps);
    for (app, ship) in [(&mut live, player), (&mut resumed, resumed_player)] {
        assert_eq!(
            app.world()
                .get::<ShipModifiers>(ship)
                .unwrap()
                .get(&ModifierSlot::SensorRadarRange),
            0.0,
            "restore must settle range before Input, without an intervening update"
        );
        app.world_mut()
            .run_system_once(tick_sensors_threat_warning)
            .unwrap();
        assert!(
            app.world()
                .get::<SensorsThreatState>(ship)
                .unwrap()
                .last_threat_uuid
                .is_none(),
            "first Input cannot detect through a disabled radar"
        );
    }
    assert_eq!(
        phoenix::sim_digest::world_digest(live.world()),
        phoenix::sim_digest::world_digest(resumed.world())
    );
    live.update();
    resumed.update();
    assert_eq!(
        phoenix::sim_digest::world_digest(live.world()),
        phoenix::sim_digest::world_digest(resumed.world())
    );
    let report = phoenix::snapshot::restore(resumed.world_mut(), &enabled);
    assert!(report.is_complete(), "{:?}", report.gaps);
    assert!(!resumed
        .world()
        .get::<ShipSystemControlSources>(resumed_player)
        .unwrap()
        .0
        .is_gm_disabled(&sid));
    assert!(
        resumed
            .world()
            .get::<ShipModifiers>(resumed_player)
            .unwrap()
            .get(&ModifierSlot::SensorRadarRange)
            > 0.0,
        "an empty saved latch must clear bootstrap suppression"
    );
    resumed
        .world_mut()
        .run_system_once(tick_sensors_threat_warning)
        .unwrap();
    assert_eq!(
        resumed
            .world()
            .get::<SensorsThreatState>(resumed_player)
            .unwrap()
            .last_threat_uuid
            .as_deref(),
        Some(enemy_uuid.as_str())
    );
}
