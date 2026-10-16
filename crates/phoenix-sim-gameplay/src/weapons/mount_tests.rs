use super::*;

#[test]
fn mount_boundaries_wrap_and_all_round_use_the_existing_frame() {
    let mount = MountConfig {
        facing_deg: 0.0,
        fire_arc_deg: 90.0,
        range: 10.0,
    };
    assert!(mount.validate().is_ok());
    let boundary = mount.geometry(5.0, -5.0, 0.0, 0.0, 0.0);
    assert!(boundary.in_arc && boundary.in_range);
    let limit = mount.geometry(0.0, -10.0, 0.0, 0.0, 0.0);
    assert!(limit.in_range && limit.in_arc);
    assert!(!mount.geometry(0.0, -10.001, 0.0, 0.0, 0.0).in_range);
    assert!(!mount.geometry(5.1, -5.0, 0.0, 0.0, 0.0).in_arc);
    assert!(bearing_in_arc(std::f32::consts::PI, 0.0, 360.0));
    assert!(
        MountConfig {
            facing_deg: 360.0,
            ..mount
        }
        .geometry(0.0, -5.0, 0.0, 0.0, 0.0)
        .in_arc
    );
    let rotated = MountConfig {
        facing_deg: 0.0,
        ..mount
    }
    .geometry(10.0, 0.0, 0.0, 0.0, std::f32::consts::FRAC_PI_2);
    assert!(rotated.in_arc && rotated.in_range);
}

#[test]
fn mount_rejects_nonfinite_and_invalid_authored_parameters() {
    let base = MountConfig {
        facing_deg: 0.0,
        fire_arc_deg: 180.0,
        range: 10.0,
    };
    for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        assert!(MountConfig {
            facing_deg: value,
            ..base
        }
        .validate()
        .is_err());
        assert!(MountConfig {
            fire_arc_deg: value,
            ..base
        }
        .validate()
        .is_err());
        assert!(MountConfig {
            range: value,
            ..base
        }
        .validate()
        .is_err());
    }
    for width in [0.0, -1.0, 360.001] {
        assert!(MountConfig {
            fire_arc_deg: width,
            ..base
        }
        .validate()
        .is_err());
    }
    assert!(MountConfig { range: 0.0, ..base }.validate().is_err());
}

#[test]
fn delivery_preserves_distinct_torpedo_payload_and_pierces_only_shields_up() {
    let delivery = DamageDelivery {
        impact: phoenix_model::damage::DamageProfile::default(),
        damage_hull: 40.0,
        damage_shields: 4.0,
        shield_pierce: 0.25,
    };
    assert!(delivery.validate().is_ok());
    assert_eq!(delivery.payload(true), 4.0);
    assert_eq!(delivery.payload(false), 40.0);
    assert_eq!(delivery.split(true), (1.0, 3.0));
    assert_eq!(delivery.split(false), (40.0, 0.0));
    let full = DamageDelivery {
        shield_pierce: 1.0,
        ..delivery
    };
    assert_eq!(full.split(true), (4.0, 0.0));
    assert_eq!(full.split(false), (40.0, 0.0));
}

#[test]
fn delivery_validation_rejects_bad_content_but_runtime_split_retains_defensive_rules() {
    let base = DamageDelivery {
        impact: phoenix_model::damage::DamageProfile::default(),
        damage_hull: 4.0,
        damage_shields: 4.0,
        shield_pierce: 0.0,
    };
    for value in [-1.0, f32::NAN, f32::INFINITY] {
        assert!(DamageDelivery {
            impact: phoenix_model::damage::DamageProfile::default(),
            damage_hull: value,
            ..base
        }
        .validate()
        .is_err());
        assert!(DamageDelivery {
            damage_shields: value,
            ..base
        }
        .validate()
        .is_err());
        assert!(DamageDelivery {
            shield_pierce: value,
            ..base
        }
        .validate()
        .is_err());
    }
    assert!(DamageDelivery {
        shield_pierce: 1.01,
        ..base
    }
    .validate()
    .is_err());
    assert_eq!(
        DamageDelivery {
            shield_pierce: f32::NAN,
            ..base
        }
        .split(true),
        (0.0, 4.0)
    );
}

#[test]
fn family_delivery_views_do_not_rewrite_integer_payload_or_launch_time_pierce() {
    let hit = super::super::blaster::HitData {
        impact: Default::default(),
        incoming: [0.0, -1.0],
        damage: 20,
        shield_pierce: 0.5,
        source_uuid: "shooter".into(),
    };
    assert_eq!(hit.delivery().split(true), (10.0, 10.0));
    assert_eq!(hit.delivery().split(false), (20.0, 0.0));
    let detonation = super::super::torpedo::TorpedoDetonation {
        marking: None,
        impact: Default::default(),
        incoming: [0.0, -1.0],
        damage_hull: 40,
        damage_shields: 4,
        shield_pierce: 0.25,
        source_uuid: Some("shooter".into()),
        tube_id: "fore".into(),
        impact_x: 0.0,
        impact_y: 0.0,
        impact_z: 0.0,
    };
    assert_eq!(detonation.delivery().split(true), (1.0, 3.0));
    assert_eq!(detonation.delivery().split(false), (40.0, 0.0));
    assert_eq!(detonation.damage_hull, 40);
}
