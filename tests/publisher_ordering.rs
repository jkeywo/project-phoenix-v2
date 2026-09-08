//! Bounded #1400 proof: three existing publishers, no production annotation.
#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]

use bevy::{ecs::message::MessageCursor, prelude::*};
use project_phoenix::{
    console::repair::server::ShipRepairTeams,
    core::messages::{ClientMessage, PowerGroupId, ServerMessage, SystemBlackboard, SystemId},
    entities::spawner::{EntitySystemHull, EntityUuid},
    headless::{build_headless_app, HeadlessArgs},
    lobby::server::{InboundMessage, OutboundMessage},
    server_app::{LastBroadcastBlackboards, LocalShip, Ship, ShipSystemBlackboards},
    ship::{control_source::ControlSource, power::ShipPowerSystem, shields::ShipShields},
    ship_plugin::{ShipConfigComponent, ShipSystemControlSources},
    sim_digest::world_digest,
    sim_rng::SimRng,
    sim_tick::{sim_tick_period, SimTick},
    world::config::WorldConfig,
};
use serde_json::{json, Value};
use std::collections::BTreeMap;

#[path = "publisher_ordering/order_proof.rs"]
mod order_proof;

const PREFIX: &str = "PHOENIX_PUBLISHER_PROOF=";
const SEED: u64 = 1400231;

fn step(app: &mut App) {
    let before = app.world().resource::<SimTick>().0;
    app.update();
    assert_eq!(app.world().resource::<SimTick>().0, before + 1);
}

fn boards(app: &App, ship: Entity) -> Value {
    let sorted: BTreeMap<_, _> = app
        .world()
        .get::<ShipSystemBlackboards>(ship)
        .unwrap()
        .0
        .iter()
        .map(|(id, value)| (id.0.clone(), value))
        .collect();
    serde_json::to_value(sorted).unwrap()
}

fn observe(
    app: &App,
    ships: &[Entity],
    messages: &mut MessageCursor<OutboundMessage>,
    wire: &mut Vec<OutboundMessage>,
) -> Value {
    let world = app.world();
    let cache: BTreeMap<_, _> = world
        .resource::<LastBroadcastBlackboards>()
        .0
        .iter()
        .map(|(id, value)| (id.0.clone(), value))
        .collect();
    let observed: Vec<_> = messages
        .read(world.resource::<Messages<OutboundMessage>>())
        .cloned()
        .collect();
    let payloads: Vec<_> = observed.iter()
        .map(|m| json!({"target":format!("{:?}",m.target),"delivery":format!("{:?}",m.delivery),"message":m.msg}))
        .collect();
    wire.extend(observed);
    let repair_cache = world
        .resource::<project_phoenix::console::repair::visibility::LastVisibleRepairBlackboard>(
    );
    let repair_projections: BTreeMap<_, _> = repair_cache.projections.iter().collect();
    let repair_stations: BTreeMap<_, _> = repair_cache.stations.iter().collect();
    let hull_cache: BTreeMap<_, _> = world
        .resource::<project_phoenix::console::repair::visibility::LastBroadcastHull>()
        .0
        .iter()
        .map(|(token, value)| (token.clone(), format!("{value:?}")))
        .collect();
    json!({"tick":world.resource::<SimTick>().0,"digest":world_digest(world),
        "ships":ships.iter().map(|ship|json!({
            "uuid":world.get::<EntityUuid>(*ship).unwrap().0,"boards":boards(app,*ship),
            "power":format!("{:?}",world.get::<ShipPowerSystem>(*ship).unwrap().0.read_state()),
            "shields":format!("{:?}",world.get::<ShipShields>(*ship).unwrap().0.snapshot()),
            "repair":format!("{:?}",world.get::<ShipRepairTeams>(*ship).unwrap().0),
            "hull":world.get::<EntitySystemHull>(*ship).unwrap().0.iter()
                .map(|(id,h)|json!({"id":id,"current":h.current,"max":h.max})).collect::<Vec<_>>()
        })).collect::<Vec<_>>(),"cache":cache,"hull_cache":hull_cache,
        "repair_cache":{"projections":repair_projections,"stations":repair_stations},
        "payloads":payloads})
}

