//! Paid focused enquiry through real joined crew and an equipped observing ship.
#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]
use bevy::prelude::*;
use phoenix_model::{
    focused_enquiry::{FocusedAim, ScanQuantityValue},
    sensing::PassiveSignature,
};
use project_phoenix::{
    core::messages::{ClientMessage, StationId, SystemControlPayload as Control, SystemId},
    entities::spawner::EntityUuid,
    headless::{build_headless_app, HeadlessArgs},
    infrastructure::InfrastructureCondition,
    lobby::{server::InboundMessage, Sessions},
    science::{passive::PassiveState, server::ShipScanRecord},
    server_app::{LocalShip, ShipSystemBlackboards},
    ship::power::ShipPowerSystem,
    ship_plugin::{HumanSeekingHosts, ShipConfigComponent, ShipSystemControlSources},
    sim_digest::world_digest,
    sim_tick::sim_tick_period,
    snapshot::{capture, ready_to_restore, restore, PhoenixSnapshot},
    world::{config::WorldConfig, server::WorldContentRuntime},
};
const HULL: &str = "tests/fixtures/sensing-enquiry-cruiser.toml";
const WORLD: &str = "tests/fixtures/sensing-enquiry-world.toml";
fn id(value: &str) -> SystemId {
    SystemId(value.into())
}
fn local(app: &mut App) -> Entity {
    app.world_mut()
        .query_filtered::<Entity, With<LocalShip>>()
        .single(app.world())
        .unwrap()
}
fn send(app: &mut App, token: &str, msg: ClientMessage) {
    app.world_mut().write_message(InboundMessage {
        token: token.into(),
        msg,
    });
}
fn steps(app: &mut App, count: usize) {
    for _ in 0..count {
        app.update();
    }
}
fn claim_messages(app: &mut App, station: &str, token: &str) -> Vec<ClientMessage> {
    let entity = local(app);
    // The authored ladder is most detailed first. Selecting complexity uses
    // the public command, including on fleet peers with a different default.
    let rating = app
        .world()
        .get::<ShipConfigComponent>(entity)
        .unwrap()
        .0
        .stations
        .iter()
        .find(|entry| entry.id.0 == station)
        .unwrap()
        .ratings
        .first()
        .unwrap()
        .name
        .clone();
    vec![
        ClientMessage::Identify {
            token: token.into(),
            name: token.into(),
        },
        ClientMessage::SelectStation {
            station: station.into(),
        },
        ClientMessage::SetReady { ready: true },
        ClientMessage::SetStationRating {
            rating_name: rating,
        },
    ]
}
fn claim(app: &mut App, station: &str, token: &str) {
    for message in claim_messages(app, station, token) {
        send(app, token, message);
        steps(app, 4);
    }
}
fn authorised(
    app: &mut App,
    token: &str,
    station: &str,
    target: &SystemId,
    payload: &Control,
) -> bool {
    let entity = local(app);
    let sessions = app.world().resource::<Sessions>();
    let Some(player) = sessions
        .0
        .players()
        .iter()
        .find(|player| player.token == token)
    else {
        return false;
    };
    player.connected
        && player.ready
        && player.station.as_ref() == Some(&StationId(station.into()))
        && sessions.0.holder_for_station(&StationId(station.into())) == Some(token)
        && project_phoenix::command_admission::policy::is_command_authorized(
            token,
            target,
            payload,
            app.world().get::<ShipSystemControlSources>(entity).unwrap(),
            sessions,
            &app.world().get::<ShipConfigComponent>(entity).unwrap().0,
            app.world().get::<HumanSeekingHosts>(entity),
        )
}
fn control(app: &mut App, token: &str, station: &str, target: &str, payload: Control) {
    for _ in 0..120 {
        if authorised(app, token, station, &id(target), &payload) {
            break;
        }
        app.update();
    }
    assert!(
        authorised(app, token, station, &id(target), &payload),
        "actual {station} holder must own {target}"
    );
    send(
        app,
        token,
        ClientMessage::ControlSystem {
            target: id(target),
            payload,
        },
    );
    steps(app, 4);
}

