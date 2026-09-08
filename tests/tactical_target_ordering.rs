//! #1400 Tactical behavior under declared order and actual registration changes.
//! No simulation observer systems, cadence overrides, or digest fixtures.
#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]

use bevy::{ecs::message::MessageCursor, ecs::schedule::ExecutorKind, prelude::*};
use project_phoenix::{
    ai::cadence::AiTickReady,
    console::weapons::{
        ActiveBeam, BlasterSystemResource, LastShipAttacker, PhaserCombatConfigResource,
        TacticalRadarSelection,
    },
    core::{
        balance::BalanceEvent,
        messages::{
            AdmittedCommands, ClientMessage, GamePhase, SystemBlackboard, SystemControlPayload,
            SystemId,
        },
    },
    entities::spawner::{EntitySystemHull, EntityUuid, FactionComponent},
    headless::HeadlessArgs,
    lobby::{server::InboundMessage, Sessions},
    server_app::{LocalShip, Ship, ShipSystemBlackboards},
    ship::{
        control_source::ControlSource,
        damage::SystemHull,
        state::{ShipPhysics, ShipRedAlert},
    },
    ship_plugin::{LastHelmInput, ShipConfigComponent, ShipSystemControlSources},
    sim_digest::world_digest,
    sim_tick::SimTick,
};
use serde_json::{json, Value};

mod common;
#[path = "common/declared_order.rs"]
mod declared_order;

#[path = "tactical_target_ordering/order_proof.rs"]
mod order_proof;

const WORLD: &str = "assets/worlds/combat_test.toml";
const HULL: &str = "assets/entities/alliance_destroyer.toml";
const A: &str = "00000000-0000-8000-8000-000000014001";
const B: &str = "00000000-0000-8000-8000-000000014002";
const TOKEN: &str = "tactical-baseline-human";
const PREFIX: &str = "PHOENIX_TACTICAL_BASELINE=";
const X: f32 = 100_000.0;

fn lock(app: &App, ship: Entity) -> Option<String> {
    match app
        .world()
        .get::<ShipSystemBlackboards>(ship)
        .unwrap()
        .0
        .get(&project_phoenix::ship::system_registry::viewscreen_system_id())
    {
        Some(SystemBlackboard::Viewscreen(bb)) => bb.combat_lock.clone(),
        _ => None,
    }
}

fn select(app: &mut App, uuid: &str) {
    app.world_mut()
        .resource_mut::<Messages<InboundMessage>>()
        .write(InboundMessage {
            token: TOKEN.into(),
            msg: ClientMessage::ControlSystem {
                target: project_phoenix::ship::system_registry::tactical_radar_system_id(),
                payload: SystemControlPayload::SetTarget { uuid: uuid.into() },
            },
        });
}

fn enable_banks(app: &mut App, ship: Entity, radar_ai: bool) {
    let config = app
        .world()
        .get::<ShipConfigComponent>(ship)
        .unwrap()
        .0
        .clone();
    let mut sources = app
        .world_mut()
        .get_mut::<ShipSystemControlSources>(ship)
        .unwrap();
    for system in &config.systems {
        let source = if matches!(
            system.kind.as_str(),
            project_phoenix::ship::system_registry::PHASER_BANK_KIND
                | project_phoenix::ship::system_registry::BLASTER_BANK_KIND
        ) {
            ControlSource::Ai
        } else {
            ControlSource::Human
        };
        sources.0.set(system.id.clone(), source);
    }
    sources.0.set(
        project_phoenix::ship::system_registry::tactical_radar_system_id(),
        if radar_ai {
            ControlSource::Ai
        } else {
            ControlSource::Human
        },
    );
}

