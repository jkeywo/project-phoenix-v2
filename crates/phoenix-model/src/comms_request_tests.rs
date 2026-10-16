use super::*;

fn reference() -> CommsRequestReference {
    CommsRequestReference {
        formation_sequence: None,
        origin_id: "request-one".into(),
        source_ship: "requester".into(),
        recipient_ship: "reporter".into(),
        emitted_tick: 10,
        heard_tick: 12,
        request: GenericCommsRequest::ReportStatus,
    }
}
fn report(now: u64) -> CommsStatusReport {
    CommsStatusReport {
        system: SystemId("comms".into()),
        available: true,
        observed_tick: now,
    }
}

#[test]
fn heard_read_decision_and_completion_are_distinct_original_events() {
    let mut progress = CommsRequestProgress::default();
    assert_eq!(progress.facts.stage(), CommsRequestStage::Heard);
    assert!(progress.validate(Some(12), 12).is_ok());
    assert!(progress.mark_read(12, 17).unwrap());
    assert!(!progress.mark_read(12, 19).unwrap());
    assert_eq!(progress.facts.read_tick, Some(17));
    assert_eq!(progress.facts.stage(), CommsRequestStage::Read);
    progress
        .decide(12, 22, CommsRequestDecision::Report, Some(report(22)))
        .unwrap();
    assert_eq!(progress.facts.read_tick, Some(17));
    assert_eq!(progress.facts.stage(), CommsRequestStage::Completed);
    progress.last_returned = Some(CommsRequestStage::Read);
    progress.dismissed = true;
    assert!(progress.validate(Some(12), 99).is_ok());
    let outcome = CommsRadioContent::Outcome {
        outcome: CommsRequestOutcome {
            request: reference(),
            facts: progress.facts.clone(),
        },
    };
    let json = serde_json::to_string(&outcome).unwrap();
    assert_eq!(
        serde_json::from_str::<CommsRadioContent>(&json).unwrap(),
        outcome
    );
    assert!(json.contains("17"));
    let before = progress.clone();
    assert!(progress
        .decide(12, 25, CommsRequestDecision::Decline, None)
        .is_err());
    assert_eq!(progress, before);
}

#[test]
fn rejected_local_decisions_and_forged_stage_clocks_leave_progress_unchanged() {
    let original = CommsRequestProgress::default();
    for (choice, payload, now) in [
        (CommsRequestDecision::Report, None, 20),
        (CommsRequestDecision::Decline, Some(report(20)), 20),
        (CommsRequestDecision::Report, Some(report(19)), 20),
        (CommsRequestDecision::Report, Some(report(10)), 10),
    ] {
        let mut progress = original.clone();
        assert!(progress.decide(12, now, choice, payload).is_err());
        assert_eq!(progress, original);
    }
    let mut declined = original.clone();
    declined
        .decide(12, 20, CommsRequestDecision::Decline, None)
        .unwrap();
    assert_eq!(declined.facts.stage(), CommsRequestStage::Decision);
    assert!(declined.validate(None, 30).is_err());
    for mutate in [
        |p: &mut CommsRequestProgress| p.facts.read_tick = None,
        |p: &mut CommsRequestProgress| p.facts.read_tick = Some(11),
        |p: &mut CommsRequestProgress| p.facts.decision.as_mut().unwrap().tick = 19,
        |p: &mut CommsRequestProgress| p.last_returned = Some(CommsRequestStage::Completed),
    ] {
        let mut bad = declined.clone();
        mutate(&mut bad);
        assert!(bad.validate(Some(12), 30).is_err());
    }
}

