/// Pure AI decision functions for console automation.
///
/// This module is Bevy-free and platform-agnostic. It contains decision
/// functions that replicate the actions a human player would take on a
/// console set to "Low" complexity.
///
/// The Bevy orchestrator lives in `console_ai_plugin`.

use crate::torpedo::TorpedoTubeId;

// ── Frequency hint state ───────────────────────────────────────────────────

/// Persistent timer state for the frequency-hint AI.
///
/// The hint fires once per target lock after `delay_secs` seconds.
/// Reset whenever the locked target changes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FrequencyHintState {
    /// UUID of the target for which the timer is currently running.
    /// `None` means no timer is running.
    pub current_target: Option<String>,
    /// Accumulated elapsed time while the current target has been locked, in
    /// seconds. Resets to 0.0 on target change.
    pub elapsed_secs: f32,
    /// Set to `true` once the hint has fired for the current target. Prevents
    /// repeated hints on the same lock.
    pub hint_sent: bool,
}

/// All inputs required by `tick_frequency_hint`.
#[derive(Clone, Debug)]
pub struct FrequencyHintInput {
    /// UUID of the currently locked target. `None` = no lock.
    pub locked_target: Option<String>,
    /// The recommended phaser frequency to hint (0.0–1.0).
    pub correct_frequency: f32,
    /// Seconds elapsed this frame (delta time).
    pub dt: f32,
    /// Configured delay before the hint fires (from TOML / server config).
    pub delay_secs: f32,
}

/// Outcome of a single `tick_frequency_hint` call.
#[derive(Clone, Debug, PartialEq)]
pub enum FrequencyHintOutput {
    /// No action this tick.
    None,
    /// Emit a `FrequencyHint` with this frequency to the Tactical console.
    Hint { frequency: f32 },
}

/// Advance the frequency-hint timer by one tick.
///
/// Rules:
/// - If there is no locked target, reset the timer and return `None`.
/// - If the locked target changes, reset the timer to 0 and return `None`.
/// - If the timer has not yet reached `delay_secs`, accumulate `dt` and
///   return `None`.
/// - At the first tick where `elapsed_secs >= delay_secs` (and the hint has
///   not already been sent for this target), emit `Hint { frequency }` and
///   mark the hint as sent.
/// - On subsequent ticks with the same target (hint already sent), return
///   `None`.
pub fn tick_frequency_hint(
    state: &mut FrequencyHintState,
    input: &FrequencyHintInput,
) -> FrequencyHintOutput {
    match &input.locked_target {
        None => {
            // No target — clear state.
            *state = FrequencyHintState::default();
            FrequencyHintOutput::None
        }
        Some(uuid) => {
            // Target changed → reset timer.
            if state.current_target.as_deref() != Some(uuid.as_str()) {
                *state = FrequencyHintState {
                    current_target: Some(uuid.clone()),
                    elapsed_secs: 0.0,
                    hint_sent: false,
                };
            }

            if state.hint_sent {
                return FrequencyHintOutput::None;
            }

            state.elapsed_secs += input.dt;
            if state.elapsed_secs >= input.delay_secs {
                state.hint_sent = true;
                FrequencyHintOutput::Hint { frequency: input.correct_frequency }
            } else {
                FrequencyHintOutput::None
            }
        }
    }
}

// ── Input types ────────────────────────────────────────────────────────────

/// State of a single torpedo tube as seen by the AI.
#[derive(Clone, Debug, PartialEq)]
pub struct TubeSummary {
    pub id: TorpedoTubeId,
    /// Tube is loaded and ready to fire.
    pub loaded: bool,
    /// Target bearing (radians from ship forward) is within this tube's arc.
    pub in_arc: bool,
}

/// All inputs required by `auto_fire_torpedo`.
#[derive(Clone, Debug)]
pub struct TorpedoAiInput {
    /// Whether the player has locked a target.
    pub target_locked: bool,
    /// Locked target's combined shield HP. Fire only when this is ≤ 0.
    pub target_shields: i32,
    /// Tubes in canonical priority order: [ForePort, ForeStarboard, Aft].
    pub tubes: [TubeSummary; 3],
    /// Torpedoes remaining in the magazine.
    pub magazine: u32,
}

// ── Decision function ──────────────────────────────────────────────────────

/// Decide which torpedo tubes to fire this tick.
///
/// Auto-fire conditions (ALL must hold):
/// - A target is locked
/// - The locked target's shields ≤ 0
/// - The tube is loaded
/// - The tube is in arc
/// - The magazine has torpedoes remaining
///
/// Returns a list of `TorpedoTubeId` values to fire in deterministic priority
/// order `[ForePort, ForeStarboard, Aft]`.  Each tube that passes all
/// conditions appears at most once in the result.
pub fn auto_fire_torpedo(input: &TorpedoAiInput) -> Vec<TorpedoTubeId> {
    if !input.target_locked || input.target_shields > 0 || input.magazine == 0 {
        return vec![];
    }
    input
        .tubes
        .iter()
        .filter(|t| t.loaded && t.in_arc)
        .map(|t| t.id)
        .collect()
}

