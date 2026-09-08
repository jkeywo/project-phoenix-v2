//! #1400: storage-order perturbation of two ordinary authored firing ships.
//! The marker round trip changes no authoritative state or production schedule.
#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]

use bevy::{ecs::schedule::ExecutorKind, prelude::*};
use project_phoenix::{
    command_admission::log::{stamp_accepted_command, PendingCommands, ShipKey},
    console::weapons::{BlasterSystemResource, TorpedoSystemResource},
    core::messages::{
        AdmittedCommand, AdmittedCommands, SystemBlackboard, SystemControlPayload, SystemId,
    },
    entities::{
        include_resolve::load_entity_config,
        spawner::{spawn_entity, EntityUuid},
    },
    headless::{build_headless_app, HeadlessArgs},
    server_app::{Ship, ShipSystemBlackboards},
    ship::{control_source::ControlSource, state::ShipPhysics},
    ship_plugin::{ShipConfigComponent, ShipSystemControlSources},
    sim_digest::world_digest,
    sim_rng::SimRng,
    sim_tick::SimTick,
    weapons::torpedo::TubeLoadState,
    world_id::{IdNamespace, WorldId, WorldIdMint},
};
use serde_json::{json, Value};

const IDS: [&str; 2] = [
    "00000000-0000-8000-8000-000000014041",
    "00000000-0000-8000-8000-000000014042",
];
const TARGETS: [&str; 2] = [
    "00000000-0000-8000-8000-000000014043",
    "00000000-0000-8000-8000-000000014044",
];
const PREFIX: &str = "PHOENIX_PROJECTILE_TRAVERSAL=";

#[derive(Component)]
struct StoragePerturbation;

fn step(app: &mut App) {
    let tick = app.world().resource::<SimTick>().0;
    app.update();
    assert_eq!(app.world().resource::<SimTick>().0, tick + 1);
}

fn queue(app: &mut App, ship: Entity, target: &str, payload: SystemControlPayload) {
    let tick = app.world().resource::<SimTick>().0;
    let key = ShipKey(app.world().get::<EntityUuid>(ship).unwrap().0.clone());
    stamp_accepted_command(
        &mut app.world_mut().resource_mut::<PendingCommands>(),
        tick,
        None,
        ship,
        key,
        AdmittedCommand {
            target: SystemId(target.into()),
            payload,
            response_token: None,
            feedback_correlation: None,
        },
    );
}

fn observe(app: &App, ships: [Entity; 2]) -> Value {
    let world = app.world();
    let rows: Vec<_> = ships.into_iter().map(|ship| {
        let p = world.get::<ShipPhysics>(ship).unwrap();
        let torpedo = &world.get::<TorpedoSystemResource>(ship).unwrap().0;
        let blasters = &world.get::<BlasterSystemResource>(ship).unwrap().0;
        json!({
            "ship": world.get::<EntityUuid>(ship).unwrap().0,
            "physics": [p.x,p.y,p.z,p.yaw,p.forward_speed,p.lateral_speed,p.vertical_speed,p.roll],
            "torpedoes": torpedo.in_flight.iter().map(|round| json!({
                "id":round.uuid,"source":round.source_uuid,"mount":round.tube_id,
                "target":round.target_uuid,"position":[round.x,round.y,round.z],
                "heading":round.heading,"life":round.lifespan_remaining
            })).collect::<Vec<_>>(),
            "magazine":torpedo.torpedoes_remaining,
            "tubes":torpedo.tubes.iter().map(|tube|json!([tube.id,tube.loaded_count,tube.target_count,format!("{:?}",tube.load_state)])).collect::<Vec<_>>(),
            "bursts":format!("{:?}",torpedo.burst_states),
            "blasters":blasters.iter().map(|bank|json!({
                "bank":bank.config.id,"cycle":format!("{:?}",bank.volley),
                "rounds":bank.in_flight.iter().map(|round|json!({
                    "id":round.id,"source":round.source_uuid,"position":[round.x,round.z],
                    "heading":round.heading,"life":round.lifespan_remaining
                })).collect::<Vec<_>>()
            })).collect::<Vec<_>>()
        })
    }).collect();
    json!({"tick":world.resource::<SimTick>().0,"digest":world_digest(world),
        "ships":rows,"mint":world.resource::<WorldIdMint>().state(),
        "rng":world.resource::<SimRng>().state()})
}

