//! Named mission deadlines (issue #1024, parent #851 "Falling Skyway").
//!
//! A scenario author needs deadlines that are *named things in the world* — a
//! `transfer_window_opens`, a `stabiliser_failure` — rather than anonymous
//! timers. A world declares them in `[[deadline]]` blocks; a `[script]` block
//! names the handler each one runs with `on_deadline("id", "handler")`; and a
//! handler can read a deadline's remaining time and state, push it out, or call
//! it off as the situation changes.
//!
//! # THIS IS NOT A SCHEDULER — it is a record over the existing queues
//!
//! The single most important property of this module, and the one review is
//! asked to hold it to: **it introduces no deferred-work queue of its own and no
//! per-tick draining system.** Phoenix already has two deferred-work queues and
//! a deadline is driven entirely by one of them:
//!
//! * [`WorldContentRuntime::pending_delayed_actions`] — the mission-clock
//!   (`elapsed_secs`) queue a `ctx.schedule.in_seconds(n).<verb>(…)` feeds.
//! * [`WorldScriptRuntime::pending_callbacks`] — the **`SimTick`-keyed** queue a
//!   `ctx.schedule.after(n, |ctx| …)` feeds, drained by the *existing*
//!   `tick_script_callbacks` system through
//!   [`PendingCallbacks::drain_due`](crate::world::script::schedule::PendingCallbacks::drain_due).
//!
//! Arming a deadline **pushes one ordinary
//! [`ScheduledCall`] onto that second queue** and remembers it on the record
//! ([`DeadlineRecord::armed`]). Firing is that queue's existing drain calling
//! that callback; nothing here polls a clock, and no system added by this slice
//! walks a list looking for due work. The record exists to give the queued call
//! a *name*, a *label*, a *visibility flag* and a *mutable due time* — which is
//! precisely what the raw `(fire_tick, script_path, fn_name)` key cannot carry.
//!
//! Slip and cancel are therefore expressed as **edits to that queue**, not as a
//! second source of truth that the queue is later reconciled against: every
//! mutation returns a [`QueueEdit`] naming the exact `ScheduledCall` to retract
//! and the exact one (if any) to push in its place. A slipped deadline cannot
//! also fire at its old time because its old call is no longer queued.
//!
//! # Ticks, not seconds
//!
//! Authoring is in whole seconds (`due_secs`, and `slip`'s argument), matching
//! the integer-only (`no_float`) script surface; the conversion to `u64` ticks
//! happens once, at arm/slip time, through the same
//! [`seconds_to_ticks`](crate::world::script::schedule::seconds_to_ticks) the
//! callback queue already uses. Only ticks are stored and only ticks are
//! compared, so two peers running the same world at the same `sim_tick_hz` fire
//! the same deadline on the same tick.
//!
//! [`WorldContentRuntime::pending_delayed_actions`]: crate::world::server::WorldContentRuntime::pending_delayed_actions
//! [`WorldScriptRuntime::pending_callbacks`]: crate::world::server::WorldScriptRuntime::pending_callbacks

use serde::{Deserialize, Serialize};

use crate::world::script::schedule::{seconds_to_ticks, ScheduledCall};

/// Where a deadline stands: the three states a script condition reads back and
/// the panel renders.
///
/// `Pending` is the default because an authored deadline that has not been
/// touched is exactly that — owed, not yet spent.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeadlineState {
    /// Armed and still owed: its callback is queued and will fire.
    #[default]
    Pending,
    /// Its tick arrived and the queue dispatched its handler.
    Fired,
    /// Called off by script before it fired. It never fires.
    Cancelled,
}

impl DeadlineState {
    /// The wire/script label, the same word the `[[deadline]]` vocabulary and
    /// the panel use. Written by hand rather than derived so the strings a
    /// script compares against are visible at the point they are promised.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Fired => "fired",
            Self::Cancelled => "cancelled",
        }
    }
}

