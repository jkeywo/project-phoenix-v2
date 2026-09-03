//! The continuous-task lifecycle vocabulary (issue #1341, PRD #1337).
//!
//! [`crate::core::narrative`] answers "what happened in the story"; this module
//! answers a narrower question the story keeps asking of *ongoing work*: **who
//! started doing what to whom, and how did it end**.
//!
//! A scan, a tractor hold, and every continuous task the later slices of PRD
//! #1337 add (#1345, #1346, #1348, #1350) share one shape:
//!
//! * an **operator** — the hull whose station-owned `[[system]]` is doing it;
//! * a **verb** — what kind of work it is;
//! * an optional **subject** — the entity it is being done to;
//! * a **start**, and exactly **one terminal** moment, with a reason.
//!
//! Nothing here is scan-shaped or tractor-shaped. Those two are the tracer
//! workflows that establish the interface; a later mechanic pushes the same
//! [`TaskLifecycleRequest`]s and gets the same timeline entries without
//! inventing a parallel telemetry.
//!
//! # The activation key
//!
//! [`TaskKey`] is the deterministic identity an activation carries from its
//! start event to its terminal event. It is built from state the fixed tick
//! already decided — the operator uuid, the system id, the verb, the subject
//! uuid — plus an **ordinal**: a per-[`TaskSlot`] counter that separates a
//! restart from the activation it replaced. That is what makes the four cases
//! issue #1341 asks about separately identifiable:
//!
//! | case | what distinguishes it |
//! |---|---|
//! | repeated | the ordinal |
//! | simultaneous | the operator, the system, and the subject |
//! | cancelled | its own terminal event, with reason [`TaskTerminalReason::Released`] |
//! | restarted | the ordinal again — the old key is closed before the new one opens |
//!
//! No clock, no RNG, no allocation address: two hosts running the same seeded
//! tick mint the same key.
//!
//! # Exactly one terminal event
//!
//! [`TaskLifecycles`] is the registry that makes "exactly one" true rather than
//! hoped for. A terminal request for a slot with no live activation is DROPPED,
//! which is what lets several sites report the same ending without coordinating:
//! the AI host reporting a withdrawn order and the command handler reporting the
//! release it caused are the same ending, and the first one through the queue is
//! the one the timeline records.
//!
//! # Determinism
//!
//! Everything here is pure or ordered: [`TaskSlot`] is `Ord` and the registry is
//! a `BTreeMap`, so every sweep over live activations walks in slot order, never
//! in the order they were opened or in a `HashMap`'s.

use bevy::prelude::Resource;
use std::collections::BTreeMap;

use crate::core::narrative::{NarrativeActor, NarrativeEvent, NarrativeKind, NarrativeValue};

/// The verb a science scan's lifecycle is recorded under (issue #1341).
///
/// A code-level semantic identifier, exactly like a `[[system]]` id and never
/// player-visible: the localizable half of a terminal moment is
/// [`TaskTerminalReason::string_id`].
pub const TASK_VERB_SCAN: &str = "scan";

/// The verb a tractor hold's lifecycle is recorded under (issue #1341).
pub const TASK_VERB_TRACTOR_HOLD: &str = "tractor_hold";

/// The verb a docking hold's lifecycle is recorded under (issue #1345). The
/// activation follows the MATE forming (`DockControl::docked` becoming true),
/// not the intent to approach — the dock's analogue of the tractor's coupling.
pub const TASK_VERB_DOCK_HOLD: &str = "dock_hold";

/// The verb a transfer-umbilical flow's lifecycle is recorded under (issue
/// #1345). The activation follows the flow actually moving capacity, not the
/// standing `running` intent — the umbilical's analogue of the tractor's
/// coupling.
pub const TASK_VERB_UMBILICAL_FLOW: &str = "umbilical_flow";

/// The verb an external repair-team dispatch's lifecycle is recorded under
/// (issue #1345). The activation follows the dispatch COMMIT
/// (`ExternalRepairDispatch::dispatched_target` being set), the same
/// dispatch-and-claim shape Security's team assignments use.
pub const TASK_VERB_EXTERNAL_REPAIR: &str = "external_repair";

/// The four ways a continuous task can end, as issue #1341's acceptance
/// criterion names them.
///
/// The CLASS, not the reason: several reasons share a class, and an
/// after-action reading that only wants "did it work" reads this while one that
/// wants "why not" reads [`TaskTerminalReason`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TaskOutcome {
    /// The task did what it set out to do.
    Completed,
    /// Somebody with the authority to stop it stopped it — the operator, or the
    /// scenario order that opened it.
    Cancelled,
    /// The task's own preconditions stopped holding: power, damage, range, or a
    /// subject it could never read.
    Failed,
    /// Something outside the task ended it: the subject left, the activation was
    /// replaced, or the mission did.
    Interrupted,
}

impl TaskOutcome {
    /// The narrative kind a terminal event of this class is recorded as.
    pub fn kind(self) -> NarrativeKind {
        match self {
            TaskOutcome::Completed => NarrativeKind::TaskCompleted,
            TaskOutcome::Cancelled => NarrativeKind::TaskCancelled,
            TaskOutcome::Failed => NarrativeKind::TaskFailed,
            TaskOutcome::Interrupted => NarrativeKind::TaskInterrupted,
        }
    }

