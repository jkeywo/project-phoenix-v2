//! The two Rhai engines and the runtime call host (issue #979, milestone M1).
//!
//! Mirrors last-aeon's host structure (`aeon_data::host`): one sandbox profile
//! (vellum's), two engines that differ only in surface.
//!
//! * [`loading_engine`] registers the *builder* host-fns (`on(event, handler)`)
//!   and is used to run each unit's top level once (`Engine::run_ast`) so those
//!   registrations are collected. Compiling a unit does not resolve types or
//!   functions, and `run_ast` only runs top-level statements — never function
//!   bodies — so the loading engine needs no effect vocabulary.
//! * [`runtime_engine`] is built from `quiet_sandbox` (a script's `print` is
//!   noise at runtime) and carries the runtime vocabulary — the `flags` type
//!   and the effect functions. It only ever `call_fn`s retained functions with
//!   `eval_ast(false)` (through `vellum_script::call_fn`), never re-running top
//!   level.
//!
//! Both apply phoenix's tighter [`MAX_OPS_PER_CALL`] limit over the vellum
//! sandbox's generous 5,000,000 default, and both call [`init_hashing_seed`]
//! defensively before constructing the engine — cheap (a `Once`), and it means
//! even a bare unit test that builds only one engine gets a seeded process.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use rhai::{Engine, Map, AST};

use crate::world::flags::FlagStore;
use crate::world::script::effects::{register_effects, EffectSink};
use crate::world::script::flags::{register_flags, Flags};
use crate::world::script::schedule::{
    register_scheduling, CallEffects, SchedClock, ScheduleSink, TickBudget,
};
use crate::world::script::{init_hashing_seed, MAX_OPS_PER_CALL};

/// One handler registration collected at load time.
///
/// The M1 seam of "script registrations build the same Trigger structs the TOML
/// front-end builds": for now a registration simply records that some `event`
/// is handled by a named function, which [`validate`](super::validate) proves
/// resolves. Later milestones grow this into full `Trigger` construction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Registration {
    /// The event the handler is registered for (opaque string in M1).
    pub event: String,
    /// The name of the script function that handles it.
    pub handler: String,
    /// The content-relative path of the unit that made the registration —
    /// carried so a finding names the right file and so a deferred-work key
    /// `(tick, script_path, fn_name)` has its path (anon names are not unique
    /// across files; M0 spike).
    pub source_path: String,
}

/// One trigger authored through the Rhai front-end (issue #980, milestone M2).
///
/// A top-level call to a registration fn (`on_destroyed`, `on_flag_set`, … —
/// one per [`TriggerCondition`](crate::world::config::TriggerCondition) variant,
/// registered by [`crate::world::script::triggers`]) builds a real
/// [`Trigger`](crate::world::config::Trigger) — the *same* struct the TOML
/// `[[trigger]]` front-end builds, through the shared
/// [`scripted_trigger`](crate::world::config::scripted_trigger) constructor — and
/// records the named handler fn that supplies its effects at runtime. That is
/// the "one evaluator, two front-ends" convergence: the built `trigger` feeds the
/// existing pipeline exactly as a TOML-authored one does, and `handler` is what
/// the cross-reference pass ([`validate`](super::validate)) proves resolves.
#[derive(Clone, Debug, PartialEq)]
pub struct ScriptTrigger {
    /// The trigger this registration built — identical to its TOML equivalent.
    pub trigger: crate::world::config::Trigger,
    /// Name of the script fn that supplies the trigger's effects at runtime.
    pub handler: String,
    /// Content-relative path of the unit that registered it (for findings and
    /// for a deferred-work key; anon names are not unique across files, M0 spike).
    pub source_path: String,
}