/// One authored `[[deadline]]` block.
///
/// ```toml
/// [[deadline]]
/// id = "transfer_window_opens"
/// label = "world.falling_skyway.deadline.transfer_window.label"
/// due_secs = 600
/// visible = true
/// ```
///
/// `label` is a `strings.csv` id, not English — AGENTS.md rule 11's display-text
/// exception. `visible` decides whether the crew sees a countdown for it at all;
/// a mission can keep a deadline entirely to itself.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Deadline {
    /// Stable id, unique within a world. Script names the deadline by this, and
    /// a duplicate is a load-time error (see
    /// [`duplicate_id`](crate::world::config::parse_world)).
    pub id: String,
    /// `strings.csv` id for the crew-facing name. Only read when `visible`.
    #[serde(default)]
    pub label: String,
    /// Whole seconds from the first simulation tick of the mission (the moment
    /// `anchor_mission_clock` stamps, not app start — a long lobby must not eat
    /// a mission's deadlines).
    pub due_secs: i64,
    /// Whether the crew sees it. Default `false`: a deadline is the mission's
    /// business until the mission says otherwise.
    #[serde(default)]
    pub visible: bool,
}

/// The script fn one deadline runs when it fires, as registered by a top-level
/// `on_deadline("id", "handler")` in a `[script]` block.
///
/// Lives here, in the pure module, rather than beside the trigger front-end that
/// registers it: the *table* is what consumes it, and putting the vocabulary
/// where its consumer is keeps the script host depending on the domain rather
/// than the other way round.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeadlineHandler {
    /// The `[[deadline]] id` this handler is for.
    pub deadline_id: String,
    /// The fn name to call when it fires.
    pub handler: String,
    /// Content-relative path of the unit that registered it. Load-bearing, not
    /// decoration: it is half of the `ScheduledCall` key, because a fn name is
    /// not unique across units (the M0 spike).
    pub source_path: String,
}

/// One deadline's live state.
///
/// The authored fields (`id`, `label`, `visible`) are copied in at arm time and
/// never change; `due_tick`, `state` and `armed` are what a run moves.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DeadlineRecord {
    /// The authored id.
    pub id: String,
    /// Supporting world that authored this local id. `None` is the root world.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_layer: Option<String>,
    /// The authored `strings.csv` label id.
    #[serde(default)]
    pub label: String,
    /// Whether the crew sees it.
    #[serde(default)]
    pub visible: bool,
    /// Absolute `SimTick` this deadline is due. Moved by [`DeadlineTable::slip`]
    /// — and moving it is what re-keys the queued call.
    pub due_tick: u64,
    /// Pending / fired / cancelled.
    #[serde(default)]
    pub state: DeadlineState,
    /// The exact [`ScheduledCall`] currently sitting on
    /// `WorldScriptRuntime::pending_callbacks` for this deadline, or `None` once
    /// it has fired or been cancelled.
    ///
    /// Stored whole rather than reconstructed, so a retraction is an exact
    /// match: two deadlines that share a handler fn *and* a fire tick produce
    /// equal keys, and removing "the first equal one" then removes exactly one
    /// of the two, which is the correct count.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub armed: Option<ScheduledCall>,
}

impl DeadlineRecord {
    /// Stable row key for presentation. Root ids stay byte-for-byte compatible;
    /// a supporting layer qualifies its local authored id by ownership.
    pub fn presentation_id(&self) -> String {
        match &self.origin_layer {
            Some(layer) => format!("{layer}#deadline.{}", self.id),
            None => self.id.clone(),
        }
    }
}

/// The edit a mutation asks of the *existing* callback queue.
///
/// Returned rather than applied, because this module has no queue: the Bevy
/// adapter owns `WorldScriptRuntime::pending_callbacks` and is the one that
/// retracts and pushes. Both fields may be `Some` (a slip), only `retract` may
/// be `Some` (a cancel), or the edit may not be produced at all (a mutation
/// naming an unknown or already-spent deadline).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct QueueEdit {
    /// The call to remove from `pending_callbacks`.
    pub retract: Option<ScheduledCall>,
    /// The call to add to `pending_callbacks`.
    pub push: Option<ScheduledCall>,
}

