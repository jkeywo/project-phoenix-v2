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

#[cfg(test)]
mod tests {
    use super::*;

    // ── Promotion (Low → High) ──────────────────────────────────────────────

    #[test]
    fn promote_when_distance_within_range() {
        let result = evaluate_lod(LodState::Low, 50.0, 100.0, 0.0, 0.0, 2.0, 0.2);
        assert_eq!(result, LodState::High);
    }

    #[test]
    fn no_promotion_when_distance_exceeds_range() {
        let result = evaluate_lod(LodState::Low, 150.0, 100.0, 0.0, 0.0, 2.0, 0.2);
        assert_eq!(result, LodState::Low);
    }

    #[test]
    fn no_promotion_when_distance_equals_sensor_range_times_12() {
        let result = evaluate_lod(LodState::Low, 120.0, 100.0, 100.0, 0.0, 2.0, 0.2);
        assert_eq!(result, LodState::Low);
    }

    // ── Demotion (High → Low) ───────────────────────────────────────────────

    #[test]
    fn demote_when_distance_exceeds_hysteresis_and_dwell_met() {
        let result = evaluate_lod(LodState::High, 200.0, 100.0, 10.0, 0.0, 2.0, 0.2);
        assert_eq!(result, LodState::Low);
    }

    #[test]
    fn no_demotion_when_distance_exceeds_threshold_but_dwell_not_met() {
        let result = evaluate_lod(LodState::High, 200.0, 100.0, 1.0, 0.0, 2.0, 0.2);
        assert_eq!(result, LodState::High);
    }

    #[test]
    fn no_demotion_when_distance_within_sensor_range_even_if_dwell_met() {
        let result = evaluate_lod(LodState::High, 50.0, 100.0, 10.0, 0.0, 2.0, 0.2);
        assert_eq!(result, LodState::High);
    }

    #[test]
    fn no_demotion_when_distance_in_hysteresis_band() {
        let result = evaluate_lod(LodState::High, 110.0, 100.0, 10.0, 0.0, 2.0, 0.2);
        assert_eq!(result, LodState::High);
    }

    #[test]
    fn stay_high_when_distance_exactly_at_sensor_range() {
        let result = evaluate_lod(LodState::High, 100.0, 100.0, 10.0, 0.0, 2.0, 0.2);
        assert_eq!(result, LodState::High);
    }

    // ── Edge cases ──────────────────────────────────────────────────────────

    #[test]
    fn zero_distance_promotes_to_high() {
        let result = evaluate_lod(LodState::Low, 0.0, 100.0, 0.0, 0.0, 2.0, 0.2);
        assert_eq!(result, LodState::High);
    }

    #[test]
    fn very_large_distance_stays_low_or_demotes() {
        let low_result = evaluate_lod(LodState::Low, 1e8, 100.0, 0.0, 0.0, 2.0, 0.2);
        assert_eq!(low_result, LodState::Low);

        let high_result = evaluate_lod(LodState::High, 1e8, 100.0, 10.0, 0.0, 2.0, 0.2);
        assert_eq!(high_result, LodState::Low);
    }

    // ── Dwell timer semantics ───────────────────────────────────────────────

    #[test]
    fn promotion_resets_dwell_timer_for_subsequent_demotion() {
        // Promote to High — this implies a state change at now_secs = 10.0
        let promoted = evaluate_lod(LodState::Low, 50.0, 100.0, 10.0, 0.0, 2.0, 0.2);
        assert_eq!(promoted, LodState::High);

        // At the same timestamp, demotion should not happen because
        // dwell_elapsed = (10.0 - 10.0) = 0.0 which is < 2.0
        // Actually — the *caller* is responsible for storing
        // last_state_change_secs as the time the promotion happened.
        // The function itself doesn't track it. This test demonstrates
        // that after an immediate promotion, if we call again with
        // last_state_change_secs = 10.0 (the promotion time), demotion
        // is correctly blocked.
        let still_high = evaluate_lod(LodState::High, 200.0, 100.0, 10.0, 10.0, 2.0, 0.2);
        assert_eq!(still_high, LodState::High);

        // After dwell_secs have passed, demotion is allowed.
        let demoted = evaluate_lod(LodState::High, 200.0, 100.0, 12.0, 10.0, 2.0, 0.2);
        assert_eq!(demoted, LodState::Low);
    }

    #[test]
    fn dwell_secs_zero_allows_immediate_demotion() {
        let result = evaluate_lod(LodState::High, 200.0, 100.0, 5.0, 5.0, 0.0, 0.2);
        assert_eq!(result, LodState::Low);
    }

    // ── Boundary: exactly at thresholds ─────────────────────────────────────

    #[test]
    fn exactly_at_sensor_range_from_low_promotes() {
        let result = evaluate_lod(LodState::Low, 100.0, 100.0, 0.0, 0.0, 2.0, 0.2);
        assert_eq!(result, LodState::High);
    }

    #[test]
    fn exactly_at_demote_threshold_from_high_with_dwell_stays_high() {
        let result = evaluate_lod(LodState::High, 120.0, 100.0, 10.0, 0.0, 2.0, 0.2);
        assert_eq!(result, LodState::High);
    }

    #[test]
    fn just_above_demote_threshold_with_dwell_demotes() {
        let result = evaluate_lod(LodState::High, 120.001, 100.0, 10.0, 0.0, 2.0, 0.2);
        assert_eq!(result, LodState::Low);
    }
}
