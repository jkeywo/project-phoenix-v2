use super::*;
fn id(value: &str) -> SystemId {
    SystemId(value.into())
}
fn profile() -> SignalProtectionConfig {
    SignalProtectionConfig {
        capacity: 3,
        strength: 4.0,
        systems: BTreeMap::from([
            (id("comms"), 2),
            (id("tactical-radar"), 2),
            (id("sensors"), 1),
            (id("passive"), 1),
        ]),
    }
}
#[test]
fn finite_allocation_follows_intent_and_actual_whole_costs_without_exposure_inputs() {
    let config = profile();
    let priority = [
        id("comms"),
        id("tactical-radar"),
        id("sensors"),
        id("passive"),
    ];
    assert_eq!(
        config.allocate(&priority).unwrap(),
        vec![id("comms"), id("sensors")]
    );
    let reverse = [
        id("passive"),
        id("sensors"),
        id("tactical-radar"),
        id("comms"),
    ];
    assert_eq!(
        config.allocate(&reverse).unwrap(),
        vec![id("passive"), id("sensors")]
    );
    assert_eq!(config.allocate(&[]).unwrap(), Vec::<SystemId>::new());
    let roundtrip: SignalProtectionConfig =
        serde_json::from_slice(&serde_json::to_vec(&config).unwrap()).unwrap();
    assert_eq!(roundtrip, config);
    assert_eq!(
        roundtrip.allocate(&priority).unwrap(),
        config.allocate(&priority).unwrap()
    );
}
#[test]
fn paid_fraction_scales_suppression_without_granting_capacity_or_natural_immunity() {
    let config = profile();
    assert_eq!(config.paid_suppression(0.25).unwrap(), 1.0);
    assert_eq!(
        receiver_factor(4.0, config.paid_suppression(0.25).unwrap()).unwrap(),
        0.25
    );
    assert_eq!(
        receiver_factor(0.0, config.paid_suppression(1.0).unwrap()).unwrap(),
        1.0
    );
    assert_eq!(receiver_factor(4.0, 0.0).unwrap(), 0.2);
    assert_eq!(receiver_factor(4.0, 8.0).unwrap(), 1.0);
    let mut extreme = config.clone();
    extreme.strength = f32::MAX;
    assert_eq!(extreme.paid_suppression(1.0).unwrap(), f64::from(f32::MAX));
    assert!(receiver_factor(f64::from(f32::MAX), 0.0)
        .unwrap()
        .is_finite());
    // This pure operation has no natural field input: callers must preserve
    // independent natural denial/terrain rather than treating this as immunity.
}
#[test]
fn malformed_profile_or_priority_never_becomes_an_allocation() {
    let config = profile();
    for priorities in [
        vec![id("comms"), id("comms")],
        vec![id("foreign")],
        vec![id("comms"); 65],
    ] {
        assert!(config.allocate(&priorities).is_err());
    }
    for bad in [f32::NAN, f32::INFINITY, -1.0, 1.1] {
        assert!(config.paid_suppression(bad).is_err());
    }
    for (a, b) in [
        (f64::NAN, 0.0),
        (0.0, f64::INFINITY),
        (-1.0, 0.0),
        (0.0, -1.0),
    ] {
        assert!(receiver_factor(a, b).is_err());
    }
    for fault in 0..5 {
        let mut bad = config.clone();
        match fault {
            0 => bad.capacity = 0,
            1 => bad.strength = f32::NAN,
            2 => bad.systems.clear(),
            3 => {
                bad.systems.insert(id("comms"), 0);
            }
            _ => {
                bad.systems.insert(id("comms"), 4);
            }
        }
        assert!(bad.validate().is_err());
    }
    assert!(serde_json::from_value::<SignalProtectionConfig>(
        serde_json::json!({"capacity":3,"strength":4,"systems":{"comms":2},"free":true})
    )
    .is_err());
}