fn prepared() -> (App, Entity) {
    let mut app = build_headless_app(&HeadlessArgs {
        world_path: WORLD.into(),
        ship_path: HULL.into(),
        deterministic: true,
        seed: Some(156_214),
        ..Default::default()
    })
    .unwrap();
    let period = sim_tick_period(app.world().resource::<WorldConfig>().global.sim_tick_hz);
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(period));
    app.finish();
    app.cleanup();
    steps(&mut app, 10);
    claim(&mut app, "science", "enquiry-scientist");
    claim(&mut app, "engineering", "enquiry-engineer");
    control(
        &mut app,
        "enquiry-engineer",
        "engineering",
        "power-reactor",
        Control::SetEnergyGeneration { enabled: false },
    );
    let entity = local(&mut app);
    assert!(
        !app.world()
            .get::<ShipPowerSystem>(entity)
            .unwrap()
            .0
            .network()
            .generation_enabled
    );
    (app, entity)
}
fn alias(app: &App, entity: Entity) -> Option<String> {
    let config = &app.world().get::<ShipConfigComponent>(entity)?.0;
    let boards = app.world().get::<ShipSystemBlackboards>(entity)?;
    let owner = id("sensors");
    let view = config.observations_for_system(&owner, &boards.0);
    let mut keys: Vec<_> = boards.0.keys().collect();
    keys.sort();
    keys.into_iter()
        .filter_map(|key| view.get(key))
        .filter_map(project_phoenix::science::contact_routing::observations)
        .flatten()
        .find(|track| track.selectable && !track.predicted)
        .map(|track| track.contact_id.clone())
}
fn matching_mission_request(app: &App, entity: Entity) -> bool {
    let config = &app.world().get::<ShipConfigComponent>(entity).unwrap().0;
    let boards = app.world().get::<ShipSystemBlackboards>(entity).unwrap();
    let owner = id("sensors");
    let view = config.observations_for_system(&owner, &boards.0);
    let work = records(app, entity)
        .focused
        .as_ref()
        .filter(|work| work.intrusive && work.aim == FocusedAim::Resources);
    let known_target = boards
        .0
        .keys()
        .filter_map(|key| view.get(key))
        .filter_map(project_phoenix::science::contact_routing::observations)
        .flatten()
        .find(|track| {
            work.is_some_and(|work| work.presentation_id == track.contact_id)
                && track
                    .identity
                    .as_ref()
                    .is_some_and(|identity| identity.name == "survey-cache")
        });
    known_target.is_some()
        && boards
            .0
            .keys()
            .filter_map(|key| view.get(key))
            .any(|board| {
                let project_phoenix::core::messages::SystemBlackboard::Viewscreen(board) = board
                else {
                    return false;
                };
                board.scored_objectives.iter().any(|objective| {
                    matches!(&objective.directive,
            project_phoenix::core::messages::AiDirective::Scan{target} if target=="survey-cache")
                })
            })
}
fn records(app: &App, entity: Entity) -> &project_phoenix::science::FocusedScanRecord {
    &app.world().get::<ShipScanRecord>(entity).unwrap().0[&id("sensors")]
}
fn subject(app: &mut App) -> Entity {
    let uuid = app.world().resource::<WorldContentRuntime>().name_to_uuid["survey-cache"].clone();
    app.world_mut()
        .query::<(Entity, &EntityUuid)>()
        .iter(app.world())
        .find(|(_, id)| id.0 == uuid)
        .unwrap()
        .0
}
fn heard_scan(app: &App, receiver: Entity) -> bool {
    app.world()
        .get::<PassiveState>(receiver)
        .unwrap()
        .0
        .values()
        .any(|receiver| {
            receiver.enabled
                && receiver
                    .signals
                    .iter()
                    .any(|signal| signal.signature == PassiveSignature::ActiveScan)
        })
}
fn release(app: &mut App) {
    send(app, "enquiry-scientist", ClientMessage::ReleaseStation);
    steps(app, 4);
}

