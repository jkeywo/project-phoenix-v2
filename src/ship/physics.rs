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

        // ── Over-cap bleed (issue #1053) ──────────────────────────────────
        // A ship's cap is not a constant. `integrate_ship_physics` multiplies
        // `max_speed`/`max_reverse_speed` by the `MaxSpeed` modifier, and the
        // helm power channel moves that modifier — so a shed can leave a ship
        // ABOVE its own cap without the ship having done anything.
        //
        // What used to happen then was the terminal `.clamp` below deleting
        // the whole excess in the tick the modifier changed: measured on
        // probe_hostile as 67.5 -> 54.0 in one tick on a x1.25 -> x1.0 swing.
        // The ship visibly lurched on a power decision, and the power
        // decider's timing was coupled into physics far harder than the
        // modifier design intends.
        //
        // Excess is now BLED, on the hull's own authored `deceleration` —
        // the drag rate a coasting hull already slows at, and the one that
        // belongs here rather than `acceleration`. At the cap under full
        // throttle the engines are balancing drag; when the cap drops, the
        // engines cannot sustain the speed the ship has, so what brings it
        // down is drag alone. Nothing new is authored: `deceleration` is a
        // per-hull TOML field the zero-thrust arm below has always used.
        let over_cap = state.forward_speed > max_fwd || state.forward_speed < -max_rev;
        let step = if over_cap {
            config.deceleration * dt
        } else {
            config.acceleration * dt
        };
        let delta = if diff.abs() <= step {
            diff
        } else {
            step.copysign(diff)
        };

        // The clamp is NARROWED, not removed. Its bounds now admit a speed
        // the ship already had, so it can no longer delete excess — but a
        // ship inside its cap still cannot push past it, because for an
        // in-range speed these are exactly the old bounds.
        //
        // The bleed cannot overshoot: `diff.abs() <= step` lands exactly on
        // `target`, which is at or inside the cap, and `target` is `thrust *
        // cap` with `|thrust|` at most 1, so an over-cap ship's `diff` always
        // points back toward the cap.
        //
        // It cannot STALL either, but only because of the guard below.
        // `deceleration` has a serde default of 0.0 and nothing rejects a hull
        // that authors none, and a zero step means `delta` is `-0.0` and the
        // ship sits over its cap for ever — a state the old unconditional
        // clamp could not produce. No shipped hull is in that position (all
        // eleven `[helm_console]` templates author a positive deceleration),
        // which is exactly why it would go unnoticed. A hull with no drag
        // keeps the OLD behaviour: it snaps to the cap, because a lurch is
        // better than a ship that can never come back inside its own limits.
        let widen = over_cap && step > 0.0;
        let ceiling = if widen {
            max_fwd.max(state.forward_speed)
        } else {
            max_fwd
        };
        let floor = if widen {
            (-max_rev).min(state.forward_speed)
        } else {
            -max_rev
        };
        (state.forward_speed + delta).clamp(floor, ceiling)
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

    /// The reverse cap is a destination, not a snap (issue #1053 re-bless).
    ///
    /// This used to assert that ONE step from -100 landed at or inside a
    /// -12.5 cap, which is the terminal clamp the over-cap bleed narrowed. The
    /// contract it was really guarding — a ship cannot end up faster astern
    /// than its cap allows — is unchanged and is what it asserts now; only
    /// "immediately" became "on the hull's drag rate".
    ///
    /// The -100 start is eight times the cap and nothing in the simulation
    /// produces it: the `MaxSpeed` modifier's whole authored range cannot swing
    /// that far. It is kept because an over-cap state arriving from anywhere at
    /// all must converge, and because a bleed that stalled or reversed would
    /// show up most obviously from absurdly far out.
    #[test]
    fn reverse_speed_converges_to_negative_max() {
        let cfg = config();
        let input = ShipPhysicsInput {
            thrust: -1.0,
            steering: 0.0,
            lateral: 0.0,
            vertical: 0.0,
        };
        let mut state = ShipPhysicsState {
            forward_speed: -100.0,
            ..default_state()
        };
        let mut previous = state.forward_speed;
        for _ in 0..1000 {
            let result = compute_physics(state, input, 0.1, &cfg);
            assert!(
                result.forward_speed >= previous - f32::EPSILON,
                "the bleed must move TOWARD the cap, never further from it: \
                 {previous} -> {}",
                result.forward_speed
            );
            previous = result.forward_speed;
            state.forward_speed = result.forward_speed;
            if (state.forward_speed + cfg.max_reverse_speed).abs() < 1e-4 {
                break;
            }
        }
        assert!(
            (state.forward_speed + cfg.max_reverse_speed).abs() < 1e-4,
            "must settle at the reverse cap, got {}",
            state.forward_speed
        );
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

    // ── Over-cap bleed (issue #1053) ────────────────────────────────────────
    // A ship's speed cap is not a constant: `integrate_ship_physics` multiplies
    // it by the `MaxSpeed` modifier, which the helm power channel moves. When
    // that cap SHRINKS under a ship already sitting at the old one, the ship is
    // over-cap through no act of its own, and what happens next is a physics
    // question rather than a clamping one.

    /// The measurement in issue #1053, as a test: a helm power shed at the cap.
    ///
    /// `probe_hostile` was seen going 67.5 -> 54.0 in a single tick when the
    /// MaxSpeed modifier swung x1.25 -> x1.0 at t=437. Those are exactly the
    /// numbers here: a 54-unit cap, a ship at 67.5 because the cap used to be
    /// 67.5, and full throttle held throughout.
    ///
    /// Three properties, because each fails differently. It must not arrive in
    /// one tick (the bug). It must descend MONOTONICALLY (a bleed that
    /// oscillates or overshoots would be a new bug wearing the fix's clothes).
    /// And it must actually ARRIVE, at the cap and not above or below it — a
    /// ship that bled forever, or settled at 53.9, would pass a "not in one
    /// tick" assertion and be worse than the clamp.
    #[test]
    fn a_cap_that_shrinks_under_a_ship_bleeds_off_instead_of_clamping() {
        let cfg = ShipPhysicsConfig {
            max_speed: 54.0,
            max_reverse_speed: 27.0,
            ..ShipPhysicsConfig::new()
        };
        let dt = 1.0 / 60.0;
        let full_ahead = ShipPhysicsInput {
            thrust: 1.0,
            ..default_input()
        };

        let mut state = ShipPhysicsState {
            forward_speed: 67.5,
            ..default_state()
        };

        let first = compute_physics(state, full_ahead, dt, &cfg);
        assert!(
            first.forward_speed > cfg.max_speed,
            "the excess must not be deleted in the tick the cap moved; got {} \
             against a cap of {}",
            first.forward_speed,
            cfg.max_speed
        );

        let mut speeds = vec![state.forward_speed];
        for _ in 0..600 {
            let r = compute_physics(state, full_ahead, dt, &cfg);
            state.forward_speed = r.forward_speed;
            speeds.push(r.forward_speed);
            if (r.forward_speed - cfg.max_speed).abs() < 1e-4 {
                break;
            }
        }

        for pair in speeds.windows(2) {
            assert!(
                pair[1] <= pair[0] + 1e-6,
                "the bleed must be monotone; went {} -> {}",
                pair[0],
                pair[1]
            );
            assert!(
                pair[1] >= cfg.max_speed - 1e-4,
                "the bleed must not undershoot the new cap; reached {}",
                pair[1]
            );
        }
        assert!(
            speeds.len() > 4,
            "a bleed spread over fewer than four ticks is the lurch this fixes; \
             took {} ticks",
            speeds.len() - 1
        );
        assert!(
            (state.forward_speed - cfg.max_speed).abs() < 1e-4,
            "the ship must settle AT the new cap, not near it; settled at {}",
            state.forward_speed
        );
    }

    /// The bleed is the hull's DRAG rate, not its engine rate, and it is the
    /// same number a coasting hull decelerates at. Pinned as a duration rather
    /// than as an internal, so it stays true through any refactor that keeps
    /// the behaviour.
    ///
    /// 13.5 units of excess at the default 25 u/s deceleration is 0.54 s — 33
    /// ticks at 60 Hz. At the ACCELERATION rate (25/3, the rate the in-range
    /// approach step uses) the same excess would take 98, and a ship over its
    /// cap is not being pushed there by its engines.
    #[test]
    fn the_over_cap_bleed_runs_at_the_hulls_deceleration_rate() {
        let cfg = ShipPhysicsConfig {
            max_speed: 54.0,
            max_reverse_speed: 27.0,
            ..ShipPhysicsConfig::new()
        };
        let dt = 1.0 / 60.0;
        let mut state = ShipPhysicsState {
            forward_speed: 67.5,
            ..default_state()
        };
        let full_ahead = ShipPhysicsInput {
            thrust: 1.0,
            ..default_input()
        };

        let mut ticks = 0;
        while state.forward_speed > cfg.max_speed + 1e-4 && ticks < 1000 {
            state.forward_speed = compute_physics(state, full_ahead, dt, &cfg).forward_speed;
            ticks += 1;
        }
        let expected = ((67.5 - 54.0) / cfg.deceleration / dt).ceil() as i32;
        assert_eq!(
            ticks, expected,
            "expected {expected} ticks at the {} u/s deceleration rate, took {ticks}",
            cfg.deceleration
        );
    }

    /// The clamp is not gone, only narrowed. A ship WITHIN its cap must still
    /// be unable to push past it — the fix is about excess that already exists,
    /// not about licence to build more.
    #[test]
    fn a_ship_inside_its_cap_still_cannot_exceed_it() {
        let cfg = config();
        let mut state = default_state();
        let full_ahead = ShipPhysicsInput {
            thrust: 1.0,
            ..default_input()
        };
        for _ in 0..2000 {
            state.forward_speed =
                compute_physics(state, full_ahead, 1.0 / 60.0, &cfg).forward_speed;
            assert!(
                state.forward_speed <= cfg.max_speed + 1e-6,
                "an accelerating ship must never pass its own cap; reached {}",
                state.forward_speed
            );
        }
    }

    /// The reverse arm, which has its own cap and its own modifier.
    /// `config.max_reverse_speed` is multiplied by the same `MaxSpeed` modifier
    /// as the forward cap, so a shed astern is the identical situation with the
    /// sign flipped, and leaving it clamping would have fixed half a bug.
    #[test]
    fn a_shrinking_reverse_cap_bleeds_off_the_same_way() {
        let cfg = ShipPhysicsConfig {
            max_speed: 54.0,
            max_reverse_speed: 27.0,
            ..ShipPhysicsConfig::new()
        };
        let dt = 1.0 / 60.0;
        let full_astern = ShipPhysicsInput {
            thrust: -1.0,
            ..default_input()
        };
        let state = ShipPhysicsState {
            forward_speed: -33.75,
            ..default_state()
        };

        let first = compute_physics(state, full_astern, dt, &cfg);
        assert!(
            first.forward_speed < -cfg.max_reverse_speed,
            "astern excess must not be deleted in one tick either; got {}",
            first.forward_speed
        );

        let mut s = state;
        for _ in 0..600 {
            s.forward_speed = compute_physics(s, full_astern, dt, &cfg).forward_speed;
            if (s.forward_speed + cfg.max_reverse_speed).abs() < 1e-4 {
                break;
            }
        }
        assert!(
            (s.forward_speed + cfg.max_reverse_speed).abs() < 1e-4,
            "must settle at the new reverse cap; settled at {}",
            s.forward_speed
        );
    }

    /// A hull with NO drag keeps the old snap, because the alternative is a
    /// ship that can never come back inside its own limits.
    ///
    /// `deceleration` has a serde default of 0.0 and nothing rejects a hull
    /// that authors none. A zero bleed rate makes `delta` `-0.0`, so a widened
    /// ceiling would hold the ship over its cap for ever — a state the
    /// unconditional clamp this replaced could not produce. Every shipped hull
    /// authors a positive deceleration, which is precisely why this would never
    /// be noticed without a test.
    #[test]
    fn a_hull_with_no_authored_drag_still_cannot_stay_over_its_cap() {
        let cfg = ShipPhysicsConfig {
            max_speed: 54.0,
            max_reverse_speed: 27.0,
            deceleration: 0.0,
            ..ShipPhysicsConfig::new()
        };
        let state = ShipPhysicsState {
            forward_speed: 67.5,
            ..default_state()
        };
        let result = compute_physics(
            state,
            ShipPhysicsInput {
                thrust: 1.0,
                ..default_input()
            },
            1.0 / 60.0,
            &cfg,
        );
        assert!(
            (result.forward_speed - cfg.max_speed).abs() < 1e-4,
            "with no drag to bleed on, the cap must still be enforced; got {}",
            result.forward_speed
        );
    }

    /// THE IMPULSE DRIVE IS THE BIG ONE, and this pins what it now does rather
    /// than leaving it to be discovered.
    ///
    /// `MaxSpeed` is not moved only by the helm power channel. `apply_impulse_to`
    /// writes the same slot at `speed_multiplier - 1.0` — x6 on the shipped
    /// Alliance Destroyer, x10 for a hull that authors none — so DROPPING
    /// impulse is a cap reduction of the same kind as a power shed and several
    /// times the size. The destroyer's numbers: cap 18, impulse cap 108,
    /// deceleration 15, so the excess is 90 and the bleed runs 6 seconds.
    ///
    /// That is a real behaviour change and it is NOT what issue #1053 measured
    /// (a x1.25 swing, 13.5 of excess, half a second). It is flagged in the
    /// commit body rather than special-cased here, because carving an exception
    /// into the physics for one modifier source is a design call — but it is
    /// pinned, so the number cannot drift unnoticed and nobody has to rediscover
    /// it from a player report.
    #[test]
    fn dropping_impulse_bleeds_for_seconds_because_it_moves_the_same_cap() {
        // The shipped Alliance Destroyer, as `alliance_destroyer.toml` authors
        // it: max_speed 18, deceleration 15, impulse speed_multiplier 6.
        let cfg = ShipPhysicsConfig {
            max_speed: 18.0,
            max_reverse_speed: 6.0,
            deceleration: 15.0,
            ..ShipPhysicsConfig::new()
        };
        let dt = 1.0 / 60.0;
        let mut state = ShipPhysicsState {
            forward_speed: 18.0 * 6.0,
            ..default_state()
        };
        let full_ahead = ShipPhysicsInput {
            thrust: 1.0,
            ..default_input()
        };

        let mut ticks = 0;
        let mut travelled = 0.0f32;
        while state.forward_speed > cfg.max_speed + 1e-3 && ticks < 10_000 {
            state.forward_speed = compute_physics(state, full_ahead, dt, &cfg).forward_speed;
            travelled += state.forward_speed * dt;
            ticks += 1;
        }
        // 90 units of excess at 15 u/s is six seconds, and the hull covers
        // roughly (108 + 18) / 2 * 6 units of ground getting there.
        assert_eq!(
            ticks, 360,
            "expected a six-second bleed, took {ticks} ticks"
        );
        assert!(
            (370.0..385.0).contains(&travelled),
            "expected ~378 units of over-cap travel, got {travelled}"
        );
    }

    /// Coasting is untouched. A ship over its cap with NO thrust already bled
    /// down the deceleration path — that arm never had the clamp — and it must
    /// keep decaying all the way to a stop rather than stopping at the cap.
    #[test]
    fn an_over_cap_ship_with_no_thrust_still_coasts_to_a_stop() {
        let cfg = ShipPhysicsConfig {
            max_speed: 54.0,
            ..ShipPhysicsConfig::new()
        };
        let mut state = ShipPhysicsState {
            forward_speed: 67.5,
            ..default_state()
        };
        for _ in 0..600 {
            state.forward_speed =
                compute_physics(state, default_input(), 1.0 / 60.0, &cfg).forward_speed;
        }
        assert_eq!(
            state.forward_speed, 0.0,
            "no thrust means all the way down, not down to the cap"
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