fn query_orders(app: &mut App) -> Value {
    // These are the production queries' required component sets. Optional
    // items do not restrict membership; filtering to our UUIDs keeps other
    // ordinary world ships visible to production without prescribing them.
    let blaster: Vec<_> = app
        .world_mut()
        .query_filtered::<(
            &EntityUuid,
            &Transform,
            &ShipPhysics,
            &BlasterSystemResource,
        ), With<Ship>>()
        .iter(app.world())
        .filter(|(id, ..)| IDS.contains(&id.0.as_str()))
        .map(|(id, ..)| id.0.clone())
        .collect();
    let torpedo: Vec<_> = app
        .world_mut()
        .query_filtered::<(
            &EntityUuid,
            &ShipSystemControlSources,
            &ShipPhysics,
            &Transform,
            &AdmittedCommands,
        ), With<Ship>>()
        .iter(app.world())
        .filter(|(id, ..)| IDS.contains(&id.0.as_str()))
        .map(|(id, ..)| id.0.clone())
        .collect();
    json!({"blaster":blaster,"torpedo":torpedo})
}

fn prepare(pinned: bool) -> (App, [Entity; 2]) {
    let mut app = build_headless_app(&HeadlessArgs {
        world_path: "assets/worlds/combat_test.toml".into(),
        ship_path: "assets/entities/alliance_destroyer.toml".into(),
        seed: Some(140041),
        deterministic: pinned,
        max_ticks: 600,
        ..Default::default()
    })
    .unwrap();
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
    let config = load_entity_config("assets/entities/alliance_destroyer.toml").unwrap();
    let ships = std::array::from_fn(|i| {
        spawn_entity(
            &mut app.world_mut().commands(),
            &config,
            Vec3::new(100_000.0 + i as f32 * 200.0, 0.0, 0.0),
            IDS[i].into(),
            None,
        )
    });
    app.world_mut().flush();
    let all: Vec<_> = app
        .world_mut()
        .query_filtered::<Entity, With<Ship>>()
        .iter(app.world())
        .collect();
    for entity in all {
        let ids: Vec<_> = app
            .world()
            .get::<ShipConfigComponent>(entity)
            .unwrap()
            .0
            .systems
            .iter()
            .map(|system| system.id.clone())
            .collect();
        let mut sources = app
            .world_mut()
            .get_mut::<ShipSystemControlSources>(entity)
            .unwrap();
        for id in ids {
            sources.0.set(id, ControlSource::Human);
        }
    }
    for (i, ship) in ships.into_iter().enumerate() {
        app.world_mut().spawn((
            EntityUuid(TARGETS[i].into()),
            Transform::from_xyz(100_000.0 + i as f32 * 200.0, 0.0, -20.0),
        ));
        queue(
            &mut app,
            ship,
            "tactical-radar",
            SystemControlPayload::SetTarget {
                uuid: TARGETS[i].into(),
            },
        );
    }
    for _ in 0..4 {
        step(&mut app);
    }
    for (i, ship) in ships.into_iter().enumerate() {
        let boards = &app.world().get::<ShipSystemBlackboards>(ship).unwrap().0;
        let Some(SystemBlackboard::Viewscreen(board)) =
            boards.get(&project_phoenix::ship::system_registry::viewscreen_system_id())
        else {
            panic!("ordinary publisher must supply each ship's combat lock");
        };
        assert_eq!(board.combat_lock.as_deref(), Some(TARGETS[i]));
    }
    // Stage one already-loaded fore round, preserving the magazine count and
    // all authored configuration. Actual launch still goes through admission,
    // the ordinary gate/handler and tick-scoped projectile mint.
    for ship in ships {
        let mut torpedo = app
            .world_mut()
            .get_mut::<TorpedoSystemResource>(ship)
            .unwrap();
        let tube = torpedo.0.tube("fore").unwrap();
        assert_eq!(tube.loaded_count, 0);
        assert_eq!(tube.load_state, TubeLoadState::Unloaded);
        assert!(torpedo.0.torpedoes_remaining > 0);
        torpedo.0.torpedoes_remaining -= 1;
        let tube = torpedo.0.tube_mut("fore").unwrap();
        tube.loaded_count = 1;
        tube.target_count = 1;
        tube.load_state = TubeLoadState::Loaded;
    }
    assert_eq!(app.world().resource::<SimRng>().seed(), 140041);
    assert_eq!(
        app.get_schedule(FixedUpdate).unwrap().get_executor_kind(),
        if pinned {
            ExecutorKind::SingleThreaded
        } else {
            ExecutorKind::MultiThreaded
        }
    );
    let threads = bevy::tasks::ComputeTaskPool::get().thread_num();
    if pinned {
        assert_eq!(threads, 1);
    } else {
        assert!(threads > 1);
    }
    (app, ships)
}