    /// The stable snake_case label written into JSON and ndjson.
    pub fn as_str(self) -> &'static str {
        match self {
            TaskOutcome::Completed => "completed",
            TaskOutcome::Cancelled => "cancelled",
            TaskOutcome::Failed => "failed",
            TaskOutcome::Interrupted => "interrupted",
        }
    }
}

/// Why a continuous task ended (issue #1341).
///
/// The refusal vocabularies the two tracer workflows already own
/// ([`crate::tractor::coupling::TractorRefusal`],
/// [`crate::science::scan::ScanRefusal`]) map ONTO this rather than being
/// duplicated by it — a beam that dropped for want of power and a scan refused
/// for want of power ended for the same reason, and an after-action reading
/// should not have to know which subsystem was speaking to say so.
///
/// What this adds over those two is the distinction they cannot make: a hold
/// that ENDED because the operator let go, because the scenario withdrew the
/// order, because the subject was destroyed, or because the mission did, all
/// look identical to `hold_status` — it simply stops being asked.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TaskTerminalReason {
    /// The work produced its result. A scan that returned a reading.
    Completed,
    /// The operator — human console or ship AI, indistinguishably (AGENTS.md
    /// rule 6) — stopped it.
    Released,
    /// The scenario order that opened it is no longer active, so the AI host
    /// that was serving it let go. The "scenario closes the task" case.
    OrderWithdrawn,
    /// A fresh activation of the same operator/system/verb arrived while this
    /// one was live. Recorded so the replaced activation still has its one
    /// terminal event and the restart is visibly a restart.
    Restarted,
    /// The selection or lock the task needed is gone, but the subject is still
    /// in the world.
    TargetLost,
    /// The subject left the world — destroyed in combat, or removed by the
    /// scenario.
    TargetDestroyed,
    /// The run ended with the task still live.
    MissionEnded,
    /// The subject is past the authored reach.
    OutOfRange,
    /// The owning system's power group is below its authored minimum.
    Unpowered,
    /// The owning system is damaged out.
    Disabled,
    /// This hull cannot do this work at all.
    NotCapable,
    /// Nothing in the world answers to the subject the command named.
    NoSuchTarget,
    /// The subject carries nothing this work can read.
    Unreadable,
    /// Interference pushed the return past anything usable.
    Blinded,
}

impl TaskTerminalReason {
    /// How many reasons this enum has. Hand-maintained for
    /// [`crate::core::narrative::NarrativeKind::KIND_COUNT`]'s reason: its only
    /// job is to fail [`Self::ALL`]'s coverage test when a reason is added,
    /// forcing whoever adds one to classify it and give it a `strings.csv` row.
    pub const REASON_COUNT: usize = 14;

    /// Every reason, in declaration order (which is also `Ord`'s).
    pub const ALL: [TaskTerminalReason; Self::REASON_COUNT] = [
        TaskTerminalReason::Completed,
        TaskTerminalReason::Released,
        TaskTerminalReason::OrderWithdrawn,
        TaskTerminalReason::Restarted,
        TaskTerminalReason::TargetLost,
        TaskTerminalReason::TargetDestroyed,
        TaskTerminalReason::MissionEnded,
        TaskTerminalReason::OutOfRange,
        TaskTerminalReason::Unpowered,
        TaskTerminalReason::Disabled,
        TaskTerminalReason::NotCapable,
        TaskTerminalReason::NoSuchTarget,
        TaskTerminalReason::Unreadable,
        TaskTerminalReason::Blinded,
    ];

    /// Which of the four classes this reason belongs to.
    pub fn outcome(self) -> TaskOutcome {
        match self {
            TaskTerminalReason::Completed => TaskOutcome::Completed,
            TaskTerminalReason::Released | TaskTerminalReason::OrderWithdrawn => {
                TaskOutcome::Cancelled
            }
            TaskTerminalReason::Restarted
            | TaskTerminalReason::TargetLost
            | TaskTerminalReason::TargetDestroyed
            | TaskTerminalReason::MissionEnded => TaskOutcome::Interrupted,
            TaskTerminalReason::OutOfRange
            | TaskTerminalReason::Unpowered
            | TaskTerminalReason::Disabled
            | TaskTerminalReason::NotCapable
            | TaskTerminalReason::NoSuchTarget
            | TaskTerminalReason::Unreadable
            | TaskTerminalReason::Blinded => TaskOutcome::Failed,
        }
    }

