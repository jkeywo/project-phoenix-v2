//! The commitments ledger (issue #1029, parent #851 "Falling Skyway").
//!
//! The captain makes promises. A negotiation beat ends with *"you have my word
//! — safe passage for your people"*, and three scenes later that word is either
//! kept or it is not. This module is the book those promises are written in: who
//! a promise was made to, what its terms are, what would count as keeping it,
//! and whether it ended up kept or broken.
//!
//! # THIS IS NOT A SCHEDULER, AND IT IS NOT AN EVALUATOR
//!
//! Two properties this module must be held to, both inherited from
//! [`crate::world::deadlines`] and both easy to erode:
//!
//! 1. **It owns no queue and no timer.** A commitment stores no due time. A
//!    promise-by-time — *"we will be there before the transfer window closes"* —
//!    is authored as a `[[deadline]]` whose handler resolves the commitment:
//!
//!    ```rhai
//!    on_deadline("transfer_window_closes", "on_window_closed");
//!
//!    fn on_window_closed(ctx) {
//!        if ctx.commitments.state("safe_passage") == "open" {
//!            ctx.commitments.break_promise("safe_passage");
//!        }
//!    }
//!    ```
//!
//!    The deadline table already owns the one queued call; the ledger owns the
//!    record. Composing them costs a four-line handler and adds nothing that
//!    ticks. A private `due_tick` on a [`Commitment`] would be a second
//!    deferred-work queue in a codebase that has decided it will have exactly
//!    two — and it would need a per-tick scan to notice its own expiry, which is
//!    precisely the thing #1024 refused to add. There is deliberately not even a
//!    `deadline_id` cross-reference field: the pairing is stated once, in the
//!    handler above, and a field here would invite a reader to believe the
//!    ledger enforces something it does not.
//!
//! 2. **It does not evaluate [`Commitment::resolves_when`].** That field is a
//!    `strings.csv` id *stating* what counts as keeping the promise — the half
//!    of the bargain a UI shows under the terms, and the half an author writes
//!    down so the scenario's intent survives contact with a later editor.
//!    Nothing here polls it, and no system this slice adds scans the ledger
//!    looking for promises that have quietly come good. Script resolves, at the
//!    beat where the fiction says the promise was tested.
//!
//! # Resolution writes campaign flags
//!
//! Keeping or breaking a promise sets a world flag —
//! [`kept_flag`]/[`broken_flag`], `commitment.<id>.kept` and
//! `commitment.<id>.broken` — through the **existing** flag path: the script
//! surface pushes an ordinary
//! [`ActionCmd::MutateFlag`](crate::world::dispatch::ActionCmd::MutateFlag) onto
//! the same ordered buffer `ctx.flags.x = 1` uses. So a resolution is a world
//! flag like any other, `push_flag_transition` emits its
//! [`WorldEvent::FlagSet`](crate::world::content::WorldEvent::FlagSet) on the
//! boolean flip, and an `on_flag_set` trigger authored against it chains without
//! this module knowing that triggers exist. A promise therefore has consequences
//! beyond the scene it was made in, and it has them through machinery that was
//! already there.
//!
//! # Three states, never two
//!
//! [`CommitmentState`] separates `Open` from `Broken` because *"not yet"* and
//! *"failed"* are different facts about the captain, and a mission that folds
//! them together cannot tell an unfinished errand from a betrayal. An id the
//! ledger has never heard of is a fourth answer again — see
//! [`CommitmentLedger::state_of`].
//!
//! # What the ledger holds IS the account
//!
//! Every field on a [`Commitment`] is something that was actually promised.
//! There is deliberately no "real terms versus stated terms" pair and no hidden
//! field: a scenario in which the crew is misled about what they agreed to
//! authors the misleading line as its own dialogue content, exactly as
//! [`InfrastructureSnapshot`](crate::core::messages::InfrastructureSnapshot) refuses a
//! reported-versus-actual pair and sends a contradicting maintenance dossier as
//! content instead. That is what makes the whole record safe to hand to a later
//! UI: inspecting it cannot reveal anything the crew was not already told.

use serde::{Deserialize, Serialize};

/// Where a promise stands.
///
/// `Open` is the default because a promise that has been made and not yet
/// tested is exactly that — owed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommitmentState {
    /// Made, and not yet resolved either way. **Not** a failure — see the module
    /// docs on why this is distinct from [`Broken`](Self::Broken).
    #[default]
    Open,
    /// The captain kept their word.
    Kept,
    /// The captain did not.
    Broken,
}

impl CommitmentState {
    /// The script/wire label. Written by hand rather than derived, so the exact
    /// strings a scenario compares against are visible at the point they are
    /// promised.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Kept => "kept",
            Self::Broken => "broken",
        }
    }
}

