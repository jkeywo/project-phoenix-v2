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
