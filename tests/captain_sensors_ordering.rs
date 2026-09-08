//! #1400: one real Captain/Sensors producer pair, no production annotation.
#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]
use bevy::{ecs::message::MessageCursor, prelude::*};
use project_phoenix::{
    ai::{cadence::AiSnapshotReady, server::AiHighFidelity},
    command_admission::{log::stamp_accepted_command, CommandLog, PendingCommands, ShipKey},
    core::{
        balance::BalanceEvent,
        messages::{
            ActionCorrelationId, ActionFeedbackOutcome, AdmittedCommand, AdmittedCommands,
            DeliveryClass, PowerGroupId, ServerMessage, SystemBlackboard,
            SystemControlPayload as P,
        },
    },
    entities::{
        config::DoctrineObjective,
        spawner::{BehaviourSection, EntityId, EntitySystemHull, EntityUuid},
    },
    headless::{build_headless_app_with, HeadlessArgs, SimRegistrationOverrides},
    infrastructure::InfrastructureCondition,
    lobby::{OutboundMessage, Target, WorldResource},
    science::{scan::ScanRefusal, server::ShipScanRecord},
    server_app::{LocalShip, RegistrationOrder, Ship, ShipSystemBlackboards},
    ship::{
        combat_activity::RecentCombatActivity,
        control_source::ControlSource,
        power::ShipPowerSystem,
        sensors::SensorRadarSelection,
        state::{ShipPhaserFrequency, ShipPhysics, ShipRedAlert},
        system_registry,
    },
    ship_plugin::{CoordinationEnqueue, ShipConfigComponent, ShipSystemControlSources},
    sim_digest::world_digest,
    sim_rng::SimRng,
    sim_tick::{sim_tick_period, SimTick},
    world::{config::WorldConfig, server::WorldContentRuntime},
    world_id::WorldIdMint,
};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
#[path = "captain_sensors_ordering/graph.rs"]
mod graph;
#[path = "captain_sensors_ordering/probes.rs"]
mod probes;
const PREFIX: &str = "PHOENIX_CAPTAIN_SENSORS=";
const SEED: u64 = 1400021;
const TOKEN: &str = "captain-sensors-prefix";
#[derive(Default)]
struct Observe {
    wire: MessageCursor<OutboundMessage>,
    balance: MessageCursor<BalanceEvent>,
    coordination: MessageCursor<CoordinationEnqueue>,
    trace: Vec<Value>,
    raw: Vec<Value>,
}
fn step(app: &mut App, ships: &[Entity], o: &mut Observe) {
    let before = app.world().resource::<SimTick>().0;
    app.update();
    let w = app.world();
    assert_eq!(w.resource::<SimTick>().0, before + 1);
    let wire:Vec<_>=o.wire.read(w.resource::<Messages<OutboundMessage>>()).map(|m|json!({"target":format!("{:?}",m.target),"delivery":format!("{:?}",m.delivery),"message":m.msg})).collect();
    let events:Vec<_>=o.balance.read(w.resource::<Messages<BalanceEvent>>()).map(|e|json!({"exact":format!("{e:?}"),"fact":serde_json::from_str::<Value>(&e.to_json()).unwrap()})).collect();
    let coordination: Vec<_> = o.coordination.read(w.resource::<Messages<CoordinationEnqueue>>()).map(|e|json!({"source":format!("{:?}",e.source_entity),"origin":format!("{:?}",e.sender_origin),"address":e.address,"payload":e.payload,"presentation":e.presentation,"label":e.sender_label,"system":e.sender_system})).collect();
    let flags: BTreeMap<_, _> = w.resource::<WorldContentRuntime>().flags.iter().collect();
    let mut rows = Vec::new();
    let mut raw = Vec::new();
    for &e in ships {
        let uuid = &w.get::<EntityUuid>(e).unwrap().0;
        let cs = &w.get::<AdmittedCommands>(e).unwrap().0;
        let bbs: BTreeMap<_, _> = w
            .get::<ShipSystemBlackboards>(e)
            .unwrap()
            .0
            .iter()
            .map(|(id, b)| (&id.0, b))
            .collect();
        let scan = w.get::<ShipScanRecord>(e).unwrap();
        let stances: BTreeMap<_, _> = w
            .get::<project_phoenix::console::command::server::ShipStationStances>(e)
            .unwrap()
            .0
            .iter()
            .map(|(s, v)| (&s.0, v))
            .collect();
        let foreign: Vec<_> = cs
            .iter()
            .filter(|c| !probes::owns(0, c) && !probes::owns(1, c))
            .cloned()
            .collect();
        rows.push(json!({"uuid":uuid,"consumers":(0..3).map(|i|probes::evidence(&probes::projected(i,cs))).collect::<Vec<_>>(),"foreign":probes::evidence(&foreign),"red_alert":w.get::<ShipRedAlert>(e).unwrap().0,"stances":stances,"last_attacker":w.get::<project_phoenix::console::weapons::LastShipAttacker>(e).unwrap().0,"combat_activity":format!("{:?}",w.get::<RecentCombatActivity>(e).unwrap()),"frequency":w.get::<ShipPhaserFrequency>(e).unwrap().0,"selection":w.get::<SensorRadarSelection>(e).unwrap().0,"scan":scan.save_state(),"scan_config":scan.config,"power":format!("{:?}",w.get::<ShipPowerSystem>(e).unwrap().0.read_state()),"physics":format!("{:?}",w.get::<ShipPhysics>(e).unwrap()),"hull":w.get::<EntitySystemHull>(e).unwrap().0.iter().map(|(id,h)|json!({"id":id,"current":h.current,"max":h.max})).collect::<Vec<_>>(),"boards":bbs}));
        raw.push(json!({"uuid":uuid,"commands":probes::evidence(cs)}));
    }
    o.raw.push(json!({"tick":before,"ships":raw}));
    o.trace.push(json!({"tick":w.resource::<SimTick>().0,"digest":format!("{:016x}",world_digest(w)),"rng":w.resource::<SimRng>().state(),"mint":w.resource::<WorldIdMint>().state(),"log":w.resource::<CommandLog>().entries(),"ships":rows,"world":w.resource::<WorldResource>().0,"flags":flags,"coordination":coordination,"events":events,"wire":wire}));
}
fn ready(app: &mut App, ships: &[Entity], o: &mut Observe) {
    for _ in 0..90 {
        if app.world().resource::<AiSnapshotReady>().0 {
            return;
        }
        step(app, ships, o);
    }
    panic!("real authored snapshot cadence never ready");
}
fn control(app: &mut App, ships: &[Entity], enabled: bool) {
    for &e in ships {
        let mut c = app
            .world_mut()
            .get_mut::<ShipSystemControlSources>(e)
            .unwrap();
        for id in [
            system_registry::red_alert_system_id(),
            system_registry::sensors_system_id(),
        ] {
            c.0.set(
                id.clone(),
                if enabled {
                    ControlSource::Ai
                } else {
                    ControlSource::Human
                },
            );
            assert_eq!(c.0.policy_for(&id).operate_ai, enabled);
        }
    }
}
fn doctrine(app: &mut App, ships: &[Entity], targets: &[Entity], scan: bool, select: bool) {
    for (&ship, &target) in ships.iter().zip(targets) {
        let uuid = app.world().get::<EntityUuid>(target).unwrap().0.clone();
        let mut rows = Vec::new();
        for kind in ["Scan", "Destroy"] {
            if (kind == "Scan" && !scan) || (kind == "Destroy" && !select) {
                continue;
            }
            let mut d = DoctrineObjective::default();
            d.id = format!("pair-{kind}-{uuid}");
            d.text = "Prepared producer proof".into();
            d.base_priority = 100.0;
            d.directive_kind = Some(kind.into());
            if kind == "Scan" {
                d.directive_scan_target = Some(uuid.clone());
            } else {
                d.directive_target = Some(uuid.clone());
            }
            rows.push(d);
        }
        // The authored destroyer already carries BehaviourSection; replace only
        // its fixture standing objectives, preserving selectors and policies.
        app.world_mut()
            .get_mut::<BehaviourSection>(ship)
            .unwrap()
            .0
            .doctrine = rows;
    }
}
fn captain_trigger(app: &mut App, ships: &[Entity], recent: bool, alert: bool) {
    let now = app.world().resource::<Time>().elapsed_secs();
    for &e in ships {
        app.world_mut().get_mut::<ShipRedAlert>(e).unwrap().0 = alert;
        let mut a = app.world_mut().get_mut::<RecentCombatActivity>(e).unwrap();
        a.last_damage_taken = recent.then_some(now);
        a.last_hostile_fire_taken = None;
        a.last_weapon_fired = None;
    }
}
fn stage(
    app: &mut App,
    ships: &[Entity],
    targets: &[Entity],
    o: &mut Observe,
    scan: bool,
    select: bool,
) {
    control(app, ships, false);
    doctrine(app, ships, targets, scan, select);
    // FixedLast arms AiSnapshotReady for the NEXT tick. Reaching that boundary
    // has not yet run aggregate_doctrine_blackboards for the new doctrine.
    // Consume one real publication tick with both producers still Human-gated.
    ready(app, ships, o);
    step(app, ships, o);
    for (&e, &target) in ships.iter().zip(targets) {
        let b = &app.world().get::<ShipSystemBlackboards>(e).unwrap().0;
        let Some(SystemBlackboard::Viewscreen(v)) = b.get(&system_registry::viewscreen_system_id())
        else {
            panic!("real aggregate")
        };
        assert_eq!(
            v.scored_objectives.iter().any(|s| s.score > 0.0
                && matches!(
                    s.directive,
                    project_phoenix::core::messages::AiDirective::Scan { .. }
                )),
            scan
        );
        let target_uuid = &app.world().get::<EntityUuid>(target).unwrap().0;
        for (present, kind) in [(scan, "Scan"), (select, "Destroy")] {
            let expected_id = format!("pair-{kind}-{target_uuid}");
            assert_eq!(
                v.scored_objectives.iter().any(|s| s.score > 0.0
                    && s.id == expected_id
                    && match &s.directive {
                        project_phoenix::core::messages::AiDirective::Scan { target }
                            if kind == "Scan" =>
                            target == target_uuid,
                        project_phoenix::core::messages::AiDirective::Destroy { target }
                            if kind == "Destroy" =>
                            target == target_uuid,
                        _ => false,
                    }),
                present,
                "the ordinary aggregate must publish the staged {kind} target"
            );
        }
        assert!(
            app.world()
                .get::<AdmittedCommands>(e)
                .unwrap()
                .0
                .iter()
                .all(|c| !probes::owns(0, c) && !probes::owns(1, c)),
            "staging publishes doctrine while Captain and Sensors remain Human-gated"
        );
    }
    // The next observed decision reads that published pool. This wait consumes
    // ordinary fixed ticks and keeps every one in the full gameplay trace.
    ready(app, ships, o);
}
fn prefix(app: &mut App, ships: &[Entity], phase: &str) -> BTreeMap<String, Vec<AdmittedCommand>> {
    let tick = app.world().resource::<SimTick>().0;
    let mut all = BTreeMap::new();
    for &e in ships {
        let uuid = app.world().get::<EntityUuid>(e).unwrap().0.clone();
        let commands: Vec<_> = [0, 1]
            .into_iter()
            .enumerate()
            .map(|(i, level)| AdmittedCommand {
                target: system_registry::power_reactor_system_id(),
                payload: P::SetPowerGroupAllocation {
                    group: PowerGroupId("weapons".into()),
                    level,
                },
                response_token: Some(TOKEN.into()),
                feedback_correlation: Some(
                    ActionCorrelationId::new(format!("{phase}-{uuid}-{i}")).unwrap(),
                ),
            })
            .collect();
        for c in &commands {
            stamp_accepted_command(
                &mut app.world_mut().resource_mut::<PendingCommands>(),
                tick,
                None,
                e,
                ShipKey(uuid.clone()),
                c.clone(),
            );
        }
        all.insert(uuid, commands);
    }
    all
}
fn decision(app: &mut App, ships: &[Entity], o: &mut Observe, phase: &str, enabled: bool) -> Value {
    ready(app, ships, o);
    control(app, ships, enabled);
    let prefix = prefix(app, ships, phase);
    app.world_mut()
        .resource_mut::<probes::Probes>()
        .begin(prefix);
    step(app, ships, o);
    let p = app.world().resource::<probes::Probes>();
    let evidence = p.report();
    let mut cursor = MessageCursor::<OutboundMessage>::default();
    let messages: Vec<_> = cursor
        .read(app.world().resource::<Messages<OutboundMessage>>())
        .collect();
    for &e in ships {
        let u = &app.world().get::<EntityUuid>(e).unwrap().0;
        assert_eq!(
            app.world()
                .get::<ShipPowerSystem>(e)
                .unwrap()
                .0
                .commanded_level_for(&PowerGroupId("weapons".into())),
            1
        );
        for c in &p.prefix[u] {
            let matching:Vec<_>=messages.iter().filter(|m|matches!(&m.msg,ServerMessage::ActionFeedback{correlation,..}if Some(correlation)==c.feedback_correlation.as_ref())).collect();
            assert_eq!(matching.len(), 1);
            assert_eq!(matching[0].target, Target::Token(TOKEN.into()));
            assert_eq!(matching[0].delivery, DeliveryClass::Reliable);
            assert!(matches!(
                matching[0].msg,
                ServerMessage::ActionFeedback {
                    outcome: ActionFeedbackOutcome::Applied,
                    ..
                }
            ));
        }
    }
    app.world_mut().resource_mut::<probes::Probes>().active = false;
    evidence
}
fn chunks(app: &App, e: Entity) -> [Vec<AdmittedCommand>; 2] {
    app.world()
        .resource::<probes::Probes>()
        .chunks(&app.world().get::<EntityUuid>(e).unwrap().0)
}
fn positive(app: &App, ships: &[Entity], targets: &[Entity]) -> usize {
    let mut checks = 0;
    for (&e, &t) in ships.iter().zip(targets) {
        let w = app.world();
        let u = &w.get::<EntityUuid>(e).unwrap().0;
        let target = &w.get::<EntityUuid>(t).unwrap().0;
        let c = chunks(app, e);
        assert_eq!(c[0].len(), 1);
        assert!(matches!(c[0][0].payload, P::SetRedAlert { active: true }));
        assert_eq!(c[1].len(), 2);
        assert!(matches!(&c[1][0].payload,P::ScanTarget{uuid}if uuid==target));
        assert!(matches!(&c[1][1].payload,P::SetScienceTarget{uuid}if uuid==target));
        let p = w.resource::<probes::Probes>();
        checks += probes::interleavings(&p.prefix[u], &c);
        assert!(w.get::<ShipRedAlert>(e).unwrap().0);
        assert_eq!(
            w.get::<SensorRadarSelection>(e).unwrap().0.as_ref(),
            Some(target)
        );
        let record = w.get::<ShipScanRecord>(e).unwrap();
        assert!(record.refusal.is_none());
        let reading = record.last.as_ref().unwrap();
        assert_eq!(&reading.subject_uuid, target);
        assert_eq!(reading.taken_at_tick, w.resource::<SimTick>().0 - 1);
        assert_eq!(reading.band, "detailed");
        assert!(reading.condition_fraction > 0.0);
        assert_eq!(reading.mass, 180000.0);
        assert!(w.get::<InfrastructureCondition>(t).is_some());
        let authored_id = &w.get::<EntityId>(t).unwrap().0;
        assert!(
            w.resource::<WorldContentRuntime>()
                .flags
                .flag(&project_phoenix::science::scanned_flag(authored_id)),
            "real scan mirrored its authored subject flag"
        );
        let mut cursor = MessageCursor::<CoordinationEnqueue>::default();
        let designations=cursor.read(w.resource::<Messages<CoordinationEnqueue>>()).filter(|event| event.source_entity==e && matches!(&event.payload,project_phoenix::core::messages::CoordinationPayload::TargetDesignation{uuid,..} if uuid==target)).count();
        assert_eq!(
            designations, 1,
            "actual per-ship Sensors consumer emitted its designation"
        );
    }
    checks
}
#[test]
fn captain_and_sensors_preserve_typed_effects_across_registrations() {
    let mut reference = None;
    let mut ordinary = None;
    let mut done = Vec::new();
    let mut registrations = BTreeMap::new();
    for order in ["canonical", "shuffle-17", "shuffle-991"] {
        for mode in ["default-1", "default-2", "pinned"] {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "captain_sensors_child",
                    "--ignored",
                    "--nocapture",
                    "--test-threads=1",
                ])
                .env("PHOENIX_PAIR_ORDER", order)
                .env("PHOENIX_PAIR_MODE", mode)
                .current_dir(env!("CARGO_MANIFEST_DIR"))
                .output()
                .unwrap();
            let stdout = String::from_utf8(output.stdout).unwrap();
            let stderr = String::from_utf8_lossy(&output.stderr);
            print!("{stdout}");
            eprint!("{stderr}");
            assert!(output.status.success(), "{order}/{mode}: {stderr}");
            assert!(stdout.contains("test result: ok. 1 passed; 0 failed; 0 ignored"));
            let rows: Vec<Value> = stdout
                .lines()
                .filter_map(|l| l.strip_prefix(PREFIX))
                .map(|s| serde_json::from_str(s).unwrap())
                .collect();
            assert_eq!(rows.len(), 1);
            let r = &rows[0];
            if let Some(previous) = registrations.insert(order, r["graph"]["registration"].clone())
            {
                assert_eq!(
                    previous, r["graph"]["registration"],
                    "same registration across fresh pools"
                );
            }
            assert_eq!(r["schema"], 1);
            assert_eq!(r["seed"], SEED);
            assert_eq!(r["mode"], mode);
            assert_eq!(r["order"], order);
            assert_eq!(r["ships"].as_array().unwrap().len(), 2);
            assert_eq!(r["phases"].as_array().unwrap().len(), 7);
            assert!(r["interleavings"].as_u64().unwrap() >= 16);
            assert!(r["trace"].as_array().unwrap().len() > 25);
            if let Some(a) = &ordinary {
                graph::assert_preserved(a, &r["graph"]);
            } else {
                ordinary = Some(r["graph"].clone());
            }
            let gameplay = json!({"ships":r["ships"],"trace":r["trace"]});
            if let Some(a) = &reference {
                assert_eq!(
                    a, &gameplay,
                    "complete per-tick gameplay, typed subsequences and wire"
                );
            } else {
                reference = Some(gameplay);
            }
            done.push(
                json!({"mode":mode,"order":order,"pid":r["pid"],"workers":r["workers"],"phases":7}),
            );
        }
    }
    assert_eq!(done.len(), 9);
    assert_eq!(
        registrations
            .values()
            .map(Value::to_string)
            .collect::<BTreeSet<_>>()
            .len(),
        3,
        "canonical and both shuffles must have genuinely different actual registration sequences"
    );
    assert_eq!(
        done.iter()
            .map(|r| r["pid"].as_u64().unwrap())
            .collect::<BTreeSet<_>>()
            .len(),
        9
    );
    println!(
        "\nPHOENIX_CAPTAIN_SENSORS_PARENT={}",
        json!({"schema":1,"pairs":1,"children":done})
    );
}
#[test]
#[ignore = "fresh-process driver invoked by parent"]
fn captain_sensors_child() {
    let mode = std::env::var("PHOENIX_PAIR_MODE").unwrap();
    let order = std::env::var("PHOENIX_PAIR_ORDER").unwrap();
    assert!(matches!(
        mode.as_str(),
        "default-1" | "default-2" | "pinned"
    ));
    let registration_order = match order.as_str() {
        "canonical" => RegistrationOrder::Canonical,
        "shuffle-17" => RegistrationOrder::Shuffled(17),
        "shuffle-991" => RegistrationOrder::Shuffled(991),
        _ => panic!("unknown registration role"),
    };
    let mut app = build_headless_app_with(
        &HeadlessArgs {
            world_path: "tests/fixtures/determinism/captain-sensors.toml".into(),
            ship_path: "assets/entities/alliance_destroyer.toml".into(),
            seed: Some(SEED),
            deterministic: mode == "pinned",
            ..Default::default()
        },
        SimRegistrationOverrides {
            registration_order,
            ..Default::default()
        },
    )
    .unwrap();
    app.finish();
    app.cleanup();
    probes::install(&mut app);
    let graph = graph::capture(&mut app, &order);
    let period = sim_tick_period(app.world().resource::<WorldConfig>().global.sim_tick_hz);
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(period));
    for _ in 0..6 {
        app.update();
    }
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
    let mut ships: Vec<_> = app
        .world_mut()
        .query_filtered::<Entity, (With<Ship>, Without<LocalShip>)>()
        .iter(app.world())
        .collect();
    assert_eq!(ships.len(), 2);
    ships.sort_by_key(|e| app.world().get::<EntityUuid>(*e).unwrap().0.clone());
    let mut all: Vec<_> = app
        .world_mut()
        .query_filtered::<Entity, With<Ship>>()
        .iter(app.world())
        .collect();
    all.sort_by_key(|e| app.world().get::<EntityUuid>(*e).unwrap().0.clone());
    for e in all {
        let ids: Vec<_> = app
            .world()
            .get::<ShipConfigComponent>(e)
            .unwrap()
            .0
            .systems
            .iter()
            .map(|s| s.id.clone())
            .collect();
        let mut c = app
            .world_mut()
            .get_mut::<ShipSystemControlSources>(e)
            .unwrap();
        for id in ids {
            c.0.set(id, ControlSource::Human);
        }
    }
    let mut targets = Vec::new();
    for (i, &e) in ships.iter().enumerate() {
        assert!(app
            .world()
            .get::<project_phoenix::lockstep::FleetSlotOf>(e)
            .is_none());
        assert!(app.world().get::<AiHighFidelity>(e).is_some());
        assert!(app.world().get::<ShipScanRecord>(e).is_some());
        assert!(app
            .world()
            .get::<project_phoenix::ship::sensors::SensorsTargetSelector>(e)
            .is_some());
        assert!(app
            .world()
            .get::<project_phoenix::console::captain::server::CaptainAiPolicy>(e)
            .is_some());
        let id = format!("subject-{}", if i == 0 { "a" } else { "b" });
        let matches: Vec<_> = app
            .world_mut()
            .query::<(Entity, &EntityId)>()
            .iter(app.world())
            .filter(|(_, n)| n.0 == id)
            .map(|(e, _)| e)
            .collect();
        assert_eq!(matches.len(), 1);
        let target = matches[0];
        let physics = app.world().get::<ShipPhysics>(e).unwrap();
        let near = Vec3::new(physics.x, 0.0, physics.z + 70.0);
        app.world_mut()
            .get_mut::<Transform>(target)
            .unwrap()
            .translation = near;
        targets.push(target);
    }
    let ship_ids: Vec<_> = ships
        .iter()
        .map(|e| app.world().get::<EntityUuid>(*e).unwrap().0.clone())
        .collect();
    let configs: Vec<_> = ships
        .iter()
        .map(|e| {
            app.world()
                .get::<ShipScanRecord>(*e)
                .unwrap()
                .config
                .clone()
        })
        .collect();
    let mut o = Observe::default();
    let mut phases = Vec::new();
    let mut interleavings = 0;
    stage(&mut app, &ships, &targets, &mut o, true, true);
    captain_trigger(&mut app, &ships, true, false);
    phases.push(
        json!({"phase":"positive","probes":decision(&mut app,&ships,&mut o,"positive",true)}),
    );
    interleavings += positive(&app, &ships, &targets);
    stage(&mut app, &ships, &targets, &mut o, false, true);
    phases
        .push(json!({"phase":"settled","probes":decision(&mut app,&ships,&mut o,"settled",true)}));
    for &e in &ships {
        assert!(chunks(&app, e).iter().all(Vec::is_empty));
    }
    stage(&mut app, &ships, &targets, &mut o, false, false);
    captain_trigger(&mut app, &ships, false, true);
    phases.push(json!({"phase":"clear","probes":decision(&mut app,&ships,&mut o,"clear",true)}));
    for &e in &ships {
        let c = chunks(&app, e);
        assert_eq!(c[0].len(), 1);
        assert_eq!(c[0][0].payload, P::SetRedAlert { active: false });
        assert_eq!(c[1].len(), 1);
        assert_eq!(c[1][0].payload, P::ClearScienceTarget);
        assert!(!app.world().get::<ShipRedAlert>(e).unwrap().0);
        assert!(app
            .world()
            .get::<SensorRadarSelection>(e)
            .unwrap()
            .0
            .is_none());
        let p = app.world().resource::<probes::Probes>();
        interleavings +=
            probes::interleavings(&p.prefix[&app.world().get::<EntityUuid>(e).unwrap().0], &c);
    }
    for &t in &targets {
        app.world_mut()
            .get_mut::<Transform>(t)
            .unwrap()
            .translation
            .z += 2000.0;
    }
    stage(&mut app, &ships, &targets, &mut o, true, false);
    captain_trigger(&mut app, &ships, true, false);
    phases
        .push(json!({"phase":"refused","probes":decision(&mut app,&ships,&mut o,"refused",true)}));
    for &e in &ships {
        let c = chunks(&app, e);
        assert_eq!(c[0].len(), 1);
        assert_eq!(c[1].len(), 1);
        assert!(matches!(c[1][0].payload, P::ScanTarget { .. }));
        let r = app.world().get::<ShipScanRecord>(e).unwrap();
        assert_eq!(r.refusal, Some(ScanRefusal::OutOfRange));
        assert!(r.last.is_none());
        let p = app.world().resource::<probes::Probes>();
        interleavings +=
            probes::interleavings(&p.prefix[&app.world().get::<EntityUuid>(e).unwrap().0], &c);
    }
    phases.push(json!({"phase":"retry","probes":decision(&mut app,&ships,&mut o,"retry",true)}));
    for &e in &ships {
        let c = chunks(&app, e);
        assert!(c[0].is_empty());
        assert_eq!(c[1].len(), 1);
        assert!(matches!(c[1][0].payload, P::ScanTarget { .. }));
        assert_eq!(
            app.world().get::<ShipScanRecord>(e).unwrap().refusal,
            Some(ScanRefusal::OutOfRange)
        );
    }
    for (&e, &t) in ships.iter().zip(&targets) {
        let p = app.world().get::<ShipPhysics>(e).unwrap();
        let pos = Vec3::new(p.x, 0.0, p.z + 70.0);
        app.world_mut().get_mut::<Transform>(t).unwrap().translation = pos;
    }
    stage(&mut app, &ships, &targets, &mut o, true, true);
    captain_trigger(&mut app, &ships, true, false);
    phases.push(json!({"phase":"human","probes":decision(&mut app,&ships,&mut o,"human",false)}));
    for &e in &ships {
        assert!(chunks(&app, e).iter().all(Vec::is_empty));
        assert!(!app.world().get::<ShipRedAlert>(e).unwrap().0);
        assert!(app
            .world()
            .get::<SensorRadarSelection>(e)
            .unwrap()
            .0
            .is_none());
        assert_eq!(
            app.world().get::<ShipScanRecord>(e).unwrap().refusal,
            Some(ScanRefusal::OutOfRange)
        );
    }
    phases.push(json!({"phase":"resume","probes":decision(&mut app,&ships,&mut o,"resume",true)}));
    interleavings += positive(&app, &ships, &targets);
    for (&e, config) in ships.iter().zip(&configs) {
        assert_eq!(
            &app.world().get::<ShipScanRecord>(e).unwrap().config,
            config
        );
    }
    println!(
        "\n{PREFIX}{}",
        json!({"schema":1,"seed":SEED,"mode":mode,"order":order,"pid":std::process::id(),"workers":workers,"executor":format!("{executor:?}"),"ships":ship_ids,"graph":graph,"phases":phases,"interleavings":interleavings,"trace":o.trace,"raw":o.raw})
    );
}
