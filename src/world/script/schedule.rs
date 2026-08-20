//! Deferred scheduling for the Rhai host (issue #981, milestone M3).
//!
//! M1 gave a script an effect buffer that drains *this tick*; M2 gave it the
//! trigger front-end. M3 lets a handler defer work to a *future* tick, two ways,
//! both of which reuse machinery that already exists rather than standing up a
//! second scheduler:
//!
//! * **`ctx.schedule.in_seconds(n).<effect>(…)`** — a delayed *effect*. The
//!   builder stamps the delay onto a buffered effect and the host turns each into
//!   a [`DelayedAction`](crate::world::delayed::DelayedAction) that flows through
//!   the **existing** `tick_delayed_actions` queue
//!   (`WorldContentRuntime::pending_delayed_actions`), dispatched exactly as a
//!   TOML `action_delays` entry is. Script gets no new deferred-effect vocabulary:
//!   every builder verb maps to the [`TriggerAction`] the declarative front-end
//!   already produces, so the applier is untouched.
//! * **`ctx.schedule.after(n, |ctx| { … })`** — a deferred *callback*. The
//!   anonymous closure acquires a stable `anon$<hex>` name at load (the fixed
//!   hashing seed, wired in M1, makes that name reproducible across processes and
//!   peers — see [`super::HASHING_SEED`]). What is scheduled is the serialisable
//!   [`ScheduledCall`] key `(fire_tick, script_path, fn_name)`: the path is part
//!   of the key because anonymous names are **not** unique across files (M0
//!   spike). At the target tick the host resolves the name against that unit's
//!   retained AST and calls it with a fresh context.
//!
//! # Integer seconds in, tick / elapsed out (`no_float`)
//!
//! The script API is integer-only, so `in_seconds` / `after` take an `INT`
//! (whole seconds). The seconds→f32-elapsed (delayed effects) and seconds→tick
//! (callbacks) conversions both happen here, at the host-fn drain boundary
//! ([`ScheduleSink::drain`]), against the [`SchedClock`] the caller reads from
//! `SimTick` / `Time` / the authored `sim_tick_hz`.
//!
//! # Per-tick budgets (the M1 `TODO(M3)`)
//!
//! [`TickBudget`] enforces the two aggregate safety limits M1 defined but left
//! unenforced: [`MAX_OPS_PER_TICK`](super::MAX_OPS_PER_TICK) summed across every
//! call in a tick, and [`MAX_CALLS_PER_TICK`](super::MAX_CALLS_PER_TICK). Both are
//! circuit breakers: once either trips, the budget refuses the tick's remaining
//! calls and the tick completes. Every peer sums the same operations and calls in
//! the same order, so every peer trips on the same tick — the trip is a pure
//! function of the call/op sequence, never a per-peer divergence.

use std::sync::{Arc, Mutex};

use rhai::{Engine, EvalAltResult, FnPtr, ImmutableString};
use serde::{Deserialize, Serialize};

use crate::world::config::TriggerAction;
use crate::world::delayed::DelayedAction;
use crate::world::script::effects::BufferedEffect;
use crate::world::script::{MAX_CALLS_PER_TICK, MAX_OPS_PER_TICK};

