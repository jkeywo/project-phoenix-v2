//! #1400: four actual producers / six pairs, without production annotations.
//! Accepted prefixes exercise ordinary continuation, not network authentication.
#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]
mod common;
#[path = "common/declared_order.rs"]
mod declared_order;
use bevy::{ecs::message::MessageCursor, prelude::*};
use project_phoenix::{
    ai::{cadence::AiSnapshotReady, server::AiHighFidelity},
    command_admission::{log::stamp_accepted_command, CommandLog, PendingCommands, ShipKey},
    console::{navigation::server::NavigationWaypoint, repair::server::ShipRepairTeams},
    core::{
        balance::BalanceEvent,
        messages::{
            ActionCorrelationId, ActionFeedbackOutcome, AdmittedCommand, AdmittedCommands,
            DeliveryClass, PowerGroupId, ServerMessage, SystemBlackboard,
            SystemControlPayload as Payload, SystemId,
        },
    },
    entities::spawner::{EntitySystemHull, EntityUuid},
    headless::HeadlessArgs,
    lobby::OutboundMessage,
    server_app::{LocalShip, Ship, ShipSystemBlackboards},
    ship::{
        control_source::ControlSource,
        power::{PowerConfigResource, ShipPowerSystem},
        shields::{PendingShieldsThreatBearing, ShipShields},
        state::{ShipPhaserFrequency, ShipRedAlert},
        system_registry,
    },
    ship_plugin::{ShipConfigComponent, ShipSystemControlSources},
    sim_digest::world_digest,
    sim_rng::SimRng,
    sim_tick::{sim_tick_period, SimTick},
    world::config::WorldConfig,
    world_id::WorldIdMint,
};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
#[path = "admitted_producer_ordering/domains.rs"]
mod domains;
#[path = "admitted_producer_ordering/fixture.rs"]
mod fixture;
#[path = "admitted_producer_ordering/graph_proof.rs"]
mod graph_proof;
#[path = "admitted_producer_ordering/probes.rs"]
mod probes;
const PREFIX: &str = "PHOENIX_PRODUCER_SIX=";
const SEED: u64 = 1400006;
const TOKEN: &str = "producer-prefix-fixture";

