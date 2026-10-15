use super::*;
use phoenix_model::comms_channel::CommsAudience;
fn config() -> CommsCaptureConfig {
    CommsCaptureConfig {
        max_level: 1,
        ticks_per_level: 3,
        held_key_ticks: Some(1),
    }
}
fn payload(level: u8, tick: u64) -> CapturedPayload {
    CapturedPayload {
        source: "real-source".into(),
        name: "Attributed claim".into(),
        body: "Protected text".into(),
        literal_body: true,
        transmission: CommsTransmission {
            formation_sequence: None,
            sustained: false,
            origin_id: format!("actual-{tick}"),
            channel: "fleet".into(),
            audience: CommsAudience::Contact {
                target_uuid: "somebody-else".into(),
            },
            emitted_tick: tick,
            received_tick: None,
            content: CommsRadioContent::Text,
        },
        level,
        key: (level > 0).then(|| "genuinely-held-key".into()),
    }
}
fn state() -> CaptureState {
    CaptureState::new(SystemId("comms".into()))
}

#[test]
fn protected_headers_hide_body_routing_and_content_until_actual_paid_frontier() {
    let mut state = state();
    let config = config();
    assert!(
        !state.observe(payload(1, 1), 2),
        "archive age cannot masquerade as an actual current emission"
    );
    assert!(state.observe(payload(1, 1), 1));
    assert!(!state.observe(payload(1, 1), 3));
    assert!(state.select(Some(1), &config, &[]));
    assert!(state.advance(3, 0.5, &config, &[]));
    assert!(!state.advance(3, 1.0, &config, &[]));
    assert!(!state.advance(4, f32::NAN, &config, &[]));
    assert!(!state.advance(4, 0.0, &config, &[]));
    assert!(!state.advance(4, 1.1, &config, &[]));
    let view = state.views(&config, &[]);
    assert_eq!(view[0].progress_per_mille, 166);
    assert!(view[0].evidence.is_none());
    let json = serde_json::to_string(&view).unwrap();
    for private in [
        "Protected text",
        "Attributed claim",
        "real-source",
        "somebody-else",
        "genuinely-held-key",
        "actual-1",
    ] {
        assert!(!json.contains(private), "private field leaked: {private}");
    }
    for tick in 4..=6 {
        assert!(state.advance(tick, 1.0, &config, &[]));
    }
    assert_eq!(state.selected(), None);
    let view = state.views(&config, &[]);
    assert_eq!(view[0].decoded_tick, Some(6));
    assert_eq!(view[0].evidence.as_ref().unwrap().body, "Protected text");
    assert_eq!(
        view[0].evidence.as_ref().unwrap().sender_name,
        "Attributed claim"
    );
    assert!(state
        .validate(&SystemId("comms".into()), &config, &[], 6)
        .is_ok());
}

#[test]
fn impossible_security_stays_unreadable_and_held_key_requires_actual_paid_step() {
    let mut state = state();
    let config = config();
    assert!(state.observe(payload(3, 1), 1));
    assert!(!state.select(Some(1), &config, &[]));
    assert!(!state.advance(20_000, 1.0, &config, &[]));
    let keys = vec!["genuinely-held-key".into()];
    assert!(state.select(Some(1), &config, &keys));
    assert!(state.views(&config, &keys)[0].evidence.is_none());
    assert!(!state.advance(20_001, 0.0, &config, &keys));
    assert!(state.advance(20_002, 1.0, &config, &keys));
    assert_eq!(state.views(&config, &keys)[0].decoded_tick, Some(20_002));
    assert!(state
        .validate(&SystemId("comms".into()), &config, &keys, 20_002)
        .is_ok());
}
#[test]
fn switching_selection_cannot_spend_one_paid_interval_on_two_retained_captures() {
    let mut state = state();
    let config = config();
    assert!(state.observe(payload(1, 1), 1));
    let mut other = payload(1, 1);
    other.transmission.origin_id = "second-actual-emission".into();
    assert!(state.observe(other, 1));
    assert!(state.select(Some(1), &config, &[]));
    assert!(state.advance(2, 1.0, &config, &[]));
    assert!(state.select(Some(2), &config, &[]));
    assert!(!state.advance(2, 1.0, &config, &[]));
    assert_eq!(state.views(&config, &[])[1].progress_per_mille, 0);
    assert!(state.advance(3, 1.0, &config, &[]));
    assert!(state
        .validate(&SystemId("comms".into()), &config, &[], 3)
        .is_ok());
    let mut bad = serde_json::to_value(state).unwrap();
    bad["rows"][1]["started_tick"] = 2.into();
    bad["rows"][1]["last_work_tick"] = 2.into();
    let bad: CaptureState = serde_json::from_value(bad).unwrap();
    assert!(bad
        .validate(&SystemId("comms".into()), &config, &[], 3)
        .is_err());
}