fn assert_selected_wire(wire: &[OutboundMessage]) {
    use project_phoenix::lobby::Target;
    let (mut power, mut shields, mut repair, mut withheld) = (false, false, false, false);
    let (mut engineering_hull, mut science_hull) = (false, false);
    for message in wire {
        match &message.msg {
            ServerMessage::BlackboardUpdate { updates } => {
                for (id, board) in updates {
                    match (id.0.as_str(), board, &message.target) {
                        ("power", SystemBlackboard::Power(_), Target::All) => power = true,
                        ("shields", SystemBlackboard::Shields(_), Target::All) => shields = true,
                        ("repair", SystemBlackboard::Repair(board), Target::Token(token))
                            if token == "publisher-engineer" =>
                        {
                            repair |= !board.teams.is_empty()
                                && !board.damageable_systems.is_empty()
                                && board.aggregate_hull_fraction.is_some_and(|hp| hp < 1.0);
                        }
                        ("repair", SystemBlackboard::Repair(board), Target::Token(token))
                            if token == "publisher-science" =>
                        {
                            assert!(board.teams.is_empty());
                            assert!(board.system_hull.is_empty());
                            assert!(board.damageable_systems.is_empty());
                            assert!(board.priority_targets.is_empty());
                            assert!(board.queue_depth.is_empty());
                            assert_eq!(board.aggregate_hull_fraction, None);
                            assert_eq!(board.destroyed_hull_fraction, None);
                            assert_eq!(board.external_dispatch_range, None);
                            assert_eq!(board.external_dispatch_target, None);
                            assert_eq!(board.external_dispatch_target_name, None);
                            assert_eq!(board.external_dispatch_candidate_name, None);
                            assert_eq!(board.external_dispatch_candidate_refusal, None);
                            assert_eq!(board.external_dispatch_refusal, None);
                            assert_eq!(board.external_dispatch_team_idx, None);
                            assert_eq!(board.external_dispatch_target_condition, None);
                            withheld = true;
                        }
                        _ => {}
                    }
                }
            }
            ServerMessage::SystemHullUpdate {
                aggregate_fraction: Some(hp),
                ..
            } if *hp < 1.0 => {
                if let Target::Token(token) = &message.target {
                    engineering_hull |= token == "publisher-engineer";
                    science_hull |= token == "publisher-science";
                }
            }
            _ => {}
        }
    }
    assert!(
        power && shields,
        "both selected shared publishers reached the wire"
    );
    assert!(
        repair,
        "Engineering received actual damaged-hull Repair detail"
    );
    assert!(
        withheld,
        "Science received the explicit empty Repair projection"
    );
    assert!(
        engineering_hull && science_hull,
        "both recipients received the changed Hull aggregate"
    );
}

fn seed_transition(app: &mut App, ships: &[Entity], phase: usize) {
    for (index, ship) in ships.iter().copied().enumerate() {
        let mut power = app.world_mut().get_mut::<ShipPowerSystem>(ship).unwrap();
        let group = PowerGroupId("weapons".into());
        let floor = power.0.floor_for(&group);
        power
            .0
            .set_group_allocation(&group, if phase == 0 { floor } else { floor + 1 })
            .unwrap();
        drop(power);
        let count = app
            .world()
            .get::<ShipShields>(ship)
            .unwrap()
            .0
            .snapshot()
            .len();
        assert!(count > 1);
        app.world_mut()
            .get_mut::<ShipShields>(ship)
            .unwrap()
            .0
            .set_focused_facing(Some((phase + index) % count));
        let (id, max) = app
            .world()
            .get::<EntitySystemHull>(ship)
            .unwrap()
            .0
            .iter()
            .next()
            .map(|(id, entry)| (id.clone(), entry.max))
            .unwrap();
        app.world_mut()
            .get_mut::<EntitySystemHull>(ship)
            .unwrap()
            .0
            .set_hp(&id, max * if phase == 0 { 0.6 } else { 0.8 });
        let mut teams = app.world_mut().get_mut::<ShipRepairTeams>(ship).unwrap();
        assert!(!teams.0.slots().is_empty());
        teams.0.dispatch(0, id, "fixture target".into());
    }
}

