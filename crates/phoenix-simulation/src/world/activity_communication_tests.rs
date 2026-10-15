//! Boundary-unit proofs. Actual Human/AI radio and full-host continuation are
//! exercised by the integrated communication scenario, not invented here.
use super::*;
use phoenix_model::{
    activity::*,
    comms_request::*,
    messages::{CommsMessage, CommsPriority},
};
use phoenix_sim_world::world::activities::*;
fn fixture() -> (WorldContentRuntime, WorldConfig) {
    let mut runtime = WorldContentRuntime::default();
    let mut base = WorldConfig::default();
    base.name_to_uuid
        .insert("port".into(), "actual-port".into());
    let definition = SituationDefinition {
        id: "status".into(),
        initial: "reported".into(),
        state: vec![SituationState {
            id: "reported".into(),
            when: SituationCondition::CommunicationReported {},
            call: None,
            next: None,
        }],
    };
    runtime
        .situations
        .arm(
            &SituationConfig {
                id: "htn/contact/root/status".into(),
                definition: "status".into(),
                request: ActivityRequest::Communication {
                    actor: "actual-ship".into(),
                    owner: SystemId("comms".into()),
                    recipient: "port".into(),
                    channel: "public".into(),
                    request: GenericCommsRequest::ReportStatus,
                },
            },
            &definition,
            5,
        )
        .unwrap();
    runtime.situation_definitions.push(definition);
    (runtime, base)
}
fn message(id: &str, sender: &str, recipient: &str, wire: serde_json::Value) -> CommsMessage {
    let mut result = CommsMessage::injected(
        id.into(),
        sender.into(),
        sender.into(),
        "status".into(),
        Default::default(),
        vec![],
        "thread".into(),
        true,
        CommsPriority::Routine,
    );
    result.recipient_ship = Some(phoenix_model::wire::ShipKey(recipient.into()));
    result.transmission = Some(serde_json::from_value(wire).unwrap());
    result
}
fn actual_reference() -> CommsRequestReference {
    CommsRequestReference {
        formation_sequence: None,
        origin_id: "request-origin".into(),
        source_ship: "actual-ship".into(),
        recipient_ship: "actual-port".into(),
        emitted_tick: 10,
        heard_tick: 12,
        request: GenericCommsRequest::ReportStatus,
    }
}
#[test]
fn known_only_publication_exclusive_prepare_and_dropped_payment_have_no_acceptance_write() {
    let (mut runtime, base) = fixture();
    let owner = SystemId("comms".into());
    let initial = runtime.situations.clone();
    assert!(communication_requests(
        Some(&runtime),
        Some(&base),
        "actual-ship",
        &owner,
        10,
        |_| false
    )
    .is_empty());
    let known = communication_requests(
        Some(&runtime),
        Some(&base),
        "actual-ship",
        &owner,
        10,
        |id| id == "actual-port",
    );
    assert_eq!(known.len(), 1);
    assert_eq!(known[0].recipient_uuid, "actual-port");
    let target = CommsAudience::Contact {
        target_uuid: "actual-port".into(),
    };
    assert!(prepare_communication_request(
        Some(&mut runtime),
        Some(&base),
        "actual-ship",
        &owner,
        Some(&known[0].id),
        &target,
        "wrong-channel",
        &GenericCommsRequest::ReportStatus,
        10
    )
    .is_err());
    {
        let _permit = prepare_communication_request(
            Some(&mut runtime),
            Some(&base),
            "actual-ship",
            &owner,
            Some(&known[0].id),
            &target,
            "public",
            &GenericCommsRequest::ReportStatus,
            10,
        )
        .unwrap()
        .unwrap();
    }
    assert_eq!(
        runtime.situations, initial,
        "failed ordinary payment can drop its exclusive permit"
    );
    let permit = prepare_communication_request(
        Some(&mut runtime),
        Some(&base),
        "actual-ship",
        &owner,
        Some(&known[0].id),
        &target,
        "public",
        &GenericCommsRequest::ReportStatus,
        10,
    )
    .unwrap()
    .unwrap();
    bind_communication_request(permit, "request-origin", 10);
    let accepted = runtime.situations.instances[&known[0].id]
        .accepted
        .as_ref()
        .unwrap();
    assert!(accepted.physical_job().is_none());
    assert!(
        communication_request_release(&runtime, "actual-ship", &owner, &known[0].id, 10).is_none()
    );
    assert!(communication_requests(
        Some(&runtime),
        Some(&base),
        "actual-ship",
        &owner,
        10,
        |_| true
    )
    .is_empty());
    assert!(prepare_communication_request(
        Some(&mut runtime),
        Some(&base),
        "actual-ship",
        &owner,
        Some(&known[0].id),
        &target,
        "public",
        &GenericCommsRequest::ReportStatus,
        10
    )
    .is_err());
}
#[test]
fn only_own_heard_matching_outcome_credits_original_read_and_terminal_receipt_clocks() {
    let (mut runtime, base) = fixture();
    let owner = SystemId("comms".into());
    let target = CommsAudience::Contact {
        target_uuid: "actual-port".into(),
    };
    let permit = prepare_communication_request(
        Some(&mut runtime),
        Some(&base),
        "actual-ship",
        &owner,
        Some("htn/contact/root/status"),
        &target,
        "public",
        &GenericCommsRequest::ReportStatus,
        10,
    )
    .unwrap()
    .unwrap();
    bind_communication_request(permit, "request-origin", 10);
    let original = message(
        "sent",
        "actual-ship",
        "actual-ship",
        serde_json::json!({"origin_id":"request-origin","sustained":true,"channel":"public","audience":{"kind":"contact","target_uuid":"actual-port"},"emitted_tick":10,"received_tick":null,"content":{"kind":"request","request":"report_status"}}),
    );
    let outcome = CommsRequestOutcome {
        request: actual_reference(),
        facts: CommsRequestFacts {
            read_tick: Some(15),
            decision: Some(DatedCommsDecision {
                choice: CommsRequestDecision::Report,
                tick: 20,
            }),
            report: Some(CommsStatusReport {
                system: SystemId("comms".into()),
                available: true,
                observed_tick: 20,
            }),
        },
    };
    let returned = message(
        "returned",
        "actual-port",
        "actual-ship",
        serde_json::json!({"origin_id":"return-origin","channel":"public","audience":{"kind":"contact","target_uuid":"actual-ship"},"emitted_tick":25,"received_tick":35,"content":{"kind":"outcome","outcome":outcome}}),
    );
    let mut foreign = returned.clone();
    foreign.recipient_ship = Some(phoenix_model::wire::ShipKey("other-ship".into()));
    observe_communication_receipts(&mut runtime, &[original.clone(), foreign], 35);
    assert!(runtime.pending_world_events.is_empty());
    let mut unrelated = returned.clone();
    if let CommsRadioContent::Outcome { outcome } =
        &mut unrelated.transmission.as_mut().unwrap().content
    {
        outcome.request.origin_id = "unrelated-request".into();
    }
    observe_communication_receipts(&mut runtime, &[original.clone(), unrelated], 35);
    assert!(runtime.pending_world_events.is_empty());
    observe_communication_receipts(&mut runtime, &[original.clone(), returned.clone()], 34);
    assert!(
        runtime.pending_world_events.is_empty(),
        "future actual receipt cannot be credited early"
    );
    observe_communication_receipts(&mut runtime, &[original.clone(), returned.clone()], 35);
    assert_eq!(runtime.pending_world_events.len(), 2);
    crate::world::activities::acknowledge_pending(&mut runtime, 35);
    let row = &runtime.situations.instances["htn/contact/root/status"];
    assert_eq!(row.achieved.as_ref().unwrap().tick, 35);
    assert!(matches!(
        &row.achieved.as_ref().unwrap().outcome,
        ActivityOutcome::CommunicationRead { read_tick: 15, .. }
    ));
    assert_eq!(row.settled.as_ref().unwrap().tick, 35);
    assert_eq!(
        runtime
            .situations
            .ready(&runtime.situation_definitions)
            .len(),
        1
    );
    let saved: SituationInstances =
        serde_json::from_slice(&serde_json::to_vec(&runtime.situations).unwrap()).unwrap();
    for now in 35..395 {
        saved.validate(&runtime.situation_definitions, now).unwrap();
        observe_communication_receipts(&mut runtime, &[original.clone(), returned.clone()], now);
        assert!(runtime.pending_world_events.is_empty());
        assert_eq!(runtime.situations, saved);
    }
}
