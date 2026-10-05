//! Pure LOD (level of detail) evaluation for AI entities.
//!
//! Decides whether an NPC entity should run at high or low simulation
//! fidelity based on distance from the player ship, with hysteresis and
//! a minimum dwell time to prevent rapid oscillation.
//!
//! This module contains no Bevy imports — fully unit-testable on native.

/// AI simulation fidelity level.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LodState {
    /// Full simulation: AI decision-making, collision, weapons, etc.
    High,
    /// Reduced simulation: throttled or skipped AI tick, simplified physics.
    Low,
}

/// Evaluate whether an entity should be promoted or demoted.
///
/// # Arguments
///
/// * `current_state`  — The current LOD state.
/// * `distance`       — Distance from the player ship (or viewpoint).
/// * `sensor_range`   — The entity's nominal sensor range.
/// * `now_secs`       — Current simulation time in seconds.
/// * `last_state_change_secs` — Time of the last LOD state transition.
/// * `dwell_secs`     — Minimum time (seconds) that must elapse before a
///   demotion is allowed.
/// * `hysteresis`     — Fractional hysteresis band applied on top of
///   `sensor_range` (e.g. `0.2` for +20%).
///
/// # Logic
///
/// * Promotion (Low → High): immediate when `distance <= sensor_range`.
/// * Demotion (High → Low): requires
///   `distance > sensor_range * (1.0 + hysteresis)` **AND**
///   `(now_secs - last_state_change_secs) >= dwell_secs`.
/// * If neither threshold is crossed, stay in the current state.
pub fn evaluate_lod(
    current_state: LodState,
    distance: f32,
    sensor_range: f32,
    now_secs: f64,
    last_state_change_secs: f64,
    dwell_secs: f64,
    hysteresis: f32,
) -> LodState {
    // Guard against a malformed `sensor_range` (NaN, +/-infinity, or
    // negative/zero) reaching the comparisons below. NaN comparisons are
    // always `false`, which would leave a High-fidelity ship stuck High
    // forever — the demote check `distance > NaN` never fires. `+infinity`
    // has the opposite failure: `distance <= infinity` is always true and
    // `distance > infinity` is never true, so every ship promotes and none
    // ever demotes, pinning the whole scenario permanently High. Both are
    // worse than the safe default: treat any non-finite or non-positive
    // sensor range as zero, so a malformed `AiProfile` fails toward the
    // cheap Low-fidelity path (no promotion; any High ship demotes once its
    // dwell timer elapses) rather than either extreme.
    let sensor_range = if sensor_range.is_finite() && sensor_range > 0.0 {
        sensor_range
    } else {
        0.0
    };
    let demote_threshold = sensor_range * (1.0 + hysteresis);
    match current_state {
        LodState::Low => {
            if distance <= sensor_range {
                LodState::High
            } else {
                LodState::Low
            }
        }
        LodState::High => {
            let dwell_elapsed = (now_secs - last_state_change_secs) >= dwell_secs;
            if distance > demote_threshold && dwell_elapsed {
                LodState::Low
            } else {
                LodState::High
            }
        }
    }
}

/// Decay (or ramp) `current_speed` toward `target_speed` at `rate_per_sec`
/// (world-units/s²), clamped so a single tick can never overshoot past the
/// target in either direction.
///
/// Used by the low-LOD dead-reckoning fallback (issue #933) to bring a
/// demoted ship's frozen exit speed back to a sane cruise speed instead of
/// carrying its exit velocity — however fast it happened to be moving at the
/// moment of demotion, boosted or otherwise — forever. Pure function of its
/// arguments: no RNG, no hidden state.
///
/// A non-positive `rate_per_sec` or `dt` is treated as "no decay this call"
/// rather than dividing/stepping by a degenerate value, so a malformed
/// authored rate fails toward leaving the speed exactly where it was (safe)
/// rather than snapping it instantaneously to the target or NaN-ing out.
pub fn decay_speed_toward(
    current_speed: f32,
    target_speed: f32,
    rate_per_sec: f32,
    dt: f32,
) -> f32 {
    // `<= 0.0` alone would miss NaN (every NaN comparison is `false`, so a NaN
    // rate/dt would fall through and silently no-op the clamps below into
    // NaN propagation); `is_nan()` catches it explicitly.
    if rate_per_sec.is_nan() || rate_per_sec <= 0.0 || dt.is_nan() || dt <= 0.0 {
        return current_speed;
    }
    let step = rate_per_sec * dt;
    if current_speed > target_speed {
        (current_speed - step).max(target_speed)
    } else if current_speed < target_speed {
        (current_speed + step).min(target_speed)
    } else {
        current_speed
    }
}

/// Turn `current_yaw` toward `desired_yaw` by at most `max_step` radians,
/// taking the shorter way around the circle.
///
/// Used by the low-LOD dead-reckoning fallback (issue #933) to gently steer a
/// demoted `Destroy`-directive ship's dead-reckoned heading back toward its
/// standing target instead of coasting on its frozen exit heading forever.
/// Pure function of its arguments: no RNG, no hidden state.
///
/// A non-positive `max_step` returns `current_yaw` unchanged rather than
/// turning backwards or NaN-ing out on a malformed authored turn rate.
pub fn step_yaw_toward(current_yaw: f32, desired_yaw: f32, max_step: f32) -> f32 {
    if max_step.is_nan() || max_step <= 0.0 {
        return current_yaw;
    }
    let two_pi = std::f32::consts::TAU;
    // Normalize the shortest signed delta into (-PI, PI] so a turn never goes
    // the "long way" around the circle.
    let mut delta = (desired_yaw - current_yaw) % two_pi;
    if delta > std::f32::consts::PI {
        delta -= two_pi;
    } else if delta < -std::f32::consts::PI {
        delta += two_pi;
    }
    current_yaw + delta.clamp(-max_step, max_step)
}

#[cfg(test)]
#[path = "lod_tests.rs"]
mod tests;