/// What a script asked of one named deadline.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeadlineMutation {
    /// Push the due time out by `by_secs` whole seconds. A non-positive value
    /// pulls it in, and a slip that lands in the past is due on the next tick —
    /// the boundary [`seconds_to_ticks`] already defines.
    Slip { by_secs: i64 },
    /// Call it off. It never fires.
    Cancel,
}

/// One buffered `ctx.deadlines.slip(…)` / `.cancel(…)`, in authored order.
///
/// A script's deadline writes travel as a fifth field on
/// [`CallEffects`](crate::world::script::schedule::CallEffects) rather than as
/// `ActionCmd`s, for the reason `comms_opens` is a fourth field: a deadline
/// mutation edits a queue the generic action applier holds no handle on. They
/// are applied at the same point in the same tick as the call's other effects,
/// so nothing about them is deferred.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeadlineChange {
    /// The `[[deadline]] id` addressed.
    pub id: String,
    /// Scope in which the script addressed `id`. `None` is the root world.
    pub origin_layer: Option<String>,
    /// What to do to it.
    pub mutation: DeadlineMutation,
}

/// Every deadline a world authored, in authored order.
///
/// Ordered by authoring rather than keyed by a map, for the reason
/// `PendingCallbacks` is a `Vec`: the order is a deterministic function of the
/// world file, it is what the panel renders top to bottom, and a `HashMap`'s
/// iteration order must never reach a payload or a fold.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DeadlineTable {
    /// The live records, in authored order.
    #[serde(default)]
    pub records: Vec<DeadlineRecord>,
    /// Whether [`arm`](Self::arm) has run for this mission.
    ///
    /// An explicit latch rather than "is `records` empty", so a world that
    /// authors no deadlines is not re-armed every tick, and so a restored table
    /// (which is armed by definition) is never armed a second time.
    #[serde(default)]
    pub armed: bool,
}

impl DeadlineTable {
    /// Arm every authored deadline at `now_tick`, returning the
    /// [`ScheduledCall`]s the caller must push onto the **existing**
    /// `pending_callbacks` queue.
    ///
    /// Called once, on the first simulation tick of the mission — the same tick
    /// `anchor_mission_clock` stamps — so `due_secs` measures from mission start
    /// and a long lobby costs a mission none of its deadlines (the #960 fix,
    /// applied to this vocabulary from the outset).
    ///
    /// A deadline whose handler is missing from `handlers` is skipped and left
    /// out of the table entirely: that pairing is proved at load by
    /// [`validate_deadline_handlers`](crate::world::script::validate::validate_deadline_handlers),
    /// whose error finding blocks activation, so reaching this arm is already
    /// the impossible case — dropping it here keeps a record that can never fire
    /// from showing the crew a countdown that never ends.
    pub fn arm(
        &mut self,
        authored: &[Deadline],
        handlers: &[DeadlineHandler],
        now_tick: u64,
        tick_hz: f32,
    ) -> Vec<ScheduledCall> {
        self.arm_scoped(authored, handlers, now_tick, tick_hz, None)
    }

    /// Arm one world's authored deadlines in its own local-id namespace.
    pub fn arm_scoped(
        &mut self,
        authored: &[Deadline],
        handlers: &[DeadlineHandler],
        now_tick: u64,
        tick_hz: f32,
        origin_layer: Option<&str>,
    ) -> Vec<ScheduledCall> {
        if origin_layer.is_none() {
            self.armed = true;
        }
        let mut queued = Vec::new();
        for deadline in authored {
            let Some(handler) = handlers.iter().find(|h| h.deadline_id == deadline.id) else {
                continue;
            };
            let due_tick = now_tick + seconds_to_ticks(deadline.due_secs, tick_hz);
            let call = ScheduledCall {
                fire_tick: due_tick,
                script_path: handler.source_path.clone(),
                fn_name: handler.handler.clone(),
                origin_layer: origin_layer.map(str::to_string),
            };
            queued.push(call.clone());
            self.records.push(DeadlineRecord {
                id: deadline.id.clone(),
                origin_layer: origin_layer.map(str::to_string),
                label: deadline.label.clone(),
                visible: deadline.visible,
                due_tick,
                state: DeadlineState::Pending,
                armed: Some(call),
            });
        }
        queued
    }