/// Everything one script call produced: its immediate effects and the deferred
/// work it scheduled.
///
/// `commands` are applied this tick (effects and flag writes, in the order the
/// script authored them); `delayed` extend the existing `pending_delayed_actions`
/// queue; `callbacks` extend the serialisable [`PendingCallbacks`] queue;
/// `comms_opens` extend `WorldScriptRuntime::pending_comms_opens`. Not
/// `PartialEq` because [`DelayedAction`] is not — tests compare the fields they
/// care about.
#[derive(Debug, Default, Clone)]
pub struct CallEffects {
    /// Immediate effects + flag writes, in authored order. Each is a
    /// [`BufferedEffect`]: a resolved command, or a name-resolving action the
    /// applier dispatches (issue #984, M6).
    pub commands: Vec<BufferedEffect>,
    /// Delayed effects from `in_seconds(n).<verb>(…)`, absolute fire time stamped.
    pub delayed: Vec<DelayedAction>,
    /// Deferred callbacks from `after(n, |ctx| …)`, absolute fire tick stamped.
    pub callbacks: Vec<ScheduledCall>,
    /// Comms threads the call asked to open with `ctx.effects.open_comms(#{…})`,
    /// each stamped with the running unit's script path (issue #984). A fourth
    /// FIELD rather than a `BufferedEffect` variant because an open is comms
    /// vocabulary the applier has no resources for — see
    /// [`EffectSink`](super::effects::EffectSink).
    pub comms_opens: Vec<crate::comms::content::OpenCommsRequest>,
    /// Named-deadline mutations from `ctx.deadlines.slip(…)` / `.cancel(…)`, in
    /// authored order (issue #1024). A FIFTH field for `comms_opens`' reason: a
    /// deadline mutation edits `WorldScriptRuntime::pending_callbacks`, a queue
    /// the generic action applier holds no handle on. Buffered, never deferred —
    /// the adapter replays them in the same tick, at the same point as the call's
    /// other effects.
    pub deadline_changes: Vec<crate::world::deadlines::DeadlineChange>,
    /// Commitment mutations from `ctx.commitments.record(…)` / `.keep(…)` /
    /// `.break_promise(…)`, in authored order (issue #1029). A SIXTH field for
    /// `deadline_changes`' reason: the generic action applier holds no handle on
    /// the ledger, and a promise is not an `ActionCmd`.
    ///
    /// The campaign flag a resolution writes is deliberately NOT here — that
    /// half is an ordinary `MutateFlag` in `commands`, so an `on_flag_set`
    /// trigger authored against `commitment.<id>.kept` chains through machinery
    /// that already exists.
    pub commitment_changes: Vec<crate::world::commitments::CommitmentChange>,
}

/// The clock a deferred-work drain stamps absolute fire times against.
///
/// Built by the caller from the fixed-tick simulation state: `tick` is the
/// current [`SimTick`](crate::sim_tick::SimTick), `elapsed_secs` the mission-clock
/// elapsed seconds (the same origin `action_delays` and `on_timer` use), and
/// `tick_hz` the authored `[global] sim_tick_hz`. Kept as one small `Copy` struct
/// so a call site threads a single value rather than three.
#[derive(Clone, Copy, Debug)]
pub struct SchedClock {
    /// Current logical tick.
    pub tick: u64,
    /// Mission-clock elapsed seconds (delayed-effect origin).
    pub elapsed_secs: f32,
    /// Authored simulation tick rate, for the seconds→tick conversion.
    pub tick_hz: f32,
}

impl SchedClock {
    /// A zero clock at the default 60 Hz rate — for callers that only want a
    /// call's immediate effects and never inspect the schedule it produces. The
    /// rate here is inert (nothing deferred is read back); it matches the
    /// `default_sim_tick_hz` the world config falls back to so a stray callback
    /// still converts sanely.
    pub const ZERO: Self = Self {
        tick: 0,
        elapsed_secs: 0.0,
        tick_hz: 60.0,
    };
}

/// Convert an integer-seconds delay to a whole number of sim ticks at `hz`.
///
/// Rounds to the nearest tick, so a scenario authored in seconds lands on a tick
/// boundary the same way every peer computes it (identical `hz`, identical
/// rounding). A non-positive delay fires on the next tick (`0`), matching the
/// delayed-action queue's boundary-inclusive `fire_at`.
pub fn seconds_to_ticks(secs: i64, hz: f32) -> u64 {
    if secs <= 0 {
        return 0;
    }
    ((secs as f64) * (hz as f64)).round().max(0.0) as u64
}

/// A deferred script callback, serialised as `(fire_tick, script_path, fn_name)`.
///
/// The path is load-bearing, not decoration: anonymous `anon$…` names are unique
/// *within* a file but collide *across* files (M0 spike), so a bare
/// `(tick, fn_name)` key could resolve to the wrong unit's closure after a
/// reload. `vellum_script::call_fn` already takes the content-relative path, so
/// the resolver has everything this key records.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScheduledCall {
    /// The tick this callback becomes due (`now_tick >= fire_tick`).
    pub fire_tick: u64,
    /// Content-relative path of the unit whose AST defines `fn_name`.
    pub script_path: String,
    /// The (possibly generated `anon$…`) name to call at `fire_tick`.
    pub fn_name: String,
}

