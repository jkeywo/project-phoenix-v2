// Pure Rust module encapsulating the ship's motion model.
// No Bevy or Rapier — pure computation, simulation layer applies results.
// Designed for isolated unit testing.

use crate::simmath;

/// Ship state for physics computation.
#[derive(Debug, Clone, Copy)]
pub struct ShipPhysicsState {
    /// X position in world space
    pub x: f32,
    /// Y (altitude / vertical) position in world space. Non-zero only for
    /// craft with a non-`Planar` `VerticalMovementMode` (issue #744).
    pub y: f32,
    /// Z position in world space
    pub z: f32,
    /// Yaw angle in radians (0 = facing negative Z)
    pub yaw: f32,
    /// Current forward speed (always >= 0)
    pub forward_speed: f32,
    /// Current lateral (sideways) speed. Positive = starboard (+X), negative = port (-X).
    pub lateral_speed: f32,
    /// Current vertical (up/down) speed. Positive = up (+Y), negative = down (issue #744).
    pub vertical_speed: f32,
}

/// Helm input values.
#[derive(Debug, Clone, Copy)]
pub struct ShipPhysicsInput {
    /// Thrust: -1.0 (full reverse) to 1.0 (full forward). 0.0 coasts.
    pub thrust: f32,
    /// Steering: -1.0 (full left) to 1.0 (full right)
    pub steering: f32,
    /// Lateral thrust: -1.0 (full port) to 1.0 (full starboard). 0.0 coasts.
    pub lateral: f32,
    /// Vertical thrust: -1.0 (full down) to 1.0 (full up). 0.0 coasts (issue #744).
    pub vertical: f32,
}

/// Result of physics computation.
#[derive(Debug, Clone, Copy)]
pub struct ShipPhysicsResult {
    /// New X position
    pub x: f32,
    /// New Y (altitude / vertical) position (issue #744)
    pub y: f32,
    /// New Z position
    pub z: f32,
    /// New yaw angle in radians
    pub yaw: f32,
    /// New forward speed
    pub forward_speed: f32,
    /// New lateral speed
    pub lateral_speed: f32,
    /// New vertical speed (issue #744)
    pub vertical_speed: f32,
}

/// Physics tuning constants.
#[derive(Debug, Clone, Copy)]
pub struct ShipPhysicsConfig {
    pub max_speed: f32,
    pub max_reverse_speed: f32,
    pub acceleration: f32,
    pub deceleration: f32,
    pub max_yaw_rate: f32,
    pub max_lateral_speed: f32,
    pub lateral_acceleration: f32,
    /// Maximum vertical (up/down) speed in world units per second (issue #744).
    pub max_vertical_speed: f32,
    /// Vertical acceleration in world units per second squared (issue #744).
    pub vertical_acceleration: f32,
}

impl Default for ShipPhysicsConfig {
    fn default() -> Self {
        Self::new()
    }
}

impl ShipPhysicsConfig {
    pub fn new() -> Self {
        Self {
            max_speed: 25.0,
            max_reverse_speed: 12.5,
            acceleration: 25.0 / 3.0,
            deceleration: 25.0,
            max_yaw_rate: std::f32::consts::PI / 16.0,
            max_lateral_speed: 15.0,
            lateral_acceleration: 15.0,
            // Vertical mirrors lateral tuning: a rate-limited axis with its own
            // ceiling and acceleration (issue #744).
            max_vertical_speed: 15.0,
            vertical_acceleration: 15.0,
        }
    }
}

