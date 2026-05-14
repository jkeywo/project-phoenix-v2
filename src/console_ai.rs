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
}
