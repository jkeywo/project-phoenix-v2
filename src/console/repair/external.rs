//! The pure, Bevy-free heart of external repair-team dispatch (issue #1161).
//!
//! Field-repair becomes people crossing over, not a beam. Two things live here
//! and nothing else, the way the tractor's [`crate::tractor::coupling`] keeps its
//! geometry and verdict: the authored `[repair.external_dispatch]` config
//! ([`ExternalRepairConfig`]) — the reach a team can cross and the rate it works
//! a target's condition track at — and the pure **dispatch verdicts**
//! ([`dispatch_status`] and [`named_dispatch_status`]) that decide, from live
//! scalars the adapter reads off the world, whether a team may be sent this
//! instant and, if not, the one refusal reason the console shows.
//!
//! # Why this is a module of its own, Bevy-free (rule 10)
//!
//! The eligibility decision is made here, in isolation, and unit-tested here;
//! the sibling [`crate::console::repair::external_server`] adapter gathers the
//! real components — whether the ship has anyone free (or, for a named order,
//! whether that slot is idle and which slot is already abroad), its Tactical
//! lock, the separation to that lock — calls in, and applies what comes back,
//! deciding nothing itself. Nothing here imports `bevy`, so the verdict compiles and is
//! tested with no app, no world and no schedule.
//!
//! # The relationship to the internal-sweep availability answer
//!
//! A dispatched team is *held back* from this hull's internal damage-control
//! sweep exactly the way an external operation holds one back (#1027). Since
//! #1386 the claim NAMES the team it holds rather than contributing a count:
//! `RepairTeams::free_team_indices` takes that INDEX and excludes it, instead of
//! truncating the idle tail by a number, and the excluded slot is owned by the
//! adapter's `ExternalRepairDispatch::abroad_team` — the one answer both the
//! human repair console and the repair AI ask, so neither can undercut the other
//! (AGENTS.md rule 6).
//!
//! This module owns both eligibility verdicts: [`dispatch_status`] for the
//! fieldless verb, where the host picks the lowest free team, and
//! [`named_dispatch_status`] for a named one, which reports the two refusals the
//! fieldless verb structurally cannot have — [`ExternalRepairRefusal::TeamBusy`]
//! for a team that is not idle and [`ExternalRepairRefusal::AlreadyAbroad`] for a
//! named dispatch while a DIFFERENT team holds the ship's one claim — before
//! delegating acquisition to the shared verdict.

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
    /// The NAMED team is not idle — it is already out on an internal job, or
    /// walking home from one, or there is no such slot on this hull (issue
    /// #1386). Reachable only from a named dispatch: the fieldless verb picks a
    /// team itself and so can only ever report [`Self::NoFreeTeam`].
    TeamBusy,
    /// A DIFFERENT team already holds this ship's one external claim (issue
    /// #1386). The claim stays single, so the crew recall that team before
    /// sending another; a refusal sends nobody.
    AlreadyAbroad,
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
            ExternalRepairRefusal::TeamBusy => "repair.dispatch.refused.team_busy",
            ExternalRepairRefusal::AlreadyAbroad => "repair.dispatch.refused.already_abroad",
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

/// **The NAMED dispatch verdict** (issue #1386) — may *this* team cross over to
/// the designated target this instant, or the one refusal the console shows.
///
/// The fieldless [`dispatch_status`] beside it answers a different question:
/// there the server picks the team, so the only team-side answer it can give is
/// "nobody is free". A named order says which slot the seat tapped, and the two
/// ways that order can be stale both have to be reportable: the team it names
/// has since taken an internal job ([`ExternalRepairRefusal::TeamBusy`]), or
/// another team has since taken the ship's one external claim
/// ([`ExternalRepairRefusal::AlreadyAbroad`]).
///
/// # Check order
///
/// The team the seat actually tapped is reported before the ship-wide claim:
/// "that team is busy" is the more actionable of the two, and a seat told
/// "another team is already out there" about a team that could not have gone
/// anyway would recall the wrong team. Both precede acquisition, keeping the
/// tool-state-before-acquisition order [`dispatch_status`] documents; the
/// acquisition half is then delegated to it rather than restated, so `NoTarget`
/// still precedes `OutOfRange` by construction.
///
/// `abroad_team == Some(team_idx)` is NOT a refusal: that is the team already
/// abroad being re-pointed at whatever Tactical holds now, the same
/// same-claim re-target the fieldless verb has always allowed.
///
/// `team_is_idle` is false for a slot that does not exist at all, which reads as
/// `TeamBusy` — a team this hull does not have is a team that cannot go, and
/// inventing a sixth refusal for a client that named a slot out of range would
/// give the crew a message about a state no console can produce.
pub fn named_dispatch_status(
    team_is_idle: bool,
    abroad_team: Option<u8>,
    team_idx: u8,
    target: Option<&str>,
    separation: Option<f32>,
    range: f32,
) -> Result<(), ExternalRepairRefusal> {
    if !team_is_idle {
        return Err(ExternalRepairRefusal::TeamBusy);
    }
    if abroad_team.is_some_and(|abroad| abroad != team_idx) {
        return Err(ExternalRepairRefusal::AlreadyAbroad);
    }
    // The named team IS the free team, so the availability gate is already
    // answered; what is left is acquisition, which the shared verdict owns.
    dispatch_status(true, target, separation, range)
}

#[cfg(test)]
#[path = "external_tests.rs"]
mod tests;
