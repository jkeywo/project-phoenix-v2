use super::*;

fn profile() -> GuidedRoundProfile {
    GuidedRoundProfile {
        marking: None,
        speed: 8.0,
        turn_rate: 0.4,
        lifespan: 90.0,
        detonation_radius: 0.5,
        damage_hull: 40,
        damage_shields: 12,
        shield_pierce: 0.2,
        impact: DamageProfile::default(),
        durability: 6.0,
        seeker: None,
        receiver: None,
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
    let mut value = serde_json::to_value(&initial).unwrap();
    value.as_object_mut().unwrap().remove("durability");
    assert!(serde_json::from_value::<GuidedRoundProfile>(value).is_err());
    let mut value = serde_json::to_value(&initial).unwrap();
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
            ..initial.clone()
        }
        .validate()
        .is_err());
        assert!(GuidedRoundProfile {
            lifespan: value,
            ..initial.clone()
        }
        .validate()
        .is_err());
        assert!(GuidedRoundProfile {
            durability: value,
            ..initial.clone()
        }
        .validate()
        .is_err());
    }
    for value in [f32::NAN, f32::INFINITY, -1.0] {
        assert!(GuidedRoundProfile {
            turn_rate: value,
            ..initial.clone()
        }
        .validate()
        .is_err());
        assert!(GuidedRoundProfile {
            detonation_radius: value,
            ..initial.clone()
        }
        .validate()
        .is_err());
    }
    for value in [f32::NAN, f32::INFINITY, -1.0, 1.1] {
        assert!(GuidedRoundProfile {
            shield_pierce: value,
            ..initial.clone()
        }
        .validate()
        .is_err());
    }
    assert!(GuidedRoundProfile {
        damage_hull: -1,
        ..initial.clone()
    }
    .validate()
    .is_err());
    assert!(GuidedRoundProfile {
        damage_shields: -1,
        ..initial.clone()
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
        ..initial.clone()
    }
    .validate()
    .unwrap();
}

#[test]
fn marked_profiles_are_closed_finite_nondamaging_equipment() {
    let mut value = profile();
    value.marking = Some(crate::marking::FrozenMarkingPayload {
        recipe: crate::marking::MarkingPayloadConfig {
            template: "assets/entities/marking_beacon.toml".into(),
        },
        faction: uuid::Uuid::from_u128(7),
    });
    assert!(value.validate().is_err());
    value.damage_hull = 0;
    value.damage_shields = 0;
    value.shield_pierce = 0.0;
    value.detonation_radius = 0.0;
    value.validate().unwrap();
    let json = serde_json::to_vec(&value).unwrap();
    assert_eq!(
        serde_json::from_slice::<GuidedRoundProfile>(&json).unwrap(),
        value
    );
    for fault in 0..6 {
        let mut bad = value.clone();
        match fault {
            0 => bad.damage_hull = 1,
            1 => bad.damage_shields = 1,
            2 => bad.shield_pierce = 0.1,
            3 => bad.detonation_radius = 0.1,
            4 => bad.impact.penetration = 1.0,
            _ => bad.marking.as_mut().unwrap().faction = uuid::Uuid::nil(),
        }
        assert!(bad.validate().is_err());
    }
}