#[test]
fn human_intrusive_question_resumes_under_ordinary_ai_and_its_real_receiver_drives_context() {
    let (mut live, observer) = prepared();
    let receiver = subject(&mut live);
    let initial_stock = live
        .world()
        .get::<InfrastructureCondition>(receiver)
        .unwrap()
        .0
        .capacity("machinery_parts")
        .unwrap();
    for _ in 0..180 {
        if alias(&live, observer).is_some() {
            break;
        }
        live.update();
    }
    let contact = alias(&live, observer).expect("actual permitted current measurement");
    control(
        &mut live,
        "enquiry-scientist",
        "science",
        "sensors",
        Control::SetScienceTarget {
            uuid: contact.clone(),
        },
    );
    control(
        &mut live,
        "enquiry-scientist",
        "science",
        "sensors",
        Control::ScanTarget {
            uuid: contact,
            aim: FocusedAim::Resources,
            intrusive: true,
        },
    );
    let mut detected = heard_scan(&live, receiver);
    let mut same_request_during_work = false;
    let mut ordinary_ai_asked_after_completion = false;
    release(&mut live);
    let work = records(&live, observer)
        .focused
        .as_ref()
        .expect("same paid work survives handoff");
    assert!(
        work.progress > 0.0 && work.progress < work.work_secs && work.paid_started_tick.is_some()
    );
    assert!(work.intrusive && work.aim == FocusedAim::Resources);
    assert_eq!(
        live.world()
            .get::<ShipSystemControlSources>(observer)
            .unwrap()
            .0
            .source_for(&id("sensors")),
        project_phoenix::ship::control_source::ControlSource::Ai
    );
    let saved: PhoenixSnapshot =
        serde_json::from_slice(&serde_json::to_vec(&capture(live.world())).unwrap()).unwrap();
    let (mut resumed, restored) = prepared();
    release(&mut resumed);
    assert!(ready_to_restore(resumed.world(), &saved));
    // The complete candidate must be validated before any other owner is written.
    let before_rejected = capture(resumed.world());
    for mode in 0..4 {
        let mut forged = saved.clone();
        let work = forged
            .entities
            .iter_mut()
            .find_map(|row| {
                row.scan
                    .as_mut()?
                    .instruments
                    .get_mut(&id("sensors"))?
                    .focused
                    .as_mut()
            })
            .expect("saved active question");
        match mode {
            0 => work.paid_started_tick = None,
            1 => work.work_secs += 1.0,
            2 => work.aim = FocusedAim::Anomaly,
            _ => work.progress = work.work_secs,
        }
        let rejected = restore(resumed.world_mut(), &forged);
        assert!(!rejected.is_complete(), "forged active question {mode}");
        assert_eq!(
            capture(resumed.world()),
            before_rejected,
            "no-write refusal {mode}"
        );
    }
    let report = restore(resumed.world_mut(), &saved);
    assert!(report.is_complete(), "{:?}", report.gaps);
    for step in 0..360 {
        assert_eq!(
            world_digest(live.world()),
            world_digest(resumed.world()),
            "digest {step}"
        );
        assert_eq!(
            serde_json::to_value(capture(live.world())).unwrap(),
            serde_json::to_value(capture(resumed.world())).unwrap(),
            "complete continuation {step}"
        );
        let completed = records(&live, observer)
            .history
            .iter()
            .any(|reading| reading.intrusive && reading.aim == FocusedAim::Resources);
        ordinary_ai_asked_after_completion |= completed
            && live
                .world()
                .get::<project_phoenix::core::messages::AdmittedCommands>(observer)
                .unwrap()
                .for_target("sensors")
                .any(|command| {
                    matches!(
                        &command.payload,
                        Control::ScanTarget {
                            aim: FocusedAim::General,
                            intrusive: false,
                            ..
                        }
                    )
                });
        detected |= heard_scan(&live, receiver);
        same_request_during_work |= matching_mission_request(&live, observer);
        if let Some(work) = records(&live, observer).focused.as_ref().filter(|_| {
            !records(&live, observer)
                .history
                .iter()
                .any(|reading| reading.intrusive && reading.aim == FocusedAim::Resources)
        }) {
            assert!(work.intrusive&&work.aim==FocusedAim::Resources,
                "same mission request must preserve the human question and mode during ordinary AI handoff");
        }
        live.update();
        resumed.update();
    }
    assert!(ordinary_ai_asked_after_completion,
        "ordinary bound AI still admits the pending default request once the human's work completes");
    assert!(
        same_request_during_work,
        "an actually observed matching mission request must exercise the bound AI handoff"
    );
    assert!(
        detected,
        "actual paid ActiveScan must be heard by the capable receiver"
    );
    let reading = records(&live, observer)
        .history
        .iter()
        .find(|reading| reading.intrusive && reading.aim == FocusedAim::Resources)
        .expect("ordinary Backfill completes the human's existing question");
    assert!(reading.condition_fraction.is_none() && reading.mass.is_none());
    assert_eq!(reading.quantities.len(), 1);
    assert_eq!(
        reading.quantities[0].value,
        ScanQuantityValue::Count(initial_stock)
    );
    assert_eq!(
        records(&resumed, restored).save_state(),
        records(&live, observer).save_state()
    );
    assert_eq!(
        live.world()
            .resource::<WorldContentRuntime>()
            .flags
            .counter("intrusive_scan_detected"),
        1
    );
    assert_eq!(
        resumed
            .world()
            .resource::<WorldContentRuntime>()
            .flags
            .counter("intrusive_scan_detected"),
        1
    );

    let complete = capture(live.world());
    for mode in 0..4 {
        let mut forged = complete.clone();
        let record = forged
            .entities
            .iter_mut()
            .find(|row| row.uuid == live.world().get::<EntityUuid>(observer).unwrap().0)
            .unwrap()
            .scan
            .as_mut()
            .unwrap()
            .instruments
            .get_mut(&id("sensors"))
            .unwrap();
        let reading = record
            .history
            .iter_mut()
            .find(|reading| reading.intrusive)
            .unwrap();
        match mode {
            0 => reading.quantities[0].value = ScanQuantityValue::Count(-1),
            1 => reading.condition_fraction = Some(1.0),
            2 => reading.quantities.push(reading.quantities[0].clone()),
            _ => reading.aim = FocusedAim::Environment,
        }
        if record.last.as_ref().is_some_and(|last| last.intrusive) {
            record.last = Some(reading.clone());
        }
        let rejected = restore(live.world_mut(), &forged);
        assert!(!rejected.is_complete(), "forged typed answer {mode}");
        assert_eq!(
            capture(live.world()),
            complete,
            "typed no-write refusal {mode}"
        );
    }
}
