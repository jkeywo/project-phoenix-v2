//! Real paid radio and same-inbox request owner boundaries.
use super::*;
use crate::console::comms::server::{operate_comms_response_ai, publish_comms_blackboard};
use phoenix_model::comms_request::*;

fn request_fixture() -> (App, Entity, Entity, String) {
    let (mut app, source, receiver) = fixture(10.0);
    crate::ai::host::register_ai_host_env(&mut app);
    app.world_mut()
        .get_mut::<AdmittedCommands>(source)
        .unwrap()
        .0[0]
        .payload = SystemControlPayload::SendCommsRequest {
        channel: "public".into(),
        audience: CommsAudience::All,
        request: GenericCommsRequest::ReportStatus,
        planned_request: None,
    };
    app.world_mut().run_system_once(tick_radio).unwrap();
    app.world_mut()
        .get_mut::<AdmittedCommands>(source)
        .unwrap()
        .0
        .clear();
    app.world_mut()
        .run_system_once(publish_comms_blackboard)
        .unwrap();
    let id = messages(&app, "receiver")
        .iter()
        .find(|message| crate::comms::requests::is_request(message))
        .unwrap()
        .id
        .clone();
    (app, source, receiver, id)
}

fn decide(app: &mut App, receiver: Entity, id: &str, choice: usize) -> bool {
    let entity = app.world().entity(receiver);
    let config = entity.get::<ShipConfigComponent>().unwrap().clone();
    let boards = entity
        .get::<crate::server_app::ShipSystemBlackboards>()
        .unwrap()
        .clone();
    let sources = entity.get::<ShipSystemControlSources>().unwrap().clone();
    let damage = entity.get::<ShipDamage>().cloned();
    let power = entity.get::<ShipPowerSystem>().cloned();
    let radio = entity.get::<RadioProfile>().cloned();
    let now = app.world().resource::<crate::sim_tick::SimTick>().0;
    crate::comms::requests::respond(
        &mut app.world_mut().resource_mut::<CommsInboxRes>(),
        "receiver",
        id,
        choice,
        now,
        &config,
        &boards,
        &sources,
        damage.as_ref(),
        power.as_ref(),
        radio.as_ref(),
    )
}

fn energy(app: &App, entity: Entity) -> f32 {
    app.world()
        .get::<ShipPowerSystem>(entity)
        .unwrap()
        .0
        .network()
        .consumers[&SystemId("comms".into())]
        .energy
}

#[test]
fn retained_sustained_returns_never_expose_receiver_viability_through_reporter_history_or_cost() {
    let mut histories = Vec::new();
    for initially_far in [false, true] {
        let (mut app, source, receiver, id) = request_fixture();
        let initial = energy(&app, receiver);
        if initially_far {
            app.world_mut().get_mut::<ShipPhysics>(source).unwrap().x = 10000.0;
        }
        app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 = 11;
        assert!(decide(&mut app, receiver, &id, 0));
        app.world_mut().run_system_once(tick_radio).unwrap();
        assert!(
            energy(&app, receiver) < initial,
            "new local outcome pays actual discrete work"
        );
        let mut history = Vec::new();
        for step in 12..=4200 {
            app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 = step;
            if step == 30 {
                app.world_mut().get_mut::<ShipPhysics>(source).unwrap().x = 10.0;
            }
            app.world_mut().run_system_once(tick_radio).unwrap();
            app.world_mut()
                .run_system_once(publish_comms_blackboard)
                .unwrap();
            let owned = messages(&app, "receiver")
                .into_iter()
                .map(|mut row| {
                    row.request_progress.as_mut().unwrap().last_returned = None;
                    row
                })
                .collect::<Vec<_>>();
            history.push((owned, energy(&app, receiver)));
            if step == 29 {
                let completed=messages(&app,"source").iter().filter(|message| message.transmission.as_ref().is_some_and(|wire|
                    wire.received_tick.is_some() && matches!(&wire.content,CommsRadioContent::Outcome {outcome} if outcome.facts.stage()==CommsRequestStage::Completed))).count();
                assert_eq!(completed, usize::from(!initially_far));
            }
        }
        assert_eq!(
            energy(&app, receiver),
            0.0,
            "finite operation eventually exhausts without free refills"
        );
        let outcomes=messages(&app,"source").into_iter().filter(|message|message.transmission.as_ref().is_some_and(|wire|
            wire.received_tick.is_some() && matches!(&wire.content,CommsRadioContent::Outcome {outcome} if outcome.facts.report.is_some()))).collect::<Vec<_>>();
        assert_eq!(
            outcomes.len(),
            1,
            "idempotent return, no polling or duplicate-stage mint"
        );
        let wire = outcomes[0].transmission.as_ref().unwrap();
        assert_eq!(wire.emitted_tick, 11);
        assert_eq!(
            wire.received_tick,
            Some(if initially_far { 30 } else { 11 })
        );
        assert!(
            matches!(&wire.content,CommsRadioContent::Outcome {outcome} if outcome.facts.read_tick==Some(11)
            && outcome.facts.report.as_ref().unwrap().observed_tick==11)
        );
        phoenix_model::comms_channel::validate_transmissions(
            &app.world().resource::<CommsInboxRes>().0.messages(),
            4200,
        )
        .unwrap();
        histories.push(history);
    }
    assert_eq!(
        histories[0], histories[1],
        "reporter's visible copies and all local reserves ignore reverse-link success"
    );
}