    /// The record for `id`, or `None`.
    pub fn get(&self, id: &str) -> Option<&DeadlineRecord> {
        self.get_scoped(None, id)
    }

    /// The record for local `id` in `origin_layer`'s namespace.
    pub fn get_scoped(&self, origin_layer: Option<&str>, id: &str) -> Option<&DeadlineRecord> {
        self.records
            .iter()
            .find(|r| r.id == id && r.origin_layer.as_deref() == origin_layer)
    }

    /// Remove every record owned by `origin_layer`, returning its still-armed
    /// calls so the adapter can retract exact queue entries before callbacks
    /// drain in that tick.
    pub fn remove_origin(&mut self, origin_layer: &str) -> Vec<ScheduledCall> {
        let mut armed = Vec::new();
        self.records.retain(|record| {
            if record.origin_layer.as_deref() == Some(origin_layer) {
                if let Some(call) = &record.armed {
                    armed.push(call.clone());
                }
                false
            } else {
                true
            }
        });
        armed
    }

    /// Whether anything is authored here **and** the table has never armed.
    ///
    /// This is the payload's `skip_serializing_if` (see
    /// [`crate::snapshot::ScenarioState::deadlines`]), so it decides whether the
    /// table travels at all — and the `armed` latch has to be part of it. An
    /// armed-but-record-less table is a real state: `arm_mission_deadlines`
    /// latches before it walks the authored list, and a world whose every
    /// deadline was subsequently cancelled and removed sits in exactly that
    /// state. Testing `records` alone serialised such a table as ABSENT, so the
    /// restore wrote back a `Default` with `armed = false` and the resumed
    /// mission re-armed from the world file — the silent re-arming this field
    /// exists to prevent, and a state `sim_digest::fold_scenario_records`
    /// distinguishes but the payload could not round-trip.
    pub fn is_empty(&self) -> bool {
        self.records.is_empty() && !self.armed
    }

    /// How many whole seconds are left on `id` at `now_tick`.
    ///
    /// * A **pending** deadline reports its remaining seconds, rounded *up*, so
    ///   a countdown reads `1` for the whole of its last second and reaches `0`
    ///   only on the tick it is actually due.
    /// * A **fired** deadline reports `0` — it has no time left, and saying so
    ///   is more useful to a condition than a sentinel.
    /// * A **cancelled** deadline, and an **unknown** id, report
    ///   [`NO_DEADLINE`]. Script is integer-only (`no_float`), so there is no
    ///   `Option` to return; the two cases are told apart by
    ///   [`state_of`](Self::state_of), which names the unknown id explicitly.
    pub fn remaining_secs(&self, id: &str, now_tick: u64, tick_hz: f32) -> i64 {
        self.remaining_secs_scoped(None, id, now_tick, tick_hz)
    }

    /// Scoped form of [`remaining_secs`](Self::remaining_secs).
    pub fn remaining_secs_scoped(
        &self,
        origin_layer: Option<&str>,
        id: &str,
        now_tick: u64,
        tick_hz: f32,
    ) -> i64 {
        match self.get_scoped(origin_layer, id) {
            Some(record) => match record.state {
                DeadlineState::Pending => {
                    ticks_to_secs_ceil(record.due_tick.saturating_sub(now_tick), tick_hz)
                }
                DeadlineState::Fired => 0,
                DeadlineState::Cancelled => NO_DEADLINE,
            },
            None => NO_DEADLINE,
        }
    }