    /// The stable snake_case label written into JSON and ndjson. Hand-written,
    /// not derived, so the wire vocabulary is visible where it is promised.
    pub fn as_str(self) -> &'static str {
        match self {
            TaskTerminalReason::Completed => "completed",
            TaskTerminalReason::Released => "released",
            TaskTerminalReason::OrderWithdrawn => "order_withdrawn",
            TaskTerminalReason::Restarted => "restarted",
            TaskTerminalReason::TargetLost => "target_lost",
            TaskTerminalReason::TargetDestroyed => "target_destroyed",
            TaskTerminalReason::MissionEnded => "mission_ended",
            TaskTerminalReason::OutOfRange => "out_of_range",
            TaskTerminalReason::Unpowered => "unpowered",
            TaskTerminalReason::Disabled => "disabled",
            TaskTerminalReason::NotCapable => "not_capable",
            TaskTerminalReason::NoSuchTarget => "no_such_target",
            TaskTerminalReason::Unreadable => "unreadable",
            TaskTerminalReason::Blinded => "blinded",
        }
    }

    /// The `strings.csv` id a console or after-action surface resolves through
    /// `t()`. A `match`, not a composed `format!`, so `check-strings.mjs` can
    /// see every id a new variant needs a row for — the same shape
    /// [`crate::tractor::coupling::TractorRefusal::string_id`] keeps.
    pub fn string_id(self) -> &'static str {
        match self {
            TaskTerminalReason::Completed => "task.ended.completed",
            TaskTerminalReason::Released => "task.ended.released",
            TaskTerminalReason::OrderWithdrawn => "task.ended.order_withdrawn",
            TaskTerminalReason::Restarted => "task.ended.restarted",
            TaskTerminalReason::TargetLost => "task.ended.target_lost",
            TaskTerminalReason::TargetDestroyed => "task.ended.target_destroyed",
            TaskTerminalReason::MissionEnded => "task.ended.mission_ended",
            TaskTerminalReason::OutOfRange => "task.ended.out_of_range",
            TaskTerminalReason::Unpowered => "task.ended.unpowered",
            TaskTerminalReason::Disabled => "task.ended.disabled",
            TaskTerminalReason::NotCapable => "task.ended.not_capable",
            TaskTerminalReason::NoSuchTarget => "task.ended.no_such_target",
            TaskTerminalReason::Unreadable => "task.ended.unreadable",
            TaskTerminalReason::Blinded => "task.ended.blinded",
        }
    }

    /// Whether this reason is one a vanished subject would better explain.
    ///
    /// A hold whose subject was destroyed reports itself as out of range or as
    /// having lost its lock, because that is all the subsystem can see: the
    /// entity is simply not in the transform query any more. The emitter
    /// upgrades exactly these two to [`Self::TargetDestroyed`] when the subject
    /// really has left the world, and leaves every other reason alone — a beam
    /// that lost power while its target was being destroyed lost power.
    ///
    /// [`Self::NoSuchTarget`] is deliberately NOT one of them, even though its
    /// subject is equally absent from the world: it means the order named a
    /// contact that was never there, and re-reading that as a destruction would
    /// invent a hull for the timeline to mourn.
    pub fn masks_target_loss(self) -> bool {
        matches!(
            self,
            TaskTerminalReason::TargetLost | TaskTerminalReason::OutOfRange
        )
    }
}

/// The identity of a task *position*: one operator's one system doing one kind
/// of work.
///
/// A slot holds at most one live activation, which is what makes a terminal
/// request addressable without the reporting site having to remember a key it
/// never minted. Two different subjects on the same slot are two activations in
/// sequence (the second [`TaskTerminalReason::Restarted`]s the first); two
/// different slots are genuinely simultaneous and keep separate keys.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TaskSlot {
    /// The operating hull's uuid.
    pub operator: String,
    /// The ship-system id that owns the work (`"sensors"`, `"tractor"`, …).
    pub system: String,
    /// What kind of work it is — [`TASK_VERB_SCAN`], [`TASK_VERB_TRACTOR_HOLD`], …
    pub verb: String,
}

impl TaskSlot {
    /// A slot from its three parts.
    pub fn new(
        operator: impl Into<String>,
        system: impl Into<String>,
        verb: impl Into<String>,
    ) -> Self {
        Self {
            operator: operator.into(),
            system: system.into(),
            verb: verb.into(),
        }
    }
}

/// The deterministic identity of ONE activation, stable from its start event to
/// its terminal event.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TaskKey {
    /// The slot this activation occupies.
    pub slot: TaskSlot,
    /// The subject uuid, when the work has one.
    pub target: Option<String>,
    /// Which activation of this slot this is, counted from 0 for the run.
    pub ordinal: u32,
}

/// The placeholder written into a [`TaskKey`] string for a task with no
/// subject, so the encoding stays a fixed five fields.
const NO_TARGET: &str = "-";

impl TaskKey {
    /// The wire form: `operator/system/verb/target#ordinal`.
    ///
    /// `/` and `#` are the separators because neither can occur in a uuid, a
    /// system id or a verb, so the encoding is unambiguous without escaping.
    pub fn as_str(&self) -> String {
        format!(
            "{}/{}/{}/{}#{}",
            self.slot.operator,
            self.slot.system,
            self.slot.verb,
            self.target.as_deref().unwrap_or(NO_TARGET),
            self.ordinal,
        )
    }
}

/// One live activation, as the registry holds it.
#[derive(Clone, Debug, PartialEq)]
pub struct TaskActivation {
    /// Its stable identity.
    pub key: TaskKey,
    /// The station the operating system belongs to, when the operator's config
    /// resolves one. `None` rather than the system id cast to a station — the
    /// same rule the Comms beat keeps.
    pub station: Option<String>,
    /// The fixed sim tick the activation started on.
    pub start_tick: u64,
}