#[derive(Default)]
struct Observation {
    wire: MessageCursor<OutboundMessage>,
    balance: MessageCursor<BalanceEvent>,
    repaired: BTreeSet<String>,
    trace: Vec<Value>,
    raw: Vec<Value>,
}
fn step(app: &mut App, ships: &[Entity], observed: &mut Observation) {
    let before = app.world().resource::<SimTick>().0;
    app.update();
    let world = app.world();
    assert_eq!(
        world.resource::<SimTick>().0,
        before + 1,
        "one actual fixed boundary"
    );
    let events:Vec<_>=observed.balance.read(world.resource::<Messages<BalanceEvent>>()).map(|event|{
        if let BalanceEvent::RepairApplied{ship,hp}=event {if *hp>0.0 {observed.repaired.insert(ship.clone());}}
        json!({"exact":format!("{event:?}"),"fact":serde_json::from_str::<Value>(&event.to_json()).unwrap()})
    }).collect();
    let wire: Vec<_> = observed
        .wire
        .read(world.resource::<Messages<OutboundMessage>>())
        .map(|m| {
            json!({
        "target":format!("{:?}",m.target),"delivery":format!("{:?}",m.delivery),"message":m.msg})
        })
        .collect();
    let mut ship_rows = Vec::new();
    let mut raw = Vec::new();
    for ship in ships {
        let uuid = &world.get::<EntityUuid>(*ship).unwrap().0;
        let commands = &world.get::<AdmittedCommands>(*ship).unwrap().0;
        let arcs = &world.resource::<probes::Probes>().arcs[uuid];
        let projected: Vec<_> = (0..4)
            .map(|i| domains::evidence(&domains::projected(i, commands, arcs)))
            .collect();
        let foreign: Vec<_> = commands
            .iter()
            .filter(|c| !(0..4).any(|i| domains::owns(i, c, arcs)))
            .cloned()
            .collect();
        let boards: BTreeMap<_, _> = world
            .get::<ShipSystemBlackboards>(*ship)
            .unwrap()
            .0
            .iter()
            .map(|(id, b)| (&id.0, b))
            .collect();
        let sources = &world.get::<ShipSystemControlSources>(*ship).unwrap().0;
        let config = &world.get::<ShipConfigComponent>(*ship).unwrap().0;
        ship_rows.push(json!({"uuid":uuid,"commands_by_consumer":projected,"foreign_prefix":domains::evidence(&foreign),
            "power":format!("{:?}",world.get::<ShipPowerSystem>(*ship).unwrap().0.read_state()),
            "shields":format!("{:?}",world.get::<ShipShields>(*ship).unwrap().0.snapshot()),
            "focus":world.get::<ShipShields>(*ship).unwrap().0.focused_facing,
            "waypoint":format!("{:?}",world.get::<NavigationWaypoint>(*ship).unwrap()),
            "teams":world.get::<ShipRepairTeams>(*ship).unwrap().0.slots(),
            "hull":world.get::<EntitySystemHull>(*ship).unwrap().0.iter().map(|(id,h)|json!({"id":id,"current":h.current,"max":h.max})).collect::<Vec<_>>(),
            "frequency":world.get::<ShipPhaserFrequency>(*ship).unwrap().0,
            "red_alert":world.get::<ShipRedAlert>(*ship).unwrap().0,"boards":boards,
            "controls":config.systems.iter().map(|s|json!({"id":s.id,"policy":format!("{:?}",sources.policy_for(&s.id))})).collect::<Vec<_>>()
        }));
        raw.push(json!({"uuid":uuid,"commands":domains::evidence(commands)}));
    }
    observed.raw.push(json!({"tick":before,"ships":raw}));
    observed.trace.push(json!({"tick":world.resource::<SimTick>().0,"digest":format!("{:016x}",world_digest(world)),
        "rng":world.resource::<SimRng>().state(),"mint":world.resource::<WorldIdMint>().state(),
        "log":world.resource::<CommandLog>().entries(),"ships":ship_rows,"events":events,"wire":wire}));
}
fn wait_ready(app: &mut App, ships: &[Entity], observed: &mut Observation) {
    for _ in 0..30 {
        if app.world().resource::<AiSnapshotReady>().0 {
            return;
        }
        step(app, ships, observed);
    }
    panic!("ordinary AI cadence did not arm");
}
fn sources(app: &mut App, ships: &[Entity], enabled: bool) {
    for ship in ships {
        let config = app
            .world()
            .get::<ShipConfigComponent>(*ship)
            .unwrap()
            .0
            .clone();
        let arcs: Vec<_> = app
            .world()
            .get::<ShipShields>(*ship)
            .unwrap()
            .0
            .facings
            .iter()
            .map(|f| system_registry::shield_arc_system_id(&f.id).unwrap())
            .collect();
        let focus: Vec<_> = config
            .systems
            .iter()
            .filter(|s| s.kind == system_registry::SHIELDS_KIND)
            .map(|s| s.id.clone())
            .collect();
        assert_eq!(focus.len(), 1, "real authored focus capability");
        let mut selected = vec![
            system_registry::power_reactor_system_id(),
            system_registry::navigation_system_id(),
            SystemId("repair".into()),
        ];
        selected.extend(focus);
        selected.extend(arcs);
        let mut control = app
            .world_mut()
            .get_mut::<ShipSystemControlSources>(*ship)
            .unwrap();
        for id in selected {
            control.0.set(
                id.clone(),
                if enabled {
                    ControlSource::Ai
                } else {
                    ControlSource::Human
                },
            );
            assert_eq!(
                control.0.policy_for(&id).operate_ai,
                enabled,
                "actual selected authority {id:?}"
            );
        }
    }
}
fn prefix(app: &mut App, ships: &[Entity], label: &str) -> BTreeMap<String, Vec<AdmittedCommand>> {
    let tick = app.world().resource::<SimTick>().0;
    let mut result = BTreeMap::new();
    for ship in ships {
        let uuid = app.world().get::<EntityUuid>(*ship).unwrap().0.clone();
        // Explicit precondition: the real Captain consumer must restore true.
        app.world_mut().get_mut::<ShipRedAlert>(*ship).unwrap().0 = false;
        assert!(!app.world().get::<ShipRedAlert>(*ship).unwrap().0);
        let commands: Vec<_> = [false, true]
            .into_iter()
            .enumerate()
            .map(|(i, active)| AdmittedCommand {
                target: SystemId(system_registry::RED_ALERT_SYSTEM_ID.into()),
                payload: Payload::SetRedAlert { active },
                response_token: Some(TOKEN.into()),
                feedback_correlation: Some(
                    ActionCorrelationId::new(format!("{label}-{uuid}-{i}")).unwrap(),
                ),
            })
            .collect();
        for command in &commands {
            stamp_accepted_command(
                &mut app.world_mut().resource_mut::<PendingCommands>(),
                tick,
                None,
                *ship,
                ShipKey(uuid.clone()),
                command.clone(),
            );
        }
        result.insert(uuid, commands);
    }
    result
}
fn prepare_power_shields(app: &mut App, ships: &[Entity]) {
    for (index, ship) in ships.iter().enumerate() {
        let capacity = app
            .world()
            .get::<PowerConfigResource>(*ship)
            .unwrap()
            .0
            .capacity;
        let mut power = app.world_mut().get_mut::<ShipPowerSystem>(*ship).unwrap();
        let ids: Vec<_> = power.0.iter().map(|(id, _)| id.clone()).collect();
        // Reachable resting reactor state, retaining the actual authored policy/budget.
        for id in ids {
            power.0.set_group_allocation(&id, 2).unwrap();
        }
        power.0.battery_charge = capacity;
        drop(power);
        app.world_mut().get_mut::<ShipRedAlert>(*ship).unwrap().0 = true;
        let mut shields = app.world_mut().get_mut::<ShipShields>(*ship).unwrap();
        assert!(shields.0.facings.len() > index);
        let bearing = shields.0.facings[index].center_deg.to_radians();
        shields.0.set_focused_facing(None);
        drop(shields);
        app.world_mut()
            .get_mut::<PendingShieldsThreatBearing>(*ship)
            .unwrap()
            .0 = Some(bearing);
    }
}
fn assert_prefix_applied(app: &App, ships: &[Entity]) {
    let world = app.world();
    let mut cursor = MessageCursor::<OutboundMessage>::default();
    let output: Vec<_> = cursor
        .read(world.resource::<Messages<OutboundMessage>>())
        .collect();
    let expected = &world.resource::<probes::Probes>().expected_prefix;
    for ship in ships {
        assert!(
            world.get::<ShipRedAlert>(*ship).unwrap().0,
            "actual per-ship Captain consumer applied the final true request from false"
        );
        let uuid = &world.get::<EntityUuid>(*ship).unwrap().0;
        let commands = &expected[uuid];
        assert_eq!(commands.len(), 2);
        for (command, active) in commands.iter().zip([false, true]) {
            assert_eq!(command.target.0, system_registry::RED_ALERT_SYSTEM_ID);
            assert_eq!(command.payload, Payload::SetRedAlert { active });
            let correlation = command.feedback_correlation.as_ref().unwrap();
            let matching: Vec<_> = output
                .iter()
                .filter(|message| {
                    matches!(
                        &message.msg, ServerMessage::ActionFeedback { correlation: actual, .. }
                            if actual == correlation
                    )
                })
                .collect();
            assert_eq!(matching.len(), 1, "each actual prefix command settles once");
            assert_eq!(
                matching[0].target,
                project_phoenix::lobby::Target::Token(TOKEN.into())
            );
            assert_eq!(matching[0].delivery, DeliveryClass::Reliable);
            assert!(
                matches!(
                    &matching[0].msg,
                    ServerMessage::ActionFeedback {
                        outcome: ActionFeedbackOutcome::Applied,
                        ..
                    }
                ),
                "both idempotent false and effective true requests are consumed"
            );
        }
    }
}
fn probe_evidence(app: &App) -> Value {
    let p = app.world().resource::<probes::Probes>();
    assert_eq!(p.before.len(), 8);
    assert_eq!(p.after.len(), 8);
    json!(p.before.iter().map(|((producer,uuid),before)|json!({"producer":producer,"uuid":uuid,
        "before":domains::evidence(before),"after":domains::evidence(&p.after[&(*producer,uuid.clone())])})).collect::<Vec<_>>())
}
#[test]
fn four_producers_preserve_prefix_effects_and_consumer_subsequences() {
    let mut reference = None;
    let mut ordinary = None;
    let mut completed = Vec::new();
    let mut registration_roles = std::collections::BTreeMap::new();
    for order in ["ordinary", "shuffle-a", "shuffle-b"] {
        for mode in ["default-1", "default-2", "pinned"] {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "producer_order_child",
                    "--ignored",
                    "--nocapture",
                    "--test-threads=1",
                ])
                .env("PHOENIX_PRODUCER_ORDER", order)
                .env("PHOENIX_PRODUCER_MODE", mode)
                .current_dir(env!("CARGO_MANIFEST_DIR"))
                .output()
                .unwrap();
            let stdout = String::from_utf8(output.stdout).unwrap();
            let stderr = String::from_utf8_lossy(&output.stderr);
            print!("{stdout}");
            eprint!("{stderr}");
            assert!(
                output.status.success(),
                "{order}/{mode}: {stderr}\n{stdout}"
            );
            assert!(stdout.contains("test result: ok. 1 passed; 0 failed; 0 ignored"));
            let rows: Vec<Value> = stdout
                .lines()
                .filter_map(|line| line.strip_prefix(PREFIX))
                .map(|s| serde_json::from_str(s).unwrap())
                .collect();
            assert_eq!(rows.len(), 1, "one framed child report");
            let report = &rows[0];
            assert_eq!(report["schema"], 1);
            assert_eq!(report["seed"], SEED);
            assert_eq!(report["order"], order);
            declared_order::observe_role(&mut registration_roles, &report["graph"]);
            assert_eq!(report["mode"], mode);
            assert_eq!(report["ships"].as_array().unwrap().len(), 2);
            assert_eq!(report["positive_producers"], 8);
            assert_eq!(report["human_controlled_noemissions"], 8);
            assert!(report["interleavings"].as_u64().unwrap() >= 24);
            assert!(
                report["trace"].as_array().unwrap().len() > 300,
                "authored Repair travel and work observed"
            );
            if let Some(baseline) = &ordinary {
                graph_proof::assert_preserved(baseline, &report["graph"]);
            } else {
                ordinary = Some(report["graph"].clone());
            }
            let gameplay = json!({"ships":report["ships"],"trace":report["trace"]});
            if let Some(baseline) = &reference {
                assert_eq!(
                    baseline, &gameplay,
                    "actual per-tick effects, complete consumer subsequences and wire"
                );
            } else {
                reference = Some(gameplay);
            }
            completed.push(json!({"mode":mode,"order":order,"pid":report["pid"],"workers":report["workers"],"positive_producers":8,"human_controlled_noemissions":8}));
        }
    }
    assert_eq!(completed.len(), 9);
    let pids: BTreeSet<_> = completed
        .iter()
        .map(|r| r["pid"].as_u64().unwrap())
        .collect();
    assert_eq!(pids.len(), 9, "nine fresh processes");
    println!(
        "\nPHOENIX_PRODUCER_SIX_PARENT={}",
        json!({"schema":1,"children":completed,"pairs":6})
    );
}
#[test]
#[ignore = "fresh-process child driven by the ordinary parent"]
fn producer_order_child() {
    let mode = std::env::var("PHOENIX_PRODUCER_MODE").unwrap();
    let order = std::env::var("PHOENIX_PRODUCER_ORDER").unwrap();
    assert!(matches!(
        mode.as_str(),
        "default-1" | "default-2" | "pinned"
    ));
    let mut app = declared_order::build(
        HeadlessArgs {
            world_path: "assets/worlds/probe_fleet_duel.toml".into(),
            ship_path: "assets/entities/alliance_cruiser.toml".into(),
            seed: Some(SEED),
            deterministic: mode == "pinned",
            ..Default::default()
        },
        &order,
    );
    probes::install(&mut app);
    let proof = graph_proof::install(&mut app, &order);
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
    if mode == "pinned" {
        assert_eq!(workers, 1);
        assert_eq!(executor, bevy::ecs::schedule::ExecutorKind::SingleThreaded);
    } else {
        assert!(workers > 1);
        assert_eq!(executor, bevy::ecs::schedule::ExecutorKind::MultiThreaded);
    }
    assert_eq!(app.world().resource::<SimRng>().seed(), SEED);
    // This world supplies a second GameStart cruiser and a script-authored
    // hostile cruiser. Both are ordinary NPCs: no LocalShip human-seeking
    // resolver can silently overwrite the fixture's per-system authorities.
    let mut ships: Vec<_> = app
        .world_mut()
        .query_filtered::<Entity, (With<Ship>, Without<LocalShip>)>()
        .iter(app.world())
        .collect();
    assert_eq!(ships.len(), 2, "two actual non-local authored cruisers");
    ships.sort_by_key(|e| app.world().get::<EntityUuid>(*e).unwrap().0.clone());
    for ship in &ships {
        assert!(
            app.world()
                .get::<project_phoenix::lockstep::FleetSlotOf>(*ship)
                .is_none(),
            "no live/frozen crew override hidden behind test control assignments"
        );
    }
    let all_ships: Vec<_> = app
        .world_mut()
        .query_filtered::<Entity, With<Ship>>()
        .iter(app.world())
        .collect();
    for ship in all_ships {
        let config = app
            .world()
            .get::<ShipConfigComponent>(ship)
            .unwrap()
            .0
            .clone();
        let mut cs = app
            .world_mut()
            .get_mut::<ShipSystemControlSources>(ship)
            .unwrap();
        for s in &config.systems {
            cs.0.set(s.id.clone(), ControlSource::Human);
        }
    }
    let mut arcs = BTreeMap::new();
    for ship in &ships {
        assert!(
            app.world().get::<AiHighFidelity>(*ship).is_some(),
            "ordinary fidelity promotion"
        );
        let uuid = app.world().get::<EntityUuid>(*ship).unwrap().0.clone();
        let ids: Vec<_> = app
            .world()
            .get::<ShipShields>(*ship)
            .unwrap()
            .0
            .facings
            .iter()
            .map(|f| system_registry::shield_arc_system_id(&f.id).unwrap())
            .collect();
        assert!(ids.len() >= 2);
        arcs.insert(uuid, ids);
    }
    {
        let mut probes = app.world_mut().resource_mut::<probes::Probes>();
        probes.ships = ships.clone();
        probes.arcs = arcs;
    }
    let expected = fixture::prepare_navigation_and_repair(&mut app, &ships);
    let mut observed = Observation::default();
    // Actual aggregate publishes the prepared doctrines while producers remain Human.
    for _ in 0..4 {
        step(&mut app, &ships, &mut observed);
    }
    wait_ready(&mut app, &ships, &mut observed);
    for ship in &ships {
        let map = &app.world().get::<ShipSystemBlackboards>(*ship).unwrap().0;
        let Some(SystemBlackboard::Viewscreen(view)) =
            map.get(&system_registry::viewscreen_system_id())
        else {
            panic!("actual prior aggregate");
        };
        assert!(
            view.scored_objectives.iter().any(|o| o.score > 0.0),
            "published positive doctrine"
        );
    }
    prepare_power_shields(&mut app, &ships);
    sources(&mut app, &ships, true);
    let admitted_prefix = prefix(&mut app, &ships, "positive");
    app.world_mut()
        .resource_mut::<probes::Probes>()
        .begin(admitted_prefix.clone());
    step(&mut app, &ships, &mut observed);
    let positive_probes = probe_evidence(&app);
    let mut interleavings = 0;
    for (index, ship) in ships.iter().enumerate() {
        let uuid = &app.world().get::<EntityUuid>(*ship).unwrap().0;
        let p = app.world().resource::<probes::Probes>();
        let chunks = p.chunks(uuid);
        assert!(
            chunks.iter().all(|c| !c.is_empty()),
            "actual positive emission from all four producers on {uuid}"
        );
        interleavings += domains::assert_actual_chunk_interleavings(
            &admitted_prefix[uuid],
            &chunks,
            &p.arcs[uuid],
        );
        assert_eq!(
            app.world()
                .get::<ShipPowerSystem>(*ship)
                .unwrap()
                .0
                .commanded_level_for(&PowerGroupId("weapons".into())),
            3,
            "actual Power consumer applied authored alert allocation"
        );
        assert_eq!(
            app.world()
                .get::<ShipShields>(*ship)
                .unwrap()
                .0
                .focused_facing,
            Some(index),
            "actual Shield consumer applied own bearing"
        );
    }
    fixture::assert_navigation_and_repair(&app, &ships, &expected, false);
    assert_prefix_applied(&app, &ships);
    app.world_mut().resource_mut::<probes::Probes>().active = false;
    // Settled Power and Navigation must not re-emit a standing order.
    wait_ready(&mut app, &ships, &mut observed);
    let settled_prefix = prefix(&mut app, &ships, "settled");
    app.world_mut()
        .resource_mut::<probes::Probes>()
        .begin(settled_prefix);
    step(&mut app, &ships, &mut observed);
    let settled_probes = probe_evidence(&app);
    assert_prefix_applied(&app, &ships);
    for ship in &ships {
        let uuid = &app.world().get::<EntityUuid>(*ship).unwrap().0;
        let chunks = app.world().resource::<probes::Probes>().chunks(uuid);
        assert!(
            chunks[0].is_empty() && chunks[2].is_empty(),
            "standing Power/Navigation no-op"
        );
    }
    app.world_mut().resource_mut::<probes::Probes>().active = false;
    // Keep real dispatches progressing for authored travel + one second of work.
    let wait_ticks = (ships
        .iter()
        .map(|e| {
            app.world()
                .get::<ShipRepairTeams>(*e)
                .unwrap()
                .0
                .timings()
                .travel_duration
        })
        .fold(0.0_f32, f32::max)
        / period.as_secs_f32())
    .ceil() as usize
        + 60;
    for _ in 0..wait_ticks {
        step(&mut app, &ships, &mut observed);
    }
    fixture::assert_navigation_and_repair(&app, &ships, &expected, true);
    for ship in &ships {
        assert!(
            observed
                .repaired
                .contains(&app.world().get::<EntityUuid>(*ship).unwrap().0),
            "actual positive RepairApplied event for each owner"
        );
    }
    wait_ready(&mut app, &ships, &mut observed);
    sources(&mut app, &ships, false);
    // Reachable human takeover: tempting power/bearing state remains unchanged by AI.
    prepare_power_shields(&mut app, &ships);
    let gated_prefix = prefix(&mut app, &ships, "human-gate");
    app.world_mut()
        .resource_mut::<probes::Probes>()
        .begin(gated_prefix);
    step(&mut app, &ships, &mut observed);
    let gated_probes = probe_evidence(&app);
    assert_prefix_applied(&app, &ships);
    for ship in &ships {
        let uuid = &app.world().get::<EntityUuid>(*ship).unwrap().0;
        assert!(
            app.world()
                .resource::<probes::Probes>()
                .chunks(uuid)
                .iter()
                .all(Vec::is_empty),
            "all four actual Human gates stand down"
        );
        assert_eq!(
            app.world()
                .get::<ShipPowerSystem>(*ship)
                .unwrap()
                .0
                .commanded_level_for(&PowerGroupId("weapons".into())),
            2
        );
        assert_eq!(
            app.world()
                .get::<ShipShields>(*ship)
                .unwrap()
                .0
                .focused_facing,
            None
        );
    }
    assert_prefix_applied(&app, &ships);
    println!(
        "\n{PREFIX}{}",
        json!({"schema":1,"seed":SEED,"order":order,"mode":mode,"pid":std::process::id(),"workers":workers,
        "executor":format!("{executor:?}"),"ships":expected,"positive_producers":8,"human_controlled_noemissions":8,"interleavings":interleavings,
        "graph":graph,"trace":observed.trace,"raw_command_order":observed.raw,
        "raw_probes":{"positive":positive_probes,"settled":settled_probes,"human_gate":gated_probes}})
    );
}