/// Mutable state the loading engine accumulates as it runs each unit's top
/// level. Held behind an `Arc<Mutex<_>>` because the builder host-fns are
/// closures registered on the engine.
#[derive(Debug, Default)]
pub struct BuilderState {
    /// Path of the unit currently being run — set by the loader before each
    /// `run_ast` so a registration is attributed to the right file.
    pub current_path: String,
    /// Registrations collected from top-level `on(..)` calls.
    pub registrations: Vec<Registration>,
    /// Triggers built by the typed registration fns (`on_destroyed`, …), one per
    /// `TriggerCondition` variant (issue #980, M2).
    pub script_triggers: Vec<ScriptTrigger>,
}

/// Build the loading engine, registering the builder vocabulary against
/// `state`.
pub fn loading_engine(state: Arc<Mutex<BuilderState>>) -> Engine {
    init_hashing_seed();
    let mut engine = vellum_script::sandbox();
    engine.set_max_operations(MAX_OPS_PER_CALL);

    // `on("event", "handler")` — records that `handler` handles `event`,
    // attributed to the unit currently running.
    let on_state = state.clone();
    engine.register_fn(
        "on",
        move |event: rhai::ImmutableString, handler: rhai::ImmutableString| {
            let mut s = on_state.lock().expect("builder state lock");
            let source_path = s.current_path.clone();
            s.registrations.push(Registration {
                event: event.to_string(),
                handler: handler.to_string(),
                source_path,
            });
        },
    );

    // The typed trigger-builder vocabulary (issue #980, M2): one registration fn
    // per `TriggerCondition` variant, each building a `Trigger` into `state`.
    super::triggers::register_trigger_builders(&mut engine, state);

    engine
}

/// Build the runtime engine with the full runtime vocabulary.
pub fn runtime_engine() -> Engine {
    init_hashing_seed();
    let mut engine = vellum_script::quiet_sandbox();
    engine.set_max_operations(MAX_OPS_PER_CALL);
    register_flags(&mut engine);
    register_effects(&mut engine);
    register_scheduling(&mut engine);
    engine
}

/// The runtime script host: owns the runtime engine and runs retained functions.
///
/// The engine carries an `on_progress` hook that records each call's operation
/// high-water mark into [`ops_counter`](Self::ops_counter), so the host can
/// charge it to a per-tick [`TickBudget`] — the operation-budget half of the M3
/// safety limits.
pub struct RuntimeHost {
    engine: Engine,
    /// High-water operation count of the most recent call, written every
    /// operation by the engine's `on_progress` hook. Read once per call.
    ops_counter: Arc<AtomicU64>,
}

impl Default for RuntimeHost {
    fn default() -> Self {
        Self::new()
    }
}

impl RuntimeHost {
    /// Build a host on a fresh runtime engine, wiring the operation counter.
    pub fn new() -> Self {
        let ops_counter = Arc::new(AtomicU64::new(0));
        let mut engine = runtime_engine();
        let counter = ops_counter.clone();
        // Count operations for the per-tick budget. Always returns `None` — it
        // never aborts a call; the fixed per-call cap is enforced independently
        // by `set_max_operations` (see `runtime_engine`). It only records the
        // running op count so the caller can charge the call to a `TickBudget`.
        engine.on_progress(move |ops| {
            counter.store(ops, Ordering::Relaxed);
            None
        });
        Self {
            engine,
            ops_counter,
        }
    }