fn seed_lock_boundary(app: &mut App, local: Entity, phase: usize) {
    use project_phoenix::{console::weapons::TacticalRadarSelection, ship::state::ShipPhysics};
    let physics = *app.world().get::<ShipPhysics>(local).unwrap();
    let old = format!("publisher-prior-{phase}");
    let next = format!("publisher-next-{phase}");
    app.world_mut().spawn((
        EntityUuid(old.clone()),
        Transform::from_xyz(physics.x + 20.0, physics.y, physics.z),
    ));
    app.world_mut().spawn((
        EntityUuid(next.clone()),
        Transform::from_xyz(physics.x - 20.0, physics.y, physics.z),
    ));
    let mut maps = app
        .world_mut()
        .get_mut::<ShipSystemBlackboards>(local)
        .unwrap();
    let Some(SystemBlackboard::Viewscreen(board)) = maps
        .0
        .get_mut(&project_phoenix::ship::system_registry::viewscreen_system_id())
    else {
        panic!("real aggregate required")
    };
    board.combat_lock = Some(old);
    drop(maps);
    app.world_mut()
        .get_mut::<TacticalRadarSelection>(local)
        .unwrap()
        .0 = Some(next);
}

fn assert_lock_boundary(app: &mut App, local: Entity, phase: usize, first: bool) {
    use project_phoenix::ship::state::ShipPhysics;
    let id = format!("publisher-{}-{phase}", if first { "prior" } else { "next" });
    let target = app
        .world_mut()
        .query::<(&EntityUuid, &Transform)>()
        .iter(app.world())
        .find_map(|(uuid, transform)| (uuid.0 == id).then_some(transform.translation))
        .unwrap();
    let maps = &app.world().get::<ShipSystemBlackboards>(local).unwrap().0;
    let Some(SystemBlackboard::Viewscreen(view)) =
        maps.get(&project_phoenix::ship::system_registry::viewscreen_system_id())
    else {
        panic!("viewscreen")
    };
    assert_eq!(
        view.combat_lock.as_deref(),
        Some(format!("publisher-next-{phase}").as_str())
    );
    let Some(SystemBlackboard::Shields(shields)) = maps.get(&SystemId("shields".into())) else {
        panic!("shields")
    };
    let owner = app.world().get::<ShipPhysics>(local).unwrap();
    let bearing = ((project_phoenix::simmath::atan2(target.z - owner.z, target.x - owner.x)
        - owner.yaw
        + std::f32::consts::PI)
        % std::f32::consts::TAU)
        .to_degrees();
    assert!(
        (shields.combat_lock_bearing.unwrap() - bearing).abs() < 0.001,
        "preserve previous aggregate lock at Publish"
    );
}

