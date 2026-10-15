use super::*;
use crate::ship::physics::{compute_physics, compute_physics_with_drive, ShipPhysicsInput};

fn profile() -> HelmControlConfig {
    HelmControlConfig {
        thrust: SystemId("drive".into()),
        steering: SystemId("rudder".into()),
        lateral: None,
    }
}
fn actual() -> ShipPhysicsState {
    ShipPhysicsState {
        x: 0.0,
        y: 0.0,
        z: 0.0,
        yaw: 0.0,
        forward_speed: 10.0,
        lateral_speed: 0.0,
        vertical_speed: 0.0,
    }
}
fn input(thrust: f32, steering: f32) -> ShipPhysicsInput {
    ShipPhysicsInput {
        thrust,
        steering,
        lateral: 0.0,
        vertical: 0.0,
    }
}

#[test]
fn fractional_requests_preserve_existing_motion_but_direct_neutral_keeps_momentum() {
    let p = profile();
    let cfg = ShipPhysicsConfig::default();
    let mut state = HelmControlState::new(&p);
    state.set_thrust(&p.thrust, 0.5).unwrap();
    state.set_steering(&p.steering, 0.25).unwrap();
    let resolved = state.resolve(&p, actual(), 0.125, &cfg, None).unwrap();
    let original = compute_physics(actual(), input(0.5, 0.25), 0.125, &cfg);
    let resolved = compute_physics_with_drive(
        actual(),
        input(resolved.thrust, resolved.steering),
        0.125,
        &cfg,
        resolved.drive,
    );
    assert_eq!(
        (resolved.x, resolved.z, resolved.yaw, resolved.forward_speed),
        (original.x, original.z, original.yaw, original.forward_speed)
    );
    state
        .set_mode(
            &p.thrust,
            HelmControlMode::Direct,
            actual(),
            &ShipPhysicsConfig::default(),
        )
        .unwrap();
    let resolved = state.resolve(&p, actual(), 0.125, &cfg, None).unwrap();
    let coasting = compute_physics_with_drive(
        actual(),
        input(resolved.thrust, 0.0),
        0.125,
        &cfg,
        resolved.drive,
    );
    assert_eq!(coasting.forward_speed, 10.0);
    assert_eq!(coasting.z, -1.25);
    assert!(compute_physics(actual(), input(0.0, 0.0), 0.125, &cfg).forward_speed < 10.0);
}

#[test]
fn matching_retains_only_same_dated_contact_and_throttle_cancels_without_steering_change() {
    let p = profile();
    let mut state = HelmControlState::new(&p);
    state.set_heading(&p.steering, 1.0).unwrap();
    let steering = state.axes[&p.steering].clone();
    state
        .match_speed(&p.thrust, "public-A", 12.0, 42, 42)
        .unwrap();
    let held = state.clone();
    assert!(!state
        .refresh_match(&p.thrust, "public-B", 20.0, 43, 43)
        .unwrap());
    assert!(!state
        .refresh_match(&p.thrust, "public-A", 20.0, 42, 43)
        .unwrap());
    assert_eq!(state, held);
    assert!(state
        .refresh_match(&p.thrust, "public-A", 20.0, 44, 43)
        .is_err());
    assert_eq!(state, held);
    assert!(state
        .refresh_match(&p.thrust, "public-A", 13.0, 43, 43)
        .unwrap());
    state.cancel_match(&p.thrust).unwrap();
    assert!(matches!(
        &state.axes[&p.thrust],
        AxisIntent::Thrust(ThrustIntent::Assisted {
            demand: SpeedDemand::Absolute { speed: 13.0 }
        })
    ));
    state.set_thrust(&p.thrust, -0.5).unwrap();
    assert!(matches!(
        &state.axes[&p.thrust],
        AxisIntent::Thrust(ThrustIntent::Assisted {
            demand: SpeedDemand::Fraction { value: -0.5 }
        })
    ));
    assert_eq!(state.axes[&p.steering], steering);
    let before = state.clone();
    assert!(state
        .set_mode(
            &SystemId("foreign".into()),
            HelmControlMode::Direct,
            actual(),
            &ShipPhysicsConfig::default()
        )
        .is_err());
    assert!(state.set_speed(&p.steering, 1.0).is_err());
    assert_eq!(state, before);
}

