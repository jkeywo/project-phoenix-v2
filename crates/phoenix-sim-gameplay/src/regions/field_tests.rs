use super::*;

#[test]
fn candidate_field_validates_actual_origin_and_maximum_evolution_effects() {
    use crate::regions::effects::RegionEffectKind;
    let mut config = profile();
    config.evolution = Some(FieldEvolution {
        duration_secs: 1.0,
        translation: [f32::MAX, 0.0, 0.0],
        final_scale: 1.0,
        final_strength: 2.0,
    });
    assert!(RegionFieldState::new(config.clone(), 0, [f32::MAX, 0.0, 0.0]).is_err());
    let state = RegionFieldState::new(config, 0, [0.0; 3]).unwrap();
    let shape = RegionShape::Sphere { radius: 1.0 };
    assert!(state
        .validate_environment(
            &shape,
            &[RegionEffectKind::SlowZone {
                thrust_modifier: Some(-f32::MAX),
                yaw_rate_modifier: None
            }]
        )
        .is_err());
    assert!(state
        .validate_environment(
            &shape,
            &[RegionEffectKind::RadarDampening { multiplier: -1.0 }]
        )
        .is_ok());
    let mut state = state;
    state.profile.evolution.as_mut().unwrap().final_scale = f32::MAX;
    assert!(state
        .validate_environment(&RegionShape::Sphere { radius: 2.0 }, &[])
        .is_err());
    state.profile.evolution.as_mut().unwrap().final_scale = 1e-20;
    assert!(state
        .validate_environment(&RegionShape::Sphere { radius: 1e-30 }, &[])
        .is_err());
    assert!(state
        .validate_environment(
            &RegionShape::Box {
                half_extents: [1e-30, 1.0, 1.0],
                yaw: 0.0
            },
            &[]
        )
        .is_err());
    assert!(state
        .validate_environment(
            &RegionShape::Torus {
                inner_radius: 0.0,
                outer_radius: 1e-30
            },
            &[]
        )
        .is_err());
    assert!(state
        .validate_environment(
            &RegionShape::Torus {
                inner_radius: 0.0,
                outer_radius: 1.0
            },
            &[]
        )
        .is_ok());
    state.profile.evolution.as_mut().unwrap().final_scale = f32::from_bits(1);
    assert!(state
        .validate_environment(
            &RegionShape::Torus {
                inner_radius: 1.0,
                outer_radius: f32::from_bits(1.0_f32.to_bits() + 1)
            },
            &[]
        )
        .is_err());
}

fn profile() -> FieldConfig {
    FieldConfig {
        falloff: FieldFalloff::Step,
        strength: 1.0,
        sensor_absorption_per_unit: 0.2,
        comms_absorption_per_unit: 0.0,
        evolution: None,
    }
}

#[test]
fn default_step_preserves_existing_shape_membership() {
    for shape in [
        RegionShape::Sphere { radius: 10.0 },
        RegionShape::Box {
            half_extents: [10.0, 2.0, 5.0],
            yaw: 0.7,
        },
        RegionShape::Torus {
            inner_radius: 2.0,
            outer_radius: 10.0,
        },
    ] {
        let origin = Vec3::new(3.0, 0.0, -4.0);
        for x in -20..=20 {
            for z in -20..=20 {
                let point = Vec3::new(x as f32, 0.0, z as f32);
                assert_eq!(
                    local_exposure(
                        &shape,
                        FieldPose::stationary(origin),
                        FieldFalloff::Step,
                        point
                    ),
                    if shape.contains(point, origin) {
                        1.0
                    } else {
                        0.0
                    }
                );
            }
        }
    }
}

#[test]
fn evolution_is_bounded_and_json_continues_from_original_anchor() {
    let mut config = profile();
    config.evolution = Some(FieldEvolution {
        duration_secs: 10.0,
        translation: [20.0, 0.0, -10.0],
        final_scale: 2.0,
        final_strength: 0.0,
    });
    let state = RegionFieldState::new(config, 100, [5.0, 0.0, 5.0]).unwrap();
    let restored: RegionFieldState =
        serde_json::from_str(&serde_json::to_string(&state).unwrap()).unwrap();
    assert_eq!(
        state.sample_pose(100, 0.5).unwrap(),
        FieldPose::stationary(Vec3::new(5.0, 0.0, 5.0))
    );
    let midpoint = state.sample_pose(110, 0.5).unwrap();
    assert_eq!(
        midpoint,
        FieldPose {
            origin: Vec3::new(15.0, 0.0, 0.0),
            scale: 1.5,
            strength: 0.5
        }
    );
    assert_eq!(restored.sample_pose(113, 0.5), state.sample_pose(113, 0.5));
    assert_eq!(
        state.sample_pose(u64::MAX, 0.5).unwrap(),
        FieldPose {
            origin: Vec3::new(25.0, 0.0, -5.0),
            scale: 2.0,
            strength: 0.0
        }
    );
    assert!(state.sample_pose(100, 0.0).is_err());
}

#[test]
fn falloff_uses_actual_scaled_shape_and_preserves_center_strength() {
    let shape = RegionShape::Sphere { radius: 10.0 };
    let pose = FieldPose {
        origin: Vec3::ZERO,
        scale: 2.0,
        strength: 0.8,
    };
    assert_eq!(
        local_exposure(&shape, pose, FieldFalloff::Linear, Vec3::ZERO),
        0.8
    );
    assert_eq!(
        local_exposure(&shape, pose, FieldFalloff::Linear, Vec3::X * 10.0),
        0.4
    );
    assert_eq!(
        local_exposure(&shape, pose, FieldFalloff::Smooth, Vec3::X * 20.0),
        0.0
    );
    assert_eq!(
        local_exposure(&shape, pose, FieldFalloff::Step, Vec3::X * 20.0),
        0.8
    );
}

#[test]
fn invalid_profiles_and_overflowed_causal_endpoints_refuse() {
    let mut invalid = profile();
    invalid.strength = f32::NAN;
    assert!(invalid.validate().is_err());
    invalid = profile();
    invalid.comms_absorption_per_unit = -1.0;
    assert!(invalid.validate().is_err());
    invalid = profile();
    invalid.evolution = Some(FieldEvolution {
        duration_secs: 1.0,
        translation: [f32::MAX, 0.0, 0.0],
        final_scale: 1.0,
        final_strength: 1.0,
    });
    assert!(RegionFieldState::new(invalid, 0, [f32::MAX, 0.0, 0.0]).is_err());
}
