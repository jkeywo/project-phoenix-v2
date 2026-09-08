//! Observation fixture only: full registered schedule, no added ordering edges.
//! The same bytes must run on source-bound pre-change and candidate builds.
#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]
use bevy::prelude::*;
use project_phoenix::{
    command_admission::log::{stamp_accepted_command, PendingCommands, ShipKey},
    console::weapons::{BlasterSystemResource, TorpedoSystemResource},
    core::messages::{
        AdmittedCommand, AdmittedCommands, SystemBlackboard, SystemControlPayload, SystemId,
        ViewscreenBlackboard,
    },
    entities::{config::TorpedoTubeConfig, spawner::EntityUuid},
    headless::{build_headless_app, HeadlessArgs},
    server_app::{Ship, ShipSystemBlackboards, WeaponFiredThisTick},
    ship::{control_source::ControlSourceResolver, state::ShipPhysics},
    ship_plugin::{ShipConfigComponent, ShipSystemControlSources},
    sim_tick::SimTick,
    weapons::{
        blaster::{BlasterBankConfig, BlasterSystem},
        torpedo::{TorpedoConfig, TorpedoSystem},
    },
    world_id::{IdNamespace, WorldId, WorldIdMint},
};
use serde_json::{json, Value};
const SHOOTER: &str = "00000000-0000-8000-8000-000000001400";
const TARGET: &str = "00000000-0000-8000-8000-000000001401";
const X: f32 = 100_000.0;

fn step(app: &mut App) -> u64 {
    let before = app.world().resource::<SimTick>().0;
    app.update();
    assert_eq!(
        app.world().resource::<SimTick>().0,
        before + 1,
        "one actual fixed step"
    );
    before
}
fn queue(app: &mut App, ship: Entity, target: &str, payload: SystemControlPayload) {
    let tick = app.world().resource::<SimTick>().0;
    stamp_accepted_command(
        &mut app.world_mut().resource_mut::<PendingCommands>(),
        tick,
        None,
        ship,
        ShipKey(SHOOTER.into()),
        AdmittedCommand {
            target: SystemId(target.into()),
            payload,
            response_token: None,
            feedback_correlation: None,
        },
    );
}
fn tube(id: &str) -> TorpedoTubeConfig {
    TorpedoTubeConfig {
        id: id.into(),
        facing_deg: 0.0,
        fire_arc_deg: 90.0,
        load_time: None,
        marker: None,
        barrels: vec![],
        pattern: vec![],
        volley_max: 2,
        ai_target_count: Some(0),
        ai: None,
    }
}
fn observe(app: &App, ship: Entity) -> Value {
    let world = app.world();
    let torp = &world.get::<TorpedoSystemResource>(ship).unwrap().0;
    let blaster = &world.get::<BlasterSystemResource>(ship).unwrap().0[0];
    let physics = world.get::<ShipPhysics>(ship).unwrap();
    let mut rounds: Vec<Value> = torp
        .in_flight
        .iter()
        .map(|p| {
            json!({
                "id":p.uuid,"family":"torpedo","mount":p.tube_id,"source":p.source_uuid,
                "target":p.target_uuid,"position":[p.x,p.y,p.z],"heading":p.heading,
                "life":p.lifespan_remaining
            })
        })
        .chain(blaster.in_flight.iter().map(|p| {
            json!({
                "id":p.id,"family":"blaster","mount":blaster.config.id,"source":p.source_uuid,
                "position":[p.x,0.0,p.z],"heading":p.heading,"life":p.lifespan_remaining
            })
        }))
        .collect();
    rounds.sort_by(|a, b| a["id"].as_str().cmp(&b["id"].as_str()));
    json!({"rounds":rounds,"physics":[physics.x,physics.y,physics.z,physics.yaw,physics.forward_speed,physics.lateral_speed,physics.vertical_speed],
        "magazine":torp.torpedoes_remaining,"loaded":torp.tubes.iter().map(|t| t.loaded_count).collect::<Vec<_>>(),
        "loading":torp.tubes.iter().map(|t| format!("{:?}",t.load_state)).collect::<Vec<_>>(),"bursts":format!("{:?}",torp.burst_states),"blaster_cycle":format!("{:?}",blaster.volley),
        "mint":world.resource::<WorldIdMint>().state()})
}

