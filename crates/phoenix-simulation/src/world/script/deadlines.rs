//! The `deadlines` script vocabulary (issue #1024).
//!
//! Two halves, on the two engines, mirroring how triggers already work — the
//! loading engine *declares*, the runtime engine *reads and mutates*:
//!
//! ```rhai
//! // Loading engine, at a unit's top level: name the fn a deadline runs.
//! on_deadline("transfer_window_opens", "on_transfer_window");
//!
//! // Runtime engine, inside any handler:
//! fn on_strike_settled(ctx) {
//!     if ctx.deadlines.remaining("transfer_window_opens") < 60 {
//!         ctx.deadlines.slip("transfer_window_opens", 120);
//!     }
//!     if ctx.deadlines.state("stabiliser_failure") == "pending" {
//!         ctx.deadlines.cancel("stabiliser_failure");
//!     }
//! }
//! ```
//!
//! [`Deadlines`] is the first `ctx` handle that is genuinely **read/write**
//! rather than write-only: `ctx.effects` and `ctx.schedule` only buffer, and
//! `ctx.flags` reads back only what the same call wrote. So it follows
//! [`Flags`](super::flags::Flags)' shape deliberately — a per-call snapshot of
//! the live table, mutated in place for read-after-write, with the *mutations*
//! recorded separately for the host to apply for real.
//!
//! # Why the mutations are recorded rather than applied
//!
//! Slipping a deadline edits `WorldScriptRuntime::pending_callbacks` — the
//! existing deferred-work queue — and a script call holds no handle on it. So
//! `slip`/`cancel` buffer a [`DeadlineChange`] onto the call's
//! [`CallEffects::deadline_changes`](super::schedule::CallEffects::deadline_changes),
//! and the Bevy adapter replays them against the real table, taking each
//! returned [`QueueEdit`](crate::world::deadlines::QueueEdit) to the queue.
//! Buffered, not deferred: they apply in the same tick, at the same point as
//! the call's other effects. On the failure path the buffer is dropped whole
//! with the rest of the call's effects (settled decision 10), so a raising
//! handler slips nothing.
//!
//! # Integer-only (`no_float`)
//!
//! `remaining` returns whole seconds and `slip` takes them, matching the rest of
//! the script surface. The seconds→tick conversion happens once, in the pure
//! table, through the same
//! [`seconds_to_ticks`](super::schedule::seconds_to_ticks) the callback queue
//! uses — so a deadline and an `after(n, …)` authored for the same moment land
//! on the same tick.

use std::sync::{Arc, Mutex};

use rhai::ImmutableString;

use crate::world::deadlines::{DeadlineChange, DeadlineHandler, DeadlineMutation, DeadlineTable};
use crate::world::script::engine::BuilderState;
use crate::world::script::registry::HostRegistry;

/// The `deadlines` custom type handed to a script call.
///
/// Cloneable and interior-mutable like [`Flags`](super::flags::Flags): the clone
/// in the context map and the clone the host retains share one snapshot and one
/// change buffer, so the host observes every mutation the script authored.
///
/// `now_tick` and `tick_hz` come from the call's
/// [`SchedClock`](super::schedule::SchedClock) — the same clock a deferred
/// effect is stamped against — so "remaining" is measured against exactly the
/// tick the handler is running on.
#[derive(Clone)]
pub struct Deadlines {
    /// A snapshot of the live table, mutated in place so a `remaining` read
    /// *after* a `slip` in the same call sees the new time. Discarded when the
    /// call ends; the real table is moved by the adapter replaying `changes`.
    snapshot: Arc<Mutex<DeadlineTable>>,
    /// The mutations, in authored order, for the host to drain.
    changes: Arc<Mutex<Vec<DeadlineChange>>>,
    now_tick: u64,
    tick_hz: f32,
    origin_layer: Option<String>,
}

impl Deadlines {
    /// A fresh per-call view over a snapshot of `base`, measured at `now_tick`.
    pub fn new(base: &DeadlineTable, now_tick: u64, tick_hz: f32) -> Self {
        Self::with_origin(base, now_tick, tick_hz, None)
    }