fn observe(
    app: &App,
    ship: Entity,
    targets: [Entity; 2],
    reader: &mut MessageCursor<BalanceEvent>,
) -> Value {
    let world = app.world();
    let beam = world.get::<ActiveBeam>(ship).unwrap();
    let blasters = world.get::<BlasterSystemResource>(ship).unwrap();
    let physics = world.get::<ShipPhysics>(ship).unwrap();
    let events: Vec<_> = reader.read(world.resource::<Messages<BalanceEvent>>())
        .map(|event| json!({"exact":format!("{event:?}"),"fact":serde_json::from_str::<Value>(&event.to_json()).unwrap()})).collect();
    let config = &world.get::<ShipConfigComponent>(ship).unwrap().0;
    let sources = &world.get::<ShipSystemControlSources>(ship).unwrap().0;
    json!({
        "tick":world.resource::<SimTick>().0,
        "digest":format!("{:016x}",world_digest(world)),
        "selection":world.get::<TacticalRadarSelection>(ship).unwrap().0,
        "combat_lock":lock(app,ship),
        "next_ai_ready":world.resource::<AiTickReady>().0,
        "control_sources":config.systems.iter().filter(|s|matches!(s.kind.as_str(),project_phoenix::ship::system_registry::PHASER_BANK_KIND | project_phoenix::ship::system_registry::BLASTER_BANK_KIND | project_phoenix::ship::system_registry::TACTICAL_RADAR_KIND)).map(|s|json!({"system":s.id,"source":format!("{:?}",sources.source_for(&s.id))})).collect::<Vec<_>>(),
        "beam":beam.live_banks().map(|(bank,slot)|json!({"bank":bank,"victim":slot.target_uuid,"remaining":slot.remaining_secs,"accumulator":slot.damage_accumulator,"cooldown":slot.pending_cooldown_secs})).collect::<Vec<_>>(),
        "blasters":blasters.0.iter().map(|bank|json!({
            "bank":bank.config.id,"volley":format!("{:?}",bank.volley),
            "projectiles":bank.in_flight.iter().map(|p|json!({"id":p.id,"source":p.source_uuid,"position":[p.x,p.z],"heading":p.heading,"life":p.lifespan_remaining})).collect::<Vec<_>>()
        })).collect::<Vec<_>>(),
        "targets":targets.into_iter().map(|entity|world.get::<EntitySystemHull>(entity).map(|h|h.0.total_current())).collect::<Vec<_>>(),
        "physics":[physics.x,physics.y,physics.z,physics.yaw,physics.forward_speed,physics.lateral_speed,physics.roll,physics.vertical_speed],
        "events":events,
    })
}

fn step(
    app: &mut App,
    ship: Entity,
    targets: [Entity; 2],
    reader: &mut MessageCursor<BalanceEvent>,
    ticks: &mut Vec<Value>,
    orders: &mut Vec<Value>,
) {
    let before = app.world().resource::<SimTick>().0;
    let prior_lock = lock(app, ship);
    let ready = app.world().resource::<AiTickReady>().0;
    app.update();
    assert_eq!(
        app.world().resource::<SimTick>().0,
        before + 1,
        "exactly one fixed step"
    );
    let commands = &app.world().get::<AdmittedCommands>(ship).unwrap().0;
    orders.push(json!({"tick":before,"ai_ready":ready,"frozen_lock_consumed":prior_lock,
        "commands":commands.iter().map(|c|json!({"target":c.target,"payload":c.payload,"origin":if c.response_token.as_deref()==Some(TOKEN){"fixture-human"}else if c.response_token.as_deref().is_some_and(|t|t.starts_with("ai:")){"ai"}else{"other"}})).collect::<Vec<_>>() }));
    ticks.push(observe(app, ship, targets, reader));
}

