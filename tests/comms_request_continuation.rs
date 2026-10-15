//! Human typed request -> physical radio -> ordinary Comms AI -> original-tick return.
#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]
use bevy::prelude::*;
use phoenix_model::{
    comms_channel::CommsAudience,
    comms_request::{CommsRadioContent, CommsRequestStage, GenericCommsRequest},
};
use project_phoenix::{
    comms::server::{CommsInboxRes, CommsRuntime},
    core::messages::{AdmittedCommands, ClientMessage, SystemControlPayload as Control, SystemId},
    entities::spawner::EntityUuid,
    headless::{build_headless_app, HeadlessArgs},
    lobby::{server::InboundMessage, Sessions},
    server_app::LocalShip,
    ship::power::ShipPowerSystem,
    ship_plugin::{HumanSeekingHosts, ShipConfigComponent, ShipSystemControlSources},
    sim_digest::world_digest,
    sim_tick::sim_tick_period,
    snapshot::{capture, ready_to_restore, restore, PhoenixSnapshot},
    world::config::WorldConfig,
};
const WORLD: &str = "tests/fixtures/comms-request-world.toml";
const HULL: &str = "assets/entities/alliance_cruiser.toml";
const TOKEN: &str = "comms-request-officer";
fn steps(app: &mut App, count: usize) {
    for _ in 0..count {
        app.update();
    }
}
fn local(app: &mut App) -> Entity {
    app.world_mut()
        .query_filtered::<Entity, With<LocalShip>>()
        .single(app.world())
        .unwrap()
}
fn send(app: &mut App, msg: ClientMessage) {
    app.world_mut().write_message(InboundMessage {
        token: TOKEN.into(),
        msg,
    });
}
fn boot() -> App {
    let mut app = build_headless_app(&HeadlessArgs {
        world_path: WORLD.into(),
        ship_path: HULL.into(),
        deterministic: true,
        seed: Some(156216),
        ..Default::default()
    })
    .unwrap();
    let period = sim_tick_period(app.world().resource::<WorldConfig>().global.sim_tick_hz);
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(period));
    app.finish();
    app.cleanup();
    steps(&mut app, 10);
    app
}
fn prepared() -> (App, Entity) {
    let mut app = boot();
    let entity = local(&mut app);
    let graph = &app.world().get::<ShipConfigComponent>(entity).unwrap().0;
    let station = graph
        .system_home_station(graph.system(&SystemId("comms".into())).unwrap())
        .unwrap()
        .clone();
    let rating_name = graph
        .stations
        .iter()
        .find(|row| row.id == station)
        .unwrap()
        .ratings
        .first()
        .unwrap()
        .name
        .clone();
    for msg in [
        ClientMessage::Identify {
            token: TOKEN.into(),
            name: TOKEN.into(),
        },
        ClientMessage::SelectStation {
            station: station.0.clone(),
        },
        ClientMessage::SetReady { ready: true },
        ClientMessage::SetStationRating { rating_name },
    ] {
        send(&mut app, msg);
        steps(&mut app, 4);
    }
    let payload = Control::SendCommsRequest {
        channel: "public".into(),
        audience: CommsAudience::All,
        request: GenericCommsRequest::ReportStatus,
        planned_request: None,
    };
    for _ in 0..120 {
        let sessions = app.world().resource::<Sessions>();
        let owns = sessions.0.holder_for_station(&station) == Some(TOKEN)
            && sessions
                .0
                .players()
                .iter()
                .any(|player| player.token == TOKEN && player.ready && player.connected)
            && project_phoenix::command_admission::policy::is_command_authorized(
                TOKEN,
                &SystemId("comms".into()),
                &payload,
                app.world().get::<ShipSystemControlSources>(entity).unwrap(),
                sessions,
                &app.world().get::<ShipConfigComponent>(entity).unwrap().0,
                app.world().get::<HumanSeekingHosts>(entity),
            );
        if owns {
            return (app, entity);
        }
        app.update();
    }
    panic!("ordinary identified/ready Comms holder did not acquire actual Console authority");
}
fn inbox(app: &App) -> Vec<project_phoenix::core::messages::CommsMessage> {
    app.world().resource::<CommsInboxRes>().0.messages()
}
fn first_difference(a: &serde_json::Value, b: &serde_json::Value, path: &str) -> Option<String> {
    match (a, b) {
        (serde_json::Value::Object(a), serde_json::Value::Object(b)) => {
            for key in a
                .keys()
                .chain(b.keys())
                .collect::<std::collections::BTreeSet<_>>()
            {
                match (a.get(key), b.get(key)) {
                    (Some(a), Some(b)) => {
                        if let Some(diff) = first_difference(a, b, &format!("{path}.{key}")) {
                            return Some(diff);
                        }
                    }
                    _ => return Some(format!("{path}.{key}: presence differs")),
                }
            }
            None
        }
        (serde_json::Value::Array(a), serde_json::Value::Array(b)) => {
            if a.len() != b.len() {
                return Some(format!("{path}: length{} vs{}", a.len(), b.len()));
            }
            a.iter()
                .zip(b)
                .enumerate()
                .find_map(|(i, (a, b))| first_difference(a, b, &format!("{path}[{i}]")))
        }
        _ => (a != b).then(|| format!("{path}: {a} vs {b}")),
    }
}
#[test]
fn human_request_partial_ai_reading_and_original_clock_report_resume_exactly_for_360_steps() {
    let (mut live, source) = prepared();
    let source_uuid = live.world().get::<EntityUuid>(source).unwrap().0.clone();
    let mut paid_source = false;
    send(
        &mut live,
        ClientMessage::ControlSystem {
            target: SystemId("comms".into()),
            payload: Control::SendCommsRequest {
                channel: "public".into(),
                audience: CommsAudience::All,
                request: GenericCommsRequest::ReportStatus,
                planned_request: None,
            },
        },
    );
    let mut checkpoint = None;
    for _ in 0..120 {
        live.update();
        paid_source |= live
            .world()
            .get::<ShipPowerSystem>(source)
            .unwrap()
            .0
            .emitted_work(&SystemId("comms".into()))
            >= 1.0;
        let runtime = live.world().resource::<CommsRuntime>();
        if runtime.pending_ai_responses.iter().any(|(key, record)| {
            key.ship_uuid != source_uuid
                && record.request_reading.is_some_and(|reading| {
                    reading.remaining_ticks > 0 && reading.remaining_ticks < reading.required_ticks
                })
        }) {
            checkpoint = Some(capture(live.world()));
            break;
        }
    }
    let saved: PhoenixSnapshot = serde_json::from_slice(
        &serde_json::to_vec(
            &checkpoint.expect("actual paid request reaches partial ordinary AI reading"),
        )
        .unwrap(),
    )
    .unwrap();
    assert!(
        paid_source,
        "actual request emission pays finite source work"
    );
    let received = inbox(&live)
        .into_iter()
        .find(|message| {
            message.sender_uuid == source_uuid
                && message.transmission.as_ref().is_some_and(|wire| {
                    wire.received_tick.is_some()
                        && matches!(wire.content, CommsRadioContent::Request { .. })
                })
        })
        .unwrap();
    let reporter = received.recipient_ship.as_ref().unwrap().0.clone();
    let heard = received
        .transmission
        .as_ref()
        .unwrap()
        .received_tick
        .unwrap();
    assert!(received
        .request_progress
        .as_ref()
        .unwrap()
        .facts
        .read_tick
        .is_none());
    let (mut resumed, _) = prepared();
    for _ in 0..240 {
        if ready_to_restore(resumed.world(), &saved) {
            break;
        }
        resumed.update();
    }
    assert!(ready_to_restore(resumed.world(), &saved));
    let report = restore(resumed.world_mut(), &saved);
    assert!(report.is_complete(), "{report:?}");
    let mut ai_response = false;
    for step in 0..360 {
        let actual = capture(live.world());
        let continued = capture(resumed.world());
        assert!(
            actual == continued,
            "step{step}: {:?}",
            first_difference(
                &serde_json::to_value(&actual).unwrap(),
                &serde_json::to_value(&continued).unwrap(),
                "snapshot"
            )
        );
        assert_eq!(
            world_digest(live.world()),
            world_digest(resumed.world()),
            "step{step}"
        );
        let mut actors = live.world_mut().query::<(&EntityUuid, &AdmittedCommands)>();
        ai_response|=actors.iter(live.world()).any(|(uuid,commands)|uuid.0==reporter&&commands.0.iter().any(|command|
            command.response_token.as_deref()==Some(format!("ai:{reporter}").as_str())&&matches!(&command.payload,Control::RespondToMessage{message_id,response_index:0} if message_id==&received.id)));
        live.update();
        resumed.update();
    }
    assert!(
        ai_response,
        "actual receiving Comms controller emits the normal admitted Report"
    );
    let requests = inbox(&live);
    let recipient = requests
        .iter()
        .find(|message| message.id == received.id)
        .unwrap();
    let facts = &recipient.request_progress.as_ref().unwrap().facts;
    assert_eq!(facts.stage(), CommsRequestStage::Completed);
    assert!(facts.read_tick.unwrap() >= heard + 30);
    let returned = requests
        .iter()
        .find_map(|message| {
            message
                .transmission
                .as_ref()
                .and_then(|wire| match &wire.content {
                    CommsRadioContent::Outcome { outcome }
                        if wire.received_tick.is_some()
                            && outcome.request.origin_id
                                == received.transmission.as_ref().unwrap().origin_id
                            && outcome.facts.report.is_some() =>
                    {
                        Some((wire, outcome))
                    }
                    _ => None,
                })
        })
        .unwrap();
    assert_eq!(
        returned.1.facts, *facts,
        "return keeps original local event ticks"
    );
    assert!(returned.0.received_tick.unwrap() >= returned.0.emitted_tick);
    let before = capture(live.world());
    let digest = world_digest(live.world());
    for fault in 0..4 {
        let mut bad = before.clone();
        bad.tick += 5;
        let message=bad.comms.as_mut().unwrap().inbox.iter_mut().find(|message|message.transmission.as_ref().is_some_and(|wire|matches!(&wire.content,CommsRadioContent::Outcome{outcome} if outcome.facts.report.is_some()))).unwrap();
        let CommsRadioContent::Outcome { outcome } =
            &mut message.transmission.as_mut().unwrap().content
        else {
            unreachable!()
        };
        match fault {
            0 => outcome.request.recipient_ship = source_uuid.clone(),
            1 => outcome.facts.read_tick = Some(heard - 1),
            2 => outcome.facts.report.as_mut().unwrap().observed_tick += 1,
            _ => outcome.facts.report.as_mut().unwrap().system = SystemId("helm-thrust".into()),
        }
        assert!(!restore(live.world_mut(), &bad).is_complete());
        assert_eq!(capture(live.world()), before);
        assert_eq!(world_digest(live.world()), digest);
    }
}
