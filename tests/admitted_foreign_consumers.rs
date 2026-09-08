//! Five of the 275 audited foreign-payload pairs, using actual production instances.
//! No production annotation, replacement consumer, admission bypass claim or allowance edit.
#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]

mod common;
#[path = "common/declared_order.rs"]
mod declared_order;
use bevy::{ecs::message::MessageCursor, prelude::*};
use project_phoenix::{
    ai::cadence::AiSnapshotReady,
    command_admission::{log::stamp_accepted_command, CommandLog, PendingCommands, ShipKey},
    console::weapons::{CurrentPhaserMode, TacticalRadarSelection, TorpedoSystemResource},
    core::{
        balance::BalanceEvent,
        messages::{
            ActionCorrelationId, ActionFeedbackOutcome, AdmittedCommand, AdmittedCommands,
            CameraView, DeliveryClass, PhaserMode, ServerMessage, SystemControlPayload, SystemId,
            ViewMode,
        },
    },
    entities::spawner::EntityUuid,
    headless::HeadlessArgs,
    lobby::{OutboundMessage, Target},
    server_app::{LocalShip, Ship, ShipSystemBlackboards},
    ship::{
        control_source::ControlSource,
        state::{ShipPhaserFrequency, ShipPhysics, ShipRedAlert, ShipViewMode},
        system_registry,
    },
    ship_plugin::{LastHelmInput, ShipConfigComponent, ShipSystemControlSources},
    sim_digest::world_digest,
    sim_rng::SimRng,
    sim_tick::{sim_tick_period, SimTick},
    world::config::WorldConfig,
    world_id::WorldIdMint,
};
use serde_json::{json, Value};
use std::collections::BTreeMap;

#[path = "admitted_foreign_consumers/graph_proof.rs"]
mod graph_proof;

const PREFIX: &str = "PHOENIX_FOREIGN_CONSUMER=";
const SEED: u64 = 1400275;
const TOKEN: &str = "foreign-consumer-fixture";
const WITNESSES: [&str; 2] = ["foreign-target-a", "foreign-target-b"];
const FAMILIES: [&str; 5] = [
    "red-alert",
    "target",
    "volley",
    "phaser-mode",
    "phaser-frequency",
];

#[derive(Default)]
struct Readers {
    wire: MessageCursor<OutboundMessage>,
    balance: MessageCursor<BalanceEvent>,
}

fn consumer_state(app: &App, ship: Entity) -> Value {
    let world = app.world();
    json!({
        "red-alert":world.get::<ShipRedAlert>(ship).unwrap().0,
        "target":world.get::<TacticalRadarSelection>(ship).unwrap().0,
        "volley":world.get::<TorpedoSystemResource>(ship).unwrap().0.tubes.iter()
            .map(|tube|json!({"id":tube.id,"target":tube.target_count})).collect::<Vec<_>>(),
        "phaser-mode":world.resource::<CurrentPhaserMode>().0,
        "phaser-frequency":world.get::<ShipPhaserFrequency>(ship).unwrap().0,
    })
}

fn observe(app: &App, ship: Entity, readers: &mut Readers) -> (Value, Vec<OutboundMessage>) {
    let world = app.world();
    let wire: Vec<_> = readers
        .wire
        .read(world.resource::<Messages<OutboundMessage>>())
        .cloned()
        .collect();
    let events: Vec<_> = readers.balance.read(world.resource::<Messages<BalanceEvent>>())
        .map(|event| json!({"exact":format!("{event:?}"),"fact":serde_json::from_str::<Value>(&event.to_json()).unwrap()})).collect();
    let boards: BTreeMap<_, _> = world
        .get::<ShipSystemBlackboards>(ship)
        .unwrap()
        .0
        .iter()
        .map(|(id, board)| (&id.0, board))
        .collect();
    let mut subsequences = BTreeMap::<String, Vec<Value>>::new();
    for command in &world.get::<AdmittedCommands>(ship).unwrap().0 {
        subsequences
            .entry(command.target.0.clone())
            .or_default()
            .push(json!({
            "payload":command.payload,"response_token":command.response_token,
            "correlation":command.feedback_correlation}));
    }
    let config = &world.get::<ShipConfigComponent>(ship).unwrap().0;
    let sources = &world.get::<ShipSystemControlSources>(ship).unwrap().0;
    let row = json!({"tick":world.resource::<SimTick>().0,
        "digest":format!("{:016x}",world_digest(world)),
        "rng":world.resource::<SimRng>().state(),"mint":world.resource::<WorldIdMint>().state(),
        "consumers":consumer_state(app,ship),"view":format!("{:?}",world.get::<ShipViewMode>(ship).unwrap()),
        "boards":boards,"commands_by_target":subsequences,"log":world.resource::<CommandLog>().entries(),
        "sources":config.systems.iter().map(|system|json!({"id":system.id,
            "source":format!("{:?}",sources.source_for(&system.id)),
            "policy":format!("{:?}",sources.policy_for(&system.id))})).collect::<Vec<_>>(),
        "events":events,"wire":wire.iter().map(|message|json!({
            "target":format!("{:?}",message.target),"delivery":format!("{:?}",message.delivery),
            "message":message.msg})).collect::<Vec<_>>()});
    (row, wire)
}

