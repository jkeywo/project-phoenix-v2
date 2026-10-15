use super::*;
use crate::messages::SystemControlPayload as Control;
#[test]
fn exact_own_priority_controls_and_finite_status_roundtrip_without_exposure_or_victim_fields() {
    for control in [
        Control::SetSignalProtectionEnabled { enabled: true },
        Control::SetSignalProtectionEnabled { enabled: false },
        Control::SetSignalProtectionPriorities {
            systems: vec![SystemId("sensors".into()), SystemId("comms".into())],
        },
        Control::SetSignalProtectionPriorities { systems: vec![] },
    ] {
        let wire = serde_json::to_value(&control).unwrap();
        let restored: Control = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(serde_json::to_value(restored).unwrap(), wire);
    }
    let view = ProtectionControlsView {
        enabled: true,
        priorities: vec![SystemId("comms".into())],
        capacity: 2,
        systems: vec![ProtectionSystemOption {
            system: SystemId("comms".into()),
            display_name: "own radio".into(),
            cost: 2,
            home_station: Some(crate::identity::StationId("comms".into())),
        }],
        active: true,
        protected_systems: vec![SystemId("comms".into())],
        controllable: false,
    };
    let mut wire = serde_json::to_value(&view).unwrap();
    assert_eq!(
        serde_json::from_value::<ProtectionControlsView>(wire.clone()).unwrap(),
        view
    );
    wire["hostile_emitter"] = serde_json::json!("unseen");
    assert!(serde_json::from_value::<ProtectionControlsView>(wire).is_err());
    let status = crate::system_status::SystemStatus::observe("own".into(), None, false, false);
    assert!(status.signal_protection.is_none());
}
