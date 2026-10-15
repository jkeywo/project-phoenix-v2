use super::*;

fn reference() -> CommsRequestReference {
    CommsRequestReference {
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