    /// The underlying engine (compile ASTs against a matching engine; ASTs are
    /// engine-independent once compiled).
    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    /// Run a retained function under a per-tick `budget`, returning its immediate
    /// effects and the deferred work it scheduled, stamped against `clock`.
    ///
    /// The context map is `extra` with the `flags`, `effects` and `schedule`
    /// handles inserted; a script reads it as its single parameter
    /// (`fn on_x(ctx)`). `base_flags` is the live store the flag overlay snapshots.
    ///
    /// The `budget` is threaded across every call in a tick (the M0 aggregate
    /// caps): a call refused by [`admit_call`](TickBudget::admit_call) — the tick
    /// is already tripped, or the call cap is reached — is **dropped**, returning
    /// empty effects and scheduling nothing, so the tick's remaining script work
    /// is deterministically skipped. A completed call's operations are charged to
    /// the budget, which may trip it for the *next* call.
    ///
    /// This is the failure boundary (settled decision 10). On a script error it
    /// **panics in dev** (`debug_assertions`) and, in release, discards this
    /// call's effects whole, logs, and returns empty effects so the game
    /// continues. Callers that need to inspect the error use [`try_call`].
    ///
    /// [`try_call`]: RuntimeHost::try_call
    pub fn call(
        &self,
        budget: &mut TickBudget,
        clock: &SchedClock,
        ast: &AST,
        path: &str,
        fn_name: &str,
        base_flags: &FlagStore,
        extra: Map,
    ) -> CallEffects {
        if !budget.admit_call() {
            // Dropped: the call cap is reached or the tick has already tripped.
            return CallEffects::default();
        }
        match self.try_call(clock, ast, path, fn_name, base_flags, extra) {
            Ok((effects, ops)) => {
                budget.charge_ops(ops);
                effects
            }
            Err(err) => {
                // Charge the operations the failed call still consumed, so a
                // runaway that trips the per-call cap also counts against the
                // tick (and does so identically on every peer).
                budget.charge_ops(self.ops_counter.load(Ordering::Relaxed));
                if cfg!(debug_assertions) {
                    panic!("{err}");
                }
                // Release: discard the call's effects whole (they were never
                // drained), log, and continue. Plain helper with no log config
                // in scope, so a bare `warn!` per AGENTS.md.
                bevy::log::warn!(
                    target: crate::logging::LogCat::World.target(),
                    "{err}; discarding this call's effects"
                );
                CallEffects::default()
            }
        }
    }

    /// Like [`call`](RuntimeHost::call) but without the budget or the
    /// panic/discard policy: returns the error, and — on success — the call's
    /// effects plus the operation count it consumed (for the caller to charge to
    /// a budget). On error the buffers are dropped whole (never returned).
    pub fn try_call(
        &self,
        clock: &SchedClock,
        ast: &AST,
        path: &str,
        fn_name: &str,
        base_flags: &FlagStore,
        extra: Map,
    ) -> Result<(CallEffects, u64), vellum_script::CallError> {
        let sink = EffectSink::new();
        // Flags share the one ordered command buffer, so a flag write is emitted
        // in authored order, interleaved with effects (issue #981 hazard 2).
        let flags = Flags::new(base_flags, sink.clone());
        let schedule = ScheduleSink::new();

        let mut ctx = extra;
        ctx.insert("effects".into(), rhai::Dynamic::from(sink.clone()));
        ctx.insert("flags".into(), rhai::Dynamic::from(flags));
        ctx.insert("schedule".into(), rhai::Dynamic::from(schedule.clone()));

        // Reset the op counter, then call. On error we return before draining
        // anything, so the effect buffer and the schedule buffer are dropped
        // whole. The returned `Dynamic` is discarded: effects are collected
        // imperatively via the buffers, not from a return value.
        self.ops_counter.store(0, Ordering::Relaxed);
        let _ = vellum_script::call_fn(&self.engine, ast, path, fn_name, ctx)?;
        let ops = self.ops_counter.load(Ordering::Relaxed);

        // `sink` already carries effects and flag writes in authored order.
        let commands = sink.take();
        let (delayed, callbacks) = schedule.drain(clock, path);
        Ok((
            CallEffects {
                commands,
                delayed,
                callbacks,
            },
            ops,
        ))
    }