/// How a promise was resolved — the two outcomes script can ask for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommitmentOutcome {
    /// Kept: writes [`kept_flag`].
    Kept,
    /// Broken: writes [`broken_flag`].
    Broken,
}

impl CommitmentOutcome {
    /// The state a resolution moves a record to.
    fn state(self) -> CommitmentState {
        match self {
            Self::Kept => CommitmentState::Kept,
            Self::Broken => CommitmentState::Broken,
        }
    }

    /// The campaign flag this outcome sets on `id`.
    fn flag(self, id: &str) -> String {
        match self {
            Self::Kept => kept_flag(id),
            Self::Broken => broken_flag(id),
        }
    }
}

/// The campaign flag set when `id` is kept: `commitment.<id>.kept`.
///
/// A free function rather than an inlined `format!` at each site, because the
/// name is a contract with scenario authors — an `on_flag_set` trigger is
/// written against this exact string — and a contract stated in two places can
/// be changed in one of them.
pub fn kept_flag(id: &str) -> String {
    format!("commitment.{id}.kept")
}

/// The campaign flag set when `id` is broken: `commitment.<id>.broken`.
///
/// A *separate* flag rather than a value on the kept one, for the reason
/// [`CommitmentState`] has three variants: an `on_flag_set` trigger firing on
/// "the promise resolved" cannot tell the mission which way it went, and the two
/// outcomes are the whole point.
pub fn broken_flag(id: &str) -> String {
    format!("commitment.{id}.broken")
}

/// One promise on the books.
///
/// Recorded at runtime, in the dialogue beat where the captain gives their word
/// — there is no `[[commitment]]` block. A promise is not a thing a world file
/// can declare, because whether it exists at all depends on what the player
/// chose to say.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Commitment {
    /// Stable id, unique within a run. Script names the promise by this, and it
    /// is the `<id>` in [`kept_flag`]/[`broken_flag`].
    pub id: String,
    /// The party the promise was made to: an entity name or a faction name, as
    /// the script wrote it.
    ///
    /// **Deliberately not resolved to a UUID.** A promise is made to a *party*,
    /// not to an entity handle: the ship you gave your word to can be destroyed,
    /// jump out, or never have been a single hull in the first place (*"the
    /// Skyway strike committee"*), and the promise outlives all three. A
    /// resolved handle would make the ledger's memory shorter than the fiction's
    /// — and it is what forces the record to be buffered through a name-resolving
    /// applier, which a pure module cannot be.
    pub made_to: String,
    /// `strings.csv` id for the terms — what was promised, in the crew's words.
    /// Never English (AGENTS.md rule 11's display-text exception).
    pub terms: String,
    /// `strings.csv` id stating what would count as keeping it.
    ///
    /// Declared, never evaluated — see the module docs. It exists so the bargain
    /// is data rather than an implication of whichever handler happens to call
    /// [`resolve`](CommitmentLedger::resolve).
    #[serde(default)]
    pub resolves_when: String,
    /// Open / kept / broken.
    #[serde(default)]
    pub state: CommitmentState,
    /// The `SimTick` the promise was made on.
    pub made_at_tick: u64,
    /// The `SimTick` it was resolved on, or `None` while it is still open.
    ///
    /// Stamped rather than derived so a later debrief can order a run's promises
    /// against its other events without re-simulating it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_at_tick: Option<u64>,
}

/// Recording a promise under an id the ledger already holds.
///
/// An error rather than a silent overwrite or a silent no-op, because both of
/// those lie: overwriting discards terms the crew was actually given, and
/// ignoring the second record leaves the mission believing it made a promise
/// whose terms are somebody else's. A scenario that can reach the same promise
/// twice guards it — `if ctx.commitments.state("safe_passage") == "unknown"` —
/// which is one line, and is one line the author has to have thought about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DuplicateCommitment {
    /// The id that was already on the books.
    pub id: String,
}

impl std::fmt::Display for DuplicateCommitment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "commitment '{}' is already on the books; ids are unique within a run \
             (guard with ctx.commitments.state(\"{}\") == \"unknown\")",
            self.id, self.id
        )
    }
}

/// What a script asked of the ledger.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommitmentMutation {
    /// Write a new promise onto the books.
    Record {
        /// The party it was made to.
        made_to: String,
        /// `strings.csv` id for the terms.
        terms: String,
        /// `strings.csv` id for what counts as keeping it.
        resolves_when: String,
    },
    /// Close an open promise, one way or the other.
    Resolve {
        /// Kept or broken.
        outcome: CommitmentOutcome,
    },
}