#[test]
fn three_publishers_preserve_maps_consumers_and_wire_in_opposed_orders() {
    let mut reference: Option<Value> = None;
    let mut external: Option<Value> = None;
    let mut external_edges: Option<Value> = None;
    for order in ["ordinary", "forward", "reverse", "rotated"] {
        for mode in ["default-1", "default-2", "pinned"] {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "publisher_order_child",
                    "--ignored",
                    "--nocapture",
                    "--test-threads=1",
                ])
                .env("PHOENIX_PUBLISHER_ORDER", order)
                .env("PHOENIX_PUBLISHER_MODE", mode)
                .current_dir(env!("CARGO_MANIFEST_DIR"))
                .output()
                .unwrap();
            let stdout = String::from_utf8_lossy(&output.stdout);
            print!("{stdout}");
            assert!(
                output.status.success(),
                "{order}/{mode}: {}\n{}",
                output.status,
                String::from_utf8_lossy(&output.stderr)
            );
            let rows: Vec<_> = stdout
                .lines()
                .filter_map(|line| line.strip_prefix(PREFIX))
                .collect();
            assert_eq!(rows.len(), 1);
            let report: Value = serde_json::from_str(rows[0]).unwrap();
            assert_eq!(report["order"], order);
            assert_eq!(report["mode"], mode);
            assert_eq!(report["seed"], SEED);
            assert_ne!(report["process"], std::process::id());
            assert!(report["process"].as_u64().unwrap() > 0);
            assert_eq!(report["trace"].as_array().unwrap().len(), 24);
            if mode == "pinned" {
                assert_eq!(report["workers"], 1);
                assert_eq!(report["executor"], "SingleThreaded");
            } else {
                assert!(report["workers"].as_u64().unwrap() > 1);
                assert_eq!(report["executor"], "MultiThreaded");
            }
            assert_eq!(report["setup"].as_array().unwrap().len(), 6);
            let observations = json!({"setup":report["setup"],"trace":report["trace"]});
            if let Some(expected) = &reference {
                assert_eq!(&observations, expected);
            } else {
                reference = Some(observations);
            }
            if let Some(expected) = &external {
                assert_eq!(&report["graph"]["external_conflicts"], expected);
            } else {
                external = Some(report["graph"]["external_conflicts"].clone());
            }
            if let Some(expected) = &external_edges {
                assert_eq!(&report["graph"]["external_edges"], expected);
            } else {
                external_edges = Some(report["graph"]["external_edges"].clone());
            }
        }
    }
}