#[test]
fn desired_speed_braking_is_a_force_request_and_zero_funding_cannot_brake() {
    let p = profile();
    let cfg = ShipPhysicsConfig::default();
    let mut state = HelmControlState::new(&p);
    state.set_speed(&p.thrust, 0.0).unwrap();
    let resolved = state.resolve(&p, actual(), 0.125, &cfg, None).unwrap();
    assert_eq!(resolved.drive, LongitudinalDriveMode::Force);
    assert!(resolved.thrust < 0.0);
    let paid = compute_physics_with_drive(
        actual(),
        input(resolved.thrust, 0.0),
        0.125,
        &cfg,
        resolved.drive,
    );
    let unfunded = compute_physics_with_drive(
        actual(),
        input(resolved.thrust * 0.0, 0.0),
        0.125,
        &cfg,
        resolved.drive,
    );
    assert!(paid.forward_speed < 10.0);
    assert_eq!(unfunded.forward_speed, 10.0);
    state
        .set_mode(
            &p.thrust,
            HelmControlMode::Direct,
            actual(),
            &ShipPhysicsConfig::default(),
        )
        .unwrap();
    state.set_thrust(&p.thrust, 1.0).unwrap();
    state
        .set_mode(
            &p.thrust,
            HelmControlMode::Assisted,
            actual(),
            &ShipPhysicsConfig::default(),
        )
        .unwrap();
    assert!(matches!(
        &state.axes[&p.thrust],
        AxisIntent::Thrust(ThrustIntent::Assisted {
            demand: SpeedDemand::Absolute { speed: 10.0 }
        })
    ));
}

#[test]
fn extreme_finite_force_and_assisted_servo_keep_bounded_velocity() {
    let p = profile();
    let cfg = ShipPhysicsConfig {
        max_speed: f32::MAX,
        max_reverse_speed: f32::MAX,
        acceleration: f32::MAX,
        ..ShipPhysicsConfig::default()
    };
    let actual = ShipPhysicsState {
        forward_speed: f32::MAX,
        ..actual()
    };
    // The intermediate impulse exceeds f32, while the resulting velocity and
    // displacement are representable. This used to produce negative infinity.
    let direct = compute_physics_with_drive(
        actual,
        input(-1.0, 0.0),
        1.5,
        &cfg,
        LongitudinalDriveMode::Force,
    );
    assert!(direct.forward_speed.is_finite());
    assert!(direct.z.is_finite());
    assert_eq!(direct.forward_speed, -f32::MAX / 2.0);

    let mut state = HelmControlState::new(&p);
    state.set_speed(&p.thrust, -f32::MAX).unwrap();
    let resolved = state.resolve(&p, actual, 2.0, &cfg, None).unwrap();
    assert_eq!(resolved.thrust, -1.0, "overflow cannot disable the servo");
    let result = compute_physics_with_drive(
        actual,
        input(resolved.thrust, 0.0),
        1.0,
        &cfg,
        resolved.drive,
    );
    assert_eq!(result.forward_speed, 0.0);
}

#[test]
fn direct_steering_alone_does_not_rotate_existing_world_momentum_for_free() {
    let cfg = ShipPhysicsConfig {
        max_speed: 10.0,
        acceleration: 0.0,
        deceleration: 0.0,
        lateral_acceleration: 0.0,
        max_yaw_rate: std::f32::consts::FRAC_PI_2,
        low_speed_turn_boost: 0.0,
        ..Default::default()
    };
    let result = crate::ship::physics::compute_physics_with_axis_modes(
        actual(),
        input(1.0, 1.0),
        1.0,
        &cfg,
        LongitudinalDriveMode::VelocityFraction,
        false,
        true,
    );
    assert!(result.x.abs() < 1e-5);
    assert!((result.z + 10.0).abs() < 1e-5);
    assert!((result.lateral_speed + 10.0).abs() < 1e-5);
}