/// The serialisable pending-callback queue.
///
/// A newtype over an ordered `Vec` so a save round-trips it as a unit and the
/// due/still-pending split preserves authored order (the same ordering guarantee
/// [`partition_delayed_actions`](crate::world::delayed::partition_delayed_actions)
/// gives the delayed-effect queue).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingCallbacks(pub Vec<ScheduledCall>);

impl PendingCallbacks {
    /// A fresh empty queue.
    pub fn new() -> Self {
        Self::default()
    }

    /// Append a scheduled callback.
    pub fn push(&mut self, call: ScheduledCall) {
        self.0.push(call);
    }

    /// Append several scheduled callbacks, preserving their order.
    pub fn extend(&mut self, calls: impl IntoIterator<Item = ScheduledCall>) {
        self.0.extend(calls);
    }

    /// Whether nothing is pending.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Number of pending callbacks.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Remove the first queued callback equal to `call`, returning whether one
    /// was found.
    ///
    /// The retraction half of a named deadline's re-keying (issue #1024): when a
    /// deadline slips or is cancelled, the call it armed is taken back OUT of
    /// this queue, which is what stops a slipped deadline also firing at its old
    /// time. "First equal" rather than "all equal" is deliberate — two deadlines
    /// may legitimately share a handler fn AND a fire tick, producing equal keys,
    /// and retracting one of them must leave the other queued.
    pub fn retract(&mut self, call: &ScheduledCall) -> bool {
        match self.0.iter().position(|queued| queued == call) {
            Some(index) => {
                self.0.remove(index);
                true
            }
            None => false,
        }
    }

    /// Split off the callbacks due at `now_tick` (`now_tick >= fire_tick`),
    /// returning them in original order and retaining the rest, still ordered.
    ///
    /// Pure: no clock read, no dispatch — the caller reads the current tick and
    /// resolves each returned callback against its unit's AST.
    pub fn drain_due(&mut self, now_tick: u64) -> Vec<ScheduledCall> {
        let mut due = Vec::new();
        let mut still_pending = Vec::new();
        for call in std::mem::take(&mut self.0) {
            if now_tick >= call.fire_tick {
                due.push(call);
            } else {
                still_pending.push(call);
            }
        }
        self.0 = still_pending;
        due
    }
}

/// The per-tick operation and call budget (the M1 `TODO(M3)`).
///
/// A running sum of operations and calls across every script call in one tick,
/// with a single sticky `tripped` flag. It is *reset once per tick* and threaded
/// through every [`RuntimeHost::call`](super::engine::RuntimeHost::call) that
/// tick, so the aggregate spans all chaining passes exactly as the M0 spike's
/// caps require.
///
/// Determinism: `admit_call` and `charge_ops` are pure state transitions over the
/// call/op sequence. Two peers that make the same calls, charging the same
/// operation counts in the same order, reach `tripped` on the same call — so a
/// tripped budget drops the *same* remaining work on every peer rather than
/// diverging.
#[derive(Clone, Debug, Default)]
pub struct TickBudget {
    ops_used: u64,
    calls_used: u32,
    tripped: bool,
}

impl TickBudget {
    /// A fresh budget for the start of a tick.
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether the budget has tripped (either aggregate reached).
    pub fn tripped(&self) -> bool {
        self.tripped
    }

    /// Operations charged so far this tick.
    pub fn ops_used(&self) -> u64 {
        self.ops_used
    }

    /// Calls admitted so far this tick.
    pub fn calls_used(&self) -> u32 {
        self.calls_used
    }

    /// Whether the NEXT [`admit_call`](Self::admit_call) would be admitted —
    /// pure, so a caller can pre-flight a call it must refuse *visibly* rather
    /// than discover the refusal from an empty result (issue #984).
    ///
    /// [`tripped`](Self::tripped) alone is NOT that predicate and testing it
    /// instead is the bug this exists to remove: the call that *reaches*
    /// [`MAX_CALLS_PER_TICK`] is refused and trips the budget in the same step,
    /// so a `tripped()` pre-flight passes on a call that is about to be
    /// dropped. [`admit_call`](Self::admit_call) is implemented over this, so
    /// the gate a caller tests and the gate the host applies cannot drift.
    pub fn can_admit(&self) -> bool {
        !self.tripped && self.calls_used < MAX_CALLS_PER_TICK
    }

