/// Pure AI decision functions for console automation.
///
/// This module is Bevy-free and platform-agnostic. It contains decision
/// functions that replicate the actions a human player would take on a
/// console set to "Low" complexity.
///
/// The Bevy orchestrator lives in `console_ai_plugin`.

use crate::torpedo::TorpedoTubeId;

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
