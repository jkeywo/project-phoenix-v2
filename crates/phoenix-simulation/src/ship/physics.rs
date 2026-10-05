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
#[path = "physics_tests.rs"]
mod tests;