#[test]
fn clear_retains_same_console_lineage_and_remote_departure_cannot_create_a_decision() {
    let (mut app, _, receiver, id) = request_fixture();
    let before = messages(&app, "receiver");
    assert_eq!(
        app.world_mut()
            .resource_mut::<CommsInboxRes>()
            .0
            .clear_for_ship(Some("receiver")),
        1
    );
    let current = messages(&app, "receiver");
    assert_eq!(
        current[0], before[0],
        "unread request cannot be silently cleared"
    );
    let now = app.world().resource::<crate::sim_tick::SimTick>().0;
    assert!(crate::comms::requests::mark_read(
        &mut app.world_mut().resource_mut::<CommsInboxRes>(),
        "receiver",
        &id,
        now
    ));
    assert_eq!(
        app.world_mut()
            .resource_mut::<CommsInboxRes>()
            .0
            .clear_for_ship(Some("receiver")),
        1
    );
    let retained = messages(&app, "receiver");
    assert_eq!(retained.len(), 2);
    assert!(retained[0].request_progress.as_ref().unwrap().dismissed);
    assert!(app
        .world_mut()
        .resource_mut::<CommsInboxRes>()
        .0
        .orphan_sender("source")
        .is_empty());
    assert_eq!(
        messages(&app, "receiver"),
        retained,
        "no despawn-derived refusal/orphan oracle"
    );
    app.world_mut()
        .run_system_once(publish_comms_blackboard)
        .unwrap();
    let boards = app
        .world()
        .get::<crate::server_app::ShipSystemBlackboards>(receiver)
        .unwrap();
    let SystemBlackboard::Comms(board) = &boards.0[&SystemId("comms".into())] else {
        panic!("Comms publication")
    };
    let mut expected = retained;
    for message in &mut expected {
        message.request_progress.as_mut().unwrap().last_returned = None;
    }
    assert_eq!(
        board.messages, expected,
        "same Console can inspect cleared history"
    );
    assert!(board.messages[0]
        .request_progress
        .as_ref()
        .unwrap()
        .last_returned
        .is_none());
    assert!(
        decide(&mut app, receiver, &id, 1),
        "clearing cannot discard a local response opportunity"
    );
}