#[test]
fn tactical_target_baseline_agrees_across_fresh_default_default_and_pinned_runs() {
    // No App or global pool is created in this parent. Each case and mode gets
    // a fresh process; default seeds are programmatic, not CLI --seed pins.
    for case in ["retarget", "clear", "death-reacquire"] {
        let mut reports = Vec::new();
        for role in ["default-1", "default-2", "pinned"] {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "tactical_target_baseline_child",
                    "--ignored",
                    "--nocapture",
                ])
                .env("PHOENIX_TACTICAL_BASELINE_ROLE", role)
                .env("PHOENIX_TACTICAL_BASELINE_CASE", case)
                .env_remove("PHOENIX_TACTICAL_TEST_ORDER")
                .current_dir(env!("CARGO_MANIFEST_DIR"))
                .output()
                .expect("fresh baseline child");
            let stdout = String::from_utf8_lossy(&output.stdout);
            assert!(
                output.status.success(),
                "{case}/{role}: {}\n{stdout}\n{}",
                output.status,
                String::from_utf8_lossy(&output.stderr)
            );
            let lines: Vec<_> = stdout
                .lines()
                .filter_map(|line| line.strip_prefix(PREFIX))
                .collect();
            assert_eq!(lines.len(), 1, "exactly one actual child report: {stdout}");
            let report: Value = serde_json::from_str(lines[0]).unwrap();
            assert_eq!(report["role"], role);
            assert_eq!(report["case"], case);
            println!("\n{PREFIX}{}", lines[0]);
            reports.push(report);
        }
        // The raw shared-vector append order is retained as an observation,
        // not silently turned into an ordering requirement. Actual state,
        // victim/event timing and every tick digest must agree independently.
        println!(
            "PHOENIX_TACTICAL_ORDER_COMPARISON={}",
            json!({"case":case,
            "default_orders_equal":reports[0]["orders"]==reports[1]["orders"],
            "pinned_orders_equal":reports[0]["orders"]==reports[2]["orders"]})
        );
        assert_eq!(
            reports[0]["ticks"], reports[1]["ticks"],
            "{case}: fresh default runs diverged"
        );
        assert_eq!(
            reports[0]["ticks"], reports[2]["ticks"],
            "{case}: default/pinned gameplay diverged"
        );
        for report in &reports[1..] {
            assert_eq!(
                order_proof::per_target(&reports[0]),
                order_proof::per_target(report)
            );
        }
    }
}

