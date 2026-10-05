pub use phoenix_sim_gameplay::core::balance::*;

/// Landed-damage-rate floor (HP/sec, summed across both sides) above which a
/// budget-exhausted run is a live `timeout` rather than a stalemate `draw`.
/// Also a reporting heuristic — a hair above zero so a single stray tick of
/// chip damage in the closing window does not read as an active fight.
pub const CLOSING_ACTIVE_RATE: f32 = 0.01;

/// The classified outcome plus the per-side margins that carry the balance
/// signal. Every run gets one (AC1); draw/timeout runs lean on the margins.
#[derive(Debug, Clone, PartialEq)]
pub struct OutcomeReport {
    pub outcome: RunOutcome,
    pub player: SideMargins,
    pub enemy: SideMargins,
    /// The structured post-mission report the scenario authored, or an empty
    /// one when it authored none (issue #1344). Carried HERE rather than beside
    /// the margins in the run report because it is what
    /// [`RunOutcome::Reported`] means: the classification and the rows that
    /// caused it have to travel together, or a reader can see `reported` with
    /// nothing to read.
    pub report: crate::core::report::MissionReport,
}

impl OutcomeReport {
    /// Serialise as `"outcome": ..., "sides": {...}, "report": {...}` (object
    /// body, no outer braces) so the report can inline it between its other
    /// fields.
    ///
    /// `report` is always written, even when empty — the key set is the shape a
    /// reader parses against, and a key that comes and goes is a key every
    /// consumer has to test for twice. An unreported run says
    /// `{"rows": [], "total": 0}`, which states "this scenario authored no
    /// report" rather than leaving a gap.
    pub fn to_json(&self) -> String {
        format!(
            "\"outcome\": {:?},\n  \"sides\": {{\"player\": {}, \"enemy\": {}}},\n  \"report\": {}",
            self.outcome.as_str(),
            self.player.to_json(),
            self.enemy.to_json(),
            self.report.to_json(),
        )
    }
}

/// Classify a finished run. PURE — no ECS, no clock beyond the stamps already
/// folded into the margins — so every outcome is unit-testable (AC3).
///
/// Precedence:
/// 1. The run ended holding a non-empty [`crate::core::report::MissionReport`]
///    → [`RunOutcome::Reported`] (issue #1344). Outranks the victory/defeat
///    branch below for the reason spelled out on the variant: a report-bearing
///    ending is described by its rows, not by a single word, and that is true
///    of a catastrophic ending as much as an orderly one. A run still
///    `InProgress` cannot be report-bearing however many rows it has written —
///    the mission has not ended, so there is nothing to report on yet.
/// 2. Reached `GamePhase::GameOver` → victory or defeat from `outcome_flag`.
///    A scenario `game_over` with **no** declared outcome defaults to
///    **victory**: the scenario ran to a scripted end-state, and the built-in
///    player-death path is the separately-latched [`Outcome::Defeat`]. So an
///    undeclared scripted end is the ship surviving to the finish, not losing.
/// 3. Budget exhausted (still `InProgress`) → draw vs timeout from the closing
///    window: damage still landing means both sides were fighting (timeout); a
///    silent window means mutual ineffectiveness (draw). Both carry the same
///    margin payload.
///
/// `report` is taken by value and moved into the result, so a caller with no
/// report passes `MissionReport::default()` — the empty report, which is
/// exactly "this scenario authored none" and keeps every existing scenario's
/// ending unchanged (AC5).
pub fn classify(
    final_phase_is_game_over: bool,
    outcome_flag: Option<Outcome>,
    player: SideMargins,
    enemy: SideMargins,
    report: crate::core::report::MissionReport,
) -> OutcomeReport {
    let outcome = if final_phase_is_game_over {
        if !report.is_empty() {
            RunOutcome::Reported
        } else {
            match outcome_flag {
                Some(Outcome::Defeat) => RunOutcome::Defeat,
                // Declared victory, or an undeclared scripted end (default victory).
                Some(Outcome::Victory) | None => RunOutcome::Victory,
            }
        }
    } else {
        let closing = player.closing_damage_rate + enemy.closing_damage_rate;
        if closing > CLOSING_ACTIVE_RATE {
            RunOutcome::Timeout
        } else {
            RunOutcome::Draw
        }
    };
    OutcomeReport {
        outcome,
        player,
        enemy,
        report,
    }
}

use bevy::prelude::*;
#[cfg(test)]
#[path = "balance_tests.rs"]
mod tests;
