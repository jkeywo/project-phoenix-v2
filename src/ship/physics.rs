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
    /// Extra turn authority granted for flying SLOW, from
    /// `[helm_console] low_speed_turn_boost`.
    ///
    /// The effective yaw rate is `max_yaw_rate * (1 + X * (1 - speed_fraction))`,
    /// where `speed_fraction` is the ship's current speed as a fraction of its
    /// own cap. So `X` is the bonus at a DEAD STOP and the multiplier lerps
    /// linearly down to x1 at the speed cap. `0.0` (the default) restores the
    /// old speed-independent behaviour exactly.
    ///
    /// This is what stops two evenly-matched hulls locking into a circling
    /// stalemate: whoever backs off the throttle out-turns the one that didn't.
    pub low_speed_turn_boost: f32,
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
            // Off by default: a hull opts in from its own TOML.
            low_speed_turn_boost: 0.0,
            max_lateral_speed: 15.0,
            lateral_acceleration: 15.0,
            // Vertical mirrors lateral tuning: a rate-limited axis with its own
            // ceiling and acceleration (issue #744).
            max_vertical_speed: 15.0,
            vertical_acceleration: 15.0,
        }
    }
}

/// The yaw rate a ship actually turns at right now, given how fast it is going.
///
/// Slow hulls turn harder: the multiplier is `1 + low_speed_turn_boost` at rest
/// and lerps linearly to `1` at the speed cap, so a helm trades throttle for
/// turn authority. Speed is measured against the cap for the direction of
/// travel — full astern is not "slow" — and a hull with no cap (an unauthored
/// `max_speed = 0`) is treated as stationary.
fn effective_yaw_rate(speed: f32, config: &ShipPhysicsConfig) -> f32 {
    if config.low_speed_turn_boost <= 0.0 {
        return config.max_yaw_rate;
    }
    let cap = if speed < 0.0 {
        config.max_reverse_speed
    } else {
        config.max_speed
    };
    let speed_fraction = if cap > 0.0 {
        (speed.abs() / cap).clamp(0.0, 1.0)
    } else {
        0.0
    };
    config.max_yaw_rate * (1.0 + config.low_speed_turn_boost * (1.0 - speed_fraction))
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

    // Compute new yaw, with the low-speed turn boost folded into the rate.
    let yaw_change = steering * effective_yaw_rate(new_speed, config) * dt;
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

    // ── Low-speed turn boost ────────────────────────────────────────────────
    // The throttle-for-turn trade. Each test pins one property of the lerp,
    // because each has its own way of going wrong silently.

    fn boosted_config(boost: f32) -> ShipPhysicsConfig {
        ShipPhysicsConfig {
            low_speed_turn_boost: boost,
            ..ShipPhysicsConfig::new()
        }
    }

    fn full_right() -> ShipPhysicsInput {
        ShipPhysicsInput {
            steering: 1.0,
            ..default_input()
        }
    }

    /// A hull that authored no boost — or omitted the field entirely, which
    /// deserialises to 0.0 — must turn at exactly the rate it always did.
    #[test]
    fn zero_turn_boost_leaves_the_yaw_rate_untouched() {
        let cfg = boosted_config(0.0);
        let stationary = compute_physics(default_state(), full_right(), 1.0, &cfg);
        assert!((stationary.yaw - cfg.max_yaw_rate).abs() < 1e-4);
    }

    /// At a dead stop the multiplier is the full `1 + X`.
    #[test]
    fn turn_boost_is_at_maximum_when_stopped() {
        let cfg = boosted_config(0.5);
        let result = compute_physics(default_state(), full_right(), 1.0, &cfg);
        assert!(
            (result.yaw - cfg.max_yaw_rate * 1.5).abs() < 1e-4,
            "expected x1.5 at rest, got {}",
            result.yaw / cfg.max_yaw_rate
        );
    }

    /// At the speed cap the boost is entirely gone — flank speed must not be
    /// quietly more agile than it was before the feature landed.
    #[test]
    fn turn_boost_is_gone_at_the_speed_cap() {
        let cfg = boosted_config(0.5);
        let state = ShipPhysicsState {
            forward_speed: cfg.max_speed,
            ..default_state()
        };
        let input = ShipPhysicsInput {
            thrust: 1.0,
            ..full_right()
        };
        let result = compute_physics(state, input, 1.0, &cfg);
        assert!(
            (result.yaw - cfg.max_yaw_rate).abs() < 1e-4,
            "expected x1.0 at flank, got {}",
            result.yaw / cfg.max_yaw_rate
        );
    }

    /// Halfway up the throttle is halfway through the lerp — the interpolation
    /// is linear in speed, not stepped at the endpoints.
    #[test]
    fn turn_boost_lerps_linearly_between_the_endpoints() {
        let cfg = boosted_config(0.5);
        let state = ShipPhysicsState {
            forward_speed: cfg.max_speed * 0.5,
            ..default_state()
        };
        let input = ShipPhysicsInput {
            thrust: 0.5,
            ..full_right()
        };
        let result = compute_physics(state, input, 1.0, &cfg);
        assert!(
            (result.yaw - cfg.max_yaw_rate * 1.25).abs() < 1e-3,
            "expected x1.25 at half throttle, got {}",
            result.yaw / cfg.max_yaw_rate
        );
    }

    /// Full astern is not "slow". Reverse is measured against
    /// `max_reverse_speed`, so a ship backing off at its reverse cap gets no
    /// boost — otherwise every hull would fight the whole engagement in
    /// reverse, which is the deadlock this feature exists to break.
    #[test]
    fn full_reverse_earns_no_turn_boost() {
        let cfg = boosted_config(0.5);
        let state = ShipPhysicsState {
            forward_speed: -cfg.max_reverse_speed,
            ..default_state()
        };
        let input = ShipPhysicsInput {
            thrust: -1.0,
            ..full_right()
        };
        let result = compute_physics(state, input, 1.0, &cfg);
        assert!(
            (result.yaw - cfg.max_yaw_rate).abs() < 1e-4,
            "expected x1.0 at full astern, got {}",
            result.yaw / cfg.max_yaw_rate
        );
    }

    /// A hull with no authored speed cap can never be anything but stopped, so
    /// the fraction must not divide by zero and hand it a NaN yaw.
    #[test]
    fn turn_boost_survives_an_unauthored_speed_cap() {
        let cfg = ShipPhysicsConfig {
            max_speed: 0.0,
            max_reverse_speed: 0.0,
            low_speed_turn_boost: 0.5,
            ..ShipPhysicsConfig::new()
        };
        let result = compute_physics(default_state(), full_right(), 1.0, &cfg);
        assert!(result.yaw.is_finite());
        assert!((result.yaw - cfg.max_yaw_rate * 1.5).abs() < 1e-4);
    }

    /// The boost is symmetric: turning to port gains exactly what turning to
    /// starboard does.
    #[test]
    fn turn_boost_applies_to_both_directions() {
        let cfg = boosted_config(0.3);
        let left = ShipPhysicsInput {
            steering: -1.0,
            ..default_input()
        };
        let result = compute_physics(default_state(), left, 1.0, &cfg);
        assert!((result.yaw + cfg.max_yaw_rate * 1.3).abs() < 1e-4);
    }
}
