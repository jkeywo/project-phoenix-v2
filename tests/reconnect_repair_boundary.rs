//! Whole-App admitted Repair + Identify boundary (#1400). No owner requests,
//! team dispatch/tick calls, synthetic blackboards or schedule edges are injected.
#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]

use bevy::{ecs::message::MessageCursor, prelude::*};
use project_phoenix::{
    console::repair::{
        visibility::{LastBroadcastHull, LastVisibleRepairBlackboard},
        ShipRepairTeams,
    },
    core::{
        balance::BalanceEvent,
        messages::{
            ActionCorrelationId, ActionFeedbackOutcome, AdmittedCommands, ClientMessage,
            DeliveryClass, RepairTarget, ServerMessage, StationId, SystemBlackboard,
            SystemControlPayload, SystemId, TeamSlot,
        },
    },
    entities::spawner::{EntitySystemHull, EntityUuid},
    headless::{build_headless_app, HeadlessArgs},
    lobby::{
        server::{InboundMessage, OutboundMessage},
        Sessions, Target,
    },
    server_app::{LastBroadcastBlackboards, LocalShip},
    ship_plugin::{ActiveStationRatings, ShipSystemControlSources},
    sim_digest::world_digest,
    sim_rng::SimRng,
    sim_tick::{sim_tick_period, SimTick},
    world::config::WorldConfig,
};
use serde_json::{json, Value};
use std::process::Command;

const OWNER: &str = "repair-boundary-engineer";
const OTHER: &str = "repair-boundary-helm";
const SEED: u64 = 1_400_276;
const ROLE_ENV: &str = "PHOENIX_REPAIR_RECONNECT_ROLE";
const PREFIX: &str = "PHOENIX_REPAIR_RECONNECT=";
const CHILD: &str = "repair_identify_child";
const CORRELATION: &str = "repair-boundary-dispatch";
const TEST_COMMAND_DELAY_TICKS: u64 = 2;

fn send(app: &mut App, token: &str, msg: ClientMessage) {
    app.world_mut().write_message(InboundMessage {
        token: token.into(),
        msg,
    });
}

fn identify(app: &mut App, token: &str) {
    send(
        app,
        token,
        ClientMessage::Identify {
            token: token.into(),
            name: token.into(),
        },
    );
}

fn step(app: &mut App) {
    let before = app.world().resource::<SimTick>().0;
    app.update();
    assert_eq!(app.world().resource::<SimTick>().0, before + 1);
}

fn local(app: &mut App) -> Entity {
    app.world_mut()
        .query_filtered::<Entity, With<LocalShip>>()
        .single(app.world())
        .unwrap()
}

fn assert_human(app: &App, ship: Entity) {
    let sessions = &app.world().resource::<Sessions>().0;
    let ratings = &app.world().get::<ActiveStationRatings>(ship).unwrap().0;
    for (token, station) in [(OWNER, "engineering"), (OTHER, "helm")] {
        let id = StationId(station.into());
        let player = sessions
            .players()
            .iter()
            .find(|p| p.token == token)
            .unwrap();
        assert!(player.connected && player.ready);
        assert_eq!(player.station.as_ref(), Some(&id));
        assert_eq!(sessions.holder_for_station(&id), Some(token));
        assert_eq!(ratings.get(&id).map(String::as_str), Some("Std"));
    }
    let sources = &app.world().get::<ShipSystemControlSources>(ship).unwrap().0;
    for name in ["repair", "helm-thrust"] {
        let id = SystemId(name.into());
        assert!(sources.entries().any(|(key, _)| key == &id));
        let policy = sources.policy_for(&id);
        assert!(policy.accept_human_input && !policy.operate_ai);
    }
}

fn caches(app: &App) -> Value {
    let world = app.world();
    let mut boards: Vec<_> = world
        .resource::<LastBroadcastBlackboards>()
        .0
        .iter()
        .collect();
    boards.sort_by(|a, b| a.0.cmp(b.0));
    let mut hull: Vec<_> = world.resource::<LastBroadcastHull>().0.iter().collect();
    hull.sort_by(|a, b| a.0.cmp(b.0));
    let repair = world.resource::<LastVisibleRepairBlackboard>();
    let mut projections: Vec<_> = repair.projections.iter().collect();
    projections.sort_by(|a, b| a.0.cmp(b.0));
    let mut stations: Vec<_> = repair.stations.iter().collect();
    stations.sort_by(|a, b| a.0.cmp(b.0));
    json!({"blackboards": boards, "hull": format!("{hull:?}"),
        "repair": projections, "stations": stations})
}