    /// Convenience for callers wanting only a call's immediate effects: a fresh
    /// per-tick budget and a zero clock. Applies the failure policy. Any deferred
    /// work the call scheduled is discarded, so this is for effect-only handlers
    /// and simple tests.
    pub fn call_immediate(
        &self,
        ast: &AST,
        path: &str,
        fn_name: &str,
        base_flags: &FlagStore,
        extra: Map,
    ) -> Vec<crate::world::dispatch::ActionCmd> {
        let mut budget = TickBudget::new();
        self.call(
            &mut budget,
            &SchedClock::ZERO,
            ast,
            path,
            fn_name,
            base_flags,
            extra,
        )
        .commands
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::config::TriggerAction;
    use crate::world::dispatch::{ActionCmd, FlagMutation};

    /// A clock five minutes and 300 ticks in, at 60 Hz — enough offset that a
    /// stamped fire time is visibly `now + delay`.
    fn clock() -> SchedClock {
        SchedClock {
            tick: 300,
            elapsed_secs: 5.0,
            tick_hz: 60.0,
        }
    }

    #[test]
    fn runtime_engine_builds_and_runs_a_trivial_fn() {
        let host = RuntimeHost::new();
        let ast = host.engine().compile("fn noop(ctx) { }").expect("compiles");
        let cmds = host.call_immediate(&ast, "t.rhai", "noop", &FlagStore::new(), Map::new());
        assert!(cmds.is_empty());
    }

    #[test]
    fn effects_and_flag_writes_emit_in_authored_order() {
        // Issue #981 hazard 2: a flag write authored BEFORE an effect must emit
        // before it, not after every effect. The M1 host appended all flag
        // writes last, so this interleaving would have failed.
        let host = RuntimeHost::new();
        let ast = host
            .engine()
            .compile(
                r#"fn on_x(ctx) {
                    ctx.flags.armed = 1;
                    ctx.effects.complete_objective("obj1");
                    ctx.flags.increment("score", 50);
                    ctx.effects.fail_objective("obj2");
                }"#,
            )
            .expect("compiles");
        let cmds = host.call_immediate(&ast, "t.rhai", "on_x", &FlagStore::new(), Map::new());
        assert_eq!(
            cmds,
            vec![
                ActionCmd::MutateFlag {
                    target_layer: None,
                    name: "armed".to_string(),
                    mutation: FlagMutation::SetValue(1),
                },
                ActionCmd::CompleteObjective {
                    id: "obj1".to_string()
                },
                ActionCmd::MutateFlag {
                    target_layer: None,
                    name: "score".to_string(),
                    mutation: FlagMutation::Increment(50),
                },
                ActionCmd::FailObjective {
                    id: "obj2".to_string()
                },
            ]
        );
    }