/// One buffered `ctx.commitments.record(…)` / `.keep(…)` / `.break_promise(…)`,
/// in authored order.
///
/// Travels on
/// [`CallEffects::commitment_changes`](crate::world::script::schedule::CallEffects::commitment_changes)
/// as its own field for the reason `deadline_changes` is one: the generic action
/// applier holds no handle on the ledger, and a promise is not an `ActionCmd`.
/// Buffered, never deferred — the adapter replays them in the same tick, at the
/// same point as the call's other effects. The campaign flag a resolution writes
/// is the exception that proves it: *that* half is an ordinary `ActionCmd` and
/// rides the ordered effect buffer with everything else.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommitmentChange {
    /// The promise addressed.
    pub id: String,
    /// What to do about it.
    pub mutation: CommitmentMutation,
}

/// Every promise a run has made, in the order they were made.
///
/// A `Vec` rather than a map, for [`DeadlineTable`]'s reason: the order is a
/// deterministic function of what the crew did, it is the order a debrief reads
/// them in, and a `HashMap`'s iteration order must never reach a payload or a
/// fold.
///
/// [`DeadlineTable`]: crate::world::deadlines::DeadlineTable
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CommitmentLedger {
    /// The promises, oldest first.
    #[serde(default)]
    pub records: Vec<Commitment>,
}

impl CommitmentLedger {
    /// The record for `id`, or `None`.
    pub fn get(&self, id: &str) -> Option<&Commitment> {
        self.records.iter().find(|c| c.id == id)
    }

    /// Whether any promise has been made at all.
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// Every promise still owed, oldest first — the inspection surface a later
    /// UI lists.
    ///
    /// Yields whole records because there is nothing on one to withhold (see the
    /// module docs): a caller that can see a promise exists is a caller the crew
    /// already told.
    pub fn open(&self) -> impl Iterator<Item = &Commitment> {
        self.records
            .iter()
            .filter(|c| c.state == CommitmentState::Open)
    }

    /// `"open"` / `"kept"` / `"broken"`, or `"unknown"` for a promise that was
    /// never made.
    ///
    /// Four answers, not three: *"there is no such promise"* is a different fact
    /// from *"there is one and it is still owed"*, and it is the guard a scenario
    /// uses to avoid recording the same promise twice.
    pub fn state_of(&self, id: &str) -> &'static str {
        match self.get(id) {
            Some(c) => c.state.as_str(),
            None => "unknown",
        }
    }

    /// Write a new promise onto the books at `now_tick`.
    ///
    /// `Err(DuplicateCommitment)` if `id` is already recorded — in any state. A
    /// resolved promise still occupies its id: the run made it, and re-using the
    /// name would erase that it was ever kept.
    pub fn record(
        &mut self,
        id: &str,
        made_to: &str,
        terms: &str,
        resolves_when: &str,
        now_tick: u64,
    ) -> Result<(), DuplicateCommitment> {
        if self.get(id).is_some() {
            return Err(DuplicateCommitment { id: id.to_string() });
        }
        self.records.push(Commitment {
            id: id.to_string(),
            made_to: made_to.to_string(),
            terms: terms.to_string(),
            resolves_when: resolves_when.to_string(),
            state: CommitmentState::Open,
            made_at_tick: now_tick,
            resolved_at_tick: None,
        });
        Ok(())
    }

    /// Close `id` as kept or broken at `now_tick`, returning the campaign flag
    /// the caller must set.
    ///
    /// `None` — nothing happened, nothing to write — for an unknown id or for a
    /// promise that is already resolved. Spending a promise twice is a no-op
    /// rather than an error, the way a deadline's second cancel is: the first
    /// resolution is the one that happened, and re-writing it would let a
    /// late-firing handler quietly convert a kept promise into a broken one. The
    /// state a script reads back says which it is.
    pub fn resolve(
        &mut self,
        id: &str,
        outcome: CommitmentOutcome,
        now_tick: u64,
    ) -> Option<String> {
        let record = self.records.iter_mut().find(|c| c.id == id)?;
        if record.state != CommitmentState::Open {
            return None;
        }
        record.state = outcome.state();
        record.resolved_at_tick = Some(now_tick);
        Some(outcome.flag(id))
    }

    /// Apply one buffered mutation, returning the campaign flag a resolution
    /// asks the caller to set (`None` for a record, and for a resolution that
    /// changed nothing).
    ///
    /// The one entry point the script surface and the Bevy adapter share, so the
    /// per-call snapshot and the live ledger cannot diverge in what a mutation
    /// means.
    pub fn apply(
        &mut self,
        change: &CommitmentChange,
        now_tick: u64,
    ) -> Result<Option<String>, DuplicateCommitment> {
        match &change.mutation {
            CommitmentMutation::Record {
                made_to,
                terms,
                resolves_when,
            } => self
                .record(&change.id, made_to, terms, resolves_when, now_tick)
                .map(|()| None),
            CommitmentMutation::Resolve { outcome } => {
                Ok(self.resolve(&change.id, *outcome, now_tick))
            }
        }
    }
}

#[cfg(test)]
#[path = "commitments_tests.rs"]
mod tests;