#[test]
fn public_capture_is_evidence_without_addressed_request_or_action_reference() {
    let mut state = state();
    let config = config();
    let mut source = payload(0, 1);
    source.literal_body = false;
    source.body = "comms.request.report_status".into();
    source.transmission.content = CommsRadioContent::Request {
        request: phoenix_model::comms_request::GenericCommsRequest::ReportStatus,
    };
    assert!(state.observe(source, 1));
    let evidence = state.views(&config, &[]).remove(0).evidence.unwrap();
    assert_eq!(evidence.kind, CapturedContentKind::Request);
    let json = serde_json::to_value(evidence).unwrap();
    for field in [
        "source",
        "audience",
        "request",
        "request_progress",
        "origin_id",
        "recipient_ship",
    ] {
        assert!(json.get(field).is_none());
    }
    assert!(!state.select(Some(1), &config, &[]));
}

#[test]
fn partial_paid_decode_continues_exactly_after_json_and_rejects_malformed_state() {
    let mut original = state();
    let config = config();
    original.observe(payload(1, 1), 1);
    original.select(Some(1), &config, &[]);
    original.advance(3, 0.3, &config, &[]);
    let json = serde_json::to_value(&original).unwrap();
    let mut restored: CaptureState = serde_json::from_value(json.clone()).unwrap();
    assert_eq!(original, restored);
    for tick in 4..364 {
        let paid = if tick % 2 == 0 { 0.07 } else { 0.0 };
        assert_eq!(
            original.advance(tick, paid, &config, &[]),
            restored.advance(tick, paid, &config, &[])
        );
        assert_eq!(
            serde_json::to_value(&original).unwrap(),
            serde_json::to_value(&restored).unwrap()
        );
    }
    for (path, bad) in [
        ("owner", serde_json::json!("foreign")),
        ("next_sequence", serde_json::json!(1)),
    ] {
        let mut candidate = json.clone();
        candidate[path] = bad;
        let candidate: CaptureState = serde_json::from_value(candidate).unwrap();
        assert!(candidate
            .validate(&SystemId("comms".into()), &config, &[], 3)
            .is_err());
    }
    for (field, bad) in [
        ("captured_tick", 0),
        ("last_work_tick", 10),
        ("decoded_tick", 3),
        ("work", 900_000),
    ] {
        let mut candidate = json.clone();
        candidate["rows"][0][field] = bad.into();
        let candidate: CaptureState = serde_json::from_value(candidate).unwrap();
        assert!(
            candidate
                .validate(&SystemId("comms".into()), &config, &[], 3)
                .is_err(),
            "{field}"
        );
    }
    let mut impossible = json;
    impossible["selected"] = serde_json::Value::Null;
    impossible["rows"][0]["work"] = 3_000.into();
    impossible["rows"][0]["decoded_tick"] = 3.into();
    let impossible: CaptureState = serde_json::from_value(impossible).unwrap();
    assert!(
        impossible
            .validate(&SystemId("comms".into()), &config, &[], 3)
            .is_err(),
        "one paid interval cannot contain three full intervals of decoding"
    );
}

#[test]
fn deterministic_retirement_is_bounded_and_never_recaptures_old_sustained_history() {
    let mut state = state();
    let config = config();
    for tick in 1..=MAX_CAPTURE_ROWS as u64 {
        assert!(state.observe(payload(1, tick), tick));
    }
    state.select(Some(1), &config, &[]);
    assert!(state.observe(
        payload(1, MAX_CAPTURE_ROWS as u64 + 1),
        MAX_CAPTURE_ROWS as u64 + 1
    ));
    assert_eq!(state.views(&config, &[]).len(), MAX_CAPTURE_ROWS);
    assert_eq!(state.selected(), None);
    assert!(
        !state.observe(payload(1, 1), 1),
        "retirement watermark rejects even the original event when replayed"
    );
    assert!(state
        .validate(&SystemId("comms".into()), &config, &[], 1_000)
        .is_ok());
}