// ── Frequency auto-match state ────────────────────────────────────────────

/// Persistent timer state for the frequency auto-match AI.
///
/// When both Tactical and Science are Low (or Science is unmanned), and a
/// target is locked, the AI waits `delay_secs` then synthesises a
/// `SetPhaserFrequency` to match the target's shield frequency.
///
/// Resets whenever the locked target changes or the trigger condition ends.
/// The frequency persists at its last set value when the trigger ends — no
/// auto-revert.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FrequencyMatchState {
    /// UUID of the target for which the timer is currently running.
    /// `None` means no timer is active.
    pub current_target: Option<String>,
    /// Accumulated elapsed time while the current target has been locked, in
    /// seconds. Resets to 0.0 on target change.
    pub elapsed_secs: f32,
    /// Set to `true` once the `SetPhaserFrequency` has been synthesised for the
    /// current target. Prevents repeated matches on the same lock.
    pub match_sent: bool,
}

/// All inputs required by `tick_auto_match_frequency`.
#[derive(Clone, Debug)]
pub struct FrequencyMatchInput {
    /// UUID of the currently locked target. `None` = no lock.
    pub locked_target: Option<String>,
    /// The target's shield frequency to match (0.0–1.0).
    pub target_frequency: f32,
    /// Seconds elapsed this frame (delta time).
    pub dt: f32,
    /// Configured delay before the match fires (from TOML / server config).
    pub delay_secs: f32,
    /// Whether the trigger condition is active (both Tactical and Science Low,
    /// or Science unmanned). When `false`, reset state and return `None`.
    pub trigger_active: bool,
}

/// Outcome of a single `tick_auto_match_frequency` call.
#[derive(Clone, Debug, PartialEq)]
pub enum FrequencyMatchOutput {
    /// No action this tick.
    None,
    /// Synthesise `SetPhaserFrequency` with this frequency.
    Match { frequency: f32 },
}

/// Advance the auto-match timer by one tick.
///
/// Rules:
/// - If `trigger_active` is `false`, reset state and return `None`.
/// - If there is no locked target, reset the timer and return `None`.
/// - If the locked target changes, reset the timer to 0 and return `None`.
/// - If the timer has not yet reached `delay_secs`, accumulate `dt` and
///   return `None`.
/// - At the first tick where `elapsed_secs >= delay_secs` (and the match has
///   not already been sent for this target), emit `Match { frequency }` and
///   mark the match as sent.
/// - On subsequent ticks with the same target (match already sent), return
///   `None`.
pub fn tick_auto_match_frequency(
    state: &mut FrequencyMatchState,
    input: &FrequencyMatchInput,
) -> FrequencyMatchOutput {
    if !input.trigger_active {
        *state = FrequencyMatchState::default();
        return FrequencyMatchOutput::None;
    }

    match &input.locked_target {
        None => {
            *state = FrequencyMatchState::default();
            FrequencyMatchOutput::None
        }
        Some(uuid) => {
            // Target changed → reset timer.
            if state.current_target.as_deref() != Some(uuid.as_str()) {
                *state = FrequencyMatchState {
                    current_target: Some(uuid.clone()),
                    elapsed_secs: 0.0,
                    match_sent: false,
                };
            }

            if state.match_sent {
                return FrequencyMatchOutput::None;
            }

            state.elapsed_secs += input.dt;
            if state.elapsed_secs >= input.delay_secs {
                state.match_sent = true;
                FrequencyMatchOutput::Match { frequency: input.target_frequency }
            } else {
                FrequencyMatchOutput::None
            }
        }
    }
}

// ── Power AI ───────────────────────────────────────────────────────────────

/// Engage state machine used by both Power Low AI rules.
///
/// Each rule independently tracks whether a battery overflow point is
/// currently engaged. Engagement is gated by a sustained-condition timer;
/// disengagement is immediate on battery/condition drop.
#[derive(Clone, Debug, PartialEq, Default)]
pub enum EngageState {
    /// No engagement active.
    #[default]
    Idle,
    /// Condition has been sustained for `elapsed_secs` so far.
    Counting { elapsed_secs: f32 },
    /// Overflow point is currently engaged.
    Engaged,
    /// Just disengaged; waiting for battery to reach `battery_recharge_pct`
    /// before counting can restart.
    WaitingForRecharge,
}

