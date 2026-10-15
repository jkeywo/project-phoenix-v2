use super::*;

fn radio() -> CommsRadioConfig {
    CommsRadioConfig {
        system: SystemId("comms".into()),
        request_reading_ticks: 30,
        keys: vec!["fleet".into()],
        channels: vec![CommsChannelConfig {
            id: "protected".into(),
            range: 100.0,
            work_units: 4,
            relay: false,
            key: Some("fleet".into()),
        }],
    }
}

#[test]
fn channels_require_actual_finite_reach_paid_work_and_held_keys() {
    let original = radio();
    assert!(original.validate(100.0).is_ok());
    for mutate in [
        |r: &mut CommsRadioConfig| r.request_reading_ticks = 0,
        |r: &mut CommsRadioConfig| r.channels[0].range = f32::NAN,
        |r: &mut CommsRadioConfig| r.channels[0].range = 101.0,
        |r: &mut CommsRadioConfig| r.channels[0].work_units = 0,
        |r: &mut CommsRadioConfig| r.keys.clear(),
        |r: &mut CommsRadioConfig| r.channels.push(r.channels[0].clone()),
    ] {
        let mut bad = original.clone();
        mutate(&mut bad);
        assert!(bad.validate(100.0).is_err());
    }
}

#[test]
fn reception_keeps_the_original_emission_and_cannot_claim_a_future_receipt() {
    let sent = CommsTransmission {
        formation_sequence: None,
        sustained: false,
        origin_id: "emission".into(),
        channel: "public".into(),
        audience: CommsAudience::All,
        emitted_tick: 10,
        received_tick: None,
        content: Default::default(),
    };
    let mut heard = sent.clone();
    heard.received_tick = Some(12);
    assert!(heard.validate(12).is_ok());
    assert_eq!(heard.emitted_tick, sent.emitted_tick);
    assert!(heard.validate(11).is_err());
    heard.received_tick = Some(9);
    assert!(heard.validate(12).is_err());
    let bytes = serde_json::to_vec(&sent).unwrap();
    assert_eq!(sent, serde_json::from_slice(&bytes).unwrap());
}

#[test]
fn player_text_accepts_unicode_and_multiline_without_control_sequences() {
    assert!(valid_text("Résumé: ship ready\nAwaiting orders."));
    for body in ["", " \n\t", "ship\0ready", "\u{1b}[31m"] {
        assert!(!valid_text(body));
    }
    assert!(!valid_text(&"a".repeat(4097)));
}

#[test]
fn copied_text_keeps_one_original_and_cannot_forge_sent_or_received_ownership() {
    use crate::{
        messages::{CommsMessage, CommsPriority},
        wire::ShipKey,
    };
    let mut sent = CommsMessage::injected(
        "own-copy".into(),
        "source".into(),
        "Source".into(),
        "Ready".into(),
        Default::default(),
        vec![],
        "radio/public/all".into(),
        true,
        CommsPriority::Routine,
    );
    sent.literal_body = true;
    sent.is_read = true;
    sent.recipient_ship = Some(ShipKey("source".into()));
    sent.transmission = Some(CommsTransmission {
        formation_sequence: None,
        sustained: false,
        origin_id: "one-emission".into(),
        channel: "public".into(),
        audience: CommsAudience::All,
        emitted_tick: 10,
        received_tick: None,
        content: Default::default(),
    });
    let mut heard = sent.clone();
    heard.id = "heard-copy".into();
    heard.recipient_ship = Some(ShipKey("receiver".into()));
    heard.is_read = false;
    heard.transmission.as_mut().unwrap().received_tick = Some(10);
    assert!(validate_transmissions(&[sent.clone(), heard.clone()], 10).is_ok());
    for original in [&sent, &heard] {
        let mut duplicate = original.clone();
        duplicate.id = "different-copy-of-the-same-delivery".into();
        assert!(validate_transmissions(&[original.clone(), duplicate], 10).is_err());
    }
    let mut other = heard.clone();
    other.id = "other-recipient-copy".into();
    other.recipient_ship = Some(ShipKey("other-receiver".into()));
    assert!(validate_transmissions(&[sent.clone(), heard.clone(), other], 10).is_ok());
    assert!(
        validate_transmissions(&[heard.clone()], 10).is_ok(),
        "sender may clear its own history"
    );
    let mut forged = heard.clone();
    forged.body = "Changed in transit".into();
    assert!(validate_transmissions(&[sent.clone(), forged], 10).is_err());
    heard.transmission.as_mut().unwrap().received_tick = None;
    assert!(validate_transmissions(&[sent, heard], 10).is_err());
}

#[test]
fn sustained_delivery_cannot_be_attached_to_unscoped_text_or_group_traffic() {
    let mut wire = CommsTransmission {
        formation_sequence: None,
        sustained: true,
        origin_id: "actual-emission".into(),
        channel: "public".into(),
        audience: CommsAudience::Contact {
            target_uuid: "known-recipient".into(),
        },
        emitted_tick: 12,
        received_tick: None,
        content: crate::comms_request::CommsRadioContent::Request {
            request: crate::comms_request::GenericCommsRequest::ReportStatus,
        },
    };
    assert!(wire.validate(12).is_ok());
    wire.audience = CommsAudience::All;
    assert!(wire.validate(12).is_err());
    wire.audience = CommsAudience::Contact {
        target_uuid: "known-recipient".into(),
    };
    wire.content = crate::comms_request::CommsRadioContent::Text;
    assert!(wire.validate(12).is_err());
}