#[test]
#[ignore = "source-bound weapon ordering observation; capture before selecting production order"]
fn observe_simultaneous_projectile_identity_assignment() {
    // One test per fresh process. Programmatic seed does not enable deterministic mode.
    let deterministic = std::env::var("PHOENIX_PROJECTILE_PINNED").as_deref() == Ok("1");
    let args = HeadlessArgs {
        world_path: "assets/worlds/combat_test.toml".into(),
        ship_path: "assets/entities/alliance_destroyer.toml".into(),

        seed: Some(1400),
        deterministic,
        max_ticks: 30,
        ..Default::default()
    };
    let mut app = build_headless_app(&args).unwrap();
    let period = project_phoenix::sim_tick::sim_tick_period(
        app.world()
            .resource::<project_phoenix::world::config::WorldConfig>()
            .global
            .sim_tick_hz,
    );
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(period));
    app.finish();
    app.cleanup();
    for _ in 0..4 {
        app.update();
    }
    assert_eq!(app.world().resource::<Time<Fixed>>().timestep(), period);
    // Fixture setup removes unrelated weapons, never systems or schedule edges.
    let armed: Vec<Entity> = app
        .world_mut()
        .query_filtered::<Entity, With<Ship>>()
        .iter(app.world())
        .collect();
    for entity in armed {
        app.world_mut()
            .entity_mut(entity)
            .remove::<TorpedoSystemResource>()
            .remove::<BlasterSystemResource>();
    }
    let mut config = app
        .world_mut()
        .query::<&ShipConfigComponent>()
        .iter(app.world())
        .next()
        .unwrap()
        .0
        .clone();
    config.systems.clear();
    config.stations.clear();
    let mut torp = TorpedoSystem::from_configs(
        &[tube("fore_port"), tube("fore_starboard")],
        TorpedoConfig {
            burst_interval_secs: period.as_secs_f32() * 0.5,
            load_time: 100.0,
            ..Default::default()
        },
    );
    torp.torpedoes_remaining -= 3;
    for (id, count) in [("fore_port", 2), ("fore_starboard", 1)] {
        let tube = torp.tube_mut(id).unwrap();
        tube.loaded_count = count;
        tube.target_count = count;
    }
    let mut blackboards = ShipSystemBlackboards::default();
    blackboards.0.insert(
        project_phoenix::ship::system_registry::viewscreen_system_id(),
        SystemBlackboard::Viewscreen(ViewscreenBlackboard {
            combat_lock: Some(TARGET.into()),
            ..Default::default()
        }),
    );
    app.world_mut().spawn((
        EntityUuid(TARGET.into()),
        Transform::from_xyz(X, 0.0, -1000.0),
    ));
    let ship = app
        .world_mut()
        .spawn((
            Ship,
            EntityUuid(SHOOTER.into()),
            ShipConfigComponent(config),
            ShipSystemControlSources(ControlSourceResolver::new()),
            ShipPhysics {
                x: X,
                ..Default::default()
            },
            Transform::from_xyz(X, 0.0, 0.0),
            blackboards,
            AdmittedCommands::default(),
            WeaponFiredThisTick::default(),
            TorpedoSystemResource(torp),
            BlasterSystemResource(vec![BlasterSystem::new(BlasterBankConfig {
                id: "fore".into(),
                volley_count: 1,
                charge_time_secs: 0.0,
                cooldown_secs: 100.0,
                recoil_impulse: 2.0,
                range: 2000.0,
                ..Default::default()
            })]),
        ))
        .id();
    queue(
        &mut app,
        ship,
        "torpedo-tube-fore-port",
        SystemControlPayload::FireTorpedo { target_uuid: None },
    );
    let priming_tick = step(&mut app);
    let prime = observe(&app, ship);
    assert_eq!(
        prime["rounds"].as_array().unwrap().len(),
        1,
        "one successful prime launch"
    );
    assert_eq!(
        app.world()
            .get::<TorpedoSystemResource>(ship)
            .unwrap()
            .0
            .burst_states
            .len(),
        1
    );
    queue(
        &mut app,
        ship,
        "torpedo-tube-fore-starboard",
        SystemControlPayload::FireTorpedo { target_uuid: None },
    );
    queue(
        &mut app,
        ship,
        "blaster-fore",
        SystemControlPayload::ChargeBlasterStart,
    );
    let tick = step(&mut app);
    let observed = observe(&app, ship);
    let current: Vec<_> = observed["rounds"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|p| WorldId::parse(p["id"].as_str().unwrap()).unwrap().tick == tick)
        .collect();
    assert_eq!(
        current.len(),
        3,
        "one delayed torpedo, one immediate torpedo, one blaster"
    );
    let mut seq: Vec<_> = current
        .iter()
        .map(|p| {
            let id = WorldId::parse(p["id"].as_str().unwrap()).unwrap();
            assert_eq!(id.namespace, IdNamespace::Projectile);
            id.seq
        })
        .collect();
    seq.sort();
    assert_eq!(seq, vec![0, 1, 2]);
    assert_eq!(
        app.world()
            .resource::<WorldIdMint>()
            .minted_so_far(IdNamespace::Projectile),
        3
    );
    let torp = &app.world().get::<TorpedoSystemResource>(ship).unwrap().0;
    assert!(torp.burst_states.is_empty());
    assert!(torp.tubes.iter().all(|t| t.loaded_count == 0));
    let delayed = torp
        .in_flight
        .iter()
        .find(|p| p.tube_id == "fore_port" && WorldId::parse(&p.uuid).unwrap().tick == tick)
        .unwrap();
    let immediate = torp
        .in_flight
        .iter()
        .find(|p| p.tube_id == "fore_starboard")
        .unwrap();
    assert!(
        delayed.z < immediate.z,
        "delayed round moved in lifecycle before immediate fire"
    );
    assert!(delayed.lifespan_remaining < immediate.lifespan_remaining);
    assert_eq!(immediate.lifespan_remaining, torp.config.lifespan);
    // The emptied first tube reserves its next round in T's ordinary lifecycle.
    assert_eq!(
        observed["magazine"].as_u64().unwrap() + 1,
        prime["magazine"].as_u64().unwrap()
    );
    assert!(matches!(
        torp.tubes[0].load_state,
        project_phoenix::weapons::torpedo::TubeLoadState::Loading { .. }
    ));
    assert!(
        app.world().get::<BlasterSystemResource>(ship).unwrap().0[0]
            .volley
            .on_cooldown
    );
    assert_ne!(
        observed["physics"][4], prime["physics"][4],
        "nonzero recoil must affect physical state"
    );
    let mut continuation = Vec::new();
    for _ in 0..3 {
        step(&mut app);
        let next = observe(&app, ship);
        // The second tube reserves one round on T+1; neither 100s load completes.
        assert_eq!(
            next["magazine"].as_u64().unwrap() + 2,
            prime["magazine"].as_u64().unwrap()
        );
        let torp = &app.world().get::<TorpedoSystemResource>(ship).unwrap().0;
        assert!(torp.tubes.iter().all(|t| t.loaded_count == 0
            && matches!(
                t.load_state,
                project_phoenix::weapons::torpedo::TubeLoadState::Loading { .. }
            )));
        continuation.push(next);
    }
    let executor = format!(
        "{:?}",
        app.world()
            .resource::<Schedules>()
            .get(FixedUpdate)
            .unwrap()
            .get_executor_kind()
    );
    println!("PROJECTILE_OBSERVATION_BEGIN\n{}\nPROJECTILE_OBSERVATION_END",serde_json::to_string_pretty(&json!({
        "deterministic":deterministic,"pool_threads":bevy::tasks::ComputeTaskPool::get().thread_num(),"executor":executor,
        "priming_tick":priming_tick,"tick":tick,"prime":prime,"observed":observed,"continuation":continuation})).unwrap());
}