    /// The state label for `id`, or `"unknown"` for an id this world never
    /// authored — the one answer [`remaining_secs`](Self::remaining_secs) cannot
    /// give, because a cancelled deadline and a typo both have no time left.
    pub fn state_of(&self, id: &str) -> &'static str {
        self.state_of_scoped(None, id)
    }

    /// Scoped form of [`state_of`](Self::state_of).
    pub fn state_of_scoped(&self, origin_layer: Option<&str>, id: &str) -> &'static str {
        match self.get_scoped(origin_layer, id) {
            Some(record) => record.state.as_str(),
            None => "unknown",
        }
    }

    /// Apply one buffered mutation, returning the edit the caller must make to
    /// the existing `pending_callbacks` queue.
    ///
    /// `None` — no edit, nothing changed — for an unknown id or for a deadline
    /// that has already fired or been cancelled. Spending a deadline twice is a
    /// no-op rather than an error: a handler that slips a deadline it has just
    /// watched fire is a scenario-authoring mistake, not a crash, and the state
    /// it can read back says so.
    pub fn apply(
        &mut self,
        change: &DeadlineChange,
        now_tick: u64,
        tick_hz: f32,
    ) -> Option<QueueEdit> {
        let record = self
            .records
            .iter_mut()
            .find(|r| r.id == change.id && r.origin_layer == change.origin_layer)?;
        if record.state != DeadlineState::Pending {
            return None;
        }
        let retract = record.armed.take();
        match change.mutation {
            DeadlineMutation::Slip { by_secs } => {
                // Signed on purpose: a slip may pull a deadline IN as well as
                // push it out, and `seconds_to_ticks` already floors a
                // non-positive delay at "the next tick". Measured from the
                // deadline's own due tick, never from `now_tick`, so slipping
                // twice adds up the way an author reads it.
                record.due_tick = if by_secs >= 0 {
                    record.due_tick + seconds_to_ticks(by_secs, tick_hz)
                } else {
                    record
                        .due_tick
                        .saturating_sub(seconds_to_ticks(-by_secs, tick_hz))
                        .max(now_tick)
                };
                // Re-keyed, never re-resolved: the unit and fn a deadline fires
                // are fixed at arm time, and a slip moves only WHEN. A pending
                // record with nothing armed cannot happen on a live table — but
                // rather than assert that, this simply queues nothing, so the
                // due tick still moves and the panel still counts down.
                let call = retract.as_ref().map(|old| ScheduledCall {
                    fire_tick: record.due_tick,
                    ..old.clone()
                });
                record.armed = call.clone();
                Some(QueueEdit {
                    retract,
                    push: call,
                })
            }
            DeadlineMutation::Cancel => {
                record.state = DeadlineState::Cancelled;
                Some(QueueEdit {
                    retract,
                    push: None,
                })
            }
        }
    }

    /// Mark every deadline whose queued call is in `due` as fired.
    ///
    /// Called by the *existing* `tick_script_callbacks` drain with the calls it
    /// just split off, before it dispatches them — so a deadline's own handler
    /// reads its state as `"fired"`, which is the honest answer at the moment it
    /// runs. This is a lookup inside a drain that already happens, not a second
    /// drain: nothing here reads a clock or scans for due work.
    pub fn note_fired(&mut self, due: &[ScheduledCall]) {
        for call in due {
            if let Some(record) = self
                .records
                .iter_mut()
                .find(|r| r.state == DeadlineState::Pending && r.armed.as_ref() == Some(call))
            {
                record.state = DeadlineState::Fired;
                record.armed = None;
            }
        }
    }
}

/// What [`DeadlineTable::remaining_secs`] reports for a cancelled or unknown
/// deadline — the integer-only stand-in for "there is no countdown here".
///
/// A negative number rather than `0`, because `0` is a real answer (a deadline
/// that has just fired) and the two must never read the same to a condition.
pub const NO_DEADLINE: i64 = -1;

/// Whole seconds spanned by `ticks` at `hz`, rounded **up**.
///
/// Rounds up so a countdown shows `1` for the whole of its final second and
/// `0` only once the deadline is genuinely due; rounding down would show `0`
/// for a whole second while the deadline was still pending. `hz` at or below
/// zero is not a rate — no world can author one (`parse_world` validates the
/// authored floor), and the guard here means a bare fixture cannot divide by it
/// either.
fn ticks_to_secs_ceil(ticks: u64, hz: f32) -> i64 {
    if hz <= 0.0 {
        return 0;
    }
    ((ticks as f64) / (hz as f64)).ceil() as i64
}

#[cfg(test)]
#[path = "deadlines_tests.rs"]
mod tests;