#[test]
fn projectile_ids_follow_ships_across_storage_order() {
    let mut reference = None;
    for role in ["default-1", "default-2", "pinned"] {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "projectile_ship_traversal_child",
                "--ignored",
                "--nocapture",
            ])
            .env("PHOENIX_PROJECTILE_TRAVERSAL_ROLE", role)
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        print!("{stdout}");
        assert!(
            output.status.success(),
            "{role}: {}\n{}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
        let reports: Vec<_> = stdout
            .lines()
            .filter_map(|line| line.strip_prefix(PREFIX))
            .collect();
        assert_eq!(reports.len(), 1);
        let report: Value = serde_json::from_str(reports[0]).unwrap();
        assert_eq!(report["role"], role);
        if let Some(expected) = &reference {
            assert_eq!(
                expected, &report["observations"],
                "complete per-ship traces must also agree across default/default/pinned"
            );
        } else {
            reference = Some(report["observations"].clone());
        }
    }
}

#[test]
#[ignore = "fresh-process parent owns default/pinned pools"]
fn projectile_ship_traversal_child() {
    let role = std::env::var("PHOENIX_PROJECTILE_TRAVERSAL_ROLE").unwrap();
    assert!(matches!(
        role.as_str(),
        "default-1" | "default-2" | "pinned"
    ));
    let mut reference = None;
    let mut observations = Vec::new();
    for perturbed in [false, true] {
        let (mut app, ships) = prepare(role == "pinned");
        let original = observe(&app, ships);
        let order_before = query_orders(&mut app);
        if perturbed {
            app.world_mut()
                .entity_mut(ships[0])
                .insert(StoragePerturbation);
            app.world_mut()
                .entity_mut(ships[0])
                .remove::<StoragePerturbation>();
        }
        let order_after = query_orders(&mut app);
        assert_eq!(
            original,
            observe(&app, ships),
            "marker must change no authoritative state"
        );
        assert_eq!(order_before["blaster"], json!(IDS));
        assert_eq!(order_before["torpedo"], json!(IDS));
        if perturbed {
            let reversed = json!([IDS[1], IDS[0]]);
            assert_eq!(order_after["blaster"], reversed);
            assert_eq!(order_after["torpedo"], reversed);
        } else {
            assert_eq!(order_before, order_after);
        }
        if let Some(first) = &reference {
            assert_eq!(first, &original, "identical initial world state");
        } else {
            reference = Some(original.clone());
        }
        for ship in ships {
            queue(
                &mut app,
                ship,
                "torpedo-tube-fore",
                SystemControlPayload::FireTorpedo { target_uuid: None },
            );
            queue(
                &mut app,
                ship,
                "blaster-port",
                SystemControlPayload::ChargeBlasterStart,
            );
        }
        let mut trace = Vec::new();
        for _ in 0..180 {
            step(&mut app);
            trace.push(observe(&app, ships));
        }
        for (index, id) in IDS.iter().enumerate() {
            for family in ["torpedo", "blaster"] {
                let witnessed = trace.iter().any(|row| {
                    let ship = &row["ships"][index];
                    let rounds: Vec<_> = if family == "torpedo" {
                        ship["torpedoes"].as_array().unwrap().iter().collect()
                    } else {
                        ship["blasters"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .flat_map(|bank| bank["rounds"].as_array().unwrap())
                            .collect()
                    };
                    rounds.iter().any(|round| {
                        round["source"] == *id
                            && WorldId::parse(round["id"].as_str().unwrap())
                                .unwrap()
                                .namespace
                                == IdNamespace::Projectile
                    })
                });
                assert!(witnessed, "real authored {family} must launch on {id}");
            }
        }
        observations.push(json!({"perturbed":perturbed,"before":original,
            "query_before":order_before,"query_after":order_after,"trace":trace}));
    }
    println!(
        "\n{PREFIX}{}",
        json!({"role":role,"process":std::process::id(),
        "threads":bevy::tasks::ComputeTaskPool::get().thread_num(),"observations":observations})
    );
    assert_eq!(observations[0]["trace"],observations[1]["trace"],
        "same ships/commands must retain per-ship projectile identities and state after storage movement");
}