    /// Reserve a call slot. Returns `false` — dropping the call — when the tick
    /// is already tripped or the call cap [`MAX_CALLS_PER_TICK`] is reached.
    /// Reaching the cap trips the budget so every following call this tick is
    /// dropped too.
    pub fn admit_call(&mut self) -> bool {
        if !self.can_admit() {
            // Reaching the cap trips the budget (an already-tripped one simply
            // stays tripped), so every following call this tick is dropped too.
            self.tripped = true;
            return false;
        }
        self.calls_used += 1;
        true
    }

    /// Charge a completed call's operations. Reaching the aggregate
    /// [`MAX_OPS_PER_TICK`] trips the budget, so the tick's remaining calls are
    /// dropped by the next [`admit_call`](Self::admit_call).
    pub fn charge_ops(&mut self, ops: u64) {
        self.ops_used = self.ops_used.saturating_add(ops);
        if self.ops_used >= MAX_OPS_PER_TICK {
            self.tripped = true;
        }
    }
}

/// One buffered piece of deferred work, still carrying a *relative* delay.
///
/// Held relative (seconds from now) until [`ScheduleSink::drain`] stamps the
/// absolute fire time from the [`SchedClock`], so a handler never has to know the
/// current tick to schedule against it.
enum Deferred {
    /// A delayed effect: `in_seconds(delay).<verb>(…)`, as the same
    /// `TriggerAction` the TOML front-end builds. Boxed so this variant does not
    /// dwarf `Callback` (`clippy::large_enum_variant`): `TriggerAction` grew once
    /// objective contributions rode along on `AddObjective` (issue #1110).
    Effect {
        delay_secs: i64,
        action: Box<TriggerAction>,
    },
    /// A deferred callback: `after(delay, |ctx| …)`, by its (generated) name.
    Callback { delay_secs: i64, fn_name: String },
}

/// A call-scoped buffer of deferred work.
///
/// Cloneable and interior-mutable like [`EffectSink`](super::effects::EffectSink):
/// the clone handed into the context map and the clone the host retains share one
/// buffer, so the host observes everything the script scheduled. Dropped whole on
/// the failure path with the rest of the call's effects (settled decision 10).
#[derive(Clone, Default)]
pub struct ScheduleSink(Arc<Mutex<Vec<Deferred>>>);

impl ScheduleSink {
    /// A fresh, empty buffer for one call.
    pub fn new() -> Self {
        Self::default()
    }

    fn push(&self, item: Deferred) {
        self.0.lock().expect("schedule sink lock").push(item);
    }

    /// Number of buffered items (test/introspection helper).
    pub fn len(&self) -> usize {
        self.0.lock().expect("schedule sink lock").len()
    }

    /// Whether nothing was scheduled.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Drain the buffer, stamping absolute fire times from `clock` and
    /// attributing every callback to `script_path`.
    ///
    /// Delayed effects become [`DelayedAction`]s ready to extend
    /// `pending_delayed_actions` (`fire_at_elapsed = clock.elapsed_secs + delay`,
    /// the seconds→f32 conversion). Callbacks become [`ScheduledCall`]s
    /// (`fire_tick = clock.tick + seconds_to_ticks(delay)`, the seconds→tick
    /// conversion). Order within each kind is preserved.
    pub fn drain(
        &self,
        clock: &SchedClock,
        script_path: &str,
    ) -> (Vec<DelayedAction>, Vec<ScheduledCall>) {
        let items = std::mem::take(&mut *self.0.lock().expect("schedule sink lock"));
        let mut delayed = Vec::new();
        let mut callbacks = Vec::new();
        for item in items {
            match item {
                Deferred::Effect { delay_secs, action } => {
                    delayed.push(DelayedAction {
                        action: *action,
                        // Script-scheduled work is authored at base scope in M3
                        // (no sub-world layer origin to thread yet), mirroring the
                        // effect sink's `loader_path: None` note.
                        origin_layer: None,
                        entity_name: None,
                        fire_at_elapsed: clock.elapsed_secs + delay_secs.max(0) as f32,
                    });
                }
                Deferred::Callback {
                    delay_secs,
                    fn_name,
                } => {
                    callbacks.push(ScheduledCall {
                        fire_tick: clock.tick + seconds_to_ticks(delay_secs, clock.tick_hz),
                        script_path: script_path.to_string(),
                        fn_name,
                    });
                }
            }
        }
        (delayed, callbacks)
    }
}

