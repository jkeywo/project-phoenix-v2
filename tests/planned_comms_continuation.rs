//! Released HTN choice -> ordinary Comms AI -> actual radio -> heard report.
#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]
use bevy::prelude::*;
use phoenix_model::{
    activity::{ActivityExecutionIdentity, ActivityOutcome},
    comms_request::CommsRadioContent,
};
use project_phoenix::{
    comms::server::{CommsInboxRes, CommsRuntime},
    core::messages::{AdmittedCommands, SystemControlPayload as Control, SystemId},
    entities::spawner::EntityUuid,
    headless::{build_headless_app, HeadlessArgs},
    server_app::LocalShip,
    ship::power::ShipPowerSystem,
    sim_digest::world_digest,
    sim_tick::sim_tick_period,
    snapshot::{capture, ready_to_restore, restore, PhoenixSnapshot},
    world::{config::WorldConfig, server::WorldContentRuntime},
};
const WORLD: &str = "tests/fixtures/communication-htn-world.toml";
fn boot() -> App {
    let mut app = build_headless_app(&HeadlessArgs {
        world_path: WORLD.into(),
        ship_path: "assets/entities/alliance_cruiser.toml".into(),
        deterministic: true,
        seed: Some(1562224),
        ..Default::default()
    })
    .unwrap();
    let period = sim_tick_period(app.world().resource::<WorldConfig>().global.sim_tick_hz);
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(period));
    app.finish();
    app.cleanup();
    for _ in 0..10 {
        app.update();
    }
    app
}
fn inbox(app: &App) -> Vec<project_phoenix::core::messages::CommsMessage> {
    app.world().resource::<CommsInboxRes>().0.messages()
}
fn same(live: &App, resumed: &App, step: usize) {
    let actual = capture(live.world());
    let continued = capture(resumed.world());
    assert!(
        actual == continued,
        "complete capture differs at step {step}: {:?}",
        first_difference(
            &serde_json::to_value(actual).unwrap(),
            &serde_json::to_value(continued).unwrap(),
            "snapshot"
        )
    );
    assert_eq!(
        world_digest(live.world()),
        world_digest(resumed.world()),
        "digest at step {step}"
    );
}
fn first_difference(a: &serde_json::Value, b: &serde_json::Value, path: &str) -> Option<String> {
    use serde_json::Value;
    match (a, b) {
        (Value::Object(a), Value::Object(b)) => {
            for key in a
                .keys()
                .chain(b.keys())
                .collect::<std::collections::BTreeSet<_>>()
            {
                match (a.get(key), b.get(key)) {
                    (Some(a), Some(b)) => {
                        if let Some(difference) = first_difference(a, b, &format!("{path}.{key}")) {
                            return Some(difference);
                        }
                    }
                    _ => return Some(format!("{path}.{key}: presence differs")),
                }
            }
            None
        }
        (Value::Array(a), Value::Array(b)) => {
            if a.len() != b.len() {
                Some(format!("{path}: lengths differ"))
            } else {
                a.iter()
                    .zip(b)
                    .enumerate()
                    .find_map(|(i, (a, b))| first_difference(a, b, &format!("{path}[{i}]")))
            }
        }
        _ => (a != b).then(|| format!("{path}: {a} vs {b}")),
    }
}
#[test]
fn planned_reading_actual_ai_emission_and_heard_receipt_resume_360_complete_steps() {
    let mut live = boot();
    let source = live
        .world_mut()
        .query_filtered::<Entity, With<LocalShip>>()
        .single(live.world())
        .unwrap();
    let source_uuid = live.world().get::<EntityUuid>(source).unwrap().0.clone();
    let mut checkpoint = None;
    let mut instance = String::new();
    for _ in 0..240 {
        live.update();
        let reading = live
            .world()
            .resource::<CommsRuntime>()
            .pending_ai_responses
            .iter()
            .find(|(key, row)| {
                key.ship_uuid == source_uuid
                    && key.planned_request
                    && row.request_reading.is_some_and(|reading| {
                        reading.remaining_ticks > 0
                            && reading.remaining_ticks < reading.required_ticks
                    })
            });
        if let Some((key, _)) = reading {
            instance = key.message_id.clone();
            checkpoint = Some(capture(live.world()));
            break;
        }
    }
    let saved: PhoenixSnapshot = serde_json::from_slice(
        &serde_json::to_vec(&checkpoint.expect(
            "released known-contact request starts partial reading in the real existing Comms AI",
        ))
        .unwrap(),
    )
    .unwrap();
    let row = &saved.scenario.as_ref().unwrap().situations.instances[&instance];
    assert!(
        row.accepted.is_none(),
        "reading or release is not a fabricated transmission acceptance"
    );
    assert!(inbox(&live).iter().all(|message| !message
        .transmission
        .as_ref()
        .is_some_and(|wire| wire.sustained)));
    let mut resumed = boot();
    for _ in 0..240 {
        if ready_to_restore(resumed.world(), &saved) {
            break;
        }
        resumed.update();
    }
    assert!(ready_to_restore(resumed.world(), &saved));
    let before = capture(resumed.world());
    let before_digest = world_digest(resumed.world());
    for fault in 0..3 {
        let mut bad = saved.clone();
        let (mut key, mut reading) = bad
            .comms
            .as_mut()
            .unwrap()
            .pending_ai_responses
            .iter()
            .find(|(key, _)| key.planned_request)
            .map(|(key, row)| (key.clone(), *row))
            .unwrap();
        bad.comms
            .as_mut()
            .unwrap()
            .pending_ai_responses
            .retain(|(existing, _)| existing != &key);
        match fault {
            0 => key.message_id = "foreign/unreleased/request".into(),
            1 => reading.request_reading.as_mut().unwrap().last_tick = bad.tick + 1,
            _ => key.planned_request = false,
        }
        bad.comms
            .as_mut()
            .unwrap()
            .pending_ai_responses
            .push((key, reading));
        assert!(
            !restore(resumed.world_mut(), &bad).is_complete(),
            "forged planned reading {fault}"
        );
        assert_eq!(
            capture(resumed.world()),
            before,
            "preflight writes no live state"
        );
        assert_eq!(world_digest(resumed.world()), before_digest);
    }
    let report = restore(resumed.world_mut(), &saved);
    assert!(report.is_complete(), "{report:?}");
    let mut admitted_ai_request = false;
    let mut paid_source = false;
    let mut admitted_report = false;
    for step in 0..360 {
        same(&live, &resumed, step);
        let mut actors = live.world_mut().query::<(&EntityUuid, &AdmittedCommands)>();
        for (uuid, commands) in actors.iter(live.world()) {
            for command in &commands.0 {
                let actual_ai =
                    command.response_token.as_deref() == Some(format!("ai:{}", uuid.0).as_str());
                admitted_ai_request |= uuid.0 == source_uuid
                    && actual_ai
                    && matches!(&command.payload,
                    Control::SendCommsRequest { planned_request:Some(context), .. } if context==&instance);
                admitted_report |= uuid.0 != source_uuid
                    && actual_ai
                    && matches!(
                        &command.payload,
                        Control::RespondToMessage {
                            response_index: 0,
                            ..
                        }
                    );
            }
        }
        paid_source |= live
            .world()
            .get::<ShipPowerSystem>(source)
            .unwrap()
            .0
            .emitted_work(&SystemId("comms".into()))
            >= 1.0;
        live.update();
        resumed.update();
    }
    same(&live, &resumed, 360);
    assert!(
        admitted_ai_request,
        "ordinary existing Comms AI must submit the actual planned command through admission"
    );
    assert!(paid_source, "the real source radio must buy work");
    assert!(
        admitted_report,
        "the actual receiver's ordinary Comms AI must make its local report"
    );
    let current = capture(live.world());
    let row = &current.scenario.as_ref().unwrap().situations.instances[&instance];
    let accepted = row.accepted.as_ref().unwrap();
    let ActivityExecutionIdentity::Communication {
        origin_id,
        recipient,
        emitted_tick,
    } = &accepted.identity
    else {
        panic!("communication must retain actual radio identity, never a fabricated physical JobId")
    };
    let messages = inbox(&live);
    let sent = messages
        .iter()
        .filter(|message| {
            message.sender_uuid == source_uuid
                && message
                    .recipient_ship
                    .as_ref()
                    .is_some_and(|id| id.0 == source_uuid)
                && message.transmission.as_ref().is_some_and(|wire| {
                    wire.sustained && wire.origin_id == *origin_id && wire.received_tick.is_none()
                })
        })
        .collect::<Vec<_>>();
    assert_eq!(
        sent.len(),
        1,
        "one canonical paid emission, no retry remint"
    );
    let heard = messages
        .iter()
        .filter(|message| {
            message.sender_uuid == source_uuid
                && message
                    .recipient_ship
                    .as_ref()
                    .is_some_and(|id| id.0 == *recipient)
                && message.transmission.as_ref().is_some_and(|wire| {
                    wire.sustained && wire.origin_id == *origin_id && wire.received_tick.is_some()
                })
        })
        .collect::<Vec<_>>();
    assert_eq!(
        heard.len(),
        1,
        "retained delivery never duplicates an actual received request"
    );
    let read = row.achieved.as_ref().unwrap();
    let terminal = row.settled.as_ref().unwrap();
    assert!(
        matches!(&read.outcome,ActivityOutcome::CommunicationRead {reference,read_tick,..}
        if reference.origin_id==*origin_id && reference.emitted_tick==*emitted_tick && *read_tick>=reference.heard_tick)
    );
    let ActivityOutcome::CommunicationReturned {
        outcome,
        receipt_origin_id,
    } = &terminal.outcome
    else {
        panic!("actual heard returned report required")
    };
    let returned=messages.iter().find(|message| message.sender_uuid==*recipient && message.recipient_ship.as_ref().is_some_and(|id|id.0==source_uuid)
        && message.transmission.as_ref().is_some_and(|wire| wire.origin_id==*receipt_origin_id && wire.received_tick==Some(terminal.tick)
            && matches!(&wire.content,CommsRadioContent::Outcome{outcome:actual} if actual==outcome))).unwrap();
    assert_eq!(
        returned.transmission.as_ref().unwrap().received_tick,
        Some(read.tick)
    );
    let world = live.world().resource::<WorldContentRuntime>();
    assert_eq!(
        world.flags.counter("actual_status_report"),
        1,
        "only the authored post-report Scenario call grants this factual credit"
    );
    assert!(
        world
            .situations
            .instances
            .values()
            .any(
                |row| serde_json::to_value(&row.request).unwrap()["kind"] == "repair"
                    && row.armed_tick >= terminal.tick
            ),
        "the authored Reported HTN edge releases real downstream work after its heard evidence"
    );
    let before = capture(live.world());
    let digest = world_digest(live.world());
    for fault in 0..3 {
        let mut bad = before.clone();
        let row = bad
            .scenario
            .as_mut()
            .unwrap()
            .situations
            .instances
            .get_mut(&instance)
            .unwrap();
        match fault {
            0 => {
                if let ActivityExecutionIdentity::Communication { recipient, .. } =
                    &mut row.accepted.as_mut().unwrap().identity
                {
                    *recipient = source_uuid.clone();
                }
            }
            1 => row.accepted.as_mut().unwrap().owner = SystemId("helm-thrust".into()),
            _ => {
                bad.comms
                    .as_mut()
                    .unwrap()
                    .inbox
                    .iter_mut()
                    .find(|message| {
                        message
                            .transmission
                            .as_ref()
                            .is_some_and(|wire| wire.sustained)
                    })
                    .unwrap()
                    .transmission
                    .as_mut()
                    .unwrap()
                    .origin_id = "forged/unaccepted/origin".into()
            }
        }
        assert!(!restore(live.world_mut(), &bad).is_complete());
        assert_eq!(capture(live.world()), before);
        assert_eq!(world_digest(live.world()), digest);
    }
}