/// A lifecycle moment on its way from a workflow site to the event stream
/// (issue #1341).
///
/// Buffered onto an [`crate::effect_queue::EffectQueue`] rather than written
/// straight to `Messages<NarrativeEvent>`, for the reason every other #1223
/// effect is: the sites that know a hold began or ended (`handle_tractor_
/// commands`, `tick_tractor`, `tick_scans`) are ordinary fixed-tick systems near
/// Bevy's parameter limit, and a queue push costs them one `Option<ResMut<_>>`
/// instead of a writer plus every lookup a key needs.
///
/// It also puts the ordinal, the station resolution and the one-terminal rule in
/// ONE place — the emitter — rather than in every site that will ever report a
/// task.
#[derive(Clone, Debug, PartialEq)]
pub enum TaskLifecycleRequest {
    /// Work began on this slot, against this subject.
    Start {
        slot: TaskSlot,
        target: Option<String>,
    },
    /// Work on this slot ended. Dropped by the emitter if the slot holds no live
    /// activation, which is what makes duplicate reports of one ending harmless.
    End {
        slot: TaskSlot,
        reason: TaskTerminalReason,
    },
}

/// Every live task activation, and the per-slot ordinal counter that keeps
/// restarts distinct (issue #1341).
///
/// Presentation-class state for the #894 digest boundary: the fixed tick never
/// reads it, nothing branches on it, and neither `sim_digest` nor `snapshot`
/// walks it. It decides only what the after-action surface SHOWS.
///
/// A resource rather than a `Local` on the emitter — unlike the objective and
/// deadline views in [`crate::narrative`] — for one reason: the run's LAST
/// terminal events are the ones nothing in the schedule can emit, because the
/// run has stopped. `crate::headless::report::finalize_task_lifecycles` drains
/// what is left here at the report boundary, so a task that was still running
/// when the mission ended still gets its one terminal event.
#[derive(Resource, Debug, Clone, Default, PartialEq)]
pub struct TaskLifecycles {
    active: BTreeMap<TaskSlot, TaskActivation>,
    next_ordinal: BTreeMap<TaskSlot, u32>,
    last_terminal: BTreeMap<TaskSlot, LastTerminal>,
}

/// The terminal moment a slot last recorded, and how many identical
/// instantaneous activations have been folded into it since (issue #1341).
///
/// The whole activation is kept, not just its subject and reason, for two
/// reasons: the coalescing rule needs the subject, and the SUMMARY beat that
/// reports the folded repeats has to name the same key the retained beat
/// carried — otherwise the count would float free of the activation it counts.
#[derive(Debug, Clone, PartialEq)]
struct LastTerminal {
    /// The activation the retained terminal beat belonged to.
    activation: TaskActivation,
    /// Why it ended.
    reason: TaskTerminalReason,
    /// How many identical repeats have been coalesced into it and not yet
    /// reported. Zero once [`TaskLifecycles::take_repeats`] has drained them.
    repeats: u32,
}

impl TaskLifecycles {
    /// Open an activation on `slot`, minting its ordinal.
    ///
    /// The caller is responsible for having closed any activation already on the
    /// slot — [`Self::end`] first, and record it as
    /// [`TaskTerminalReason::Restarted`]. Nothing here silently drops one, so a
    /// missing close shows up as an overwritten `active` entry rather than as a
    /// timeline that quietly lost a beat.
    pub fn begin(
        &mut self,
        slot: TaskSlot,
        target: Option<String>,
        station: Option<String>,
        tick: u64,
    ) -> TaskActivation {
        let ordinal = self.next_ordinal.entry(slot.clone()).or_insert(0);
        let activation = TaskActivation {
            key: TaskKey {
                slot: slot.clone(),
                target,
                ordinal: *ordinal,
            },
            station,
            start_tick: tick,
        };
        *ordinal += 1;
        self.active.insert(slot, activation.clone());
        activation
    }

    /// Close the activation on `slot`, if there is one.
    pub fn end(&mut self, slot: &TaskSlot) -> Option<TaskActivation> {
        self.active.remove(slot)
    }

    /// The live activation on `slot`, if any.
    pub fn get(&self, slot: &TaskSlot) -> Option<&TaskActivation> {
        self.active.get(slot)
    }

    /// Every live activation, in slot order.
    pub fn active(&self) -> impl Iterator<Item = &TaskActivation> {
        self.active.values()
    }

    /// How many activations are live.
    pub fn len(&self) -> usize {
        self.active.len()
    }

    /// Whether nothing is running.
    pub fn is_empty(&self) -> bool {
        self.active.is_empty()
    }

    /// Take every live activation, in slot order, leaving the registry empty.
    ///
    /// The ordinal counters are deliberately kept: a run that closed everything
    /// and then started a fresh activation of the same slot must still mint a
    /// new key.
    pub fn take_all(&mut self) -> Vec<TaskActivation> {
        std::mem::take(&mut self.active).into_values().collect()
    }