/// All inputs required by `tick_power_movement_rule`.
#[derive(Clone, Debug)]
pub struct PowerMovementInput {
    /// Current forward thrust input (0.0–1.0). Comes from latest `HelmInput`.
    pub thrust: f32,
    /// Minimum thrust to count as "driving" (from TOML `thrust_threshold`).
    pub thrust_threshold: f32,
    /// Seconds sustained thrust must persist before engaging (from TOML).
    pub engage_delay_secs: f32,
    /// Battery % must be ≥ this to allow engaging (from TOML).
    pub battery_engage_min_pct: f32,
    /// Battery % must reach this to allow re-engaging after a disengage (from TOML).
    pub battery_recharge_pct: f32,
    /// Current battery percentage (0.0–100.0).
    pub battery_pct: f32,
    /// Seconds elapsed this frame.
    pub dt: f32,
    /// Whether the Power console is currently at "Low" complexity. When
    /// `false`, any pending engage is cancelled and `Idle` is returned.
    pub power_is_low: bool,
}

/// Outcome of a single `tick_power_movement_rule` call.
#[derive(Clone, Debug, PartialEq)]
pub enum PowerEngageOutput {
    /// No change — +1 overflow was NOT engaged this tick.
    NoChange,
    /// Engage the overflow point: add +1 to the target console.
    Engage,
    /// Disengage the overflow point: remove +1 from the target console.
    Disengage,
}

/// Advance the movement-rule AI state by one tick.
///
/// Rules:
/// - If `power_is_low` is false, reset state to `Idle` and return `NoChange`.
/// - While `Idle`: if thrust ≥ threshold AND battery ≥ engage_min_pct, start
///   counting; otherwise stay `Idle`.
/// - While `Counting`: if condition drops, reset to `Idle`; otherwise
///   accumulate `dt`. When `elapsed >= engage_delay_secs`, transition to
///   `Engaged` and return `Engage`.
/// - While `Engaged`: if battery < engage_min_pct, transition to
///   `WaitingForRecharge` and return `Disengage`. Otherwise `NoChange`.
/// - While `WaitingForRecharge`: if battery ≥ battery_recharge_pct, reset to
///   `Idle`. Still `NoChange` (re-engagement starts from `Idle` next tick).
pub fn tick_power_movement_rule(
    state: &mut EngageState,
    input: &PowerMovementInput,
) -> PowerEngageOutput {
    if !input.power_is_low {
        *state = EngageState::Idle;
        return PowerEngageOutput::NoChange;
    }

    let condition_met = input.thrust >= input.thrust_threshold
        && input.battery_pct >= input.battery_engage_min_pct;

    match state {
        EngageState::Idle => {
            if condition_met {
                *state = EngageState::Counting { elapsed_secs: input.dt };
                // Check if we already crossed the threshold in this tick.
                if input.dt >= input.engage_delay_secs {
                    *state = EngageState::Engaged;
                    return PowerEngageOutput::Engage;
                }
            }
            PowerEngageOutput::NoChange
        }
        EngageState::Counting { elapsed_secs } => {
            if !condition_met {
                *state = EngageState::Idle;
                return PowerEngageOutput::NoChange;
            }
            *elapsed_secs += input.dt;
            if *elapsed_secs >= input.engage_delay_secs {
                *state = EngageState::Engaged;
                PowerEngageOutput::Engage
            } else {
                PowerEngageOutput::NoChange
            }
        }
        EngageState::Engaged => {
            if input.battery_pct < input.battery_engage_min_pct {
                *state = EngageState::WaitingForRecharge;
                PowerEngageOutput::Disengage
            } else {
                PowerEngageOutput::NoChange
            }
        }
        EngageState::WaitingForRecharge => {
            if input.battery_pct >= input.battery_recharge_pct {
                *state = EngageState::Idle;
            }
            PowerEngageOutput::NoChange
        }
    }
}

/// All inputs required by `tick_power_red_alert_rule`.
#[derive(Clone, Debug)]
pub struct PowerRedAlertInput {
    /// Whether red alert is currently active.
    pub red_alert: bool,
    /// Seconds red alert must persist before engaging (from TOML).
    pub engage_delay_secs: f32,
    /// Battery % must be ≥ this to allow engaging.
    pub battery_engage_min_pct: f32,
    /// Battery % must reach this to allow re-engaging after a disengage.
    pub battery_recharge_pct: f32,
    /// Current battery percentage (0.0–100.0).
    pub battery_pct: f32,
    /// Seconds elapsed this frame.
    pub dt: f32,
    /// Whether the Power console is currently at "Low" complexity.
    pub power_is_low: bool,
}