fn state(app: &App, ship: Entity) -> Value {
    let world = app.world();
    let hull = &world.get::<EntitySystemHull>(ship).unwrap().0;
    let rows: Vec<_> = hull
        .iter()
        .map(|(id, entry)| (id, format!("{entry:?}")))
        .collect();
    json!({"tick": world.resource::<SimTick>().0, "digest": world_digest(world),
        "hull": rows, "teams": world.get::<ShipRepairTeams>(ship).unwrap().0.slots(),
        "admitted": format!("{:?}", world.get::<AdmittedCommands>(ship).unwrap().0),
        "caches": caches(app)})
}

fn read(app: &App, cursor: &mut MessageCursor<OutboundMessage>) -> Vec<OutboundMessage> {
    cursor
        .read(app.world().resource::<Messages<OutboundMessage>>())
        .cloned()
        .collect()
}

fn wire(messages: &[OutboundMessage]) -> Value {
    json!(messages
        .iter()
        .map(|m| json!({"target":format!("{:?}",m.target),
        "delivery":format!("{:?}",m.delivery),"message":m.msg}))
        .collect::<Vec<_>>())
}

fn helm_snapshots(messages: &[OutboundMessage]) -> Value {
    wire(
        &messages
            .iter()
            .filter(|m| {
                m.delivery == DeliveryClass::Snapshot
                    && match &m.target {
                        Target::All => true,
                        Target::Token(token) => token == OTHER,
                        Target::AllExcept(token) => token != OTHER,
                    }
            })
            .inspect(|m| {
                if let ServerMessage::BlackboardUpdate { updates } = &m.msg {
                    for (_, board) in updates {
                        if let SystemBlackboard::Repair(board) = board {
                            assert!(board.teams.is_empty());
                            assert!(board.system_hull.is_empty());
                            assert!(board.damageable_systems.is_empty());
                            assert!(board.aggregate_hull_fraction.is_none());
                        }
                    }
                }
            })
            .cloned()
            .collect::<Vec<_>>(),
    )
}

fn extra_engineering_snapshots(
    reconnect: &[OutboundMessage],
    baseline: &[OutboundMessage],
) -> Vec<ServerMessage> {
    let targeted = |messages: &[OutboundMessage]| {
        messages
            .iter()
            .filter(|m| {
                m.delivery == DeliveryClass::Snapshot && m.target == Target::Token(OWNER.into())
            })
            .map(|m| m.msg.clone())
            .collect::<Vec<_>>()
    };
    let mut extra = targeted(reconnect);
    // Remove only byte-equivalent baseline deliveries, retaining the complete
    // original wires in the report and the order of the reconnect-only cohort.
    for ordinary in targeted(baseline) {
        let value = serde_json::to_value(&ordinary).unwrap();
        let index = extra
            .iter()
            .position(|m| serde_json::to_value(m).unwrap() == value)
            .expect("every ordinary targeted snapshot still delivered unchanged");
        extra.remove(index);
    }
    extra
}

fn coherent_pair(extra: &[ServerMessage], target: &SystemId) -> bool {
    assert_eq!(
        extra.len(),
        2,
        "one full blackboard + hull cohort for one real Identify"
    );
    let ServerMessage::BlackboardUpdate { updates } = &extra[0] else {
        panic!("lexical blackboards owner must precede hull")
    };
    let repair: Vec<_> = updates
        .iter()
        .filter_map(|(_, board)| match board {
            SystemBlackboard::Repair(board) => Some(board),
            _ => None,
        })
        .collect();
    assert_eq!(repair.len(), 1, "actual published Repair board");
    let ServerMessage::SystemHullUpdate {
        entries,
        aggregate_fraction,
        destroyed_fraction,
    } = &extra[1]
    else {
        panic!("actual Hull owner second")
    };
    assert_eq!(
        &repair[0].system_hull, entries,
        "same fine-system visibility and HP"
    );
    assert_eq!(&repair[0].aggregate_hull_fraction, aggregate_fraction);
    assert_eq!(&repair[0].destroyed_hull_fraction, destroyed_fraction);
    // Published team text may legitimately be a different cadence. We retain
    // it verbatim rather than pretending it is the current team state.
    entries.iter().any(|entry| &entry.system_id == target)
}