#[test]
fn existing_response_controller_reads_partial_work_pauses_for_human_then_emits_normal_admitted_report(
) {
    let (mut app, _, receiver, id) = request_fixture();
    let owner = SystemId("comms".into());
    app.world_mut()
        .get_mut::<ShipSystemControlSources>(receiver)
        .unwrap()
        .0
        .set(
            owner.clone(),
            crate::ship::control_source::ControlSource::Ai,
        );
    app.world_mut()
        .run_system_once(operate_comms_response_ai)
        .unwrap();
    let key = crate::comms::server::PendingAiResponseKey::new("receiver", &id);
    let initial = app.world().resource::<CommsRuntime>().pending_ai_responses[&key];
    assert_eq!(initial.request_reading.unwrap().remaining_ticks, 30);
    for tick in 11..=20 {
        app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 = tick;
        app.world_mut()
            .run_system_once(crate::comms::requests::advance_reading)
            .unwrap();
    }
    assert_eq!(
        app.world().resource::<CommsRuntime>().pending_ai_responses[&key]
            .request_reading
            .unwrap()
            .remaining_ticks,
        20
    );
    app.world_mut()
        .get_mut::<ShipSystemControlSources>(receiver)
        .unwrap()
        .0
        .set(
            owner.clone(),
            crate::ship::control_source::ControlSource::Human,
        );
    for tick in 21..=40 {
        app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 = tick;
        app.world_mut()
            .run_system_once(crate::comms::requests::advance_reading)
            .unwrap();
        app.world_mut()
            .run_system_once(operate_comms_response_ai)
            .unwrap();
    }
    assert_eq!(
        app.world().resource::<CommsRuntime>().pending_ai_responses[&key]
            .request_reading
            .unwrap()
            .remaining_ticks,
        20
    );
    assert!(!messages(&app, "receiver")[0].is_read);
    app.world_mut()
        .get_mut::<ShipSystemControlSources>(receiver)
        .unwrap()
        .0
        .set(owner, crate::ship::control_source::ControlSource::Ai);
    for tick in 41..=60 {
        app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 = tick;
        app.world_mut()
            .run_system_once(crate::comms::requests::advance_reading)
            .unwrap();
    }
    app.world_mut()
        .run_system_once(publish_comms_blackboard)
        .unwrap();
    app.world_mut()
        .run_system_once(operate_comms_response_ai)
        .unwrap();
    assert!(!app
        .world()
        .resource::<CommsRuntime>()
        .pending_ai_responses
        .contains_key(&key));
    let read = messages(&app, "receiver")[0].clone();
    let known = crate::comms::requests::response_facts(&read, true, None);
    assert_eq!(
        known.get(crate::entities::ai_flag_hosts::REQUEST_REPORT_STATUS.name()),
        Some(1.0)
    );
    assert_eq!(
        known.get(crate::entities::ai_flag_hosts::SENDER_IN_RANGE.name()),
        None,
        "ordinary authored request response cannot borrow reverse-link viability"
    );
    let commands = &app.world().get::<AdmittedCommands>(receiver).unwrap().0;
    assert_eq!(
        commands.len(),
        1,
        "one existing controller emits one response"
    );
    assert!(
        matches!(&commands[0].payload,SystemControlPayload::RespondToMessage {message_id,response_index:0} if message_id==&id)
    );
    assert_eq!(commands[0].response_token.as_deref(), Some("ai:receiver"));
    assert!(decide(&mut app, receiver, &id, 0));
    assert_eq!(
        messages(&app, "receiver")[0]
            .request_progress
            .as_ref()
            .unwrap()
            .facts
            .read_tick,
        Some(60)
    );
}

