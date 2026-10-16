//! Ordinary Human/Backfill own intent, actual paid public text and JSON360.
#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]
use bevy::prelude::*;
use phoenix_model::{comms_request::CommsRadioContent, comms_traffic::ReportedAlert};
use project_phoenix::{
    comms::{server::CommsInboxRes, traffic::ShipCommsTraffic},
    core::messages::{
        AdmittedCommands, ClientMessage, GamePhase, SystemBlackboard,
        SystemControlPayload as Control, SystemId,
    },
    entities::spawner::EntityUuid,
    headless::{build_headless_app, HeadlessArgs},
    lobby::{server::InboundMessage, Sessions},
    server_app::LocalShip,
    ship::{control_source::ControlSource, power::ShipPowerSystem},
    ship_plugin::{HumanSeekingHosts, ShipConfigComponent, ShipSystemControlSources},
    sim_digest::world_digest,
    sim_tick::{sim_tick_period, SimTick},
    snapshot::{capture, ready_to_restore, restore, PhoenixSnapshot},
    world::{config::WorldConfig, server::WorldContentRuntime},
};
fn id(value: &str) -> SystemId {
    SystemId(value.into())
}
fn steps(app: &mut App, n: usize) {
    for _ in 0..n {
        app.update();
    }
}
fn send(app: &mut App, token: &str, msg: ClientMessage) {
    app.world_mut().write_message(InboundMessage {
        token: token.into(),
        msg,
    });
}
#[derive(Resource, Default)]
struct PaidWitness {
    reports: u64,
    ai_reports: u64,
    paid_reports: u64,
}
fn observe(
    tick: Res<SimTick>,
    fixed: Res<Time<Fixed>>,
    actors: Query<
        (
            &ShipCommsTraffic,
            &ShipPowerSystem,
            &ShipSystemControlSources,
            &AdmittedCommands,
        ),
        With<LocalShip>,
    >,
    mut witness: ResMut<PaidWitness>,
) {
    for (traffic, power, sources, commands) in &actors {
        if !traffic
            .state
            .last_report
            .is_some_and(|report| report.emitted_tick == tick.0)
        {
            continue;
        }
        witness.reports += 1;
        witness.ai_reports += u64::from(
            sources.0.source_for(&id("comms")) == ControlSource::Ai
                && commands
                    .for_target("comms")
                    .any(|command| matches!(command.payload, Control::ReportCommsAlert)),
        );
        let baseline = power
            .0
            .config()
            .consumers
            .iter()
            .find(|consumer| consumer.system == id("comms"))
            .unwrap()
            .continuous_cost_per_sec
            * fixed.timestep().as_secs_f32();
        witness.paid_reports +=
            u64::from(power.0.emitted_work(&id("comms")) > f64::from(baseline) + 1e-7);
    }
}
fn boot() -> (App, Entity, String) {
    let mut app = build_headless_app(&HeadlessArgs {
        world_path: "tests/fixtures/listening-world.toml".into(),
        ship_path: "tests/fixtures/listening-cruiser.toml".into(),
        deterministic: true,
        seed: Some(15621666),
        ..Default::default()
    })
    .unwrap();
    app.init_resource::<PaidWitness>();
    app.add_systems(
        FixedUpdate,
        observe
            .in_set(project_phoenix::sim_sets::SimSet::Physics)
            .after_ignore_deferred(project_phoenix::sim_sets::FixedStep::TickCommsRadio),
    );
    let period = sim_tick_period(app.world().resource::<WorldConfig>().global.sim_tick_hz);
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(period));
    app.finish();
    app.cleanup();
    steps(&mut app, 10);
    assert_eq!(
        app.world().resource::<State<GamePhase>>().get(),
        &GamePhase::InProgress
    );
    let actor = app
        .world_mut()
        .query_filtered::<Entity, With<LocalShip>>()
        .single(app.world())
        .unwrap();
    let listener = app.world().resource::<WorldContentRuntime>().name_to_uuid["listener"].clone();
    assert!(app
        .world_mut()
        .query::<&EntityUuid>()
        .iter(app.world())
        .any(|uuid| uuid.0 == listener));
    let stations = app
        .world()
        .get::<ShipConfigComponent>(actor)
        .unwrap()
        .0
        .stations
        .clone();
    assert_eq!(stations.len(), 6);
    for station in stations {
        let token = format!("traffic-{}", station.id.0);
        for message in [
            ClientMessage::Identify {
                token: token.clone(),
                name: token.clone(),
            },
            ClientMessage::SelectStation {
                station: station.id.0,
            },
            ClientMessage::SetStationRating {
                rating_name: station.ratings[0].name.clone(),
            },
            ClientMessage::SetReady { ready: true },
        ] {
            send(&mut app, &token, message);
            steps(&mut app, 4);
        }
    }
    (app, actor, listener)
}
fn enqueue(app: &mut App, actor: Entity, owner: &str, payload: Control) {
    let sources = app.world().get::<ShipSystemControlSources>(actor).unwrap();
    let config = &app.world().get::<ShipConfigComponent>(actor).unwrap().0;
    let sessions = app.world().resource::<Sessions>();
    let token = sessions
        .0
        .players()
        .iter()
        .find(|player| {
            project_phoenix::command_admission::policy::is_command_authorized(
                &player.token,
                &id(owner),
                &payload,
                sources,
                sessions,
                config,
                app.world().get::<HumanSeekingHosts>(actor),
            )
        })
        .unwrap()
        .token
        .clone();
    send(
        app,
        &token,
        ClientMessage::ControlSystem {
            target: id(owner),
            payload,
        },
    );
}
fn reports(app: &App, actor: Entity, recipient: &str) -> usize {
    let source = &app.world().get::<EntityUuid>(actor).unwrap().0;
    app.world()
        .resource::<CommsInboxRes>()
        .0
        .iter()
        .filter(|message| {
            message.sender_uuid == *source
                && message.is_for_ship(Some(recipient))
                && message
                    .transmission
                    .as_ref()
                    .is_some_and(|wire| wire.content == CommsRadioContent::Text)
                && [
                    "We have selected routine alert.",
                    "We have selected red alert.",
                ]
                .contains(&message.body.as_str())
        })
        .count()
}
fn state(app: &App, actor: Entity) -> project_phoenix::comms::traffic::CommsTrafficState {
    app.world()
        .get::<ShipCommsTraffic>(actor)
        .unwrap()
        .state
        .clone()
}
fn same(a: &App, b: &App, step: usize) {
    assert_eq!(
        capture(a.world()),
        capture(b.world()),
        "whole capture {step}"
    );
    assert_eq!(
        world_digest(a.world()),
        world_digest(b.world()),
        "digest {step}"
    );
}
#[test]
fn human_changed_intent_and_spacing_survive_whole_json360_without_fresh_coverage_or_forged_views() {
    let (mut live, actor, listener) = boot();
    enqueue(
        &mut live,
        actor,
        "comms",
        Control::SetCommsListeningOrder {
            priority: vec!["fleet".into(), "public".into()],
        },
    );
    steps(&mut live, 4);
    assert_eq!(state(&live, actor).priority, vec!["fleet", "public"]);
    let before = reports(&live, actor, &listener);
    enqueue(
        &mut live,
        actor,
        "red-alert",
        Control::SetRedAlert { active: true },
    );
    steps(&mut live, 4);
    enqueue(&mut live, actor, "comms", Control::ReportCommsAlert);
    steps(&mut live, 4);
    assert_eq!(
        reports(&live, actor, &listener),
        before + 1,
        "a sender can emit on a supported channel outside its own receive selection"
    );
    assert_eq!(
        state(&live, actor).last_report.unwrap().semantic,
        ReportedAlert::RedAlert
    );
    assert_eq!(
        live.world().resource::<PaidWitness>().paid_reports,
        live.world().resource::<PaidWitness>().reports
    );
    let uuid = live.world().get::<EntityUuid>(actor).unwrap().0.clone();
    let saved: PhoenixSnapshot =
        serde_json::from_slice(&serde_json::to_vec(&capture(live.world())).unwrap()).unwrap();
    for fault in 0..6 {
        let mut bad = saved.clone();
        let row = bad
            .entities
            .iter_mut()
            .find(|row| row.uuid == uuid)
            .unwrap();
        match fault {
            0 => row.comms_traffic = None,
            1 => row.comms_traffic.as_mut().unwrap().owner = id("foreign"),
            2 => {
                row.comms_traffic.as_mut().unwrap().priority = vec!["fleet".into(), "fleet".into()]
            }
            3 => {
                row.comms_traffic
                    .as_mut()
                    .unwrap()
                    .last_report
                    .as_mut()
                    .unwrap()
                    .emitted_tick = saved.tick + 1
            }
            4 => {
                let (_, SystemBlackboard::Captain(board)) = row
                    .blackboards
                    .iter_mut()
                    .find(|(_, board)| matches!(board, SystemBlackboard::Captain(_)))
                    .unwrap()
                else {
                    unreachable!()
                };
                board.alert_observation.as_mut().unwrap().observed_tick = Some(saved.tick + 1);
            }
            _ => {
                let (_, SystemBlackboard::Comms(board)) = row
                    .blackboards
                    .iter_mut()
                    .find(|(_, board)| matches!(board, SystemBlackboard::Comms(_)))
                    .unwrap()
                else {
                    unreachable!()
                };
                board.traffic.as_mut().unwrap().report_supported = false;
            }
        }
        let before = capture(live.world());
        let digest = world_digest(live.world());
        assert!(!restore(live.world_mut(), &bad).gaps.is_empty());
        assert_eq!(capture(live.world()), before);
        assert_eq!(world_digest(live.world()), digest);
    }
    let (mut resumed, resumed_actor, _) = boot();
    assert!(ready_to_restore(resumed.world(), &saved));
    assert!(restore(resumed.world_mut(), &saved).gaps.is_empty());
    same(&live, &resumed, 0);
    let frontier = state(&live, actor).last_report;
    let heard = reports(&live, actor, &listener);
    for step in 1..=360 {
        for (host, ship) in [(&mut live, actor), (&mut resumed, resumed_actor)] {
            if step == 30 {
                enqueue(
                    host,
                    ship,
                    "red-alert",
                    Control::SetRedAlert { active: false },
                );
            }
            if step == 31 || step == 100 {
                enqueue(host, ship, "comms", Control::ReportCommsAlert);
            }
        }
        live.update();
        resumed.update();
        same(&live, &resumed, step);
        if step == 40 {
            assert_eq!(state(&live, actor).last_report, frontier);
            assert_eq!(
                reports(&live, actor, &listener),
                heard,
                "refused early report neither stamps nor emits"
            );
        }
    }
    same(&live, &resumed, 360);
    assert_eq!(reports(&live, actor, &listener), heard + 1);
    assert_eq!(
        state(&live, actor).last_report.unwrap().semantic,
        ReportedAlert::Routine
    );
}
#[test]
fn ordinary_backfill_keeps_operator_priority_and_reports_only_meaningful_permitted_changes() {
    let (mut app, actor, listener) = boot();
    enqueue(
        &mut app,
        actor,
        "comms",
        Control::SetCommsListeningOrder {
            priority: vec!["fleet".into(), "public".into()],
        },
    );
    steps(&mut app, 4);
    let before = reports(&app, actor, &listener);
    let ai_before = app.world().resource::<PaidWitness>().ai_reports;
    send(
        &mut app,
        "traffic-comms",
        ClientMessage::SetStationRating {
            rating_name: "Backfill".into(),
        },
    );
    steps(&mut app, 4);
    assert_eq!(
        app.world()
            .get::<ShipSystemControlSources>(actor)
            .unwrap()
            .0
            .source_for(&id("comms")),
        ControlSource::Ai
    );
    enqueue(
        &mut app,
        actor,
        "red-alert",
        Control::SetRedAlert { active: true },
    );
    for _ in 0..360 {
        if reports(&app, actor, &listener) > before {
            break;
        }
        app.update();
    }
    assert_eq!(reports(&app, actor, &listener), before + 1);
    assert!(app.world().resource::<PaidWitness>().ai_reports > ai_before);
    assert_eq!(
        app.world().resource::<PaidWitness>().paid_reports,
        app.world().resource::<PaidWitness>().reports
    );
    assert_eq!(state(&app, actor).priority, vec!["fleet", "public"]);
    steps(&mut app, 360);
    assert_eq!(
        reports(&app, actor, &listener),
        before + 1,
        "unchanged intent never creates periodic chatter"
    );
    assert_eq!(state(&app, actor).priority, vec!["fleet", "public"]);
}