    /// Remember the terminal moment just recorded for `slot`, so an identical
    /// repeat of it can be recognised as a poll rather than as a fresh beat.
    ///
    /// Called for EVERY terminal the emitter writes, including the ones a sweep
    /// produces: what the rule below compares against is "the last thing this
    /// slot was seen to do", not "the last thing a workflow reported".
    ///
    /// A freshly recorded terminal starts a new coalescing group, so its repeat
    /// count begins at zero. The caller must have reported any repeats the slot
    /// was still carrying first — [`Self::take_repeats`] — or the count would be
    /// silently discarded, which is the whole failure this counter exists to
    /// prevent.
    pub fn record_terminal(&mut self, activation: &TaskActivation, reason: TaskTerminalReason) {
        self.last_terminal.insert(
            activation.key.slot.clone(),
            LastTerminal {
                activation: activation.clone(),
                reason,
                repeats: 0,
            },
        );
    }

    /// Count one coalesced repeat against the terminal `slot` last recorded.
    ///
    /// Called instead of writing a start/terminal pair, so the attempt is
    /// BOUNDED rather than erased: [`Self::take_repeats`] hands the total back
    /// for a single summary beat once the standing failure ends.
    pub fn note_repeat(&mut self, slot: &TaskSlot) {
        if let Some(last) = self.last_terminal.get_mut(slot) {
            last.repeats = last.repeats.saturating_add(1);
        }
    }

    /// Take the repeats folded into `slot`'s retained terminal, if any, with the
    /// activation and reason they repeated. Leaves the retained terminal in
    /// place — only the count is drained — so a further identical attempt still
    /// coalesces rather than re-opening the flood.
    pub fn take_repeats(
        &mut self,
        slot: &TaskSlot,
    ) -> Option<(TaskActivation, TaskTerminalReason, u32)> {
        let last = self.last_terminal.get_mut(slot)?;
        if last.repeats == 0 {
            return None;
        }
        let repeats = std::mem::take(&mut last.repeats);
        Some((last.activation.clone(), last.reason, repeats))
    }

    /// Whether any slot is carrying unreported repeats.
    pub fn has_pending_repeats(&self) -> bool {
        self.last_terminal.values().any(|last| last.repeats > 0)
    }

    /// Take every slot's outstanding repeats, in slot order — the run-end
    /// counterpart of [`Self::take_repeats`], for the standing order that was
    /// still being re-issued when the run stopped.
    pub fn take_all_repeats(&mut self) -> Vec<(TaskActivation, TaskTerminalReason, u32)> {
        self.last_terminal
            .values_mut()
            .filter(|last| last.repeats > 0)
            .map(|last| {
                (
                    last.activation.clone(),
                    last.reason,
                    std::mem::take(&mut last.repeats),
                )
            })
            .collect()
    }

    /// Whether an INSTANTANEOUS activation of `slot` against `target`, ending
    /// for `reason`, would repeat exactly the last terminal this slot recorded.
    ///
    /// # Why this exists
    ///
    /// An AI host that serves a standing objective re-issues its request on
    /// every authored snapshot while the objective stays unsatisfied — the
    /// Sensors host's Scan is the shipped example, and it says so at
    /// `crate::ship::sensors::ai_sensors_target_selection`. A scan that cannot
    /// reach its subject is therefore refused again on every cadence, forever,
    /// and each refusal is a whole start-and-terminal pair. Unfiltered, a single
    /// standing out-of-range order is a *rate* rather than a beat — the exact
    /// firehose [`crate::core::narrative::NarrativeKind::in_timeline_stream`]
    /// exists to keep out of the timeline — and it is unbounded in the report,
    /// the counts and the ndjson stream alike.
    ///
    /// # What it deliberately does NOT coalesce
    ///
    /// * Anything whose class is not [`TaskOutcome::Failed`]. A completion is
    ///   work that happened and a cancellation is a decision somebody made;
    ///   both are beats however often they recur.
    /// * A repeat whose subject or reason CHANGED. "Out of range, then out of
    ///   range" is one unchanged fact; "out of range, then unpowered" is two.
    /// * A task that spanned ticks. Only an activation that began and ended
    ///   inside one tick can be a poll — a hold that ran for a while and failed
    ///   is a real ending even if the previous hold failed the same way.
    ///
    /// # What "coalesced" does NOT mean
    ///
    /// It does not mean *erased*. The rule cannot tell an AI cadence retry from
    /// an engineer pressing Engage a second time on a target that is still out
    /// of reach — both are one slot, one subject, one unchanged refusal, both
    /// halves minted by one tick — and issue #1341's AC3 requires a repeat to
    /// stay identifiable. So a coalesced attempt is COUNTED
    /// ([`Self::note_repeat`]) and reported once, as a
    /// [`crate::core::narrative::NarrativeKind::TaskRepeated`] census beat
    /// carrying the retained activation's key, when the standing failure ends or
    /// the run does. One beat per standing order either way; nothing counts in
    /// the dark.
    pub fn repeats_last_failure(
        &self,
        slot: &TaskSlot,
        target: Option<&str>,
        reason: TaskTerminalReason,
    ) -> bool {
        if reason.outcome() != TaskOutcome::Failed {
            return false;
        }
        self.last_terminal.get(slot).is_some_and(|last| {
            last.reason == reason && last.activation.key.target.as_deref() == target
        })
    }
}

/// The start beat for one activation (issue #1341).
///
/// Pure — no world access — so the whole event shape is unit-testable without
/// booting an app, the same property [`crate::core::narrative::fold_narrative`]
/// has.
pub fn start_event(activation: &TaskActivation) -> NarrativeEvent {
    base_event(NarrativeKind::TaskStarted, activation)
}