/// Compute the new ship state given current state, inputs, and delta time.
///
/// # Arguments
/// * `state` - Current ship physics state
/// * `input` - Helm control inputs
/// * `dt` - Delta time in seconds
/// * `config` - Physics tuning constants
///
/// Returns the new ship state after applying physics.
pub fn compute_physics(
    state: ShipPhysicsState,
    input: ShipPhysicsInput,
    dt: f32,
    config: &ShipPhysicsConfig,
) -> ShipPhysicsResult {
    // Clamp inputs
    let thrust = input.thrust.clamp(-1.0, 1.0);
    let steering = input.steering.clamp(-1.0, 1.0);
    let lateral_input = input.lateral.clamp(-1.0, 1.0);
    let vertical_input = input.vertical.clamp(-1.0, 1.0);

    // Compute new forward speed (signed: positive = forward, negative = reverse).
    // - Non-zero thrust: drive speed toward (thrust * max_speed forward or max_reverse_speed reverse)
    // - Zero thrust: decelerate toward 0 from whichever side.
    let new_speed = if thrust.abs() > f32::EPSILON {
        let max_fwd = config.max_speed;
        let max_rev = config.max_reverse_speed;
        let target = if thrust > 0.0 {
            thrust * max_fwd
        } else {
            thrust * max_rev
        };
        let diff = target - state.forward_speed;
        let step = config.acceleration * dt;
        let delta = if diff.abs() <= step {
            diff
        } else {
            step.copysign(diff)
        };
        (state.forward_speed + delta).clamp(-max_rev, max_fwd)
    } else {
        let decel = config.deceleration * dt;
        if state.forward_speed > 0.0 {
            (state.forward_speed - decel).max(0.0)
        } else if state.forward_speed < 0.0 {
            (state.forward_speed + decel).min(0.0)
        } else {
            0.0
        }
    };

    // Compute new yaw
    let yaw_change = steering * config.max_yaw_rate * dt;
    let new_yaw = state.yaw + yaw_change;

    // Compute lateral speed
    let new_lateral_speed = crate::ship::lateral_thrust::compute_lateral_speed(
        state.lateral_speed,
        lateral_input,
        dt,
        &crate::ship::lateral_thrust::LateralThrustConfig {
            max_lateral_speed: config.max_lateral_speed,
            lateral_acceleration: config.lateral_acceleration,
        },
    );

    // Compute displacement based on new yaw, signed speed, and lateral speed
    let fwd_x = simmath::sin(new_yaw);
    let fwd_z = -simmath::cos(new_yaw);

    let (lat_dx, lat_dz) =
        crate::ship::lateral_thrust::lateral_displacement(new_yaw, new_lateral_speed, dt);

    let new_x = state.x + fwd_x * new_speed * dt + lat_dx;
    let new_z = state.z + fwd_z * new_speed * dt + lat_dz;

    // Vertical (world-Y) is yaw-independent: up is up. It reuses the same
    // rate-limited driver as lateral thrust — drive toward `input * max` at the
    // configured acceleration, decelerate to 0 on zero input (issue #744).
    let new_vertical_speed = crate::ship::lateral_thrust::compute_lateral_speed(
        state.vertical_speed,
        vertical_input,
        dt,
        &crate::ship::lateral_thrust::LateralThrustConfig {
            max_lateral_speed: config.max_vertical_speed,
            lateral_acceleration: config.vertical_acceleration,
        },
    );
    let new_y = state.y + new_vertical_speed * dt;

    ShipPhysicsResult {
        x: new_x,
        y: new_y,
        z: new_z,
        yaw: new_yaw,
        forward_speed: new_speed,
        lateral_speed: new_lateral_speed,
        vertical_speed: new_vertical_speed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_state() -> ShipPhysicsState {
        ShipPhysicsState {
            x: 0.0,
            y: 0.0,
            z: 0.0,
            yaw: 0.0,
            forward_speed: 0.0,
            lateral_speed: 0.0,
            vertical_speed: 0.0,
        }
    }

    fn default_input() -> ShipPhysicsInput {
        ShipPhysicsInput {
            thrust: 0.0,
            steering: 0.0,
            lateral: 0.0,
            vertical: 0.0,
        }
    }

    fn config() -> ShipPhysicsConfig {
        ShipPhysicsConfig::new()
    }

    #[test]
    fn zero_input_produces_zero_velocity() {
        let state = default_state();
        let input = default_input();
        let result = compute_physics(state, input, 0.016, &config());
        assert!((result.forward_speed).abs() < f32::EPSILON);
        assert!((result.x).abs() < f32::EPSILON);
        assert!((result.z).abs() < f32::EPSILON);
    }

    #[test]
    fn thrust_at_max_approaches_max_speed() {
        let state = default_state();
        let input = ShipPhysicsInput {
            thrust: 1.0,
            steering: 0.0,
            lateral: 0.0,
            vertical: 0.0,
        };
        // After 5 seconds of full thrust
        let result = compute_physics(state, input, 5.0, &config());
        assert!(result.forward_speed >= config().max_speed - 0.1);
    }

    #[test]
    fn deceleration_from_max_speed_reaches_zero() {
        let state = ShipPhysicsState {
            forward_speed: 25.0,
            ..default_state()
        };
        let input = default_input(); // No thrust
                                     // After 1 second of no thrust (decel is 25/s)
        let result = compute_physics(state, input, 1.0, &config());
        assert!(result.forward_speed < 1.0);
    }

    #[test]
    fn right_steering_produces_positive_yaw() {
        let state = default_state();
        let input = ShipPhysicsInput {
            thrust: 0.0,
            steering: 1.0,
            lateral: 0.0,
            vertical: 0.0,
        };
        let result = compute_physics(state, input, 1.0, &config());
        assert!(result.yaw > 0.0);
        // Should be max_yaw_rate * dt
        assert!((result.yaw - config().max_yaw_rate).abs() < 0.01);
    }

    #[test]
    fn left_steering_produces_negative_yaw() {
        let state = default_state();
        let input = ShipPhysicsInput {
            thrust: 0.0,
            steering: -1.0,
            lateral: 0.0,
            vertical: 0.0,
        };
        let result = compute_physics(state, input, 1.0, &config());
        assert!(result.yaw < 0.0);
        assert!((result.yaw - (-config().max_yaw_rate)).abs() < 0.01);
    }

    #[test]
    fn zero_steering_produces_no_rotation() {
        let state = ShipPhysicsState {
            yaw: 0.5,
            ..default_state()
        };
        let input = default_input();
        let result = compute_physics(state, input, 5.0, &config());
        assert!((result.yaw - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn thrust_moves_forward_along_yaw_direction() {
        let state = default_state();
        let input = ShipPhysicsInput {
            thrust: 1.0,
            steering: 0.0,
            lateral: 0.0,
            vertical: 0.0,
        };
        let result = compute_physics(state, input, 0.016, &config());
        // Facing -Z direction, should move in -Z
        assert!(result.z < 0.0);
        assert!((result.x).abs() < f32::EPSILON * 10.0);
    }

    #[test]
    fn thrust_with_steering_produces_diagonal_motion() {
        let state = default_state();
        let input = ShipPhysicsInput {
            thrust: 1.0,
            steering: 1.0,
            lateral: 0.0,
            vertical: 0.0,
        };
        let result = compute_physics(state, input, 1.0, &config());
        // Should have both X and Z displacement
        assert!((result.x).abs() > 0.1);
        assert!((result.z).abs() > 0.1);
    }

    #[test]
    fn delta_time_scales_movement() {
        let state = default_state();
        let input = ShipPhysicsInput {
            thrust: 0.5,
            steering: 0.0,
            lateral: 0.0,
            vertical: 0.0,
        };
        let config = config();
        let dt_1 = compute_physics(state, input, 0.016, &config);
        let dt_2 = compute_physics(state, input, 0.032, &config);
        // Double dt should roughly double displacement
        // At low thrust and short time, acceleration dominates so speed << max
        // displacement ~ 0.5 * a * t^2 for acceleration phase, so ratio ~ 4
        // Let's just check that dt_2 moves farther than dt_1
        assert!((-dt_2.z) > (-dt_1.z));
    }

    #[test]
    fn speed_capped_at_max() {
        let state = default_state();
        let input = ShipPhysicsInput {
            thrust: 1.0,
            steering: 0.0,
            lateral: 0.0,
            vertical: 0.0,
        };
        // Long enough time to exceed max speed if not capped
        let result = compute_physics(state, input, 10.0, &config());
        assert!(result.forward_speed <= config().max_speed);
    }

    #[test]
    fn negative_thrust_produces_reverse_motion() {
        let state = default_state();
        let input = ShipPhysicsInput {
            thrust: -1.0,
            steering: 0.0,
            lateral: 0.0,
            vertical: 0.0,
        };
        let result = compute_physics(state, input, 5.0, &config());
        assert!(result.forward_speed <= -config().max_reverse_speed + 0.1);
        // Facing -Z; reverse goes +Z
        assert!(result.z > 0.0);
    }

    #[test]
    fn reverse_speed_clamped_to_negative_max() {
        let state = ShipPhysicsState {
            forward_speed: -100.0,
            ..default_state()
        };
        let input = ShipPhysicsInput {
            thrust: -1.0,
            steering: 0.0,
            lateral: 0.0,
            vertical: 0.0,
        };
        let result = compute_physics(state, input, 0.1, &config());
        assert!(result.forward_speed >= -config().max_reverse_speed - f32::EPSILON);
    }

    #[test]
    fn no_thrust_decelerates_negative_speed_toward_zero() {
        let state = ShipPhysicsState {
            forward_speed: -25.0,
            ..default_state()
        };
        let input = default_input();
        let result = compute_physics(state, input, 1.0, &config());
        assert!(result.forward_speed > -1.0);
        assert!(result.forward_speed <= 0.0);
    }

    #[test]
    fn positive_vertical_input_climbs() {
        let state = default_state();
        let input = ShipPhysicsInput {
            vertical: 1.0,
            ..default_input()
        };
        let result = compute_physics(state, input, 1.0, &config());
        assert!(
            result.vertical_speed > 0.0,
            "positive vertical input must build a positive (up) vertical speed"
        );
        assert!(result.y > 0.0, "climbing must raise Y, got {}", result.y);
    }

    #[test]
    fn negative_vertical_input_descends() {
        let state = default_state();
        let input = ShipPhysicsInput {
            vertical: -1.0,
            ..default_input()
        };
        let result = compute_physics(state, input, 1.0, &config());
        assert!(result.vertical_speed < 0.0);
        assert!(result.y < 0.0, "descending must lower Y, got {}", result.y);
    }

    #[test]
    fn zero_vertical_input_leaves_altitude_unchanged() {
        // A ship with no vertical speed and no vertical input must not drift in Y.
        let state = default_state();
        let input = default_input();
        let result = compute_physics(state, input, 1.0, &config());
        assert_eq!(result.y, 0.0);
        assert_eq!(result.vertical_speed, 0.0);
    }

    #[test]
    fn vertical_speed_capped_at_max() {
        let state = default_state();
        let input = ShipPhysicsInput {
            vertical: 1.0,
            ..default_input()
        };
        // Long enough to exceed max if uncapped.
        let result = compute_physics(state, input, 10.0, &config());
        assert!(result.vertical_speed <= config().max_vertical_speed);
    }

    #[test]
    fn zero_vertical_input_decelerates_vertical_speed_to_zero() {
        let state = ShipPhysicsState {
            vertical_speed: 10.0,
            ..default_state()
        };
        let input = default_input();
        let result = compute_physics(state, input, 1.0, &config());
        assert!(
            result.vertical_speed.abs() < f32::EPSILON,
            "vertical speed must decay to zero on no input, got {}",
            result.vertical_speed
        );
    }

    #[test]
    fn max_yaw_rate_is_pi_over_16() {
        assert_eq!(
            ShipPhysicsConfig::new().max_yaw_rate,
            std::f32::consts::PI / 16.0,
        );
    }

    #[test]
    fn thrust_reversal_does_not_overshoot_target() {
        // At full forward speed; apply slight forward thrust → settles at slight target.
        let state = ShipPhysicsState {
            forward_speed: 25.0,
            ..default_state()
        };
        let input = ShipPhysicsInput {
            thrust: 0.2,
            steering: 0.0,
            lateral: 0.0,
            vertical: 0.0,
        };
        let cfg = config();
        let target = 0.2 * cfg.max_speed;
        // Several seconds — should converge to target, not below.
        let mut s = state;
        for _ in 0..600 {
            let r = compute_physics(s, input, 0.016, &cfg);
            s.x = r.x;
            s.z = r.z;
            s.yaw = r.yaw;
            s.forward_speed = r.forward_speed;
            s.lateral_speed = r.lateral_speed;
        }
        assert!(
            (s.forward_speed - target).abs() < 1.0,
            "expected ~{target}, got {}",
            s.forward_speed
        );
    }
}