#[test]
fn pending_reading_candidate_rejects_completed_forged_owner_clock_and_duplicate_without_writes() {
    let (mut app, _, receiver, _) = request_fixture();
    app.world_mut()
        .register_component::<crate::entities::spawner::EntityShipArcHull>();
    app.world_mut()
        .register_component::<crate::console::repair::server::RepairRequestQueue>();
    app.world_mut()
        .register_component::<crate::ship_plugin::RepairHumanAlerted>();
    app.world_mut()
        .register_component::<crate::ai::server::ObjectiveCursors>();
    app.world_mut()
        .register_component::<crate::console::weapons::blaster::BlasterSystemResource>();
    app.world_mut()
        .register_component::<crate::server_app::AsteroidUuid>();
    app.init_resource::<crate::gm::actions::action::SimulationPaused>()
        .init_resource::<crate::core::report::MissionReport>()
        .init_resource::<crate::gm::actions::action::GmActionJournal>()
        .init_resource::<crate::gm::operations::puppet::StationPuppets>()
        .init_resource::<crate::gm::operations::puppet::PendingGmStationCommands>();
    app.world_mut()
        .get_mut::<ShipSystemControlSources>(receiver)
        .unwrap()
        .0
        .set(
            SystemId("comms".into()),
            crate::ship::control_source::ControlSource::Ai,
        );
    app.world_mut()
        .run_system_once(operate_comms_response_ai)
        .unwrap();
    let before = crate::snapshot::capture(app.world());
    assert_eq!(
        before.entities.len(),
        2,
        "actual same machinery rows captured"
    );
    assert_eq!(
        before.comms.as_ref().unwrap().pending_ai_responses.len(),
        1,
        "real controller armed reading"
    );
    let digest = crate::sim_digest::world_digest(app.world());
    for fault in 0..10 {
        let mut bad = before.clone();
        bad.tick += 5;
        let comms = bad.comms.as_mut().unwrap();
        match fault {
            0 => comms
                .pending_ai_responses
                .push(comms.pending_ai_responses[0].clone()),
            1 => comms.pending_ai_responses[0].0.ship_uuid = "source".into(),
            2 => {
                comms.pending_ai_responses[0]
                    .1
                    .request_reading
                    .as_mut()
                    .unwrap()
                    .remaining_ticks = 0
            }
            3 => {
                comms.pending_ai_responses[0]
                    .1
                    .request_reading
                    .as_mut()
                    .unwrap()
                    .last_tick = bad.tick + 1
            }
            4 => {
                comms.pending_ai_responses[0]
                    .1
                    .request_reading
                    .as_mut()
                    .unwrap()
                    .required_ticks += 1
            }
            5 => comms.pending_ai_responses[0].1.due_tick += 1,
            6 => comms.pending_ai_responses[0].1.response_fingerprint = 1,
            8 | 9 => {
                let row = bad
                    .entities
                    .iter_mut()
                    .find(|row| row.uuid == "receiver")
                    .unwrap();
                row.spawn = Some(crate::world::spawn_origin::SpawnOrigin {
                    template_path: if fault == 8 {
                        "assets/entities/ship_civilian_hauler.toml"
                    } else {
                        "missing-request-owner.toml"
                    }
                    .into(),
                    name: "receiver".into(),
                    ..Default::default()
                });
            }
            _ => {
                let reading = comms.pending_ai_responses[0]
                    .1
                    .request_reading
                    .as_mut()
                    .unwrap();
                reading.remaining_ticks -= 10; // impossible work before actual delivery/arm
                comms.pending_ai_responses[0].1.due_tick = reading.due_tick().unwrap();
            }
        }
        let report = crate::snapshot::restore(app.world_mut(), &bad);
        assert!(!report.is_complete(), "fault{fault}: {report:?}");
        assert_eq!(
            crate::snapshot::capture(app.world()),
            before,
            "fault{fault}"
        );
        assert_eq!(
            crate::sim_digest::world_digest(app.world()),
            digest,
            "fault{fault}"
        );
    }
}

#[test]
fn request_facts_withhold_unbound_alert_survival_and_remote_range_instead_of_seeding_false_values()
{
    let (mut app, _, _, id) = request_fixture();
    let message = inbox_request(&app, &id);
    let facts = crate::comms::requests::response_facts(&message, true, Some(4.0));
    assert_eq!(facts.get("response_count"), Some(2.0));
    assert_eq!(facts.get("comms_available"), Some(1.0));
    assert_eq!(facts.get("request_report_status"), Some(1.0));
    let mut plain = message.clone();
    plain.transmission.as_mut().unwrap().content = CommsRadioContent::Text;
    assert_eq!(
        crate::comms::requests::response_facts(&plain, true, Some(4.0))
            .get("request_report_status"),
        None
    );
    for name in ["red_alert", "is_orphaned", "sender_in_range"] {
        assert_eq!(facts.get(name), None, "{name}");
    }
    app.world_mut().resource_mut::<CommsRuntime>().range_active = true;
    assert_eq!(
        crate::comms::requests::response_facts(&message, true, Some(4.0)),
        facts
    );
}
fn inbox_request(app: &App, id: &str) -> CommsMessage {
    app.world()
        .resource::<CommsInboxRes>()
        .0
        .iter()
        .find(|message| message.id == id)
        .unwrap()
        .clone()
}

