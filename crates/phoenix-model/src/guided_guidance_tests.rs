use super::*;

#[test]
fn frozen_equipment_is_closed_and_finite_without_a_perfect_seeker_default() {
    let seeker = SeekerProfile {
        range: 120.0,
        fire_arc_deg: 90.0,
        minimum_transmission: 0.2,
    };
    seeker.validate().unwrap();
    let receiver = GuidanceReceiverProfile {
        range: 240.0,
        minimum_transmission: 0.1,
        maximum_sample_age_secs: 2.0,
    };
    receiver.validate().unwrap();
    assert_eq!(
        serde_json::from_value::<SeekerProfile>(serde_json::to_value(seeker).unwrap()).unwrap(),
        seeker
    );
    for range in [0.0, -1.0, f32::INFINITY, f32::NAN] {
        assert!(SeekerProfile { range, ..seeker }.validate().is_err());
        assert!(GuidanceReceiverProfile { range, ..receiver }
            .validate()
            .is_err());
    }
    for minimum_transmission in [0.0, -1.0, 1.1, f32::INFINITY] {
        assert!(SeekerProfile {
            minimum_transmission,
            ..seeker
        }
        .validate()
        .is_err());
        assert!(GuidanceReceiverProfile {
            minimum_transmission,
            ..receiver
        }
        .validate()
        .is_err());
    }
    for maximum_sample_age_secs in [0.0, -1.0, f32::NAN, f32::INFINITY] {
        assert!(GuidanceReceiverProfile {
            maximum_sample_age_secs,
            ..receiver
        }
        .validate()
        .is_err());
    }
    for fire_arc_deg in [-1.0, 361.0, f32::NAN] {
        assert!(SeekerProfile {
            fire_arc_deg,
            ..seeker
        }
        .validate()
        .is_err());
    }
    let mut value = serde_json::to_value(seeker).unwrap();
    value["perfect_tracking"] = serde_json::json!(true);
    assert!(serde_json::from_value::<SeekerProfile>(value).is_err());
}

#[test]
fn held_measurement_retains_original_sample_and_provenance_across_serialisation() {
    let observation = GuidanceObservation {
        target_uuid: "physical-subject".into(),
        position: [8.0, 4.0, -9.0],
        sampled_tick: 17,
        source: GuidanceSource::Console {
            ship_uuid: "source".into(),
            console: ConsoleId("science".into()),
            system: SystemId("focused-scanner".into()),
        },
    };
    observation.validate(99).unwrap();
    let restored: GuidanceObservation =
        serde_json::from_slice(&serde_json::to_vec(&observation).unwrap()).unwrap();
    assert_eq!(restored, observation);
    assert_eq!(restored.sampled_tick, 17);
    assert!(restored.validate(16).is_err());
    let mut malformed = restored.clone();
    malformed.position[1] = f32::NAN;
    assert!(malformed.validate(99).is_err());
    let mut malformed = restored.clone();
    malformed.source = GuidanceSource::Console {
        ship_uuid: "source".into(),
        console: ConsoleId(String::new()),
        system: SystemId("focused-scanner".into()),
    };
    assert!(malformed.validate(99).is_err());
    let mut value = serde_json::to_value(restored).unwrap();
    value["receipt_tick"] = serde_json::json!(99);
    assert!(serde_json::from_value::<GuidanceObservation>(value).is_err());
}
