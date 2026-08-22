//! The pure, Bevy-free heart of external repair-team dispatch (issue #1161).
//!
//! Field-repair becomes people crossing over, not a beam. Two things live here
//! and nothing else, the way the tractor's [`crate::tractor::coupling`] keeps its
//! geometry and verdict: the authored `[repair.external_dispatch]` config
//! ([`ExternalRepairConfig`]) — the reach a team can cross and the rate it works
//! a target's condition track at — and the pure **dispatch verdict**
//! ([`dispatch_status`]) that decides, from live scalars the adapter reads off
//! the world, whether a team may be sent this instant and, if not, the one
//! refusal reason the console shows.
//!
//! # Why this is a module of its own, Bevy-free (rule 10)
//!
//! The eligibility decision is made here, in isolation, and unit-tested here;
//! the sibling [`crate::console::repair::external_server`] adapter gathers the
//! real components — the ship's free-team count, its Tactical lock, the
//! separation to that lock — calls in, and applies what comes back, deciding
//! nothing itself. Nothing here imports `bevy`, so the verdict compiles and is
//! tested with no app, no world and no schedule.
//!
//! # The relationship to the internal-sweep availability answer
//!
//! A dispatched team is *held back* from this hull's internal damage-control
//! sweep exactly the way an external operation holds one back (#1027): it counts
//! against `RepairTeams::free_team_indices`. This module owns the eligibility of
//! *sending* one; the count it contributes is owned by the adapter's
//! `ExternalRepairDispatch::committed_repair_teams`, which both the human repair
//! console and the repair AI read off the one idle pool so neither can
//! undercut the other (AGENTS.md rule 6).

use serde::{Deserialize, Serialize};

/// The authored `[repair.external_dispatch]` terms for a hull that can send a
/// repair team abroad (issue #1161).
///
/// Every field is a designer's number, read from TOML: AGENTS.md rule 11, no
/// hardcoded gameplay values. A hull that authors no `[repair.external_dispatch]`
/// table carries no [`crate::console::repair::external_server::ExternalRepairDispatch`]
/// component and cannot dispatch a team abroad — it is unchanged in every way.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalRepairConfig {
    /// The furthest a designated ally or structure may sit from the operator and
    /// still receive a dispatched team, in world units. Dispatching past it is
    /// refused ([`ExternalRepairRefusal::OutOfRange`]); drifting past it once
    /// dispatched brings the team home.
    pub range: f32,
    /// Condition points per second a dispatched team raises the target's OWN
    /// infrastructure condition track by while it works there. Additive: it does
    /// not cancel the target's ordinary decline (the team is repairing, not
    /// arresting), so it composes with a tractor's arrest on the same target —
    /// both push adjustments onto the one condition queue.
    pub repair_rate: f32,
}

impl ExternalRepairConfig {
    /// Reject an authored `[repair.external_dispatch]` table that describes a
    /// dispatch that could never do anything (issue #1161). A non-positive
    /// range, or a non-positive repair rate, are author mistakes whose only
    /// other symptom would be a console control the crew can press and that
    /// quietly never helps anyone.
    pub fn validate(&self) -> Result<(), String> {
        if !self.range.is_finite() || self.range <= 0.0 {
            return Err(format!(
                "[repair.external_dispatch] range must be a positive distance, got {}",
                self.range
            ));
        }
        if !self.repair_rate.is_finite() || self.repair_rate <= 0.0 {
            return Err(format!(
                "[repair.external_dispatch] repair_rate must be a positive rate of condition \
                 points per second, got {}",
                self.repair_rate
            ));
        }
        Ok(())
    }
}

/// The one reason a repair-team dispatch was refused (or a dispatched team was
/// brought home) this tick (issue #1161), as the console shows it — a
/// `strings.csv` id, never English. Mirrors [`crate::tractor::TractorRefusal`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExternalRepairRefusal {
    /// No team is free to send: every team is already out on an internal job or
    /// spoken for by another external commitment.
    NoFreeTeam,
    /// The ship has designated no target, so there is nowhere to send a team.
    NoTarget,
    /// The designated target sits further than the authored `range`.
    OutOfRange,
}

impl ExternalRepairRefusal {
    /// The `strings.csv` id the console resolves through `t()`. A `match`, not a
    /// composed `format!("repair.dispatch.refused.{...}")`, so `check-strings.mjs`
    /// can see every id a new variant needs a row for.
    pub fn string_id(self) -> &'static str {
        match self {
            ExternalRepairRefusal::NoFreeTeam => "repair.dispatch.refused.no_free_team",
            ExternalRepairRefusal::NoTarget => "repair.dispatch.refused.no_target",
            ExternalRepairRefusal::OutOfRange => "repair.dispatch.refused.out_of_range",
        }
    }
}