fn step(
    app: &mut App,
    ship: Entity,
    readers: &mut Readers,
    trace: &mut Vec<Value>,
    raw: &mut Vec<Value>,
) -> Vec<OutboundMessage> {
    let tick = app.world().resource::<SimTick>().0;
    app.update();
    assert_eq!(
        app.world().resource::<SimTick>().0,
        tick + 1,
        "one actual fixed step"
    );
    let commands = &app.world().get::<AdmittedCommands>(ship).unwrap().0;
    // Raw cross-target append order is retained, not promoted to gameplay order.
    raw.push(
        json!({"tick":tick,"commands":commands.iter().map(|command|json!({
        "target":command.target,"payload":command.payload,"response_token":command.response_token,
        "correlation":command.feedback_correlation})).collect::<Vec<_>>()}),
    );
    let (row, wire) = observe(app, ship, readers);
    trace.push(row);
    wire
}

fn wait_for_cadence(
    app: &mut App,
    ship: Entity,
    readers: &mut Readers,
    trace: &mut Vec<Value>,
    raw: &mut Vec<Value>,
) {
    for _ in 0..30 {
        if app.world().resource::<AiSnapshotReady>().0 {
            return;
        }
        step(app, ship, readers, trace, raw);
    }
    panic!("ordinary AI cadence did not become ready");
}

fn enqueue(
    app: &mut App,
    ship: Entity,
    target: SystemId,
    payload: SystemControlPayload,
    label: &str,
) {
    let tick = app.world().resource::<SimTick>().0;
    let uuid = app.world().get::<EntityUuid>(ship).unwrap().0.clone();
    let command = AdmittedCommand {
        target,
        payload,
        response_token: Some(TOKEN.into()),
        feedback_correlation: Some(ActionCorrelationId::new(label).unwrap()),
    };
    // Deliberately tests the accepted-command continuation seam, not network authority.
    // Actual Admission drains/stamps this queue before the unmodified Input systems.
    stamp_accepted_command(
        &mut app.world_mut().resource_mut::<PendingCommands>(),
        tick,
        None,
        ship,
        ShipKey(uuid),
        command,
    );
}

fn payloads(family: &str, tube: &SystemId, inverse: bool) -> (SystemId, Vec<SystemControlPayload>) {
    match family {
        "red-alert" => (
            system_registry::red_alert_system_id(),
            if inverse {
                vec![SystemControlPayload::SetRedAlert { active: false }]
            } else {
                vec![
                    SystemControlPayload::SetRedAlert { active: false },
                    SystemControlPayload::SetRedAlert { active: true },
                ]
            },
        ),
        "target" => (
            system_registry::tactical_radar_system_id(),
            if inverse {
                vec![SystemControlPayload::SetTarget {
                    uuid: "missing-foreign-target".into(),
                }]
            } else {
                WITNESSES
                    .iter()
                    .map(|uuid| SystemControlPayload::SetTarget {
                        uuid: (*uuid).into(),
                    })
                    .collect()
            },
        ),
        "volley" => (
            tube.clone(),
            if inverse {
                vec![SystemControlPayload::SetTorpedoVolleyTarget { count: 1 }]
            } else {
                vec![
                    SystemControlPayload::SetTorpedoVolleyTarget { count: 1 },
                    SystemControlPayload::SetTorpedoVolleyTarget { count: 2 },
                ]
            },
        ),
        "phaser-mode" => (
            SystemId(system_registry::PHASER_CONTROL_SYSTEM_ID.into()),
            if inverse {
                vec![SystemControlPayload::SetPhaserMode {
                    mode: PhaserMode::Auto,
                }]
            } else {
                vec![
                    SystemControlPayload::SetPhaserMode {
                        mode: PhaserMode::Auto,
                    },
                    SystemControlPayload::SetPhaserMode {
                        mode: PhaserMode::Manual,
                    },
                ]
            },
        ),
        "phaser-frequency" => (
            SystemId(system_registry::PHASER_CONTROL_SYSTEM_ID.into()),
            if inverse {
                vec![SystemControlPayload::SetPhaserFrequency { frequency: 0.125 }]
            } else {
                vec![
                    SystemControlPayload::SetPhaserFrequency { frequency: 0.25 },
                    SystemControlPayload::SetPhaserFrequency { frequency: 0.75 },
                ]
            },
        ),
        _ => unreachable!(),
    }
}