#[test]
fn legacy_direct_wire_and_explicit_shared_display_redact_internal_returns_and_keep_local_answers() {
    let (mut app, _, receiver, id) = request_fixture();
    assert!(
        inbox_request(&app, &id)
            .request_progress
            .unwrap()
            .last_returned
            .is_some(),
        "actual physical return frontier exists"
    );
    app.world_mut()
        .entity_mut(receiver)
        .insert(crate::server_app::LocalShip);
    app.init_resource::<crate::world::server::ObjectiveManagerRes>();
    {
        let mut sessions = app.world_mut().resource_mut::<crate::lobby::Sessions>();
        sessions
            .0
            .register("request-holder".into(), "Request holder".into())
            .unwrap();
        sessions.0.set_station(
            "request-holder",
            Some(crate::core::messages::StationId("comms".into())),
        );
    }
    {
        let mut runtime = app.world_mut().resource_mut::<CommsRuntime>();
        runtime.range_active = true;
        runtime.range_flags.insert("source".into(), false);
        runtime.needs_broadcast = true;
    }
    app.world_mut()
        .run_system_once(crate::comms::server::broadcast_comms_state)
        .unwrap();
    let outbox = app.world().resource::<crate::server_app::SimOutbox>();
    let messages = outbox
        .iter()
        .find_map(|(_, packet)| match packet {
            crate::core::messages::ServerMessage::CommsState { messages, .. } => Some(messages),
            _ => None,
        })
        .expect("same actual Console holder receives legacy delivery class");
    let held = messages.iter().find(|message| message.id == id).unwrap();
    assert!(held
        .request_progress
        .as_ref()
        .unwrap()
        .last_returned
        .is_none());
    assert!(held.responses.iter().all(|response| response.available));
    assert!(
        held.sender_in_range,
        "original receipt compatibility flag, no current reverse viability oracle"
    );
    app.init_resource::<crate::comms::server::OnScreenMessage>();
    app.world_mut()
        .entity_mut(receiver)
        .insert(crate::ship::state::ShipViewMode::default());
    app.world_mut()
        .get_mut::<AdmittedCommands>(receiver)
        .unwrap()
        .0
        .push(AdmittedCommand {
            target: SystemId("comms".into()),
            payload: SystemControlPayload::ShowOnScreen {
                message_id: id.clone(),
            },
            response_token: None,
            feedback_correlation: None,
        });
    app.world_mut()
        .run_system_once(crate::console::comms::server::handle_show_on_screen)
        .unwrap();
    let display = app
        .world()
        .resource::<crate::comms::server::OnScreenMessage>()
        .0
        .as_ref()
        .unwrap();
    assert!(display
        .request_progress
        .as_ref()
        .unwrap()
        .last_returned
        .is_none());
    assert!(
        inbox_request(&app, &id)
            .request_progress
            .unwrap()
            .last_returned
            .is_some(),
        "projection never edits canonical receipt bookkeeping"
    );
}

#[test]
fn clearing_an_inbox_id_cannot_cancel_a_distinct_planned_reading_with_the_same_text() {
    let (mut app, source, _) = fixture(10.0);
    let id = "htn/plan/request";
    let mut message = crate::core::messages::CommsMessage::injected(
        id.into(),
        "narrator".into(),
        "Narrator".into(),
        "Plot message".into(),
        Default::default(),
        vec![],
        "plot".into(),
        true,
        Default::default(),
    );
    message.recipient_ship = Some(phoenix_model::wire::ShipKey("source".into()));
    app.world_mut()
        .resource_mut::<CommsInboxRes>()
        .0
        .inject(message);
    let key = crate::comms::server::PendingAiResponseKey::planned("source", id);
    let reading = CommsRequestReading::start(30, 10).unwrap();
    let record = crate::comms::server::PendingAiResponse {
        due_tick: reading.due_tick().unwrap(),
        response_fingerprint: 0,
        request_reading: Some(reading),
    };
    app.world_mut()
        .resource_mut::<CommsRuntime>()
        .pending_ai_responses
        .insert(key.clone(), record);
    app.world_mut()
        .get_mut::<AdmittedCommands>(source)
        .unwrap()
        .0[0]
        .payload = SystemControlPayload::ClearComms;
    app.world_mut()
        .run_system_once(crate::console::comms::server::handle_clear_comms)
        .unwrap();
    assert_eq!(
        app.world()
            .resource::<CommsRuntime>()
            .pending_ai_responses
            .get(&key),
        Some(&record)
    );
}