/// The `in_seconds(n)` builder handle.
///
/// Carries the delay and a clone of the call's [`ScheduleSink`]; each effect verb
/// on it buffers a delayed effect with that delay. A fresh `in_seconds` produces
/// a fresh builder, so two delayed effects can carry different delays in one call.
#[derive(Clone)]
pub struct Schedule {
    delay_secs: i64,
    sink: ScheduleSink,
}

impl Schedule {
    fn defer(&self, action: TriggerAction) {
        self.sink.push(Deferred::Effect {
            delay_secs: self.delay_secs,
            action: Box::new(action),
        });
    }
}

/// Register the scheduling vocabulary on a runtime engine.
///
/// `ctx.schedule` is a [`ScheduleSink`]; `ctx.schedule.in_seconds(n)` returns a
/// [`Schedule`] builder whose effect verbs mirror the immediate
/// [`Effects`](super::effects) set one-for-one, and `ctx.schedule.after(n, fn)`
/// records a callback by the function pointer's (generated) name.
pub fn register_scheduling(engine: &mut Engine) {
    engine.register_type_with_name::<ScheduleSink>("Schedule");
    engine.register_type_with_name::<Schedule>("DelayBuilder");

    // `ctx.schedule.in_seconds(5).complete_objective("obj")` — the delay builder.
    engine.register_fn("in_seconds", |sink: &mut ScheduleSink, secs: i64| {
        Schedule {
            delay_secs: secs,
            sink: sink.clone(),
        }
    });

    // `ctx.schedule.after(5, |ctx| { … })` — a deferred callback by stable name.
    // The closure acquired its `anon$<hex>` name at load under the fixed seed, so
    // the name recorded here resolves against the same AST on every peer.
    engine.register_fn(
        "after",
        |sink: &mut ScheduleSink, secs: i64, callback: FnPtr| {
            sink.push(Deferred::Callback {
                delay_secs: secs,
                fn_name: callback.fn_name().to_string(),
            });
        },
    );

    // The delayed-effect vocabulary: each verb maps to the exact `TriggerAction`
    // the declarative front-end builds, so the delayed dispatch path is the same
    // one a TOML `action_delays` entry takes.
    engine.register_fn(
        "complete_objective",
        |b: &mut Schedule, id: ImmutableString| {
            b.defer(TriggerAction::CompleteObjective { id: id.to_string() });
        },
    );
    engine.register_fn("fail_objective", |b: &mut Schedule, id: ImmutableString| {
        b.defer(TriggerAction::FailObjective { id: id.to_string() });
    });
    engine.register_fn("reset_trigger", |b: &mut Schedule, id: ImmutableString| {
        b.defer(TriggerAction::ResetTrigger { id: id.to_string() });
    });
    // The deferred twin of `ctx.effects.destroy_entity` (issue #1033). It needed
    // no new machinery, which is the claim worth pinning: a `TriggerAction` is
    // already what this builder buffers, `tick_delayed_actions` already resolves
    // one through `dispatch_action` and applies the whole `DispatchResult`, so a
    // delayed destruction chains its `WorldEvent::Destroyed` by the same route an
    // immediate one does — one tick later, through `pending_world_events`, which is
    // where every delayed action's chaining events already go.
    //
    // Built by the SAME `destroy_entity_action` the immediate verb uses, so the two
    // cannot come apart.
    engine.register_fn(
        "destroy_entity",
        |b: &mut Schedule, entity: ImmutableString| -> Result<(), Box<EvalAltResult>> {
            let action = super::effects::destroy_entity_action(&entity).map_err(|e| {
                Box::new(EvalAltResult::ErrorRuntime(e.into(), rhai::Position::NONE))
            })?;
            b.defer(action);
            Ok(())
        },
    );
    engine.register_fn("load_world", |b: &mut Schedule, path: ImmutableString| {
        b.defer(TriggerAction::LoadWorld {
            path: path.to_string(),
        });
    });
    engine.register_fn("unload_world", |b: &mut Schedule, path: ImmutableString| {
        b.defer(TriggerAction::UnloadWorld {
            path: path.to_string(),
        });
    });
    engine.register_fn("game_over", |b: &mut Schedule, reason: ImmutableString| {
        // `outcome: None` — an undeclared scripted end, matching the immediate
        // `game_over` effect and `TriggerAction::GameOver { outcome: None }`.
        b.defer(TriggerAction::GameOver {
            message: Some(reason.to_string()),
            outcome: None,
        });
    });
    engine.register_fn(
        "game_over",
        |b: &mut Schedule,
         reason: ImmutableString,
         outcome: ImmutableString|
         -> Result<(), Box<EvalAltResult>> {
            // The outcome-DECLARING delayed end, the twin of the immediate
            // two-arg `ctx.effects.game_over` (issue #984). A declarative
            // `game_over` action carries `outcome` and `delay_secs` on the SAME
            // action — combat_test's victory window is exactly that shape — so
            // without this overload a delayed end could only be authored as an
            // undeclared one, and the balance classifier would read a scripted
            // victory as a draw. Validated through the same `Outcome::parse`, so
            // a typo raises and discards the call rather than deferring a bad end.
            let outcome = crate::balance::Outcome::parse(&outcome).map_err(|e| {
                Box::new(EvalAltResult::ErrorRuntime(
                    format!("game_over: {e}").into(),
                    rhai::Position::NONE,
                ))
            })?;
            b.defer(TriggerAction::GameOver {
                message: Some(reason.to_string()),
                outcome: Some(outcome),
            });
            Ok(())
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clock(tick: u64, elapsed: f32, hz: f32) -> SchedClock {
        SchedClock {
            tick,
            elapsed_secs: elapsed,
            tick_hz: hz,
        }
    }

    #[test]
    fn seconds_to_ticks_rounds_at_the_authored_rate() {
        assert_eq!(seconds_to_ticks(5, 60.0), 300);
        assert_eq!(seconds_to_ticks(1, 30.0), 30);
        // Non-positive delays fire on the next tick.
        assert_eq!(seconds_to_ticks(0, 60.0), 0);
        assert_eq!(seconds_to_ticks(-5, 60.0), 0);
    }

    #[test]
    fn pending_callbacks_round_trip_through_serialization() {
        // The `(tick, script_path, fn_name)` key is the serialisable deferred-work
        // record: a save must reload the identical queue.
        let mut queue = PendingCallbacks::new();
        queue.push(ScheduledCall {
            fire_tick: 300,
            script_path: "world.toml#script.setup".to_string(),
            fn_name: "anon$41a691411dc30a5e".to_string(),
        });
        queue.push(ScheduledCall {
            fire_tick: 42,
            script_path: "combat.rhai".to_string(),
            fn_name: "on_reinforce".to_string(),
        });

        let json = serde_json::to_string(&queue).expect("serialises");
        let restored: PendingCallbacks = serde_json::from_str(&json).expect("deserialises");
        assert_eq!(restored, queue, "the pending queue must round-trip exactly");
    }

    #[test]
    fn sink_drain_stamps_absolute_fire_times() {
        // A delayed effect converts seconds→elapsed; a callback converts
        // seconds→tick, both against the clock, and the callback is attributed to
        // the draining unit's path.
        let sink = ScheduleSink::new();
        sink.push(Deferred::Effect {
            delay_secs: 10,
            action: Box::new(TriggerAction::CompleteObjective {
                id: "later".to_string(),
            }),
        });
        sink.push(Deferred::Callback {
            delay_secs: 5,
            fn_name: "anon$abc".to_string(),
        });
        assert_eq!(sink.len(), 2);

        let (delayed, callbacks) = sink.drain(&clock(300, 5.0, 60.0), "combat.rhai");
        assert!(sink.is_empty(), "drain empties the buffer");

        assert_eq!(delayed.len(), 1);
        assert_eq!(delayed[0].fire_at_elapsed, 15.0, "elapsed 5 + delay 10");
        assert!(delayed[0].origin_layer.is_none());

        assert_eq!(
            callbacks,
            vec![ScheduledCall {
                fire_tick: 300 + 5 * 60,
                script_path: "combat.rhai".to_string(),
                fn_name: "anon$abc".to_string(),
            }]
        );
    }

    /// A DELAYED destroy buffers the identical `TriggerAction` the immediate verb
    /// does, stamped with its fire time (issue #1033, AC6).
    ///
    /// The AC is "with no new machinery", and this is what that cashes out to: the
    /// action reaches `pending_delayed_actions` as an ordinary `DelayedAction`, so
    /// `tick_delayed_actions` resolves it through the same `dispatch_action` and
    /// applies the same whole `DispatchResult` — chaining included. Nothing in the
    /// deferred path knows a destroy is different from a spawn.
    #[test]
    fn a_delayed_destroy_entity_defers_the_same_action() {
        use crate::world::script::engine::runtime_engine;
        use rhai::{Dynamic, Map};

        let engine = runtime_engine();
        let ast = engine
            .compile(r#"fn on_x(ctx) { ctx.schedule.in_seconds(8).destroy_entity("skyhook"); }"#)
            .expect("compiles");
        let sink = ScheduleSink::new();
        let mut ctx = Map::new();
        ctx.insert("schedule".into(), Dynamic::from(sink.clone()));
        let _ =
            vellum_script::call_fn(&engine, &ast, "t.rhai", "on_x", ctx).expect("the call runs");

        let (delayed, callbacks) = sink.drain(&clock(0, 2.0, 60.0), "t.rhai");
        assert!(callbacks.is_empty(), "a delayed effect is not a callback");
        assert_eq!(delayed.len(), 1);
        assert_eq!(
            delayed[0].fire_at_elapsed, 10.0,
            "elapsed 2 + delay 8, the seconds→elapsed conversion every delayed \
             effect shares"
        );
        assert_eq!(
            delayed[0].action,
            TriggerAction::DestroyEntity {
                entity: "skyhook".to_string(),
            },
            "byte-identical to what `ctx.effects.destroy_entity` buffers — both \
             build it through `destroy_entity_action`"
        );
    }

    /// The delayed `game_over` overload that DECLARES an outcome (issue #984).
    ///
    /// `combat_test`'s victory window is a declarative `game_over` carrying both
    /// `outcome = "victory"` and `delay_secs = 5.0` on the same action, so
    /// without this the conversion could only defer an UNDECLARED end and the
    /// balance classifier would read a scripted victory as a draw. Validated
    /// through the same `Outcome::parse` as the immediate form, so a typo raises
    /// and the call's whole buffer is discarded rather than a bad end deferred.
    #[test]
    fn a_delayed_game_over_can_declare_its_outcome() {
        use crate::world::script::engine::runtime_engine;
        use rhai::{Dynamic, Map};

        fn deferred(source: &str) -> Result<Vec<DelayedAction>, String> {
            let engine = runtime_engine();
            let ast = engine.compile(source).expect("compiles");
            let sink = ScheduleSink::new();
            let mut ctx = Map::new();
            ctx.insert("schedule".into(), Dynamic::from(sink.clone()));
            vellum_script::call_fn(&engine, &ast, "t.rhai", "on_x", ctx)
                .map(|_| sink.drain(&clock(0, 0.0, 60.0), "t.rhai").0)
                .map_err(|e| e.to_string())
        }

        let delayed =
            deferred(r#"fn on_x(ctx) { ctx.schedule.in_seconds(5).game_over("msg", "victory"); }"#)
                .expect("the call runs");
        assert_eq!(delayed.len(), 1);
        assert_eq!(delayed[0].fire_at_elapsed, 5.0);
        assert_eq!(
            delayed[0].action,
            TriggerAction::GameOver {
                message: Some("msg".to_string()),
                outcome: Some(crate::balance::Outcome::Victory),
            }
        );

        // The one-arg form still defers an UNDECLARED end.
        let undeclared =
            deferred(r#"fn on_x(ctx) { ctx.schedule.in_seconds(5).game_over("msg"); }"#)
                .expect("the call runs");
        assert_eq!(
            undeclared[0].action,
            TriggerAction::GameOver {
                message: Some("msg".to_string()),
                outcome: None,
            }
        );

        // And a bad outcome raises rather than deferring a nonsense end.
        assert!(
            deferred(r#"fn on_x(ctx) { ctx.schedule.in_seconds(5).game_over("m", "victni"); }"#)
                .is_err(),
            "an unparseable outcome must raise, as it does on the immediate form"
        );
    }

    #[test]
    fn drain_due_splits_by_tick_preserving_order() {
        let mut queue = PendingCallbacks::new();
        for (fire, name) in [(10, "a"), (300, "b"), (20, "c"), (5, "d")] {
            queue.push(ScheduledCall {
                fire_tick: fire,
                script_path: "s.rhai".to_string(),
                fn_name: name.to_string(),
            });
        }
        let due = queue.drain_due(20);
        let due_names: Vec<&str> = due.iter().map(|c| c.fn_name.as_str()).collect();
        // `now >= fire`, original order preserved.
        assert_eq!(due_names, vec!["a", "c", "d"]);
        let pending_names: Vec<&str> = queue.0.iter().map(|c| c.fn_name.as_str()).collect();
        assert_eq!(pending_names, vec!["b"]);
    }

    #[test]
    fn retract_removes_one_equal_call_and_leaves_its_twin() {
        // The deadline re-keying primitive (issue #1024). Equal keys are legal —
        // two deadlines may share a handler and a tick — so a retraction takes
        // exactly one.
        let call = |fire: u64, name: &str| ScheduledCall {
            fire_tick: fire,
            script_path: "s.rhai".to_string(),
            fn_name: name.to_string(),
        };
        let mut queue = PendingCallbacks::new();
        queue.push(call(300, "shared"));
        queue.push(call(300, "shared"));
        queue.push(call(600, "other"));

        assert!(
            queue.retract(&call(300, "shared")),
            "the first equal one goes"
        );
        assert_eq!(queue.len(), 2);
        assert!(queue.retract(&call(300, "shared")), "and so does its twin");
        assert_eq!(queue.len(), 1);
        assert!(
            !queue.retract(&call(300, "shared")),
            "a third retraction finds nothing and says so"
        );
        assert_eq!(
            queue.drain_due(600).len(),
            1,
            "the unrelated call is untouched"
        );
    }

    #[test]
    fn budget_call_cap_trips_and_drops_the_rest() {
        let mut budget = TickBudget::new();
        for _ in 0..MAX_CALLS_PER_TICK {
            assert!(budget.admit_call(), "calls under the cap are admitted");
        }
        assert!(
            !budget.tripped(),
            "reaching the cap exactly is still admitted"
        );
        assert!(
            !budget.admit_call(),
            "the call over the cap is dropped and trips the budget"
        );
        assert!(budget.tripped());
        assert!(
            !budget.admit_call(),
            "a tripped budget drops every later call"
        );
    }

    #[test]
    fn can_admit_agrees_with_admit_call_at_every_step() {
        // The pre-flight predicate and the gate must never disagree — including
        // on the call that REACHES the cap, which `tripped()` alone gets wrong.
        let mut budget = TickBudget::new();
        for _ in 0..MAX_CALLS_PER_TICK + 2 {
            let predicted = budget.can_admit();
            assert_eq!(
                predicted,
                budget.admit_call(),
                "can_admit must predict admit_call exactly"
            );
        }
        // And the specific case the pre-flight used to miss: at the cap, the
        // budget has NOT tripped yet, but the next call will be refused.
        let mut budget = TickBudget::new();
        for _ in 0..MAX_CALLS_PER_TICK {
            assert!(budget.admit_call());
        }
        assert!(!budget.tripped(), "reaching the cap exactly does not trip");
        assert!(
            !budget.can_admit(),
            "but the next call is already refused — what `tripped()` could not see"
        );
    }

    #[test]
    fn budget_op_aggregate_trips_across_calls() {
        let mut budget = TickBudget::new();
        // Two calls just under half the aggregate: fine.
        budget.admit_call();
        budget.charge_ops(MAX_OPS_PER_TICK / 2 - 1);
        assert!(!budget.tripped());
        budget.admit_call();
        budget.charge_ops(MAX_OPS_PER_TICK / 2 - 1);
        assert!(!budget.tripped(), "still under the aggregate");
        // The call that crosses the aggregate trips it.
        budget.admit_call();
        budget.charge_ops(2);
        assert!(budget.tripped());
        assert!(
            !budget.admit_call(),
            "once the op aggregate trips, remaining calls are dropped"
        );
    }

    #[test]
    fn budget_trip_point_is_deterministic() {
        // Same op sequence → same trip point on any peer.
        let run = || {
            let mut b = TickBudget::new();
            let mut admitted = 0u32;
            for _ in 0..10 {
                if b.admit_call() {
                    admitted += 1;
                    b.charge_ops(MAX_OPS_PER_TICK / 4);
                }
            }
            (admitted, b.tripped())
        };
        assert_eq!(
            run(),
            run(),
            "the trip is a pure function of the op sequence"
        );
    }
}