    #[test]
    fn in_seconds_stamps_a_delayed_effect() {
        // `in_seconds(n).<verb>(…)` buffers a delayed effect that surfaces as a
        // `DelayedAction` ready for `pending_delayed_actions`, with the delay
        // converted to elapsed seconds at the host boundary.
        let host = RuntimeHost::new();
        let ast = host
            .engine()
            .compile(
                r#"fn on_x(ctx) {
                    ctx.effects.complete_objective("now");
                    ctx.schedule.in_seconds(10).complete_objective("later");
                }"#,
            )
            .expect("compiles");
        let mut budget = TickBudget::new();
        let clk = clock();
        let effects = host.call(
            &mut budget,
            &clk,
            &ast,
            "t.rhai",
            "on_x",
            &FlagStore::new(),
            Map::new(),
        );
        // The immediate effect applies now; the delayed one is deferred.
        assert_eq!(
            effects.commands,
            vec![ActionCmd::CompleteObjective {
                id: "now".to_string()
            }]
        );
        assert_eq!(effects.delayed.len(), 1);
        let d = &effects.delayed[0];
        assert_eq!(
            d.action,
            TriggerAction::CompleteObjective {
                id: "later".to_string()
            }
        );
        assert_eq!(d.fire_at_elapsed, 15.0, "elapsed 5s + 10s delay");
        assert!(d.origin_layer.is_none());
        assert!(effects.callbacks.is_empty());
    }

    #[test]
    fn after_schedules_a_named_callback_from_an_anonymous_closure() {
        // `after(n, |ctx| …)` records the closure's generated `anon$…` name as a
        // serialisable `(fire_tick, script_path, fn_name)` callback, and that
        // name resolves back to a callable function on the same AST.
        let host = RuntimeHost::new();
        let ast = host
            .engine()
            .compile(
                r#"fn on_x(ctx) {
                    ctx.schedule.after(5, |ctx| { ctx.effects.complete_objective("deferred"); });
                }"#,
            )
            .expect("compiles");
        let mut budget = TickBudget::new();
        let clk = clock();
        let effects = host.call(
            &mut budget,
            &clk,
            &ast,
            "t.rhai",
            "on_x",
            &FlagStore::new(),
            Map::new(),
        );
        assert_eq!(effects.callbacks.len(), 1);
        let cb = &effects.callbacks[0];
        assert_eq!(cb.fire_tick, 300 + 5 * 60, "tick 300 + 5s at 60 Hz");
        assert_eq!(cb.script_path, "t.rhai");
        assert!(
            cb.fn_name.starts_with("anon$"),
            "an anonymous closure lifts to a generated name, got '{}'",
            cb.fn_name
        );

        // The lifted name is callable: invoking it produces the deferred effect.
        let resolved =
            host.call_immediate(&ast, "t.rhai", &cb.fn_name, &FlagStore::new(), Map::new());
        assert_eq!(
            resolved,
            vec![ActionCmd::CompleteObjective {
                id: "deferred".to_string()
            }]
        );
    }

    #[test]
    fn anonymous_closure_name_is_stable_across_hosts() {
        // The fixed hashing seed makes the generated name reproducible: two
        // independent hosts schedule the identical callback name (the basis for
        // serialising deferred work).
        let source = r#"fn on_x(ctx) {
            ctx.schedule.after(5, |ctx| { ctx.effects.complete_objective("deferred"); });
        }"#;
        let schedule_once = || {
            let host = RuntimeHost::new();
            let ast = host.engine().compile(source).expect("compiles");
            let mut budget = TickBudget::new();
            host.call(
                &mut budget,
                &clock(),
                &ast,
                "t.rhai",
                "on_x",
                &FlagStore::new(),
                Map::new(),
            )
            .callbacks
        };
        assert_eq!(
            schedule_once(),
            schedule_once(),
            "same seed + same script → identical schedule"
        );
    }

    #[test]
    fn schedule_is_deterministic_across_runs() {
        // Same seed + same script → identical immediate, delayed and callback
        // schedules on two independent runs.
        let source = r#"fn on_x(ctx) {
            ctx.effects.complete_objective("now");
            ctx.schedule.in_seconds(3).fail_objective("soon");
            ctx.schedule.after(7, |ctx| { ctx.effects.reset_trigger("t"); });
        }"#;
        let run = || {
            let host = RuntimeHost::new();
            let ast = host.engine().compile(source).expect("compiles");
            let mut budget = TickBudget::new();
            let e = host.call(
                &mut budget,
                &clock(),
                &ast,
                "t.rhai",
                "on_x",
                &FlagStore::new(),
                Map::new(),
            );
            // Reduce to comparable, `PartialEq` parts (DelayedAction is not Eq).
            let delayed: Vec<(TriggerAction, f32)> = e
                .delayed
                .iter()
                .map(|d| (d.action.clone(), d.fire_at_elapsed))
                .collect();
            (e.commands, delayed, e.callbacks)
        };
        assert_eq!(run(), run());
    }

    #[test]
    fn budget_drops_calls_once_the_call_cap_trips() {
        // A tripped budget drops a call whole: no effects, nothing scheduled.
        let host = RuntimeHost::new();
        let ast = host
            .engine()
            .compile(r#"fn on_x(ctx) { ctx.effects.complete_objective("x"); }"#)
            .expect("compiles");
        let mut budget = TickBudget::new();
        // Exhaust the call cap.
        for _ in 0..crate::world::script::MAX_CALLS_PER_TICK {
            budget.admit_call();
        }
        assert!(!budget.tripped());
        let effects = host.call(
            &mut budget,
            &SchedClock::ZERO,
            &ast,
            "t.rhai",
            "on_x",
            &FlagStore::new(),
            Map::new(),
        );
        assert!(
            effects.commands.is_empty()
                && effects.delayed.is_empty()
                && effects.callbacks.is_empty(),
            "a call over the cap is dropped whole"
        );
        assert!(budget.tripped());
    }

    #[test]
    fn budget_charges_operations_across_calls() {
        // Each real call charges a positive, deterministic op count to the tick
        // budget, so a busy tick converges toward the aggregate.
        let host = RuntimeHost::new();
        let ast = host
            .engine()
            .compile(r#"fn on_x(ctx) { ctx.effects.complete_objective("x"); }"#)
            .expect("compiles");
        let mut budget = TickBudget::new();
        host.call(
            &mut budget,
            &SchedClock::ZERO,
            &ast,
            "t.rhai",
            "on_x",
            &FlagStore::new(),
            Map::new(),
        );
        let after_one = budget.ops_used();
        assert!(after_one > 0, "a real call charges operations");
        host.call(
            &mut budget,
            &SchedClock::ZERO,
            &ast,
            "t.rhai",
            "on_x",
            &FlagStore::new(),
            Map::new(),
        );
        assert!(
            budget.ops_used() > after_one,
            "a second call adds to the tick aggregate"
        );
    }

    #[test]
    fn try_call_returns_err_on_a_runaway_script() {
        let host = RuntimeHost::new();
        // An infinite loop trips the per-call operation limit.
        let ast = host
            .engine()
            .compile("fn boom(ctx) { let i = 0; loop { i += 1; } }")
            .expect("compiles");
        let err = host
            .try_call(
                &SchedClock::ZERO,
                &ast,
                "scenario.rhai",
                "boom",
                &FlagStore::new(),
                Map::new(),
            )
            .expect_err("a runaway must be refused");
        // The failure names the file (vellum's `CallError::Runtime`).
        assert!(err.to_string().contains("scenario.rhai"), "{err}");
    }

    #[test]
    fn try_call_discards_effects_on_error() {
        let host = RuntimeHost::new();
        // Pushes one effect, then trips the op limit — the partial buffer must
        // not come back.
        let ast = host
            .engine()
            .compile(
                r#"fn boom(ctx) {
                    ctx.effects.complete_objective("obj1");
                    let i = 0; loop { i += 1; }
                }"#,
            )
            .expect("compiles");
        assert!(host
            .try_call(
                &SchedClock::ZERO,
                &ast,
                "t.rhai",
                "boom",
                &FlagStore::new(),
                Map::new()
            )
            .is_err());
    }

    #[test]
    #[should_panic(expected = "script error")]
    fn call_panics_in_dev_on_a_script_error() {
        // `cargo test` runs with `debug_assertions`, so `call` takes the panic
        // arm of settled decision 10.
        let host = RuntimeHost::new();
        let ast = host
            .engine()
            .compile("fn boom(ctx) { let i = 0; loop { i += 1; } }")
            .expect("compiles");
        let _ = host.call_immediate(&ast, "t.rhai", "boom", &FlagStore::new(), Map::new());
    }

    #[test]
    fn loading_engine_collects_registrations() {
        let state = Arc::new(Mutex::new(BuilderState {
            current_path: "world.toml#script.setup".to_string(),
            ..Default::default()
        }));
        let engine = loading_engine(state.clone());
        let ast = engine
            .compile(r#"on("flag_set:armed", "handle_armed");"#)
            .expect("compiles");
        engine.run_ast(&ast).expect("top level runs");
        drop(engine);
        let regs = Arc::try_unwrap(state)
            .map(|m| m.into_inner().expect("lock").registrations)
            .expect("engine dropped, sole owner");
        assert_eq!(
            regs,
            vec![Registration {
                event: "flag_set:armed".to_string(),
                handler: "handle_armed".to_string(),
                source_path: "world.toml#script.setup".to_string(),
            }]
        );
    }
}