#[test]
fn closed_returned_claim_rejects_future_cross_owner_and_unread_completion_shapes() {
    let mut progress = CommsRequestProgress::default();
    progress
        .decide(12, 20, CommsRequestDecision::Report, Some(report(20)))
        .unwrap();
    let good = CommsRequestOutcome {
        request: reference(),
        facts: progress.facts,
    };
    assert!(good.validate(20).is_ok());
    assert!(good.validate(19).is_err());
    for mutate in [
        |o: &mut CommsRequestOutcome| o.request.source_ship = o.request.recipient_ship.clone(),
        |o: &mut CommsRequestOutcome| o.request.heard_tick = 9,
        |o: &mut CommsRequestOutcome| o.request.origin_id = "".into(),
        |o: &mut CommsRequestOutcome| o.facts.decision = None,
    ] {
        let mut bad = good.clone();
        mutate(&mut bad);
        assert!(bad.validate(30).is_err());
    }
    let mut wire = serde_json::to_value(&good).unwrap();
    wire["private_hull"] = serde_json::json!(0.5);
    assert!(serde_json::from_value::<CommsRequestOutcome>(wire).is_err());
    assert!(
        serde_json::from_str::<CommsRadioContent>(r#"{"kind":"request","request":"attack"}"#)
            .is_err()
    );
}

fn copy(
    id: &str,
    source: &str,
    recipient: &str,
    body: &str,
    transmission: crate::comms_channel::CommsTransmission,
) -> crate::messages::CommsMessage {
    let received = transmission.received_tick.is_some();
    let request = matches!(transmission.content, CommsRadioContent::Request { .. });
    let mut message = crate::messages::CommsMessage::injected(
        id.into(),
        source.into(),
        source.into(),
        body.into(),
        Default::default(),
        if request && received {
            [REPORT_RESPONSE, DECLINE_RESPONSE]
                .into_iter()
                .map(|text| crate::messages::CommsResponseView {
                    text: text.into(),
                    important: false,
                    available: true,
                })
                .collect()
        } else {
            vec![]
        },
        "request-thread".into(),
        true,
        false,
    );
    message.recipient_ship = Some(crate::wire::ShipKey(recipient.into()));
    message.is_read = !received;
    message.transmission = Some(transmission);
    message.request_progress = Some(CommsRequestProgress::default());
    message
}
fn lineage() -> Vec<crate::messages::CommsMessage> {
    use crate::comms_channel::{CommsAudience, CommsTransmission};
    let sent = CommsTransmission {
        formation_sequence: None,
        sustained: false,
        origin_id: "request-one".into(),
        channel: "public".into(),
        audience: CommsAudience::Contact {
            target_uuid: "reporter".into(),
        },
        emitted_tick: 10,
        received_tick: None,
        content: CommsRadioContent::Request {
            request: GenericCommsRequest::ReportStatus,
        },
    };
    let mut heard = sent.clone();
    heard.received_tick = Some(12);
    let source = copy(
        "source-copy",
        "requester",
        "requester",
        REPORT_STATUS_BODY,
        sent,
    );
    let mut receiver = copy(
        "receiver-copy",
        "requester",
        "reporter",
        REPORT_STATUS_BODY,
        heard,
    );
    let progress = receiver.request_progress.as_mut().unwrap();
    progress
        .decide(12, 20, CommsRequestDecision::Report, Some(report(20)))
        .unwrap();
    progress.last_returned = Some(CommsRequestStage::Completed);
    receiver.is_read = true;
    receiver.selected_response = Some(0);
    let reply = CommsTransmission {
        formation_sequence: None,
        sustained: false,
        origin_id: "return-one".into(),
        channel: "public".into(),
        audience: CommsAudience::Contact {
            target_uuid: "requester".into(),
        },
        emitted_tick: 25,
        received_tick: None,
        content: CommsRadioContent::Outcome {
            outcome: CommsRequestOutcome {
                request: reference(),
                facts: progress.facts.clone(),
            },
        },
    };
    let mut received_reply = reply.clone();
    received_reply.received_tick = Some(26);
    vec![
        source,
        receiver,
        copy(
            "reporter-return",
            "reporter",
            "reporter",
            OUTCOME_BODY,
            reply,
        ),
        copy(
            "requester-return",
            "reporter",
            "requester",
            OUTCOME_BODY,
            received_reply,
        ),
    ]
}

#[test]
fn actual_retained_inbox_lineage_binds_return_to_reporting_recipient_and_original_events() {
    let good = lineage();
    assert!(crate::comms_channel::validate_transmissions(&good, 26).is_ok());
    let restored: Vec<crate::messages::CommsMessage> =
        serde_json::from_str(&serde_json::to_string(&good).unwrap()).unwrap();
    assert_eq!(good, restored);
    for mutate in [
        |rows: &mut Vec<crate::messages::CommsMessage>| {
            rows.remove(0);
        },
        |rows: &mut Vec<crate::messages::CommsMessage>| {
            rows.remove(1);
        },
        |rows: &mut Vec<crate::messages::CommsMessage>| {
            rows.remove(2);
        },
        |rows: &mut Vec<crate::messages::CommsMessage>| {
            rows.remove(3);
        },
        |rows: &mut Vec<crate::messages::CommsMessage>| {
            rows[1].request_progress.as_mut().unwrap().last_returned = None;
        },
        |rows: &mut Vec<crate::messages::CommsMessage>| {
            rows[1].request_progress.as_mut().unwrap().facts.read_tick = Some(19);
        },
        |rows: &mut Vec<crate::messages::CommsMessage>| {
            rows[1].selected_response = Some(1);
        },
        |rows: &mut Vec<crate::messages::CommsMessage>| {
            rows[1]
                .request_progress
                .as_mut()
                .unwrap()
                .facts
                .report
                .as_mut()
                .unwrap()
                .available = false;
        },
    ] {
        let mut bad = good.clone();
        mutate(&mut bad);
        assert!(crate::comms_channel::validate_transmissions(&bad, 26).is_err());
    }
    let mut pending = good.clone();
    pending.remove(3);
    pending[1].request_progress.as_mut().unwrap().last_returned = None;
    assert!(
        crate::comms_channel::validate_transmissions(&pending, 26).is_ok(),
        "local paid return emission is not evidence that the requester heard it"
    );
    let mut dismissed = good.clone();
    dismissed[0].request_progress.as_mut().unwrap().dismissed = true;
    dismissed[1].request_progress.as_mut().unwrap().dismissed = true;
    assert!(crate::comms_channel::validate_transmissions(&dismissed, 26).is_ok());
    assert_eq!(
        dismissed[1].request_progress.as_ref().unwrap().facts,
        good[1].request_progress.as_ref().unwrap().facts
    );
}

#[test]
fn text_and_plot_records_cannot_carry_hidden_request_progress() {
    let mut rows = lineage();
    rows[0].transmission = None;
    assert!(crate::comms_channel::validate_transmissions(&rows, 26).is_err());
    let mut rows = lineage();
    rows[0].transmission.as_mut().unwrap().content = CommsRadioContent::Text;
    rows[0].literal_body = true;
    rows[0].body = "ordinary text".into();
    assert!(crate::comms_channel::validate_transmissions(&rows, 26).is_err());
}

#[test]
fn real_eligible_reading_ticks_pause_and_resume_without_counting_human_or_offline_time() {
    let mut reading = CommsRequestReading::start(30, 10).unwrap();
    assert!(!reading.advance(20, true).unwrap());
    assert_eq!(reading.remaining_ticks, 20);
    assert!(!reading.advance(100, false).unwrap());
    assert_eq!(reading.remaining_ticks, 20);
    let mut resumed: CommsRequestReading =
        serde_json::from_str(&serde_json::to_string(&reading).unwrap()).unwrap();
    assert!(!resumed.advance(119, true).unwrap());
    assert_eq!(resumed.remaining_ticks, 1);
    assert!(resumed.advance(120, true).unwrap());
    assert_eq!(resumed.remaining_ticks, 0);
    let before = resumed;
    assert!(resumed.advance(119, true).is_err());
    assert_eq!(resumed, before);
    assert!(CommsRequestReading::start(0, 10).is_err());
    assert!(CommsRequestReading::start(30, u64::MAX - 1).is_err());
    assert!(
        reading.validate(31, 100).is_err(),
        "actual authored reading requirement is part of the continuation binding"
    );
    let before = reading;
    assert!(reading.advance(u64::MAX, false).is_err());
    assert_eq!(reading, before);
    assert!(CommsRequestReading {
        required_ticks: 30,
        remaining_ticks: 31,
        last_tick: 10
    }
    .validate(30, 10)
    .is_err());
    assert!(CommsRequestReading {
        required_ticks: 30,
        remaining_ticks: 1,
        last_tick: 11
    }
    .validate(30, 10)
    .is_err());
}

#[test]
fn planned_choices_keep_generated_instance_identity_and_cannot_supply_future_or_hidden_route_shapes(
) {
    let row = PlannedCommsRequest {
        id: format!("htn/plan/{}", "node/".repeat(40)),
        released_tick: 12,
        recipient_uuid: "already-known-contact".into(),
        channel: "public".into(),
        request: GenericCommsRequest::ReportStatus,
    };
    assert!(
        row.validate(12).is_ok(),
        "legal generated paths exceed a plain actor id bound"
    );
    assert!(row.validate(11).is_err());
    for fault in 0..3 {
        let mut bad = row.clone();
        match fault {
            0 => bad.id = "x".repeat(2049),
            1 => bad.id = "bad\ninstance".into(),
            _ => bad.recipient_uuid.clear(),
        }
        assert!(bad.validate(12).is_err());
    }
}

#[test]
fn formation_accept_is_coordination_only_and_retains_a_closed_source_sequence() {
    assert!(serde_json::from_value::<GenericCommsRequest>(
        serde_json::json!({"form_up":{"slot":[0,1],"leader":"private"}})
    )
    .is_err());
    let request = GenericCommsRequest::FormUp { slot: [-40, 80] };
    assert!(request.validate_sequence(Some(1)).is_ok());
    assert!(request.validate_sequence(None).is_err());
    assert!(request.validate_sequence(Some(0)).is_err());
    assert!(GenericCommsRequest::ReportStatus
        .validate_sequence(Some(1))
        .is_err());
    assert!(GenericCommsRequest::FormUp {
        slot: [i32::MIN, 0]
    }
    .validate()
    .is_err());
    assert_eq!(
        serde_json::to_value(GenericCommsRequest::ReportStatus).unwrap(),
        "report_status"
    );
    let mut reference = reference();
    reference.request = request;
    reference.formation_sequence = Some(1);
    let mut progress = CommsRequestProgress::default();
    progress
        .decide(12, 20, CommsRequestDecision::Accept, None)
        .unwrap();
    assert_eq!(progress.facts.stage(), CommsRequestStage::Decision);
    assert!(progress.facts.report.is_none());
    let mut outcome = CommsRequestOutcome {
        request: reference,
        facts: progress.facts,
    };
    outcome.validate(20).unwrap();
    outcome.request.request = GenericCommsRequest::ReportStatus;
    outcome.request.formation_sequence = None;
    assert!(
        outcome.validate(20).is_err(),
        "Accept cannot masquerade as a status report"
    );
}

#[test]
fn formation_source_history_refuses_duplicate_gap_reordered_or_copy_changed_sequences() {
    use crate::comms_channel::validate_transmissions;
    let make = |sequence: u64, tick: u64| {
        let mut row = lineage().remove(0);
        row.id = format!("sent-{sequence}");
        row.body = FORM_UP_BODY.into();
        let wire = row.transmission.as_mut().unwrap();
        wire.origin_id = format!("origin-{sequence}");
        wire.emitted_tick = tick;
        wire.formation_sequence = Some(sequence);
        wire.sustained = true;
        wire.content = CommsRadioContent::Request {
            request: GenericCommsRequest::FormUp { slot: [40, 0] },
        };
        row
    };
    let first = make(1, 10);
    let second = make(2, 11);
    validate_transmissions(&[first.clone(), second.clone()], 12).unwrap();
    let mut unscoped = first.clone();
    unscoped.transmission.as_mut().unwrap().audience = crate::comms_channel::CommsAudience::All;
    assert!(validate_transmissions(&[unscoped], 12).is_err());

    for rows in [
        vec![second.clone()],
        vec![first.clone(), make(1, 11)],
        vec![make(1, 11), make(2, 10)],
    ] {
        assert!(validate_transmissions(&rows, 12).is_err());
    }
    let mut heard = first.clone();
    heard.id = "heard".into();
    heard.recipient_ship = Some(crate::wire::ShipKey("reporter".into()));
    heard.is_read = false;
    heard.responses = [ACCEPT_RESPONSE, DECLINE_RESPONSE]
        .into_iter()
        .map(|text| crate::messages::CommsResponseView {
            text: text.into(),
            important: false,
            available: true,
        })
        .collect();
    heard.transmission.as_mut().unwrap().received_tick = Some(12);
    validate_transmissions(&[first.clone(), heard.clone()], 12).unwrap();
    let mut second_heard = heard.clone();
    second_heard.id = "heard-second".into();
    second_heard.transmission.as_mut().unwrap().origin_id = "origin-2".into();
    second_heard.transmission.as_mut().unwrap().emitted_tick = 11;
    second_heard
        .transmission
        .as_mut()
        .unwrap()
        .formation_sequence = Some(2);
    for message in [&mut heard, &mut second_heard] {
        message.is_read = true;
        message.selected_response = Some(0);
    }
    heard
        .request_progress
        .as_mut()
        .unwrap()
        .decide(12, 15, CommsRequestDecision::Accept, None)
        .unwrap();
    second_heard
        .request_progress
        .as_mut()
        .unwrap()
        .decide(12, 14, CommsRequestDecision::Accept, None)
        .unwrap();
    assert!(
        validate_transmissions(
            &[
                first.clone(),
                second.clone(),
                heard.clone(),
                second_heard.clone()
            ],
            15
        )
        .is_err(),
        "a candidate cannot accept an older sequence after a newer coordination decision"
    );
    heard
        .request_progress
        .as_mut()
        .unwrap()
        .facts
        .decision
        .as_mut()
        .unwrap()
        .tick = 14;
    heard.request_progress.as_mut().unwrap().facts.read_tick = Some(14);
    validate_transmissions(&[first.clone(), second, heard.clone(), second_heard], 15).unwrap();
    heard.transmission.as_mut().unwrap().formation_sequence = Some(2);
    assert!(validate_transmissions(&[first, heard], 15).is_err());
}

#[test]
fn reception_is_explicit_consent_then_exact_dated_physical_receipt_not_a_status_report() {
    use crate::{
        activity::ActivityJob,
        craft::{CraftReceptionOrder, CraftReceptionReceipt},
        job::JobId,
    };
    let mut reference = reference();
    reference.request = GenericCommsRequest::ReceiveCraft {
        craft: "own-pod".into(),
    };
    let mut progress = CommsRequestProgress::default();
    progress
        .decide(12, 15, CommsRequestDecision::Accept, None)
        .unwrap();
    let mut outcome = CommsRequestOutcome {
        request: reference.clone(),
        facts: progress.facts,
    };
    outcome.validate(20).unwrap();
    assert_eq!(outcome.facts.stage(), CommsRequestStage::Decision);
    outcome.facts.reception = Some(CraftReceptionReceipt {
        order: CraftReceptionOrder {
            request: reference,
            decision_tick: 15,
        },
        scope: ActivityJob {
            actor: "reporter".into(),
            owner: SystemId("bay".into()),
            job: JobId(1),
        },
        craft: "own-pod".into(),
        handoff_tick: 20,
        people: vec![crate::personnel::PersonRef {
            origin: "requester".into(),
            officer: "chief".into(),
        }],
    });
    outcome.validate(20).unwrap();
    assert_eq!(outcome.facts.stage(), CommsRequestStage::Completed);
    for index in 0..6 {
        let mut bad = outcome.clone();
        let receipt = bad.facts.reception.as_mut().unwrap();
        match index {
            0 => receipt.order.decision_tick = 14,
            1 => receipt.scope.actor = "another-receiver".into(),
            2 => receipt.craft = "another-pod".into(),
            3 => receipt.handoff_tick = 14,
            4 => receipt.people.push(receipt.people[0].clone()),
            _ => bad.request.request = GenericCommsRequest::ReportStatus,
        }
        assert!(bad.validate(20).is_err());
    }
    let mut older = outcome.facts.clone();
    older.reception.as_mut().unwrap().people.clear();
    assert!(!contains_facts(&outcome.facts, &older));
    let restored: CommsRequestOutcome =
        serde_json::from_slice(&serde_json::to_vec(&outcome).unwrap()).unwrap();
    assert_eq!(outcome, restored);
}
