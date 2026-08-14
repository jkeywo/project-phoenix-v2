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

use rhai::{Engine, ImmutableString};

use crate::world::deadlines::{DeadlineChange, DeadlineHandler, DeadlineMutation, DeadlineTable};
use crate::world::script::engine::BuilderState;

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
}

impl Deadlines {
    /// A fresh per-call view over a snapshot of `base`, measured at `now_tick`.
    pub fn new(base: &DeadlineTable, now_tick: u64, tick_hz: f32) -> Self {
        Self {
            snapshot: Arc::new(Mutex::new(base.clone())),
            changes: Arc::new(Mutex::new(Vec::new())),
            now_tick,
            tick_hz,
        }
    }

    /// Whole seconds left on `id` — see
    /// [`DeadlineTable::remaining_secs`](crate::world::deadlines::DeadlineTable::remaining_secs)
    /// for what a fired, cancelled or unknown deadline reports.
    fn remaining(&self, id: &str) -> i64 {
        self.snapshot
            .lock()
            .expect("deadline snapshot lock")
            .remaining_secs(id, self.now_tick, self.tick_hz)
    }

    /// `"pending"` / `"fired"` / `"cancelled"`, or `"unknown"` for an id this
    /// world never authored.
    fn state(&self, id: &str) -> String {
        self.snapshot
            .lock()
            .expect("deadline snapshot lock")
            .state_of(id)
            .to_string()
    }

    /// Record a mutation and apply it to the snapshot, so the rest of this call
    /// reads the value it just wrote.
    fn push(&self, id: &str, mutation: DeadlineMutation) {
        let change = DeadlineChange {
            id: id.to_string(),
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
pub fn register_deadlines(engine: &mut Engine) {
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
pub fn register_deadline_builders(engine: &mut Engine, state: Arc<Mutex<BuilderState>>) {
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
mod tests {
    use super::*;
    use crate::world::deadlines::{Deadline, DeadlineState};
    use crate::world::script::engine::runtime_engine;
    use rhai::{Dynamic, Map};

    const HZ: f32 = 60.0;

    fn table() -> DeadlineTable {
        let mut table = DeadlineTable::default();
        table.arm(
            &[
                Deadline {
                    id: "window".into(),
                    label: "l.window".into(),
                    due_secs: 100,
                    visible: true,
                },
                Deadline {
                    id: "collapse".into(),
                    label: "l.collapse".into(),
                    due_secs: 200,
                    visible: false,
                },
            ],
            &[
                DeadlineHandler {
                    deadline_id: "window".into(),
                    handler: "on_window".into(),
                    source_path: "w.toml#script.setup".into(),
                },
                DeadlineHandler {
                    deadline_id: "collapse".into(),
                    handler: "on_collapse".into(),
                    source_path: "w.toml#script.setup".into(),
                },
            ],
            0,
            HZ,
        );
        table
    }

    /// Run `source`'s `on_x` against a live table, returning what it printed
    /// into a flag-free out-param plus the mutations it buffered.
    fn run(source: &str, now_tick: u64) -> (Vec<DeadlineChange>, Dynamic) {
        let engine = runtime_engine();
        let ast = engine.compile(source).expect("compiles");
        let deadlines = Deadlines::new(&table(), now_tick, HZ);
        let mut ctx = Map::new();
        ctx.insert("deadlines".into(), Dynamic::from(deadlines.clone()));
        let value = vellum_script::call_fn(&engine, &ast, "t.rhai", "on_x", ctx).expect("runs");
        (deadlines.take_changes(), value)
    }

    // ── AC4: remaining time and state are readable from script ──────────────

    #[test]
    fn a_handler_reads_remaining_time_and_state() {
        let (_, value) = run(r#"fn on_x(ctx) { ctx.deadlines.remaining("window") }"#, 0);
        assert_eq!(value.as_int().expect("an INT"), 100);

        let (_, value) = run(
            r#"fn on_x(ctx) { ctx.deadlines.remaining("window") }"#,
            3000,
        );
        assert_eq!(value.as_int().expect("an INT"), 50, "50s in, 50s left");

        let (_, value) = run(r#"fn on_x(ctx) { ctx.deadlines.state("window") }"#, 0);
        assert_eq!(value.into_string().expect("a string"), "pending");

        let (_, value) = run(r#"fn on_x(ctx) { ctx.deadlines.state("nope") }"#, 0);
        assert_eq!(
            value.into_string().expect("a string"),
            "unknown",
            "a typo names itself rather than reading as a cancelled deadline"
        );
    }

    // ── AC3: slip and cancel are callable from script ────────────────────────

    #[test]
    fn slip_and_cancel_buffer_mutations_in_authored_order() {
        let (changes, _) = run(
            r#"fn on_x(ctx) {
                 ctx.deadlines.slip("window", 60);
                 ctx.deadlines.cancel("collapse");
               }"#,
            0,
        );
        assert_eq!(
            changes,
            vec![
                DeadlineChange {
                    id: "window".into(),
                    mutation: DeadlineMutation::Slip { by_secs: 60 },
                },
                DeadlineChange {
                    id: "collapse".into(),
                    mutation: DeadlineMutation::Cancel,
                },
            ]
        );
    }

    #[test]
    fn a_read_after_a_slip_sees_the_new_time_within_the_same_call() {
        // The snapshot-overlay property `Flags` establishes, applied to a
        // handler that slips and then decides what else to do about it.
        let (_, value) = run(
            r#"fn on_x(ctx) {
                 ctx.deadlines.slip("window", 60);
                 ctx.deadlines.remaining("window")
               }"#,
            0,
        );
        assert_eq!(value.as_int().expect("an INT"), 160);

        let (_, value) = run(
            r#"fn on_x(ctx) {
                 ctx.deadlines.cancel("window");
                 ctx.deadlines.state("window")
               }"#,
            0,
        );
        assert_eq!(value.into_string().expect("a string"), "cancelled");
    }

    #[test]
    fn the_live_table_is_untouched_by_a_call() {
        // The call mutates its own snapshot; only the adapter replaying the
        // drained changes moves the real table (and, with it, the queue).
        let live = table();
        let engine = runtime_engine();
        let ast = engine
            .compile(r#"fn on_x(ctx) { ctx.deadlines.cancel("window"); }"#)
            .expect("compiles");
        let deadlines = Deadlines::new(&live, 0, HZ);
        let mut ctx = Map::new();
        ctx.insert("deadlines".into(), Dynamic::from(deadlines.clone()));
        let _ = vellum_script::call_fn(&engine, &ast, "t.rhai", "on_x", ctx).expect("runs");
        assert_eq!(
            live.get("window").expect("still there").state,
            DeadlineState::Pending,
            "a script call never writes the live table directly"
        );
        assert_eq!(deadlines.take_changes().len(), 1);
    }

    #[test]
    fn taking_the_changes_twice_yields_them_once() {
        let deadlines = Deadlines::new(&table(), 0, HZ);
        deadlines.push("window", DeadlineMutation::Cancel);
        assert_eq!(deadlines.take_changes().len(), 1);
        assert!(
            deadlines.take_changes().is_empty(),
            "a drained buffer cannot replay its mutations"
        );
    }
}
