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

use std::sync::{Arc, Mutex};

use rhai::{Engine, Map, AST};

use crate::world::dispatch::ActionCmd;
use crate::world::flags::FlagStore;
use crate::world::script::effects::{register_effects, EffectSink};
use crate::world::script::flags::{register_flags, Flags};
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
    engine
}

/// The runtime script host: owns the runtime engine and runs retained functions.
pub struct RuntimeHost {
    engine: Engine,
}

impl Default for RuntimeHost {
    fn default() -> Self {
        Self::new()
    }
}

impl RuntimeHost {
    /// Build a host on a fresh runtime engine.
    pub fn new() -> Self {
        Self {
            engine: runtime_engine(),
        }
    }

    /// The underlying engine (compile ASTs against a matching engine; ASTs are
    /// engine-independent once compiled).
    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    /// Call a retained function, returning the `ActionCmd`s it produced —
    /// effect-buffer pushes followed by the flag overlay's drained writes.
    ///
    /// The context map is `extra` with the `flags` and `effects` handles
    /// inserted; a script reads it as its single parameter (`fn on_x(ctx)`).
    /// `base_flags` is the live store the flag overlay snapshots.
    ///
    /// This is the failure boundary (settled decision 10). On a script error it
    /// **panics in dev** (`debug_assertions`) and, in release, discards this
    /// call's effects whole, logs, and returns an empty vector so the game
    /// continues. Callers that need to inspect the error use [`try_call`].
    ///
    /// [`try_call`]: RuntimeHost::try_call
    pub fn call(
        &self,
        ast: &AST,
        path: &str,
        fn_name: &str,
        base_flags: &FlagStore,
        extra: Map,
    ) -> Vec<ActionCmd> {
        match self.try_call(ast, path, fn_name, base_flags, extra) {
            Ok(cmds) => cmds,
            Err(err) => {
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
                Vec::new()
            }
        }
    }

    /// Like [`call`](RuntimeHost::call) but returns the error instead of
    /// applying the panic/discard policy. On error, the call's effects are
    /// dropped (never drained), so this never returns a partial buffer.
    pub fn try_call(
        &self,
        ast: &AST,
        path: &str,
        fn_name: &str,
        base_flags: &FlagStore,
        extra: Map,
    ) -> Result<Vec<ActionCmd>, vellum_script::CallError> {
        let sink = EffectSink::new();
        let flags = Flags::new(base_flags);

        let mut ctx = extra;
        ctx.insert("effects".into(), rhai::Dynamic::from(sink.clone()));
        ctx.insert("flags".into(), rhai::Dynamic::from(flags.clone()));

        // On error we return before draining anything, so both the effect
        // buffer and the flag overlay are dropped whole. The returned `Dynamic`
        // is discarded: effects are collected imperatively via the buffer, not
        // from a return value (unlike last-aeon's effects-as-return model).
        let _ = vellum_script::call_fn(&self.engine, ast, path, fn_name, ctx)?;

        let mut cmds = sink.take();
        cmds.extend(flags.drain());
        Ok(cmds)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::dispatch::FlagMutation;

    #[test]
    fn runtime_engine_builds_and_runs_a_trivial_fn() {
        let host = RuntimeHost::new();
        let ast = host.engine().compile("fn noop(ctx) { }").expect("compiles");
        let cmds = host.call(&ast, "t.rhai", "noop", &FlagStore::new(), Map::new());
        assert!(cmds.is_empty());
    }

    #[test]
    fn call_drains_effects_then_flag_writes() {
        let host = RuntimeHost::new();
        let ast = host
            .engine()
            .compile(
                r#"fn on_x(ctx) {
                    ctx.effects.complete_objective("obj1");
                    ctx.flags.score += 50;
                }"#,
            )
            .expect("compiles");
        let cmds = host.call(&ast, "t.rhai", "on_x", &FlagStore::new(), Map::new());
        assert_eq!(
            cmds,
            vec![
                ActionCmd::CompleteObjective {
                    id: "obj1".to_string()
                },
                ActionCmd::MutateFlag {
                    target_layer: None,
                    name: "score".to_string(),
                    mutation: FlagMutation::SetValue(50),
                },
            ]
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
            .try_call(&ast, "scenario.rhai", "boom", &FlagStore::new(), Map::new())
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
            .try_call(&ast, "t.rhai", "boom", &FlagStore::new(), Map::new())
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
        let _ = host.call(&ast, "t.rhai", "boom", &FlagStore::new(), Map::new());
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
