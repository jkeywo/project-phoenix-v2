use super::*;

fn profile() -> GuidedRoundProfile {
    GuidedRoundProfile {
        speed: 8.0,
        turn_rate: 0.4,
        lifespan: 90.0,
        detonation_radius: 0.5,
        damage_hull: 40,
        damage_shields: 12,
        shield_pierce: 0.2,
        impact: DamageProfile::default(),
        durability: 6.0,
    }
}

#[test]
fn physical_profile_is_closed_required_and_round_trips_without_current_equipment() {
    let initial = profile();
    initial.validate_active(45.0, 2.0).unwrap();
    let bytes = serde_json::to_vec(&initial).unwrap();
    assert_eq!(
        serde_json::from_slice::<GuidedRoundProfile>(&bytes).unwrap(),
        initial
    );
    let mut value = serde_json::to_value(initial).unwrap();
    value.as_object_mut().unwrap().remove("durability");
    assert!(serde_json::from_value::<GuidedRoundProfile>(value).is_err());
    let mut value = serde_json::to_value(initial).unwrap();
    value["current_launcher"] = serde_json::json!("not-a-profile-field");
    assert!(serde_json::from_value::<GuidedRoundProfile>(value).is_err());
}

#[test]
fn malformed_profile_and_active_frontiers_are_refused() {
    let initial = profile();
    assert!(GuidedRoundProfile::default().validate().is_err());
    for value in [f32::NAN, f32::INFINITY, -1.0, 0.0] {
        assert!(GuidedRoundProfile {
            speed: value,
            ..initial
        }
        .validate()
        .is_err());
        assert!(GuidedRoundProfile {
            lifespan: value,
            ..initial
        }
        .validate()
        .is_err());
        assert!(GuidedRoundProfile {
            durability: value,
            ..initial
        }
        .validate()
        .is_err());
    }
    for value in [f32::NAN, f32::INFINITY, -1.0] {
        assert!(GuidedRoundProfile {
            turn_rate: value,
            ..initial
        }
        .validate()
        .is_err());
        assert!(GuidedRoundProfile {
            detonation_radius: value,
            ..initial
        }
        .validate()
        .is_err());
    }
    for value in [f32::NAN, f32::INFINITY, -1.0, 1.1] {
        assert!(GuidedRoundProfile {
            shield_pierce: value,
            ..initial
        }
        .validate()
        .is_err());
    }
    assert!(GuidedRoundProfile {
        damage_hull: -1,
        ..initial
    }
    .validate()
    .is_err());
    assert!(GuidedRoundProfile {
        damage_shields: -1,
        ..initial
    }
    .validate()
    .is_err());
    for (life, durability) in [
        (0.0, 1.0),
        (91.0, 1.0),
        (1.0, 0.0),
        (1.0, 7.0),
        (f32::NAN, 1.0),
        (1.0, f32::INFINITY),
    ] {
        assert!(initial.validate_active(life, durability).is_err());
    }
    GuidedRoundProfile {
        turn_rate: 0.0,
        detonation_radius: 0.0,
        damage_hull: 0,
        damage_shields: 0,
        ..initial
    }
    .validate()
    .unwrap();
}