fn set_gate(app: &mut App, ship: Entity, family: &str, tube: &SystemId, offline: bool) {
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
    let source = if offline {
        ControlSource::Offline
    } else {
        ControlSource::Human
    };
    match family {
        "volley" => sources.0.set(tube.clone(), source),
        "phaser-mode" | "phaser-frequency" => {
            let banks: Vec<_> = config
                .systems
                .iter()
                .filter(|system| system.kind == system_registry::PHASER_BANK_KIND)
                .collect();
            assert!(!banks.is_empty());
            for bank in banks {
                sources.0.set(bank.id.clone(), source);
            }
        }
        _ => {}
    }
}

fn assert_feedback(
    wire: &[OutboundMessage],
    labels: &[String],
    expected: Option<ActionFeedbackOutcome>,
) {
    for label in labels {
        let matching: Vec<_> = wire
            .iter()
            .filter_map(|message| match &message.msg {
                ServerMessage::ActionFeedback {
                    correlation,
                    outcome,
                } if correlation.as_str() == label => {
                    assert!(matches!(&message.target, Target::Token(token) if token == TOKEN));
                    assert_eq!(message.delivery, DeliveryClass::Reliable);
                    Some(*outcome)
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            matching,
            expected.into_iter().collect::<Vec<_>>(),
            "actual consumer response {label}"
        );
    }
}

#[test]
fn five_foreign_consumers_preserve_effects_under_declared_order_registration_changes() {
    let mut expected = None;
    let mut ordinary_graph = None;
    let mut registration_roles = std::collections::BTreeMap::new();
    for order in ["ordinary", "shuffle-a", "shuffle-b"] {
        for mode in ["default-1", "default-2", "pinned"] {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "foreign_consumer_child",
                    "--ignored",
                    "--nocapture",
                    "--test-threads=1",
                ])
                .env("PHOENIX_FOREIGN_ORDER", order)
                .env("PHOENIX_FOREIGN_MODE", mode)
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
            declared_order::observe_role(&mut registration_roles, &report["graph"]["graph"]);
            assert_eq!(report["mode"], mode);
            assert_eq!(report["seed"], SEED);
            assert!(report["process"].as_u64().unwrap() > 0);
            assert_ne!(report["process"], std::process::id());
            assert_eq!(report["cases"].as_array().unwrap().len(), 15);
            assert!(report["trace"].as_array().unwrap().len() >= 15);
            if mode == "pinned" {
                assert_eq!(report["workers"], 1);
                assert_eq!(report["executor"], "SingleThreaded");
            } else {
                assert!(report["workers"].as_u64().unwrap() > 1);
                assert_eq!(report["executor"], "MultiThreaded");
            }
            let gameplay = json!({"cases":report["cases"],"trace":report["trace"]});
            if let Some(reference) = &expected {
                assert_eq!(
                    &gameplay, reference,
                    "actual effects/per-target subsequences/wire"
                );
            } else {
                expected = Some(gameplay);
            }
            if let Some(graph) = &ordinary_graph {
                graph_proof::assert_preserved(graph, &report["graph"]);
            } else {
                ordinary_graph = Some(report["graph"].clone());
            }
        }
    }
}