#[test]
#[ignore = "parent launches this source-bound observation in a fresh process for each role/case"]
fn tactical_target_baseline_child() {
    let role = std::env::var("PHOENIX_TACTICAL_BASELINE_ROLE").expect("parent role");
    let case = std::env::var("PHOENIX_TACTICAL_BASELINE_CASE").expect("parent case");
    assert!(matches!(
        role.as_str(),
        "default-1" | "default-2" | "pinned"
    ));
    assert!(matches!(
        case.as_str(),
        "retarget" | "clear" | "death-reacquire"
    ));
    let pinned = role == "pinned";
    let args = HeadlessArgs {
        world_path: WORLD.into(),
        ship_path: HULL.into(),
        seed: Some(140013),
        deterministic: pinned,
        max_ticks: 600,
        ..Default::default()
    };
    let order = std::env::var("PHOENIX_TACTICAL_TEST_ORDER").unwrap_or_else(|_| "ordinary".into());
    let mut app = declared_order::build(args, &order);
    let period = project_phoenix::sim_tick::sim_tick_period(
        app.world()
            .resource::<project_phoenix::world::config::WorldConfig>()
            .global
            .sim_tick_hz,
    );
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(period));
    app.finish();
    app.cleanup();
    let proof = order_proof::install(&mut app, &order);
    assert_eq!(
        app.world()
            .resource::<project_phoenix::sim_rng::SimRng>()
            .seed(),
        140013,
        "same programmatic seed in every real executor mode"
    );
    let threads = bevy::tasks::ComputeTaskPool::get().thread_num();
    if pinned {
        assert_eq!(threads, 1)
    } else {
        assert!(
            threads > 1,
            "default pool must really have multiple workers"
        )
    }
    assert_eq!(
        app.get_schedule(FixedUpdate).unwrap().get_executor_kind(),
        if pinned {
            ExecutorKind::SingleThreaded
        } else {
            ExecutorKind::MultiThreaded
        }
    );
    for _ in 0..4 {
        app.update();
    }
    let order_proof = proof.finish(&mut app);
    assert_eq!(
        *app.world().resource::<State<GamePhase>>().get(),
        GamePhase::InProgress
    );
    let ship = app
        .world_mut()
        .query_filtered::<Entity, With<LocalShip>>()
        .single(app.world())
        .unwrap();
    let shooter = app.world().get::<EntityUuid>(ship).unwrap().0.clone();
    let config = app
        .world()
        .get::<ShipConfigComponent>(ship)
        .unwrap()
        .0
        .clone();
    let station = config.weapons_station().expect("authored Tactical station");
    let station_config = config.stations.iter().find(|s| s.id == station).unwrap();
    assert!(!station_config.ratings.is_empty());
    {
        let mut sessions = app.world_mut().resource_mut::<Sessions>();
        sessions
            .0
            .register(TOKEN.into(), "Tactical baseline".into())
            .unwrap();
        sessions.0.set_station(TOKEN, Some(station));
    }
    // Controlled fixture placement/state. Keep every production system and
    // authored bank/policy/cadence value; no hybrid hull or instant-fire tuning.
    app.world_mut().entity_mut(ship).insert((
        ShipPhysics {
            x: X,
            ..Default::default()
        },
        LastHelmInput::default(),
        Transform::from_xyz(X, 0.0, 0.0),
        ShipRedAlert(true),
    ));
    {
        let mut sources = app
            .world_mut()
            .get_mut::<ShipSystemControlSources>(ship)
            .unwrap();
        for system in &config.systems {
            sources.0.set(system.id.clone(), ControlSource::Human);
        }
    }
    assert!(!app
        .world()
        .get::<PhaserCombatConfigResource>(ship)
        .unwrap()
        .0
        .banks
        .is_empty());
    assert!(!app
        .world()
        .get::<BlasterSystemResource>(ship)
        .unwrap()
        .0
        .is_empty());
    let faction = uuid::Uuid::parse_str("cccccccc-3333-4333-8333-cccccccccccc").unwrap();
    let mut target = |id: &str, x: f32, hp: f32| {
        app.world_mut()
            .spawn((
                Ship,
                EntityUuid(id.into()),
                Transform::from_xyz(X + x, 0.0, -20.0),
                FactionComponent(faction),
                EntitySystemHull(SystemHull::from_config(&[(
                    SystemId("target-hull".into()),
                    hp,
                )])),
            ))
            .id()
    };
    let targets = [
        target(
            A,
            2.0,
            if case == "death-reacquire" {
                1.0
            } else {
                500.0
            },
        ),
        target(B, -2.0, 500.0),
    ];
    let mut reader = MessageCursor::<BalanceEvent>::default();
    reader.clear(app.world().resource::<Messages<BalanceEvent>>());
    let mut ticks = Vec::new();
    let mut orders = Vec::new();
    select(&mut app, A);
    step(
        &mut app,
        ship,
        targets,
        &mut reader,
        &mut ticks,
        &mut orders,
    );
    assert_eq!(
        app.world()
            .get::<TacticalRadarSelection>(ship)
            .unwrap()
            .0
            .as_deref(),
        Some(A),
        "ordinary admitted initial target"
    );
    assert_eq!(
        lock(&app, ship).as_deref(),
        Some(A),
        "ordinary PublishAggregate established the prior frozen lock"
    );
    assert!(
        !app.world().get::<ActiveBeam>(ship).unwrap().is_firing(),
        "human banks do not auto-fire during priming"
    );
    for _ in 0..60 {
        if app.world().resource::<AiTickReady>().0 {
            break;
        }
        step(
            &mut app,
            ship,
            targets,
            &mut reader,
            &mut ticks,
            &mut orders,
        );
    }
    assert!(
        app.world().resource::<AiTickReady>().0,
        "authored AI cadence must arm within one simulated second"
    );
    enable_banks(&mut app, ship, case == "death-reacquire");
    let transition_tick = app.world().resource::<SimTick>().0;
    match case.as_str() {
        "retarget" => select(&mut app, B),
        "clear" => select(&mut app, ""),
        "death-reacquire" => {
            app.world_mut().get_mut::<LastShipAttacker>(ship).unwrap().0 = Some(A.into())
        }
        _ => unreachable!(),
    }
    step(
        &mut app,
        ship,
        targets,
        &mut reader,
        &mut ticks,
        &mut orders,
    );
    if case != "death-reacquire" {
        let expected = if case == "retarget" { Some(B) } else { None };
        assert_eq!(
            app.world()
                .get::<TacticalRadarSelection>(ship)
                .unwrap()
                .0
                .as_deref(),
            expected
        );
        assert_eq!(lock(&app, ship).as_deref(), expected);
        assert_eq!(
            app.world().get::<ActiveBeam>(ship).unwrap().any_target(),
            Some(A),
            "same-tick auto-fire consumes prior published lock, not newly applied selection"
        );
        assert!(app
            .world()
            .get::<AdmittedCommands>(ship)
            .unwrap()
            .0
            .iter()
            .any(|c| matches!(c.payload, SystemControlPayload::FirePhaser)));
        assert!(app
            .world()
            .get::<AdmittedCommands>(ship)
            .unwrap()
            .0
            .iter()
            .any(|c| matches!(c.payload, SystemControlPayload::ChargeBlasterStart)));
    }
    let mut death_tick = None;
    let mut reacquired_tick = None;
    for _ in 0..90 {
        if case == "death-reacquire" && app.world().get::<EntitySystemHull>(targets[0]).is_none() {
            death_tick.get_or_insert(app.world().resource::<SimTick>().0);
            // A real weapon destroyed A. The last-attacker candidate now names
            // the second live hostile; the authored AI selector/applier decides.
            app.world_mut().get_mut::<LastShipAttacker>(ship).unwrap().0 = Some(B.into());
        }
        step(
            &mut app,
            ship,
            targets,
            &mut reader,
            &mut ticks,
            &mut orders,
        );
        if case == "death-reacquire"
            && app
                .world()
                .get::<TacticalRadarSelection>(ship)
                .unwrap()
                .0
                .as_deref()
                == Some(B)
        {
            reacquired_tick.get_or_insert(app.world().resource::<SimTick>().0);
        }
    }
    // Emit observed facts before the coverage checks so an actual failure keeps
    // its timeline in the parent's captured stdout as well as its exit status.
    println!(
        "\n{PREFIX}{}",
        json!({"schema":1,"process":std::process::id(),"role":role,"case":case,"world":WORLD,"hull":HULL,"seed":140013,"shooter":shooter,"compute_threads":threads,"fixed_update_executor":format!("{:?}",app.get_schedule(FixedUpdate).unwrap().get_executor_kind()),"transition_tick":transition_tick,"death_tick":death_tick,"reacquired_tick":reacquired_tick,"ticks":ticks,"orders":orders,"order_proof":order_proof})
    );
    if case == "death-reacquire" {
        assert!(
            death_tick.is_some(),
            "actual ordinary weapon damage must destroy A"
        );
        assert!(
            reacquired_tick > death_tick,
            "AI must reacquire the live second hostile after death"
        );
        assert!(
            orders
                .iter()
                .any(|row| row["commands"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|c| c["origin"] == "ai"
                        && c["target"] == "tactical-radar"
                        && c["payload"]
                            == serde_json::to_value(SystemControlPayload::SetTarget {
                                uuid: B.into()
                            })
                            .unwrap())),
            "actual AI admitted reacquisition of B, excluding the initial human prime"
        );
    }
    let events: Vec<_> = ticks
        .iter()
        .flat_map(|t| t["events"].as_array().unwrap())
        .map(|e| &e["fact"])
        .collect();
    assert!(
        events
            .iter()
            .any(|e| e["event"] == "WeaponFired" && e["kind"] == "beam" && e["shooter"] == shooter),
        "controlled ship's beam actually fired"
    );
    assert!(
        events.iter().any(|e| e["event"] == "WeaponFired"
            && e["kind"] == "blaster"
            && e["shooter"] == shooter),
        "controlled ship's blaster actually fired through its authored cycle"
    );
    assert!(
        events.iter().any(|e| e["event"] == "DamageApplied"
            && e["victim"] == A
            && e["attacker"] == shooter
            && e["hull_damage"].as_f64().unwrap() > 0.0),
        "actual damage attributed to the controlled ship and victim A"
    );
    if case == "death-reacquire" {
        assert!(
            events.iter().any(|e| e["event"] == "EntityDestroyed"
                && e["victim"] == A
                && e["killer"] == shooter),
            "actual weapon destruction, not fixture despawn"
        );
        assert!(
            events.iter().any(|e| e["event"] == "DamageApplied"
                && e["victim"] == B
                && e["attacker"] == shooter
                && e["hull_damage"].as_f64().unwrap() > 0.0),
            "reacquired victim B actually takes damage through the ordinary fire path"
        );
    }
}