/// The terminal beat for one activation, of the kind its reason classifies to.
///
/// `end_tick` is the fixed tick the task ended on; the event carries both it and
/// the start tick, so an after-action reading gets the duration without having
/// to pair the two beats itself.
pub fn terminal_event(
    activation: &TaskActivation,
    reason: TaskTerminalReason,
    end_tick: u64,
) -> NarrativeEvent {
    let outcome = reason.outcome();
    base_event(outcome.kind(), activation)
        .text("reason", reason.as_str())
        .text("reason_text", reason.string_id())
        .text("outcome", outcome.as_str())
        .detail(
            "start_tick",
            NarrativeValue::Int(activation.start_tick as i64),
        )
        .detail(
            "duration_ticks",
            NarrativeValue::Int(end_tick.saturating_sub(activation.start_tick) as i64),
        )
}

/// The census beat for the attempts that were coalesced into `activation`'s
/// terminal (issue #1341).
///
/// It carries the SAME key as the beat it counts against — this is "and it was
/// asked for `repeats` more times, unchanged", not a separate activation — with
/// the total including the recorded one as `attempts`, so a reader does not have
/// to do the arithmetic. Written once when the standing failure ends or the run
/// does, never per attempt: that is what keeps a polled order O(1) in the
/// timeline while leaving the repeats identifiable.
pub fn repeats_event(
    activation: &TaskActivation,
    reason: TaskTerminalReason,
    repeats: u32,
) -> NarrativeEvent {
    let outcome = reason.outcome();
    base_event(NarrativeKind::TaskRepeated, activation)
        .text("reason", reason.as_str())
        .text("reason_text", reason.string_id())
        .text("outcome", outcome.as_str())
        .detail("repeats", NarrativeValue::Int(repeats as i64))
        .detail("attempts", NarrativeValue::Int(repeats as i64 + 1))
}

