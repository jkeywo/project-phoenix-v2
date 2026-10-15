use super::*;
use crate::messages::{CommsBlackboard, SystemControlPayload as Control};
#[test]
fn exact_jamming_payloads_and_own_status_roundtrip_without_receiver_census() {
    for value in [
        Control::SetJammingTarget {
            target_uuid: Some("permitted-handle".into()),
        },
        Control::SetJammingTarget { target_uuid: None },
        Control::SetJammingEnabled { enabled: true },
    ] {
        let wire = serde_json::to_value(&value).unwrap();
        let restored: Control = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(serde_json::to_value(restored).unwrap(), wire);
    }
    let view = JammingControlsView {
        enabled: true,
        target: Some("permitted-handle".into()),
        emitting: true,
        controllable: false,
        targets: Vec::new(),
    };
    let mut wire = serde_json::to_value(&view).unwrap();
    assert_eq!(
        serde_json::from_value::<JammingControlsView>(wire.clone()).unwrap(),
        view
    );
    wire["affected_receivers"] = serde_json::json!(["hidden"]);
    assert!(serde_json::from_value::<JammingControlsView>(wire).is_err());
    let mut board = CommsBlackboard::default();
    board.receiver_interference = Some(true);
    let value = serde_json::to_value(&board).unwrap();
    assert_eq!(value["receiver_interference"], true);
    assert_eq!(
        serde_json::from_value::<CommsBlackboard>(value)
            .unwrap()
            .receiver_interference,
        Some(true)
    );
    let status =
        crate::system_status::SystemStatus::observe("own-equipment".into(), None, false, false);
    assert!(status.jamming.is_none());
}