#[test]
#[ignore = "whole-App source-bound default/default/pinned Repair reconnect proof"]
fn admitted_repair_and_identify_preserve_coherent_snapshots() {
    assert!(
        std::env::var_os(ROLE_ENV).is_none(),
        "parent may not run inside a child"
    );
    let mut reports = Vec::new();
    for role in ["default-1", "default-2", "pinned"] {
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                CHILD,
                "--ignored",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(ROLE_ENV, role)
            .output()
            .unwrap();
        let stdout = String::from_utf8(output.stdout).unwrap();
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(
            output.status.success(),
            "{role}: {}\n{stdout}\n{stderr}",
            output.status
        );
        assert!(stdout.contains("test result: ok. 1 passed; 0 failed;"));
        let rows: Vec<_> = stdout
            .lines()
            .filter_map(|line| line.strip_prefix(PREFIX))
            .collect();
        assert_eq!(rows.len(), 1, "one complete positive child record");
        let report: Value = serde_json::from_str(rows[0]).unwrap();
        assert_eq!(report["role"], role);
        assert_eq!(report["seed"], SEED);
        assert!(report["trace"].as_array().unwrap().len() > 3);
        if role == "pinned" {
            assert_eq!(report["workers"], 1);
            assert_eq!(report["executor"], "SingleThreaded");
        } else {
            assert!(report["workers"].as_u64().unwrap() > 1);
            assert_eq!(report["executor"], "MultiThreaded");
        }
        // Retain each child's actual completion summary and raw trace alongside
        // the parent's comparisons, including diagnostics from successful runs.
        print!("{stdout}");
        eprint!("{stderr}");
        reports.push(report);
    }
    for pair in reports.windows(2) {
        assert_ne!(pair[0]["process"], pair[1]["process"]);
        assert_eq!(pair[0]["setup"], pair[1]["setup"]);
        assert_eq!(
            pair[0]["trace"], pair[1]["trace"],
            "full wires/state/cache/digests across modes"
        );
    }
}