#[test]
#[ignore = "fresh-process parent owns default pools and registration perturbation"]
fn foreign_consumer_child() {
    let order = std::env::var("PHOENIX_FOREIGN_ORDER").unwrap();
    let mode = std::env::var("PHOENIX_FOREIGN_MODE").unwrap();
    assert!(matches!(
        mode.as_str(),
        "default-1" | "default-2" | "pinned"
    ));
    let mut app = declared_order::build(
        HeadlessArgs {
            world_path: "assets/worlds/combat_test.toml".into(),
            ship_path: "assets/entities/alliance_destroyer.toml".into(),
            seed: Some(SEED),
            deterministic: mode == "pinned",
            ..Default::default()
        },
        &order,
    );
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
    assert_eq!(app.world().resource::<SimRng>().seed(), SEED);
    let ship = app
        .world_mut()
        .query_filtered::<Entity, With<LocalShip>>()
        .single(app.world())
        .unwrap();
    let all_ships: Vec<_> = app
        .world_mut()
        .query_filtered::<Entity, With<Ship>>()
        .iter(app.world())
        .collect();
    for entity in all_ships {
        let config = app
            .world()
            .get::<ShipConfigComponent>(entity)
            .unwrap()
            .0
            .clone();
        let mut sources = app
            .world_mut()
            .get_mut::<ShipSystemControlSources>(entity)
            .unwrap();
        for system in &config.systems {
            sources.0.set(system.id.clone(), ControlSource::Human);
        }
    }
    let config = app
        .world()
        .get::<ShipConfigComponent>(ship)
        .unwrap()
        .0
        .clone();
    let captain = config
        .systems
        .iter()
        .find(|system| system.kind == system_registry::CAPTAIN_KIND)
        .unwrap()
        .id
        .clone();
    app.world_mut()
        .get_mut::<ShipSystemControlSources>(ship)
        .unwrap()
        .0
        .set(captain, ControlSource::Ai);
    let position = Vec3::new(100_000.0, 0.0, 0.0);
    app.world_mut().entity_mut(ship).insert((
        ShipPhysics {
            x: position.x,
            ..Default::default()
        },
        LastHelmInput::default(),
        Transform::from_translation(position),
    ));
    for (index, id) in WITNESSES.iter().enumerate() {
        app.world_mut().spawn((
            EntityUuid((*id).into()),
            Transform::from_translation(position + Vec3::new(10.0 + index as f32, 0.0, 0.0)),
        ));
    }
    let tube_id = app
        .world()
        .get::<TorpedoSystemResource>(ship)
        .unwrap()
        .0
        .tubes
        .iter()
        .find(|tube| tube.volley_max >= 2)
        .expect("authored multiple-round tube")
        .id
        .clone();
    let tube = system_registry::torpedo_tube_system_id(&tube_id).unwrap();
    assert!(app
        .world_mut()
        .get_mut::<TorpedoSystemResource>(ship)
        .unwrap()
        .0
        .set_volley_target(&tube_id, 0));
    app.world_mut().get_mut::<ShipRedAlert>(ship).unwrap().0 = false;
    app.world_mut().resource_mut::<CurrentPhaserMode>().0 = PhaserMode::Auto;
    app.world_mut()
        .get_mut::<ShipPhaserFrequency>(ship)
        .unwrap()
        .0 = 0.5;
    app.world_mut()
        .get_mut::<TacticalRadarSelection>(ship)
        .unwrap()
        .0 = None;
    let mut readers = Readers::default();
    let mut trace = Vec::new();
    let mut raw = Vec::new();
    let mut cases = Vec::new();
    let _ = observe(&app, ship, &mut readers);
    for family in FAMILIES {
        for stage in ["foreign-only", "matching", "inverse-or-refused"] {
            wait_for_cadence(&mut app, ship, &mut readers, &mut trace, &mut raw);
            let inverse = stage == "inverse-or-refused";
            set_gate(&mut app, ship, family, &tube, inverse);
            app.world_mut()
                .get_mut::<ShipViewMode>(ship)
                .unwrap()
                .request_view_mode(ViewMode::Camera(CameraView::default()));
            let before = consumer_state(&app, ship);
            let before_log = app.world().resource::<CommandLog>().len();
            let before_tick = app.world().resource::<SimTick>().0;
            assert!(
                app.world().resource::<AiSnapshotReady>().0,
                "real producer's prior FixedLast latch"
            );
            let (target, commands) = payloads(family, &tube, inverse);
            let commands = if stage == "foreign-only" {
                Vec::new()
            } else {
                commands
            };
            let labels: Vec<_> = commands
                .iter()
                .enumerate()
                .map(|(index, _)| format!("{family}-{stage}-{index}"))
                .collect();
            for (payload, label) in commands.iter().cloned().zip(&labels) {
                enqueue(&mut app, ship, target.clone(), payload, label);
            }
            let wire = step(&mut app, ship, &mut readers, &mut trace, &mut raw);
            let after = consumer_state(&app, ship);
            let admitted = &app.world().get::<AdmittedCommands>(ship).unwrap().0;
            let foreign: Vec<_> = admitted
                .iter()
                .filter(|command| {
                    matches!(
                        command.payload,
                        SystemControlPayload::SetView {
                            mode: ViewMode::Cinematic
                        }
                    )
                })
                .collect();
            assert_eq!(
                foreign.len(),
                1,
                "real producer emitted in every tested boundary"
            );
            assert!(foreign[0]
                .response_token
                .as_deref()
                .unwrap()
                .starts_with("ai:"));
            assert_eq!(
                app.world().get::<ShipViewMode>(ship).unwrap().view_mode,
                ViewMode::Cinematic
            );
            let own: Vec<_> = admitted
                .iter()
                .filter(|command| command.response_token.as_deref() == Some(TOKEN))
                .collect();
            assert_eq!(own.len(), commands.len());
            assert_eq!(
                own.iter()
                    .map(|command| command.payload.clone())
                    .collect::<Vec<_>>(),
                commands,
                "preserve own target subsequence"
            );
            assert_eq!(
                app.world().resource::<CommandLog>().len(),
                before_log + commands.len(),
                "AI append allocates no logged order"
            );
            let expected_feedback = if stage == "foreign-only" || family == "phaser-frequency" {
                None
            } else if inverse && family != "red-alert" {
                Some(ActionFeedbackOutcome::Refused)
            } else {
                Some(ActionFeedbackOutcome::Applied)
            };
            assert_feedback(&wire, &labels, expected_feedback);
            if stage == "foreign-only" {
                assert_eq!(
                    before, after,
                    "foreign payload must be ignored by every typed consumer"
                );
            } else if inverse && matches!(family, "volley" | "phaser-mode" | "phaser-frequency") {
                assert_eq!(
                    before, after,
                    "real offline refusal leaves prior consumer state intact"
                );
            } else {
                assert_ne!(
                    before[family], after[family],
                    "matching consumer effect must be nonvacuous"
                );
                match family {
                    "red-alert" => assert_eq!(after[family], !inverse),
                    "target" => assert_eq!(
                        after[family],
                        if inverse {
                            Value::Null
                        } else {
                            json!(WITNESSES[1])
                        }
                    ),
                    "volley" => assert_eq!(
                        app.world()
                            .get::<TorpedoSystemResource>(ship)
                            .unwrap()
                            .0
                            .tubes
                            .iter()
                            .find(|item| item.id == tube_id)
                            .unwrap()
                            .target_count,
                        2
                    ),
                    "phaser-mode" => assert_eq!(
                        app.world().resource::<CurrentPhaserMode>().0,
                        PhaserMode::Manual
                    ),
                    "phaser-frequency" => assert_eq!(after[family], 0.75),
                    _ => unreachable!(),
                }
            }
            cases.push(json!({"family":family,"stage":stage,"tick":before_tick,"before":before,"after":after,
                "matching":commands,"feedback":expected_feedback,"co_emitted_foreign":1}));
            set_gate(&mut app, ship, family, &tube, false);
        }
    }
    for _ in 0..6 {
        step(&mut app, ship, &mut readers, &mut trace, &mut raw);
    }
    assert_eq!(cases.len(), 15);
    println!(
        "\n{PREFIX}{}",
        json!({"order":order,"mode":mode,"seed":SEED,"process":std::process::id(),
        "workers":workers,"executor":format!("{executor:?}"),"graph":graph,"cases":cases,"trace":trace,"raw_append_order":raw})
    );
}