#[test]
fn exact_owner_and_dated_intent_json_continuation_refuses_malformed_candidates() {
    let p = profile();
    let mut state = HelmControlState::new(&p);
    state
        .match_speed(&p.thrust, "public-A", 12.0, 42, 42)
        .unwrap();
    let restored: HelmControlState =
        serde_json::from_slice(&serde_json::to_vec(&state).unwrap()).unwrap();
    restored.validate(&p, 42).unwrap();
    assert_eq!(restored, state);
    for fault in 0..3 {
        let mut candidate = restored.clone();
        match fault {
            0 => {
                candidate.axes.remove(&p.steering);
            }
            1 => {
                candidate.axes.insert(
                    SystemId("unfitted".into()),
                    AxisIntent::Thrust(ThrustIntent::default()),
                );
            }
            _ => {
                candidate.axes.insert(
                    p.thrust.clone(),
                    AxisIntent::Thrust(ThrustIntent::Assisted {
                        demand: SpeedDemand::Match {
                            contact: "public-A".into(),
                            speed: 12.0,
                            observed_tick: 43,
                        },
                    }),
                );
            }
        }
        assert!(candidate.validate(&p, 42).is_err());
        assert_eq!(state, restored);
    }
}

#[test]
fn direct_turn_changes_body_basis_without_rotating_world_momentum_or_claiming_lateral() {
    use crate::ship::physics::compute_physics_with_controls;
    let p = HelmControlConfig {
        lateral: Some(SystemId("strafe".into())),
        ..profile()
    };
    let mut state = HelmControlState::new(&p);
    let cfg = ShipPhysicsConfig {
        max_yaw_rate: std::f32::consts::FRAC_PI_2,
        ..Default::default()
    };
    let actual = ShipPhysicsState {
        lateral_speed: 3.0,
        ..actual()
    };
    let strafe = p.lateral.as_ref().unwrap();
    state
        .set_mode(&p.thrust, HelmControlMode::Direct, actual, &cfg)
        .unwrap();
    assert!(matches!(
        state.axes[strafe],
        AxisIntent::Lateral(LateralIntent::Assisted { value: 0.0 })
    ));
    state
        .set_mode(strafe, HelmControlMode::Direct, actual, &cfg)
        .unwrap();
    state
        .set_mode(&p.steering, HelmControlMode::Direct, actual, &cfg)
        .unwrap();
    state.set_steering(&p.steering, 1.0).unwrap();
    let request = state.resolve(&p, actual, 1.0, &cfg, None).unwrap();
    let result = compute_physics_with_controls(
        actual,
        input(request.thrust, request.steering),
        1.0,
        &cfg,
        request.drive,
        request.lateral.unwrap().1,
    );
    assert!((result.x - 3.0).abs() < 0.00001);
    assert!((result.z + 10.0).abs() < 0.00001);
    assert!((result.forward_speed - 3.0).abs() < 0.00001);
    assert!((result.lateral_speed + 10.0).abs() < 0.00001);
    assert_eq!(result.yaw, std::f32::consts::FRAC_PI_2);
}

#[test]
fn actual_cruiser_profile_requires_its_distinct_paid_physical_axis_owners() {
    let original =
        crate::test_fixtures::load_entity_config("assets/entities/alliance_cruiser.toml").unwrap();
    let profile = original.helm_control.as_ref().unwrap();
    validate_profile(profile, &original).unwrap();
    HelmControlState::new(profile).validate(profile, 0).unwrap();
    for fault in 0..5 {
        let mut candidate = original.clone();
        match fault {
            0 => candidate.helm_control.as_mut().unwrap().thrust = SystemId("helm-steering".into()),
            1 => candidate
                .energy
                .as_mut()
                .unwrap()
                .consumers
                .retain(|row| row.system != profile.thrust),
            2 => {
                candidate
                    .interior
                    .as_mut()
                    .unwrap()
                    .placements
                    .remove(&profile.steering);
            }
            3 => {
                candidate
                    .ship_config
                    .as_mut()
                    .unwrap()
                    .systems
                    .iter_mut()
                    .find(|row| row.id == profile.thrust)
                    .unwrap()
                    .console = None
            }
            _ => candidate.helm_console.as_mut().unwrap().acceleration = f32::INFINITY,
        }
        assert!(
            validate_profile(candidate.helm_control.as_ref().unwrap(), &candidate).is_err(),
            "unsupported profile {fault}"
        );
    }
}