/// The half of the event shape every beat here shares: the key as the semantic
/// id, the operator/station/system as the source, the subject as the target, and
/// the verb and ordinal as structured detail.
fn base_event(kind: NarrativeKind, activation: &TaskActivation) -> NarrativeEvent {
    let key = &activation.key;
    let mut event = NarrativeEvent::new(kind, key.as_str())
        .from_actor(NarrativeActor {
            entity: Some(key.slot.operator.clone()),
            station: activation.station.clone(),
            system: Some(key.slot.system.clone()),
        })
        .text("verb", key.slot.verb.clone())
        .detail("ordinal", NarrativeValue::Int(key.ordinal as i64));
    if let Some(target) = &key.target {
        event = event.to_target(target.clone());
    }
    event
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slot() -> TaskSlot {
        TaskSlot::new("uuid-tender", "tractor", TASK_VERB_TRACTOR_HOLD)
    }

    /// The coverage guard: every reason is deliberately classified and given a
    /// `strings.csv` id. Adding a reason without touching `ALL` fails to compile
    /// the array; adding one to `ALL` without bumping `REASON_COUNT` fails to
    /// compile too — this test then forces the remaining two decisions.
    #[test]
    fn every_reason_is_classified_and_localizable() {
        assert_eq!(
            TaskTerminalReason::ALL.len(),
            TaskTerminalReason::REASON_COUNT
        );
        let mut labels = std::collections::BTreeSet::new();
        let mut ids = std::collections::BTreeSet::new();
        for reason in TaskTerminalReason::ALL {
            assert!(
                labels.insert(reason.as_str()),
                "duplicate terminal-reason label {:?}",
                reason.as_str()
            );
            assert!(
                ids.insert(reason.string_id()),
                "duplicate terminal-reason String Id {:?}",
                reason.string_id()
            );
            assert!(
                reason.string_id().starts_with("task.ended."),
                "{:?} must resolve through the task.ended.* family",
                reason
            );
        }
        // All four classes are reachable — a vocabulary that could never report
        // a cancellation would satisfy every other assertion here.
        let classes: std::collections::BTreeSet<TaskOutcome> = TaskTerminalReason::ALL
            .iter()
            .map(|r| r.outcome())
            .collect();
        assert_eq!(classes.len(), 4, "{classes:?}");
    }

    /// Only the two reasons a HOLD can give for a subject it once had are
    /// re-read as a destruction. `NoSuchTarget` names a contact that was never
    /// there, and must not be turned into a hull the timeline mourns.
    #[test]
    fn only_a_lost_hold_can_be_re_read_as_a_destroyed_subject() {
        let masking: Vec<&str> = TaskTerminalReason::ALL
            .iter()
            .filter(|r| r.masks_target_loss())
            .map(|r| r.as_str())
            .collect();
        assert_eq!(masking, vec!["target_lost", "out_of_range"]);
    }

    /// The key is a pure function of state the tick already decided, and the
    /// ordinal is what separates a restart from the activation it replaced.
    #[test]
    fn a_restart_of_the_same_slot_mints_a_distinct_key() {
        let mut registry = TaskLifecycles::default();
        let first = registry.begin(slot(), Some("uuid-hulk".into()), None, 10);
        assert_eq!(
            first.key.as_str(),
            "uuid-tender/tractor/tractor_hold/uuid-hulk#0"
        );
        assert!(registry.end(&slot()).is_some());
        let second = registry.begin(slot(), Some("uuid-hulk".into()), None, 40);
        assert_eq!(
            second.key.as_str(),
            "uuid-tender/tractor/tractor_hold/uuid-hulk#1",
            "the same operator holding the same hull a second time is a SECOND task"
        );
        assert_ne!(first.key, second.key);
    }

    /// Two different slots run at once and keep separate identities — the
    /// "simultaneous tasks stay separately identifiable" half of AC3.
    #[test]
    fn simultaneous_slots_do_not_share_an_activation() {
        let mut registry = TaskLifecycles::default();
        let hold = registry.begin(slot(), Some("uuid-hulk".into()), None, 1);
        let scan = registry.begin(
            TaskSlot::new("uuid-tender", "sensors", TASK_VERB_SCAN),
            Some("uuid-depot".into()),
            None,
            1,
        );
        assert_eq!(registry.len(), 2);
        assert_ne!(hold.key.as_str(), scan.key.as_str());
        // …and each ordinal counts its OWN slot, so the second task is not
        // numbered as though it were the first one's restart.
        assert_eq!(hold.key.ordinal, 0);
        assert_eq!(scan.key.ordinal, 0);
    }

    /// A terminal request for a slot with nothing live is dropped — the whole
    /// of the "exactly one terminal event" rule.
    #[test]
    fn ending_an_unstarted_slot_yields_nothing() {
        let mut registry = TaskLifecycles::default();
        assert!(registry.end(&slot()).is_none());
        registry.begin(slot(), None, None, 0);
        assert!(registry.end(&slot()).is_some());
        assert!(
            registry.end(&slot()).is_none(),
            "a second report of the same ending must add nothing"
        );
    }

    /// `take_all` is the mission-end sweep: it empties the live set in slot
    /// order but keeps the counters, so a resumed slot still mints a fresh key.
    #[test]
    fn take_all_empties_the_live_set_but_not_the_counters() {
        let mut registry = TaskLifecycles::default();
        registry.begin(slot(), None, None, 0);
        registry.begin(
            TaskSlot::new("uuid-tender", "sensors", TASK_VERB_SCAN),
            None,
            None,
            0,
        );
        let taken = registry.take_all();
        assert_eq!(taken.len(), 2);
        assert_eq!(
            taken
                .iter()
                .map(|a| a.key.slot.system.as_str())
                .collect::<Vec<_>>(),
            vec!["sensors", "tractor"],
            "the sweep walks in slot order, never in the order tasks opened"
        );
        assert!(registry.is_empty());
        assert_eq!(registry.begin(slot(), None, None, 0).key.ordinal, 1);
    }

    /// The two beats share their identity and point the same way; the terminal
    /// one adds the reason, the class, and the span.
    #[test]
    fn the_two_beats_share_one_identity() {
        let mut registry = TaskLifecycles::default();
        let activation = registry.begin(
            slot(),
            Some("uuid-hulk".into()),
            Some("engineering".into()),
            30,
        );
        let start = start_event(&activation);
        let end = terminal_event(&activation, TaskTerminalReason::Released, 90);

        assert_eq!(start.kind, NarrativeKind::TaskStarted);
        assert_eq!(end.kind, NarrativeKind::TaskCancelled);
        assert_eq!(start.id, end.id, "one activation, one key");
        assert_eq!(start.target.as_deref(), Some("uuid-hulk"));
        assert_eq!(end.target.as_deref(), Some("uuid-hulk"));
        assert_eq!(start.source.entity.as_deref(), Some("uuid-tender"));
        assert_eq!(start.source.system.as_deref(), Some("tractor"));
        assert_eq!(start.source.station.as_deref(), Some("engineering"));
        assert_eq!(
            end.detail.get("reason"),
            Some(&NarrativeValue::Text("released".into()))
        );
        assert_eq!(
            end.detail.get("reason_text"),
            Some(&NarrativeValue::Text("task.ended.released".into())),
            "the localizable half of the reason is a String Id, never prose"
        );
        assert_eq!(
            end.detail.get("outcome"),
            Some(&NarrativeValue::Text("cancelled".into()))
        );
        assert_eq!(
            end.detail.get("duration_ticks"),
            Some(&NarrativeValue::Int(60))
        );
    }

    /// The poll rule: an unchanged FAILURE repeats, and nothing else does.
    #[test]
    fn only_an_unchanged_failure_counts_as_a_repeat() {
        let mut registry = TaskLifecycles::default();
        let scan = TaskSlot::new("uuid-tender", "sensors", TASK_VERB_SCAN);
        // The activation a terminal is recorded against — the registry now keeps
        // the whole thing, because the repeat COUNT has to be reported under the
        // same key the retained beat carried.
        let recorded = |registry: &mut TaskLifecycles, target: &str| {
            registry.begin(scan.clone(), Some(target.to_string()), None, 0)
        };
        assert!(
            !registry.repeats_last_failure(
                &scan,
                Some("uuid-hulk"),
                TaskTerminalReason::OutOfRange
            ),
            "a slot that has never ended anything cannot be repeating itself"
        );

        let hulk = recorded(&mut registry, "uuid-hulk");
        registry.record_terminal(&hulk, TaskTerminalReason::OutOfRange);
        assert!(registry.repeats_last_failure(
            &scan,
            Some("uuid-hulk"),
            TaskTerminalReason::OutOfRange
        ));
        // A different subject, a different reason, or a different slot are all
        // different facts.
        assert!(!registry.repeats_last_failure(
            &scan,
            Some("uuid-depot"),
            TaskTerminalReason::OutOfRange
        ));
        assert!(!registry.repeats_last_failure(
            &scan,
            Some("uuid-hulk"),
            TaskTerminalReason::Unpowered
        ));
        assert!(!registry.repeats_last_failure(
            &slot(),
            Some("uuid-hulk"),
            TaskTerminalReason::OutOfRange
        ));

        // Only the FAILED class coalesces: a completion is work that happened
        // and a cancellation is a decision somebody made, however often either
        // recurs.
        for reason in TaskTerminalReason::ALL {
            registry.record_terminal(&hulk, reason);
            assert_eq!(
                registry.repeats_last_failure(&scan, Some("uuid-hulk"), reason),
                reason.outcome() == TaskOutcome::Failed,
                "{reason:?}"
            );
        }
    }

    /// A coalesced repeat is COUNTED, never erased: the registry hands the total
    /// back once, under the key of the terminal the repeats were folded into, so
    /// issue #1341's "repeated tasks remain separately identifiable" holds
    /// without the timeline growing with the cadence.
    #[test]
    fn coalesced_repeats_are_counted_and_drained_once() {
        let mut registry = TaskLifecycles::default();
        let scan = TaskSlot::new("uuid-tender", "sensors", TASK_VERB_SCAN);
        let activation = registry.begin(scan.clone(), Some("uuid-hulk".into()), None, 7);
        registry.record_terminal(&activation, TaskTerminalReason::OutOfRange);

        assert!(!registry.has_pending_repeats(), "nothing has repeated yet");
        assert!(registry.take_repeats(&scan).is_none());

        for _ in 0..40 {
            registry.note_repeat(&scan);
        }
        assert!(registry.has_pending_repeats());
        let (repeated, reason, count) = registry
            .take_repeats(&scan)
            .expect("40 coalesced attempts must be recoverable");
        assert_eq!(count, 40);
        assert_eq!(reason, TaskTerminalReason::OutOfRange);
        assert_eq!(
            repeated.key, activation.key,
            "the count is reported under the key of the beat it was folded into"
        );
        // Drained, not duplicated…
        assert!(registry.take_repeats(&scan).is_none());
        assert!(!registry.has_pending_repeats());
        // …but the retained terminal itself survives, so the NEXT identical
        // attempt still coalesces rather than re-opening the flood.
        assert!(registry.repeats_last_failure(
            &scan,
            Some("uuid-hulk"),
            TaskTerminalReason::OutOfRange
        ));

        // The census beat carries the count, the total including the recorded
        // attempt, and the retained activation's key.
        let event = repeats_event(&repeated, reason, count);
        assert_eq!(event.kind, NarrativeKind::TaskRepeated);
        assert!(!event.kind.is_task_lifecycle(), "it terminates nothing");
        assert_eq!(event.id, activation.key.as_str());
        assert_eq!(event.detail.get("repeats"), Some(&NarrativeValue::Int(40)));
        assert_eq!(event.detail.get("attempts"), Some(&NarrativeValue::Int(41)));
        assert_eq!(
            event.detail.get("reason"),
            Some(&NarrativeValue::Text("out_of_range".into()))
        );
    }

    /// A run that stops with several slots still polling reports each one's
    /// count, in slot order — never in the order they happened to start.
    #[test]
    fn every_slots_repeats_drain_in_slot_order() {
        let mut registry = TaskLifecycles::default();
        let tractor = slot();
        let scan = TaskSlot::new("uuid-tender", "sensors", TASK_VERB_SCAN);
        // Opened tractor-first, so the drain order cannot be the insertion one.
        let held = registry.begin(tractor.clone(), Some("uuid-hulk".into()), None, 0);
        let read = registry.begin(scan.clone(), Some("uuid-hulk".into()), None, 0);
        registry.record_terminal(&held, TaskTerminalReason::OutOfRange);
        registry.record_terminal(&read, TaskTerminalReason::NoSuchTarget);
        registry.note_repeat(&tractor);
        registry.note_repeat(&scan);
        registry.note_repeat(&scan);

        let drained = registry.take_all_repeats();
        assert_eq!(
            drained
                .iter()
                .map(|(a, _, n)| (a.key.slot.system.as_str(), *n))
                .collect::<Vec<_>>(),
            vec![("sensors", 2), ("tractor", 1)],
            "slot order, and each slot's own count"
        );
        assert!(registry.take_all_repeats().is_empty(), "drained once");
    }

    /// A task with no subject encodes a fixed five-field key rather than a
    /// shorter one, so a reader never has to count separators.
    #[test]
    fn a_subjectless_task_still_encodes_five_fields() {
        let mut registry = TaskLifecycles::default();
        let activation = registry.begin(slot(), None, None, 0);
        assert_eq!(
            activation.key.as_str(),
            "uuid-tender/tractor/tractor_hold/-#0"
        );
        assert!(start_event(&activation).target.is_none());
    }
}