/// Advance the red-alert-rule AI state by one tick.
///
/// Symmetric to `tick_power_movement_rule`, but the condition is
/// `red_alert == true AND battery ≥ engage_min_pct`.
pub fn tick_power_red_alert_rule(
    state: &mut EngageState,
    input: &PowerRedAlertInput,
) -> PowerEngageOutput {
    let movement_input = PowerMovementInput {
        thrust: if input.red_alert { 1.0 } else { 0.0 },
        thrust_threshold: 0.5,
        engage_delay_secs: input.engage_delay_secs,
        battery_engage_min_pct: input.battery_engage_min_pct,
        battery_recharge_pct: input.battery_recharge_pct,
        battery_pct: input.battery_pct,
        dt: input.dt,
        power_is_low: input.power_is_low,
    };
    tick_power_movement_rule(state, &movement_input)
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── FrequencyHintState helpers ────────────────────────────────────────

    fn hint_input(target: Option<&str>, frequency: f32, dt: f32, delay: f32) -> FrequencyHintInput {
        FrequencyHintInput {
            locked_target: target.map(str::to_owned),
            correct_frequency: frequency,
            dt,
            delay_secs: delay,
        }
    }

    // ── tick_frequency_hint ───────────────────────────────────────────────

    #[test]
    fn no_target_returns_none_and_resets_state() {
        let mut state = FrequencyHintState::default();
        let out = tick_frequency_hint(&mut state, &hint_input(None, 0.5, 1.0, 3.0));
        assert_eq!(out, FrequencyHintOutput::None);
        assert!(state.current_target.is_none());
    }

    #[test]
    fn under_delay_returns_none() {
        let mut state = FrequencyHintState::default();
        // 1s tick with 3s delay → no hint
        let out = tick_frequency_hint(&mut state, &hint_input(Some("t1"), 0.5, 1.0, 3.0));
        assert_eq!(out, FrequencyHintOutput::None);
        assert!(!state.hint_sent);
    }

    #[test]
    fn at_delay_fires_hint_with_correct_frequency() {
        let mut state = FrequencyHintState::default();
        // 3s tick with 3s delay → fires
        let out = tick_frequency_hint(&mut state, &hint_input(Some("t1"), 0.75, 3.0, 3.0));
        assert_eq!(out, FrequencyHintOutput::Hint { frequency: 0.75 });
        assert!(state.hint_sent);
    }

    #[test]
    fn above_delay_fires_hint() {
        let mut state = FrequencyHintState::default();
        // Two ticks: 2s + 2s > 3s → fires on second tick
        tick_frequency_hint(&mut state, &hint_input(Some("t1"), 0.5, 2.0, 3.0));
        let out = tick_frequency_hint(&mut state, &hint_input(Some("t1"), 0.5, 2.0, 3.0));
        assert_eq!(out, FrequencyHintOutput::Hint { frequency: 0.5 });
    }

    #[test]
    fn hint_fires_only_once_per_target_lock() {
        let mut state = FrequencyHintState::default();
        // Fire hint
        tick_frequency_hint(&mut state, &hint_input(Some("t1"), 0.5, 5.0, 3.0));
        // Second tick same target → no additional hint
        let out = tick_frequency_hint(&mut state, &hint_input(Some("t1"), 0.5, 5.0, 3.0));
        assert_eq!(out, FrequencyHintOutput::None);
    }

    #[test]
    fn target_change_resets_timer() {
        let mut state = FrequencyHintState::default();
        // Almost at delay with t1
        tick_frequency_hint(&mut state, &hint_input(Some("t1"), 0.5, 2.9, 3.0));
        assert!(!state.hint_sent);
        // Switch to t2 → timer resets
        let out = tick_frequency_hint(&mut state, &hint_input(Some("t2"), 0.5, 2.9, 3.0));
        assert_eq!(out, FrequencyHintOutput::None);
        assert_eq!(state.current_target.as_deref(), Some("t2"));
        // Only 2.9s elapsed for t2 → still no hint
        assert!(!state.hint_sent);
    }

    #[test]
    fn target_change_then_delay_fires_hint_for_new_target() {
        let mut state = FrequencyHintState::default();
        // Nearly full for t1
        tick_frequency_hint(&mut state, &hint_input(Some("t1"), 0.5, 2.9, 3.0));
        // Switch targets → timer resets, accumulate enough for t2
        tick_frequency_hint(&mut state, &hint_input(Some("t2"), 0.9, 1.0, 3.0));
        tick_frequency_hint(&mut state, &hint_input(Some("t2"), 0.9, 1.0, 3.0));
        let out = tick_frequency_hint(&mut state, &hint_input(Some("t2"), 0.9, 1.5, 3.0));
        // elapsed = 3.5 >= 3.0 → hint
        assert_eq!(out, FrequencyHintOutput::Hint { frequency: 0.9 });
    }

    #[test]
    fn clearing_target_resets_hint_sent_flag() {
        let mut state = FrequencyHintState::default();
        // Fire hint for t1
        tick_frequency_hint(&mut state, &hint_input(Some("t1"), 0.5, 5.0, 3.0));
        assert!(state.hint_sent);
        // Clear target → reset
        tick_frequency_hint(&mut state, &hint_input(None, 0.5, 1.0, 3.0));
        assert!(!state.hint_sent);
        assert!(state.current_target.is_none());
    }

    // ── tick_auto_match_frequency ─────────────────────────────────────────

    fn match_input(
        target: Option<&str>,
        frequency: f32,
        dt: f32,
        delay: f32,
        trigger_active: bool,
    ) -> FrequencyMatchInput {
        FrequencyMatchInput {
            locked_target: target.map(str::to_owned),
            target_frequency: frequency,
            dt,
            delay_secs: delay,
            trigger_active,
        }
    }

    #[test]
    fn auto_match_trigger_inactive_returns_none_and_resets() {
        let mut state = FrequencyMatchState {
            current_target: Some("t1".into()),
            elapsed_secs: 10.0,
            match_sent: false,
        };
        let out = tick_auto_match_frequency(&mut state, &match_input(Some("t1"), 0.5, 1.0, 3.0, false));
        assert_eq!(out, FrequencyMatchOutput::None);
        assert!(state.current_target.is_none());
        assert_eq!(state.elapsed_secs, 0.0);
    }

    #[test]
    fn auto_match_no_target_returns_none_and_resets() {
        let mut state = FrequencyMatchState::default();
        let out = tick_auto_match_frequency(&mut state, &match_input(None, 0.5, 1.0, 3.0, true));
        assert_eq!(out, FrequencyMatchOutput::None);
        assert!(state.current_target.is_none());
    }

    #[test]
    fn auto_match_under_delay_returns_none() {
        let mut state = FrequencyMatchState::default();
        let out = tick_auto_match_frequency(&mut state, &match_input(Some("t1"), 0.5, 1.0, 3.0, true));
        assert_eq!(out, FrequencyMatchOutput::None);
        assert!(!state.match_sent);
    }

    #[test]
    fn auto_match_at_delay_fires_with_correct_frequency() {
        let mut state = FrequencyMatchState::default();
        let out = tick_auto_match_frequency(&mut state, &match_input(Some("t1"), 0.75, 3.0, 3.0, true));
        assert_eq!(out, FrequencyMatchOutput::Match { frequency: 0.75 });
        assert!(state.match_sent);
    }

    #[test]
    fn auto_match_above_delay_fires() {
        let mut state = FrequencyMatchState::default();
        tick_auto_match_frequency(&mut state, &match_input(Some("t1"), 0.5, 2.0, 3.0, true));
        let out = tick_auto_match_frequency(&mut state, &match_input(Some("t1"), 0.5, 2.0, 3.0, true));
        assert_eq!(out, FrequencyMatchOutput::Match { frequency: 0.5 });
    }

    #[test]
    fn auto_match_fires_only_once_per_target() {
        let mut state = FrequencyMatchState::default();
        tick_auto_match_frequency(&mut state, &match_input(Some("t1"), 0.5, 5.0, 3.0, true));
        let out = tick_auto_match_frequency(&mut state, &match_input(Some("t1"), 0.5, 5.0, 3.0, true));
        assert_eq!(out, FrequencyMatchOutput::None);
    }

    #[test]
    fn auto_match_target_change_resets_timer() {
        let mut state = FrequencyMatchState::default();
        tick_auto_match_frequency(&mut state, &match_input(Some("t1"), 0.5, 2.9, 3.0, true));
        let out = tick_auto_match_frequency(&mut state, &match_input(Some("t2"), 0.5, 2.9, 3.0, true));
        assert_eq!(out, FrequencyMatchOutput::None);
        assert_eq!(state.current_target.as_deref(), Some("t2"));
        assert!(!state.match_sent);
    }

    #[test]
    fn auto_match_target_change_then_delay_fires_for_new_target() {
        let mut state = FrequencyMatchState::default();
        tick_auto_match_frequency(&mut state, &match_input(Some("t1"), 0.5, 2.9, 3.0, true));
        // Switch target — timer resets to 0, accumulate enough for t2
        tick_auto_match_frequency(&mut state, &match_input(Some("t2"), 0.9, 1.0, 3.0, true));
        tick_auto_match_frequency(&mut state, &match_input(Some("t2"), 0.9, 1.0, 3.0, true));
        let out = tick_auto_match_frequency(&mut state, &match_input(Some("t2"), 0.9, 1.5, 3.0, true));
        assert_eq!(out, FrequencyMatchOutput::Match { frequency: 0.9 });
    }

    #[test]
    fn auto_match_trigger_flip_to_inactive_resets_state() {
        let mut state = FrequencyMatchState::default();
        // Nearly at delay
        tick_auto_match_frequency(&mut state, &match_input(Some("t1"), 0.5, 2.9, 3.0, true));
        // Trigger turns off (e.g. either console goes Full)
        let out = tick_auto_match_frequency(&mut state, &match_input(Some("t1"), 0.5, 1.0, 3.0, false));
        assert_eq!(out, FrequencyMatchOutput::None);
        assert!(state.current_target.is_none(), "state must reset when trigger deactivates");
        assert_eq!(state.elapsed_secs, 0.0);
    }

    #[test]
    fn auto_match_no_auto_revert_after_match_sent_trigger_ends() {
        let mut state = FrequencyMatchState::default();
        // Fire the match
        tick_auto_match_frequency(&mut state, &match_input(Some("t1"), 0.5, 5.0, 3.0, true));
        assert!(state.match_sent);
        // Trigger ends — state resets but we are NOT emitting a revert
        let out = tick_auto_match_frequency(&mut state, &match_input(Some("t1"), 0.5, 1.0, 3.0, false));
        // Must NOT emit a Match (which would be a revert) and must just return None
        assert_eq!(out, FrequencyMatchOutput::None,
            "frequency must persist at last value — no auto-revert when trigger ends");
    }

    fn tube(id: TorpedoTubeId, loaded: bool, in_arc: bool) -> TubeSummary {
        TubeSummary { id, loaded, in_arc }
    }

    fn all_ready_input() -> TorpedoAiInput {
        TorpedoAiInput {
            target_locked: true,
            target_shields: 0,
            tubes: [
                tube(TorpedoTubeId::ForePort, true, true),
                tube(TorpedoTubeId::ForeStarboard, true, true),
                tube(TorpedoTubeId::Aft, true, true),
            ],
            magazine: 10,
        }
    }

    // ── Condition: shields ─────────────────────────────────────────────────

    #[test]
    fn shields_above_zero_no_fire() {
        let mut input = all_ready_input();
        input.target_shields = 50;
        let result = auto_fire_torpedo(&input);
        assert!(result.is_empty(), "should not fire when target shields > 0");
    }

    #[test]
    fn shields_at_zero_fires() {
        let mut input = all_ready_input();
        input.target_shields = 0;
        let result = auto_fire_torpedo(&input);
        assert!(!result.is_empty(), "should fire when target shields == 0");
    }

    #[test]
    fn shields_below_zero_fires() {
        let mut input = all_ready_input();
        input.target_shields = -5;
        let result = auto_fire_torpedo(&input);
        assert!(!result.is_empty(), "should fire when target shields < 0");
    }

    // ── Condition: target lock ─────────────────────────────────────────────

    #[test]
    fn no_target_lock_no_fire() {
        let mut input = all_ready_input();
        input.target_locked = false;
        let result = auto_fire_torpedo(&input);
        assert!(result.is_empty(), "should not fire when no target is locked");
    }

    // ── Condition: magazine ────────────────────────────────────────────────

    #[test]
    fn empty_magazine_no_fire() {
        let mut input = all_ready_input();
        input.magazine = 0;
        let result = auto_fire_torpedo(&input);
        assert!(result.is_empty(), "should not fire when magazine is empty");
    }

    // ── Condition: loaded ──────────────────────────────────────────────────

    #[test]
    fn unloaded_tube_not_in_result() {
        let mut input = all_ready_input();
        // Only fore-port is unloaded
        input.tubes[0].loaded = false;
        let result = auto_fire_torpedo(&input);
        assert!(
            !result.contains(&TorpedoTubeId::ForePort),
            "unloaded ForePort should not appear in result"
        );
    }

    #[test]
    fn all_tubes_unloaded_no_fire() {
        let mut input = all_ready_input();
        for t in &mut input.tubes {
            t.loaded = false;
        }
        let result = auto_fire_torpedo(&input);
        assert!(result.is_empty(), "should not fire when all tubes unloaded");
    }

    // ── Condition: arc ─────────────────────────────────────────────────────

    #[test]
    fn tube_not_in_arc_not_in_result() {
        let mut input = all_ready_input();
        // Only fore-starboard is out of arc
        input.tubes[1].in_arc = false;
        let result = auto_fire_torpedo(&input);
        assert!(
            !result.contains(&TorpedoTubeId::ForeStarboard),
            "out-of-arc ForeStarboard should not appear in result"
        );
    }

    #[test]
    fn no_tube_in_arc_no_fire() {
        let mut input = all_ready_input();
        for t in &mut input.tubes {
            t.in_arc = false;
        }
        let result = auto_fire_torpedo(&input);
        assert!(result.is_empty(), "should not fire when no tube in arc");
    }

    // ── Tube priority ──────────────────────────────────────────────────────

    #[test]
    fn priority_order_is_fore_port_then_fore_starboard_then_aft() {
        let result = auto_fire_torpedo(&all_ready_input());
        assert_eq!(
            result,
            vec![TorpedoTubeId::ForePort, TorpedoTubeId::ForeStarboard, TorpedoTubeId::Aft],
            "tubes must appear in deterministic priority order"
        );
    }

    #[test]
    fn only_aft_ready_returns_just_aft() {
        let mut input = all_ready_input();
        input.tubes[0].loaded = false; // ForePort unloaded
        input.tubes[1].in_arc = false; // ForeStarboard out of arc
        let result = auto_fire_torpedo(&input);
        assert_eq!(result, vec![TorpedoTubeId::Aft]);
    }

    #[test]
    fn fore_port_and_aft_ready_returns_in_order() {
        let mut input = all_ready_input();
        input.tubes[1].loaded = false; // ForeStarboard unloaded
        let result = auto_fire_torpedo(&input);
        assert_eq!(result, vec![TorpedoTubeId::ForePort, TorpedoTubeId::Aft]);
    }

    // ── Power movement rule helpers ───────────────────────────────────────

    fn movement_input(thrust: f32, battery_pct: f32, dt: f32) -> PowerMovementInput {
        PowerMovementInput {
            thrust,
            thrust_threshold: 0.7,
            engage_delay_secs: 3.0,
            battery_engage_min_pct: 50.0,
            battery_recharge_pct: 100.0,
            battery_pct,
            dt,
            power_is_low: true,
        }
    }

    // ── tick_power_movement_rule ──────────────────────────────────────────

    #[test]
    fn thrust_below_threshold_no_engage() {
        let mut state = EngageState::Idle;
        let out = tick_power_movement_rule(&mut state, &movement_input(0.5, 80.0, 1.0));
        assert_eq!(out, PowerEngageOutput::NoChange);
        assert_eq!(state, EngageState::Idle);
    }

    #[test]
    fn thrust_at_threshold_with_battery_starts_counting() {
        let mut state = EngageState::Idle;
        let out = tick_power_movement_rule(&mut state, &movement_input(0.7, 80.0, 1.0));
        assert_eq!(out, PowerEngageOutput::NoChange);
        assert!(matches!(state, EngageState::Counting { .. }));
    }

    #[test]
    fn sustained_thrust_and_battery_engages_after_delay() {
        let mut state = EngageState::Idle;
        // 2 ticks of 1.5s = 3.0s total ≥ engage_delay_secs(3.0)
        tick_power_movement_rule(&mut state, &movement_input(0.8, 80.0, 1.5));
        let out = tick_power_movement_rule(&mut state, &movement_input(0.8, 80.0, 1.5));
        assert_eq!(out, PowerEngageOutput::Engage, "should engage after delay elapsed");
        assert_eq!(state, EngageState::Engaged);
    }

    #[test]
    fn thrust_drop_before_engage_resets_timer() {
        let mut state = EngageState::Idle;
        // Start counting
        tick_power_movement_rule(&mut state, &movement_input(0.8, 80.0, 1.0));
        assert!(matches!(state, EngageState::Counting { .. }));
        // Thrust drops below threshold
        let out = tick_power_movement_rule(&mut state, &movement_input(0.3, 80.0, 1.0));
        assert_eq!(out, PowerEngageOutput::NoChange);
        assert_eq!(state, EngageState::Idle, "timer should reset when thrust drops");
    }

    #[test]
    fn battery_dip_below_min_during_engage_disengages() {
        let mut state = EngageState::Engaged;
        // Battery drops below 50%
        let out = tick_power_movement_rule(&mut state, &movement_input(0.8, 40.0, 1.0));
        assert_eq!(out, PowerEngageOutput::Disengage);
        assert_eq!(state, EngageState::WaitingForRecharge);
    }

    #[test]
    fn waiting_for_recharge_no_re_engage_until_full() {
        let mut state = EngageState::WaitingForRecharge;
        // Battery at 80% — below recharge_pct (100%)
        let out = tick_power_movement_rule(&mut state, &movement_input(0.8, 80.0, 1.0));
        assert_eq!(out, PowerEngageOutput::NoChange);
        assert_eq!(state, EngageState::WaitingForRecharge);
    }

    #[test]
    fn recharge_to_full_transitions_to_idle_allowing_re_engage() {
        let mut state = EngageState::WaitingForRecharge;
        // Battery reaches 100% (recharge_pct)
        let out = tick_power_movement_rule(&mut state, &movement_input(0.8, 100.0, 1.0));
        assert_eq!(out, PowerEngageOutput::NoChange);
        assert_eq!(state, EngageState::Idle, "should return to Idle when battery recharged");
    }

    #[test]
    fn switching_to_full_complexity_cancels_pending_engage() {
        let mut state = EngageState::Counting { elapsed_secs: 2.9 };
        let mut input = movement_input(0.8, 80.0, 0.5);
        input.power_is_low = false; // switched to Full
        let out = tick_power_movement_rule(&mut state, &input);
        assert_eq!(out, PowerEngageOutput::NoChange);
        assert_eq!(state, EngageState::Idle, "pending engage cancelled when switching to Full");
    }

    #[test]
    fn switching_to_full_while_engaged_disengages_immediately() {
        // The plugin must disengage before cancelling. This tests that
        // the state machine goes to Idle when power_is_low is false,
        // and the plugin is responsible for synthesising the Disengage action
        // when the console goes Full while engaged.
        let mut state = EngageState::Engaged;
        let mut input = movement_input(0.8, 80.0, 1.0);
        input.power_is_low = false;
        let out = tick_power_movement_rule(&mut state, &input);
        assert_eq!(out, PowerEngageOutput::NoChange);
        assert_eq!(state, EngageState::Idle, "state resets to Idle on Full");
    }

    #[test]
    fn low_battery_prevents_counting() {
        let mut state = EngageState::Idle;
        // Thrust high but battery below minimum (50%)
        let out = tick_power_movement_rule(&mut state, &movement_input(0.9, 40.0, 1.0));
        assert_eq!(out, PowerEngageOutput::NoChange);
        assert_eq!(state, EngageState::Idle, "should not count when battery too low");
    }

    // ── tick_power_red_alert_rule ─────────────────────────────────────────

    fn red_alert_input(red_alert: bool, battery_pct: f32, dt: f32) -> PowerRedAlertInput {
        PowerRedAlertInput {
            red_alert,
            engage_delay_secs: 3.0,
            battery_engage_min_pct: 10.0,
            battery_recharge_pct: 100.0,
            battery_pct,
            dt,
            power_is_low: true,
        }
    }

    #[test]
    fn no_red_alert_no_weapons_engage() {
        let mut state = EngageState::Idle;
        let out = tick_power_red_alert_rule(&mut state, &red_alert_input(false, 80.0, 1.0));
        assert_eq!(out, PowerEngageOutput::NoChange);
        assert_eq!(state, EngageState::Idle);
    }

    #[test]
    fn sustained_red_alert_and_battery_engages_weapons() {
        let mut state = EngageState::Idle;
        tick_power_red_alert_rule(&mut state, &red_alert_input(true, 50.0, 1.5));
        let out = tick_power_red_alert_rule(&mut state, &red_alert_input(true, 50.0, 1.5));
        assert_eq!(out, PowerEngageOutput::Engage, "should engage after delay under red alert");
    }

    #[test]
    fn red_alert_battery_dip_disengages_weapons() {
        let mut state = EngageState::Engaged;
        // Battery drops below 10% (min for red alert rule)
        let out = tick_power_red_alert_rule(&mut state, &red_alert_input(true, 5.0, 1.0));
        assert_eq!(out, PowerEngageOutput::Disengage);
        assert_eq!(state, EngageState::WaitingForRecharge);
    }

    #[test]
    fn red_alert_re_engage_gated_on_recharge_pct() {
        let mut state = EngageState::WaitingForRecharge;
        // At 80%, recharge_pct=100% → still waiting
        tick_power_red_alert_rule(&mut state, &red_alert_input(true, 80.0, 1.0));
        assert_eq!(state, EngageState::WaitingForRecharge);
        // At 100%, transitions to Idle
        tick_power_red_alert_rule(&mut state, &red_alert_input(true, 100.0, 1.0));
        assert_eq!(state, EngageState::Idle);
    }

    // ── Both rules stacking ───────────────────────────────────────────────

    #[test]
    fn both_rules_can_engage_simultaneously() {
        let mut helm_state = EngageState::Idle;
        let mut weapons_state = EngageState::Idle;

        // Sustained thrust + red alert + full battery → both engage after delay
        for _ in 0..3 {
            tick_power_movement_rule(&mut helm_state, &movement_input(0.9, 80.0, 1.0));
            tick_power_red_alert_rule(&mut weapons_state, &red_alert_input(true, 80.0, 1.0));
        }

        // At 3s elapsed both should be Engaged now
        assert_eq!(helm_state, EngageState::Engaged, "movement rule should be Engaged");
        assert_eq!(weapons_state, EngageState::Engaged, "red alert rule should be Engaged");
    }
}