/// **The dispatch verdict.** `Ok(())` when a team may be sent to the designated
/// target this instant, else the one refusal the console shows (issue #1161).
///
/// Pure: the adapter reads the live world into these scalars and applies the
/// answer. Used at dispatch time (so "no free team / no designated target / out
/// of range is refused") and re-run every tick a dispatch is live with a team
/// already claimed (`has_free_team = true`, `target = Some`), so the only thing
/// that can drop a live dispatch is drifting past the range.
///
/// # Check order is the console's "most actionable first"
///
/// A ship with nobody to send cannot help whatever it designates, so the
/// team-availability check is reported before target acquisition — the same
/// tool-state-before-acquisition order the tractor's `hold_status` takes with
/// its hardware and power gates. Among the acquisition checks there is no range
/// to a target that was never designated, so `NoTarget` precedes `OutOfRange`.
///
/// `separation` is the distance from the operator to the designated target, or
/// `None` when there is no target or the designated entity cannot be found —
/// either way there is nothing in range, which is why a missing separation with
/// a present target still reads as `OutOfRange`.
pub fn dispatch_status(
    has_free_team: bool,
    target: Option<&str>,
    separation: Option<f32>,
    range: f32,
) -> Result<(), ExternalRepairRefusal> {
    if !has_free_team {
        return Err(ExternalRepairRefusal::NoFreeTeam);
    }
    if target.is_none() {
        return Err(ExternalRepairRefusal::NoTarget);
    }
    match separation {
        Some(sep) if sep <= range => Ok(()),
        _ => Err(ExternalRepairRefusal::OutOfRange),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modifiers::repair_teams::RepairTeams;

    // ── The dispatch verdict ─────────────────────────────────────────────────

    #[test]
    fn a_free_team_a_designated_target_in_range_may_be_dispatched() {
        assert_eq!(
            dispatch_status(true, Some("ally"), Some(300.0), 600.0),
            Ok(())
        );
        // Exactly at the range boundary still dispatches.
        assert_eq!(
            dispatch_status(true, Some("ally"), Some(600.0), 600.0),
            Ok(())
        );
    }

    #[test]
    fn no_free_team_refuses_first_of_all() {
        // The tool-state gate wins even when there is also no target and nothing
        // in range: the crew are told the one nearest the tool itself.
        assert_eq!(
            dispatch_status(false, None, None, 600.0),
            Err(ExternalRepairRefusal::NoFreeTeam)
        );
        assert_eq!(
            dispatch_status(false, Some("ally"), Some(10.0), 600.0),
            Err(ExternalRepairRefusal::NoFreeTeam)
        );
    }

    #[test]
    fn no_designated_target_refuses_no_target_before_range() {
        assert_eq!(
            dispatch_status(true, None, None, 600.0),
            Err(ExternalRepairRefusal::NoTarget)
        );
    }

    #[test]
    fn a_target_past_the_authored_range_refuses_out_of_range() {
        assert_eq!(
            dispatch_status(true, Some("ally"), Some(600.1), 600.0),
            Err(ExternalRepairRefusal::OutOfRange)
        );
        // A present target whose entity cannot be found (no separation) is also
        // "nothing in range" — the same reading the tractor takes.
        assert_eq!(
            dispatch_status(true, Some("ally"), None, 600.0),
            Err(ExternalRepairRefusal::OutOfRange)
        );
    }

    // ── Withdrawal from the internal sweep (the #1027 availability answer) ─────

    #[test]
    fn a_dispatched_team_is_withdrawn_from_the_internal_sweep() {
        // Three idle teams, one committed abroad: `free_team_indices` eats one
        // from the TOP, so the internal sweep sees two — the same "one place
        // which teams are available is answered".
        // This is what makes helping an ally a real trade against fixing your own
        // shields: the console and the repair AI both read this number.
        let teams = RepairTeams::new(3);
        assert_eq!(teams.free_team_indices(0), vec![0, 1, 2]);
        assert_eq!(
            teams.free_team_indices(1),
            vec![0, 1],
            "one dispatched team is unavailable to the internal sweep"
        );
        // The dispatched team is still Idle in every readout — held back, not
        // moved — so it is `is_committed_to_operation`, not a busy slot.
        assert!(teams.is_committed_to_operation(2, 1));
        assert!(!teams.is_committed_to_operation(0, 1));
    }

    #[test]
    fn committing_every_team_leaves_the_hull_none_for_its_own_damage() {
        let teams = RepairTeams::new(2);
        assert!(
            teams.free_team_indices(2).is_empty(),
            "two teams both spoken for abroad leaves the hull's own sweep nothing — the \
             capacity-as-cost trade"
        );
    }

    // ── Config validation ────────────────────────────────────────────────────

    #[test]
    fn a_well_formed_external_repair_config_validates() {
        assert!(ExternalRepairConfig {
            range: 600.0,
            repair_rate: 8.0,
        }
        .validate()
        .is_ok());
    }

    #[test]
    fn a_zero_or_negative_range_or_rate_is_rejected() {
        assert!(ExternalRepairConfig {
            range: 0.0,
            repair_rate: 8.0
        }
        .validate()
        .is_err());
        assert!(ExternalRepairConfig {
            range: -1.0,
            repair_rate: 8.0
        }
        .validate()
        .is_err());
        assert!(ExternalRepairConfig {
            range: 600.0,
            repair_rate: 0.0
        }
        .validate()
        .is_err());
        assert!(ExternalRepairConfig {
            range: 600.0,
            repair_rate: -3.0
        }
        .validate()
        .is_err());
    }

    #[test]
    fn the_config_round_trips_through_toml() {
        let authored = r#"
range = 800.0
repair_rate = 12.0
"#;
        let parsed: ExternalRepairConfig =
            toml::from_str(authored).expect("external dispatch config parses");
        assert_eq!(parsed.range, 800.0);
        assert_eq!(parsed.repair_rate, 12.0);
        parsed.validate().expect("valid");
    }

    #[test]
    fn an_unknown_field_is_a_parse_error_rather_than_a_silently_ignored_typo() {
        let err = toml::from_str::<ExternalRepairConfig>("range = 600.0\nrepar_rate = 8.0")
            .expect_err("a misspelt field must not be swallowed");
        assert!(
            err.to_string().contains("repar_rate"),
            "the error must name the offending field, got {err}"
        );
    }
}