#[test]
#[ignore = "child: only run through admitted_repair_and_identify_preserve_coherent_snapshots"]
fn repair_identify_child() {
    let role = std::env::var(ROLE_ENV).expect("fresh parent-selected child role");
    assert!(["default-1", "default-2", "pinned"].contains(&role.as_str()));
    let pinned = role == "pinned";
    let args = HeadlessArgs {
        world_path: "assets/worlds/probe_reach_anchor.toml".into(),
        ship_path: "assets/entities/alliance_cruiser.toml".into(),
        seed: Some(SEED),
        deterministic: pinned,
        ..Default::default()
    };
    let mut apps = [
        build_headless_app(&args).unwrap(),
        build_headless_app(&args).unwrap(),
    ];
    for app in &mut apps {
        let period = sim_tick_period(app.world().resource::<WorldConfig>().global.sim_tick_hz);
        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(period));
        app.finish();
        app.cleanup();
        for _ in 0..6 {
            app.update();
        }
        assert_eq!(app.world().resource::<Time<Fixed>>().timestep(), period);
        for token in [OWNER, OTHER] {
            identify(app, token);
        }
        step(app);
        for (token, station) in [(OWNER, "engineering"), (OTHER, "helm")] {
            send(
                app,
                token,
                ClientMessage::SelectStation {
                    station: station.into(),
                },
            );
        }
        step(app);
        for token in [OWNER, OTHER] {
            send(app, token, ClientMessage::SetReady { ready: true });
        }
        step(app);
        for _ in 0..20 {
            step(app);
        }
        assert_eq!(app.world().resource::<SimRng>().seed(), SEED);
    }
    let ships = [local(&mut apps[0]), local(&mut apps[1])];
    let workers = bevy::tasks::ComputeTaskPool::get().thread_num();
    let executor = if pinned {
        assert_eq!(workers, 1);
        bevy::ecs::schedule::ExecutorKind::SingleThreaded
    } else {
        assert!(workers > 1);
        bevy::ecs::schedule::ExecutorKind::MultiThreaded
    };
    let target = SystemId("helm-engine-port".into());
    let mut setups = Vec::new();
    for (app, ship) in apps.iter_mut().zip(ships) {
        assert_human(app, ship);
        assert_eq!(
            app.get_schedule(FixedUpdate).unwrap().get_executor_kind(),
            executor
        );
        let config = &app
            .world()
            .get::<project_phoenix::ship_plugin::ShipConfigComponent>(ship)
            .unwrap()
            .0;
        assert!(config
            .systems_for_station(&StationId("helm".into()))
            .any(|system| system.id == target));
        let teams = &app.world().get::<ShipRepairTeams>(ship).unwrap().0;
        assert!(teams
            .slots()
            .iter()
            .all(|slot| matches!(slot, TeamSlot::Idle)));
        let timings = teams.timings();
        assert!(timings.travel_duration > 0.0 && timings.repair_rate_hp_per_sec > 0.0);
        let count = teams.slots().len();
        assert!(count > 0);
        let maximum = app
            .world()
            .get::<EntitySystemHull>(ship)
            .unwrap()
            .0
            .get(&target)
            .unwrap()
            .max;
        // Damage only a real fine-system HP row; counts/timings/config/teams stay authored.
        app.world_mut()
            .get_mut::<EntitySystemHull>(ship)
            .unwrap()
            .0
            .set_hp(&target, maximum * 0.5);
        setups.push(json!({"count":count,"travel":timings.travel_duration,
            "repair_rate":timings.repair_rate_hp_per_sec,"target":target,"initial_hp":maximum * 0.5}));
        // Let ordinary Repair publish and live recipient caches observe the damage.
        for _ in 0..3 {
            step(app);
        }
    }
    assert_eq!(setups[0], setups[1]);
    assert_eq!(state(&apps[0], ships[0]), state(&apps[1], ships[1]));
    let mut cursors = [MessageCursor::default(), MessageCursor::default()];
    let mut balance = [
        MessageCursor::<BalanceEvent>::default(),
        MessageCursor::default(),
    ];
    for i in 0..2 {
        read(&apps[i], &mut cursors[i]);
        balance[i]
            .read(apps[i].world().resource::<Messages<BalanceEvent>>())
            .count();
    }
    // Solo boot defaults to immediate ingress. Configure the existing test
    // transport-delay seam explicitly so pre-due admission is non-vacuous;
    // this changes neither the authored team count nor any repair timing.
    for app in &mut apps {
        app.insert_resource(project_phoenix::command_admission::log::CommandDelay(
            TEST_COMMAND_DELAY_TICKS,
        ));
    }
    let delay = apps[0]
        .world()
        .resource::<project_phoenix::command_admission::log::CommandDelay>()
        .0;
    assert!(delay > 0);
    assert_eq!(delay, TEST_COMMAND_DELAY_TICKS);
    let due = apps[0].world().resource::<SimTick>().0 + delay;
    let payload = SystemControlPayload::DispatchRepairTeam {
        team_idx: 0,
        target: RepairTarget::Station(StationId("helm".into())),
    };
    let correlation = ActionCorrelationId::new(CORRELATION).unwrap();
    for app in &mut apps {
        assert_eq!(
            app.world()
                .resource::<project_phoenix::command_admission::log::CommandDelay>()
                .0,
            delay
        );
        send(
            app,
            OWNER,
            ClientMessage::ControlSystemCorrelated {
                correlation: correlation.clone(),
                target: SystemId("repair".into()),
                payload: payload.clone(),
            },
        );
    }
    let dt = apps[0]
        .world()
        .resource::<Time<Fixed>>()
        .timestep()
        .as_secs_f32();
    let travel = apps[0]
        .world()
        .get::<ShipRepairTeams>(ships[0])
        .unwrap()
        .0
        .timings()
        .travel_duration;
    let limit = delay as usize + (travel / dt * 2.0).ceil() as usize + 12;
    let mut trace = Vec::new();
    let mut pre_due_idle = 0;
    let (mut hidden, mut visible, mut travelling, mut on_site, mut repairs) = (0, 0, 0, 0, 0);
    for _ in 0..limit {
        let tick = apps[0].world().resource::<SimTick>().0;
        let reconnect = tick >= due;
        if tick <= due {
            for (app, ship) in apps.iter().zip(ships) {
                assert!(
                    matches!(
                        app.world().get::<ShipRepairTeams>(ship).unwrap().0.slots()[0],
                        TeamSlot::Idle
                    ),
                    "not applied early"
                );
            }
        }
        // Ordinary Identify on the exact admitted-command tick and every travel/
        // arrival boundary. This cannot miss the actual visibility transition.
        if reconnect {
            identify(&mut apps[0], OWNER);
        }
        let mut messages = Vec::new();
        let mut restored = Vec::new();
        for i in 0..2 {
            step(&mut apps[i]);
            assert_human(&apps[i], ships[i]);
            messages.push(read(&apps[i], &mut cursors[i]));
            let uuid = &apps[i].world().get::<EntityUuid>(ships[i]).unwrap().0;
            let hp: Vec<_> = balance[i]
                .read(apps[i].world().resource::<Messages<BalanceEvent>>())
                .filter_map(|event| match event {
                    BalanceEvent::RepairApplied { ship, hp } if ship == uuid => Some(*hp),
                    _ => None,
                })
                .collect();
            restored.push(hp);
        }
        assert_eq!(
            state(&apps[0], ships[0]),
            state(&apps[1], ships[1]),
            "reconnect cannot change shared state or caches"
        );
        assert_eq!(helm_snapshots(&messages[0]), helm_snapshots(&messages[1]));
        assert_eq!(restored[0], restored[1]);
        repairs += restored[0].len();
        for i in 0..2 {
            let feedback = messages[i].iter().filter(|m| m.target == Target::Token(OWNER.into())
                && m.delivery == DeliveryClass::Reliable && matches!(&m.msg,
                    ServerMessage::ActionFeedback { correlation: actual, outcome: ActionFeedbackOutcome::Applied } if actual == &correlation)).count();
            assert_eq!(feedback, usize::from(tick == due));
            if tick == due {
                let admitted = &apps[i].world().get::<AdmittedCommands>(ships[i]).unwrap().0;
                assert_eq!(
                    admitted
                        .iter()
                        .filter(|command| command.target.0 == "repair"
                            && command.payload == payload
                            && command.response_token.as_deref() == Some(OWNER)
                            && command.feedback_correlation.as_ref() == Some(&correlation))
                        .count(),
                    1
                );
            }
        }
        let extra = extra_engineering_snapshots(&messages[0], &messages[1]);
        let welcomes = |rows: &[OutboundMessage]| {
            rows.iter()
                .filter(|m| {
                    m.target == Target::Token(OWNER.into())
                        && m.delivery == DeliveryClass::Reliable
                        && matches!(m.msg, ServerMessage::Welcome { .. })
                })
                .count()
        };
        assert_eq!(welcomes(&messages[0]), usize::from(reconnect));
        assert_eq!(welcomes(&messages[1]), 0);
        let exposed = if reconnect {
            Some(coherent_pair(&extra, &target))
        } else {
            assert!(extra.is_empty());
            None
        };
        if let Some(exposed) = exposed {
            if exposed {
                visible += 1;
            } else {
                hidden += 1;
            }
        }
        match &apps[0]
            .world()
            .get::<ShipRepairTeams>(ships[0])
            .unwrap()
            .0
            .slots()[0]
        {
            TeamSlot::Travelling { system_id, .. } => {
                assert_eq!(system_id.as_ref(), Some(&target));
                travelling += 1;
                assert_eq!(exposed, Some(false), "travelling is not on-site visibility");
            }
            TeamSlot::Repairing { system_id, .. } => {
                assert_eq!(system_id.as_ref(), Some(&target));
                on_site += 1;
                if on_site >= 2 {
                    assert_eq!(
                        exposed,
                        Some(true),
                        "settled on-site detail reaches Engineering"
                    );
                }
            }
            TeamSlot::Idle if tick < due => {
                pre_due_idle += 1;
            }
            other => panic!("unexpected authored progression: {other:?}"),
        }
        trace.push(
            json!({"before_tick":tick,"reconnect":reconnect,"exposed":exposed,
            "state":state(&apps[0],ships[0]),"reconnect_wire":wire(&messages[0]),
            "ordinary_wire":wire(&messages[1]),"repair_hp_events":restored[0]}),
        );
        if on_site >= 4 && visible >= 3 {
            break;
        }
    }
    assert!(pre_due_idle > 0, "observed real pre-due idle boundaries");
    assert_eq!(pre_due_idle, delay as usize);
    assert!(travelling > 0 && on_site >= 4 && repairs > 0 && hidden > 0 && visible >= 3);
    assert!(
        apps[0]
            .world()
            .get::<EntitySystemHull>(ships[0])
            .unwrap()
            .0
            .get(&target)
            .unwrap()
            .current
            > setups[0]["initial_hp"].as_f64().unwrap() as f32
    );
    println!(
        "\n{PREFIX}{}",
        json!({"role":role,"process":std::process::id(),"seed":SEED,
        "workers":workers,"executor":format!("{executor:?}"),"setup":setups[0],
        "configured_command_delay_ticks":delay,"observed_pre_due_idle_boundaries":pre_due_idle,
        "trace":trace})
    );
}