#[test]
fn current_human_operator_sends_the_same_released_choice_and_stale_context_cannot_accept() {
    use phoenix_model::comms_channel::CommsAudience;
    use project_phoenix::{
        core::messages::ClientMessage,
        lobby::{server::InboundMessage, Sessions},
        ship_plugin::{HumanSeekingHosts, ShipConfigComponent, ShipSystemControlSources},
    };
    const TOKEN: &str = "planned-comms-officer";
    let mut app = boot();
    let entity = app
        .world_mut()
        .query_filtered::<Entity, With<LocalShip>>()
        .single(app.world())
        .unwrap();
    let graph = &app.world().get::<ShipConfigComponent>(entity).unwrap().0;
    let owner = SystemId("comms".into());
    let station = graph
        .system_home_station(graph.system(&owner).unwrap())
        .unwrap()
        .clone();
    let rating = graph
        .station(&station)
        .unwrap()
        .ratings
        .first()
        .unwrap()
        .name
        .clone();
    let send = |app: &mut App, msg| {
        app.world_mut().write_message(InboundMessage {
            token: TOKEN.into(),
            msg,
        });
    };
    for msg in [
        ClientMessage::Identify {
            token: TOKEN.into(),
            name: TOKEN.into(),
        },
        ClientMessage::SelectStation {
            station: station.0.clone(),
        },
        ClientMessage::SetReady { ready: true },
        ClientMessage::SetStationRating {
            rating_name: rating,
        },
    ] {
        send(&mut app, msg);
        app.update();
    }
    let mut choice = None;
    for _ in 0..120 {
        let graph = &app.world().get::<ShipConfigComponent>(entity).unwrap().0;
        let boards = &app
            .world()
            .get::<project_phoenix::server_app::ShipSystemBlackboards>(entity)
            .unwrap()
            .0;
        let known = graph.observations_for_system(&owner, boards);
        if let Some(phoenix_model::messages::SystemBlackboard::Comms(board)) = known.get(&owner) {
            if let Some(row) = board.planned_requests.first() {
                let payload = Control::SendCommsRequest {
                    channel: row.channel.clone(),
                    audience: CommsAudience::Contact {
                        target_uuid: row.recipient_uuid.clone(),
                    },
                    request: row.request,
                    planned_request: Some(row.id.clone()),
                };
                if project_phoenix::command_admission::policy::is_command_authorized(
                    TOKEN,
                    &owner,
                    &payload,
                    app.world().get::<ShipSystemControlSources>(entity).unwrap(),
                    app.world().resource::<Sessions>(),
                    graph,
                    app.world().get::<HumanSeekingHosts>(entity),
                ) {
                    choice = Some((row.clone(), payload));
                    break;
                }
            }
        }
        app.update();
    }
    let (row, payload) = choice.expect(
        "ordinary current Comms holder sees and can operate the same released known-contact row",
    );
    assert!(app
        .world()
        .resource::<WorldContentRuntime>()
        .situations
        .instances[&row.id]
        .accepted
        .is_none());
    let mut forged = payload.clone();
    if let Control::SendCommsRequest {
        planned_request, ..
    } = &mut forged
    {
        *planned_request = Some("stale/foreign/request".into());
    }
    send(
        &mut app,
        ClientMessage::ControlSystem {
            target: owner.clone(),
            payload: forged,
        },
    );
    for _ in 0..4 {
        app.update();
    }
    assert!(
        app.world()
            .resource::<WorldContentRuntime>()
            .situations
            .instances[&row.id]
            .accepted
            .is_none(),
        "a current operator still cannot bind a stale or foreign mission context"
    );
    assert!(
        inbox(&app).iter().all(|message| !message
            .transmission
            .as_ref()
            .is_some_and(|wire| wire.sustained)),
        "a rejected planned context cannot mint an ordinary-looking retained emission"
    );
    send(
        &mut app,
        ClientMessage::ControlSystem {
            target: owner.clone(),
            payload,
        },
    );
    let mut admitted = false;
    let mut paid = false;
    for _ in 0..120 {
        app.update();
        admitted |= app
            .world()
            .get::<AdmittedCommands>(entity)
            .unwrap()
            .0
            .iter()
            .any(|command| {
                command.response_token.as_deref() == Some(TOKEN)
                    && matches!(&command.payload,
                Control::SendCommsRequest{planned_request:Some(id),..} if id==&row.id)
            });
        paid |= app
            .world()
            .get::<ShipPowerSystem>(entity)
            .unwrap()
            .0
            .emitted_work(&owner)
            >= 1.0;
        if app
            .world()
            .resource::<WorldContentRuntime>()
            .situations
            .instances[&row.id]
            .accepted
            .is_some()
        {
            break;
        }
    }
    assert!(
        admitted && paid,
        "real human command admission and paid operation must precede acceptance"
    );
    let runtime = app.world().resource::<WorldContentRuntime>();
    let accepted = runtime.situations.instances[&row.id]
        .accepted
        .as_ref()
        .unwrap();
    assert!(
        matches!(&accepted.identity,ActivityExecutionIdentity::Communication{recipient,..} if recipient==&row.recipient_uuid)
    );
    assert!(
        !app.world()
            .resource::<CommsRuntime>()
            .pending_ai_responses
            .keys()
            .any(|key| key.planned_request && key.message_id == row.id),
        "same successful human emission retires any existing partial AI reading"
    );
}