#[test]
fn formation_axes_use_only_current_generation_context_and_manual_inputs_replace_individually() {
    let p = profile();
    let cfg = ShipPhysicsConfig::default();
    let mut state = HelmControlState::new(&p);
    state
        .follow_navigation(&p.thrust, Some(7), actual())
        .unwrap();
    state
        .follow_navigation(&p.steering, Some(7), actual())
        .unwrap();
    let inputs = HelmNavigationInputs {
        tick: Some(10),
        axes: [
            (
                p.thrust.clone(),
                NavigationMotion {
                    generation: 7,
                    position: [0.0, -1000.0],
                    velocity: [0.0, -5.0],
                },
            ),
            (
                p.steering.clone(),
                NavigationMotion {
                    generation: 7,
                    position: [0.0, -1000.0],
                    velocity: [0.0, -5.0],
                },
            ),
        ]
        .into(),
    };
    assert!(inputs.current(Some(11)).is_none());
    assert!(inputs.current(None).is_none());
    let paid_request = state
        .resolve(&p, actual(), 0.125, &cfg, inputs.current(Some(10)))
        .unwrap();
    assert!(paid_request.thrust > 0.0);
    assert_eq!(paid_request.steering, 0.0);
    let missing = state.resolve(&p, actual(), 0.125, &cfg, None).unwrap();
    assert!(
        missing.thrust < 0.0,
        "a lost measurement asks the ordinary paid servo to brake, without rewriting inertia"
    );
    let mut newer = inputs.clone();
    for motion in newer.axes.values_mut() {
        motion.generation = 8;
    }
    assert_eq!(
        state
            .resolve(&p, actual(), 0.125, &cfg, Some(&newer))
            .unwrap(),
        missing
    );
    let encoded = serde_json::to_vec(&state).unwrap();
    let restored: HelmControlState = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(restored, state);
    restored.validate(&p, 10).unwrap();
    state.set_thrust(&p.thrust, 0.4).unwrap();
    assert!(
        matches!(state.axes[&p.thrust],AxisIntent::Thrust(ThrustIntent::Assisted{demand:SpeedDemand::Fraction{value}}) if value==0.4)
    );
    assert!(matches!(
        state.axes[&p.steering],
        AxisIntent::Steering(SteeringIntent::Assisted {
            demand: SteeringDemand::Navigation { generation: 7 }
        })
    ));
    state.set_steering(&p.steering, -0.5).unwrap();
    assert_eq!(
        state
            .resolve(&p, actual(), 0.125, &cfg, None)
            .unwrap()
            .steering,
        -0.5
    );
}

#[test]
fn formation_lateral_and_heading_use_world_slot_without_hidden_heading_or_nonfinite_output() {
    let mut p = profile();
    p.lateral = Some(SystemId("side".into()));
    let mut cfg = ShipPhysicsConfig::default();
    cfg.max_lateral_speed = 20.0;
    let mut state = HelmControlState::new(&p);
    for owner in [&p.thrust, &p.steering, p.lateral.as_ref().unwrap()] {
        state.follow_navigation(owner, Some(1), actual()).unwrap();
    }
    let inputs = HelmNavigationInputs {
        tick: Some(2),
        axes: state
            .axes
            .keys()
            .map(|owner| {
                (
                    owner.clone(),
                    NavigationMotion {
                        generation: 1,
                        position: [1000.0, 0.0],
                        velocity: [0.0, 0.0],
                    },
                )
            })
            .collect(),
    };
    let request = state
        .resolve(&p, actual(), 0.125, &cfg, Some(&inputs))
        .unwrap();
    assert!(request.steering > 0.0);
    assert!(request.lateral.unwrap().0 > 0.0);
    assert!(
        request.thrust < 0.0,
        "thrust alone cannot invent a leader-facing frame"
    );
    let before = state.clone();
    assert!(state
        .follow_navigation(&p.thrust, Some(0), actual())
        .is_err());
    assert_eq!(state, before);
    let mut malformed = inputs;
    for motion in malformed.axes.values_mut() {
        motion.velocity[0] = f32::INFINITY;
    }
    let safe = state
        .resolve(&p, actual(), 0.125, &cfg, Some(&malformed))
        .unwrap();
    assert!(safe.thrust.is_finite());
    assert_eq!(safe.steering, 0.0);
    assert_eq!(safe.lateral.unwrap().0, 0.0);
    state.follow_navigation(&p.thrust, None, actual()).unwrap();
    assert!(
        matches!(state.axes[&p.thrust],AxisIntent::Thrust(ThrustIntent::Assisted{demand:SpeedDemand::Absolute{speed}}) if speed==actual().forward_speed)
    );
}