    /// A per-call view scoped to the world that owns the running handler.
    pub fn with_origin(
        base: &DeadlineTable,
        now_tick: u64,
        tick_hz: f32,
        origin_layer: Option<&str>,
    ) -> Self {
        Self {
            snapshot: Arc::new(Mutex::new(base.clone())),
            changes: Arc::new(Mutex::new(Vec::new())),
            now_tick,
            tick_hz,
            origin_layer: origin_layer.map(str::to_string),
        }
    }

    /// Whole seconds left on `id` — see
    /// [`DeadlineTable::remaining_secs`](crate::world::deadlines::DeadlineTable::remaining_secs)
    /// for what a fired, cancelled or unknown deadline reports.
    fn remaining(&self, id: &str) -> i64 {
        self.snapshot
            .lock()
            .expect("deadline snapshot lock")
            .remaining_secs_scoped(
                self.origin_layer.as_deref(),
                id,
                self.now_tick,
                self.tick_hz,
            )
    }

    /// `"pending"` / `"fired"` / `"cancelled"`, or `"unknown"` for an id this
    /// world never authored.
    fn state(&self, id: &str) -> String {
        self.snapshot
            .lock()
            .expect("deadline snapshot lock")
            .state_of_scoped(self.origin_layer.as_deref(), id)
            .to_string()
    }

    /// Record a mutation and apply it to the snapshot, so the rest of this call
    /// reads the value it just wrote.
    fn push(&self, id: &str, mutation: DeadlineMutation) {
        let change = DeadlineChange {
            id: id.to_string(),
            origin_layer: self.origin_layer.clone(),
            mutation,
        };
        self.snapshot.lock().expect("deadline snapshot lock").apply(
            &change,
            self.now_tick,
            self.tick_hz,
        );
        self.changes
            .lock()
            .expect("deadline changes lock")
            .push(change);
    }

    /// Drain the buffered mutations. Called by the host on the success path
    /// only — on the failure path the buffer is dropped whole with the rest of
    /// the call's effects.
    pub fn take_changes(&self) -> Vec<DeadlineChange> {
        std::mem::take(&mut *self.changes.lock().expect("deadline changes lock"))
    }
}

/// Register the runtime `deadlines` vocabulary on a runtime engine.
///
/// Read verbs are plain fns rather than an indexer (the shape
/// [`register_flags`](super::flags::register_flags) uses) because a deadline has
/// two readable properties, not one value: `remaining` and `state` answer
/// different questions and an indexer could only serve one of them.
pub(crate) fn register_deadlines(engine: &mut HostRegistry) {
    engine.register_type_with_name::<Deadlines>("Deadlines");

    engine.register_fn(
        "remaining",
        |d: &mut Deadlines, id: ImmutableString| -> i64 { d.remaining(&id) },
    );
    engine.register_fn(
        "state",
        |d: &mut Deadlines, id: ImmutableString| -> String { d.state(&id) },
    );
    engine.register_fn(
        "slip",
        |d: &mut Deadlines, id: ImmutableString, by_secs: i64| {
            d.push(&id, DeadlineMutation::Slip { by_secs });
        },
    );
    engine.register_fn("cancel", |d: &mut Deadlines, id: ImmutableString| {
        d.push(&id, DeadlineMutation::Cancel);
    });
}

/// Register the loading-engine `on_deadline("id", "handler")` declaration.
///
/// The twin of the trigger builders in [`super::triggers`], and registered for
/// the same reason: a handler fn's owning *unit* is only knowable while that
/// unit's top level is running, and the unit path is half of the
/// [`ScheduledCall`](super::schedule::ScheduledCall) key the deadline arms with
/// (anon and short fn names are not unique across files — the M0 spike).
///
/// Unlike a trigger registration this returns nothing to chain onto: a
/// deadline's *when* is authored in its `[[deadline]]` block, not here, and its
/// `.when(…)`-style gating is ordinary control flow inside the handler.
pub(crate) fn register_deadline_builders(
    engine: &mut HostRegistry,
    state: Arc<Mutex<BuilderState>>,
) {
    engine.register_fn(
        "on_deadline",
        move |deadline_id: ImmutableString, handler: ImmutableString| {
            let mut s = state.lock().expect("builder state lock");
            let source_path = s.current_path.clone();
            s.deadline_handlers.push(DeadlineHandler {
                deadline_id: deadline_id.to_string(),
                handler: handler.to_string(),
                source_path,
            });
        },
    );
}

#[cfg(test)]
#[path = "deadlines_tests.rs"]
mod tests;