#[test]
#[ignore = "fresh-process parent owns each actual pool and test-local order"]
fn publisher_order_child() {
    let mode = std::env::var("PHOENIX_PUBLISHER_MODE").unwrap();
    assert!(matches!(
        mode.as_str(),
        "default-1" | "default-2" | "pinned"
    ));
    let order = std::env::var("PHOENIX_PUBLISHER_ORDER").unwrap();
    let mut app = build_headless_app(&HeadlessArgs {
        world_path: "assets/worlds/probe_fleet_duel.toml".into(),
        ship_path: "assets/entities/alliance_cruiser.toml".into(),
        seed: Some(SEED),
        deterministic: mode == "pinned",
        ..Default::default()
    })
    .unwrap();
    let proof = order_proof::install(&mut app, &order);
    let period = sim_tick_period(app.world().resource::<WorldConfig>().global.sim_tick_hz);
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(period));
    app.finish();
    app.cleanup();
    for _ in 0..6 {
        app.update();
    }
    let graph = proof.finish(&mut app);
    let workers = bevy::tasks::ComputeTaskPool::get().thread_num();
    let executor = app.get_schedule(FixedUpdate).unwrap().get_executor_kind();
    assert_eq!(app.world().resource::<SimRng>().seed(), SEED);
    if mode == "pinned" {
        assert_eq!(workers, 1);
        assert_eq!(executor, bevy::ecs::schedule::ExecutorKind::SingleThreaded);
    } else {
        assert!(workers > 1);
        assert_eq!(executor, bevy::ecs::schedule::ExecutorKind::MultiThreaded);
    }
    let mut ships: Vec<_> = app
        .world_mut()
        .query_filtered::<(Entity, &EntityUuid), (
            With<Ship>,
            With<ShipRepairTeams>,
            With<ShipPowerSystem>,
            With<ShipShields>,
        )>()
        .iter(app.world())
        .map(|(entity, uuid)| (uuid.0.clone(), entity))
        .collect();
    ships.sort_by(|a, b| a.0.cmp(&b.0));
    assert!(ships.len() >= 2, "real second authored hull required");
    let ships: Vec<_> = ships.into_iter().map(|(_, entity)| entity).collect();
    for ship in &ships {
        let config = app
            .world()
            .get::<ShipConfigComponent>(*ship)
            .unwrap()
            .0
            .clone();
        let mut sources = app
            .world_mut()
            .get_mut::<ShipSystemControlSources>(*ship)
            .unwrap();
        for system in &config.systems {
            sources.0.set(system.id.clone(), ControlSource::Human);
        }
    }
    let mut cursor = MessageCursor::<OutboundMessage>::default();
    let mut wire = Vec::new();
    let mut setup = Vec::new();
    let mut observed_science_host_change = false;
    for (token, station) in [
        ("publisher-engineer", "engineering"),
        ("publisher-science", "science"),
    ] {
        for msg in [
            ClientMessage::Identify {
                token: token.into(),
                name: token.into(),
            },
            ClientMessage::SelectStation {
                station: station.into(),
            },
            ClientMessage::SetReady { ready: true },
        ] {
            let prior_host = app
                .world()
                .resource::<project_phoenix::comms::server::CommsRuntime>()
                .last_broadcast_host
                .clone();
            let wire_start = wire.len();
            app.world_mut().write_message(InboundMessage {
                token: token.into(),
                msg,
            });
            step(&mut app);
            setup.push(observe(&app, &ships, &mut cursor, &mut wire));
            let current_host = &app
                .world()
                .resource::<project_phoenix::comms::server::CommsRuntime>()
                .last_broadcast_host;
            if current_host != &prior_host {
                if let Some(host) = current_host {
                    assert_eq!(
                        wire[wire_start..]
                            .iter()
                            .filter(|message| {
                                message.target
                                    == project_phoenix::lobby::Target::Token(host.clone())
                                    && message.delivery
                                        == project_phoenix::core::messages::DeliveryClass::Reliable
                                    && matches!(message.msg, ServerMessage::CommsState { .. })
                            })
                            .count(),
                        1,
                        "a changed Comms host receives state in the same tick"
                    );
                    observed_science_host_change |= host == "publisher-science";
                }
            }
        }
        assert_eq!(
            app.world()
                .resource::<project_phoenix::lobby::Sessions>()
                .0
                .holder_for_station(&project_phoenix::core::messages::StationId(station.into())),
            Some(token)
        );
    }
    assert!(observed_science_host_change);
    let local = app
        .world_mut()
        .query_filtered::<Entity, With<LocalShip>>()
        .single(app.world())
        .unwrap();
    let mut trace = Vec::new();
    for phase in 0..2 {
        let before = boards(&app, local);
        seed_transition(&mut app, &ships, phase);
        seed_lock_boundary(&mut app, local, phase);
        if phase == 0 {
            for ship in &ships {
                let mut maps = app
                    .world_mut()
                    .get_mut::<ShipSystemBlackboards>(*ship)
                    .unwrap();
                let before = maps.0.len();
                maps.0.retain(|id, _| {
                    !matches!(
                        id.0.as_str(),
                        "power" | "power-reactor" | "power-battery" | "shields" | "repair"
                    ) && !id.0.starts_with("shield-arc-")
                });
                assert!(
                    before >= maps.0.len() + 2,
                    "real existing output before insertion arm"
                );
            }
        }
        for tick in 0..12 {
            step(&mut app);
            assert_lock_boundary(&mut app, local, phase, tick == 0);
            let row = observe(&app, &ships, &mut cursor, &mut wire);
            if tick == 0 {
                let current = boards(&app, local);
                for key in ["power", "shields", "repair"] {
                    assert_ne!(current[key], before[key], "nonvacuous owner {key}");
                }
                assert!(matches!(
                    app.world()
                        .get::<ShipSystemBlackboards>(local)
                        .unwrap()
                        .0
                        .get(&SystemId("repair".into())),
                    Some(SystemBlackboard::Repair(_))
                ));
            }
            trace.push(row);
        }
    }
    assert_selected_wire(&wire);
    assert_ne!(
        trace[0]["ships"][0]["repair"], trace[11]["ships"][0]["repair"],
        "real repair continuation advances, not just copied initial output"
    );
    println!(
        "\n{PREFIX}{}",
        json!({"mode":mode,"order":order,"seed":SEED,"process":std::process::id(),
        "workers":workers,"executor":format!("{executor:?}"),"graph":graph,"setup":setup,"trace":trace})
    );
}
