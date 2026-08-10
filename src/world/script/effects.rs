//! The per-call effect buffer (issue #979, Rhai milestone M1).
//!
//! Registered runtime host functions push onto a call-scoped [`EffectSink`],
//! which drains into the **existing** [`ActionCmd`] boundary in
//! `world::dispatch`. Script gets no new effect vocabulary: every host function
//! here produces an `ActionCmd` the declarative trigger front-end already
//! produces, so the applier (`world::server`) is untouched.
//!
//! The sink is an `Arc<Mutex<Vec<ActionCmd>>>` so it can be registered on the
//! shared runtime engine once and still be a fresh, per-call buffer: the host
//! builds one sink per call, hands a clone into the context map, and — because
//! the handle is reference-counted with interior mutability — the retained
//! clone observes everything the script pushed. See [`engine::RuntimeHost`].
//!
//! The vocabulary is deliberately integer-and-string-only (the API is
//! integer-only, `no_float`). The M1 set is the subset of `ActionCmd`s that need
//! no entity name→UUID resolution (that plumbing arrives with a later
//! milestone); flag mutations live in [`flags`], not here.
//!
//! [`ActionCmd`]: crate::world::dispatch::ActionCmd
//! [`engine::RuntimeHost`]: crate::world::script::engine::RuntimeHost
//! [`flags`]: crate::world::script::flags

use std::sync::{Arc, Mutex};

use rhai::{Engine, ImmutableString};

use crate::world::dispatch::ActionCmd;

/// A call-scoped buffer of [`ActionCmd`]s a script produced.
///
/// Cloneable and interior-mutable: every clone shares one underlying `Vec`, so
/// the runtime host can register the effect host-fns on the engine once and
/// still collect exactly one call's effects.
#[derive(Clone, Default)]
pub struct EffectSink(Arc<Mutex<Vec<ActionCmd>>>);

impl EffectSink {
    /// A fresh, empty buffer for one call.
    pub fn new() -> Self {
        Self::default()
    }

    /// Push one command onto the buffer.
    ///
    /// `pub(crate)` because [`Flags`](super::flags::Flags) shares this one buffer
    /// so a flag mutation lands in the emitted sequence *at the point the script
    /// authored it*, interleaved with effects, rather than being appended after
    /// them (issue #981 flag-ordering hazard).
    pub(crate) fn push(&self, cmd: ActionCmd) {
        self.0.lock().expect("effect sink lock").push(cmd);
    }

    /// Drain the buffer, leaving it empty. Called by the host on the success
    /// path only — on the failure path the buffer is dropped whole, which is
    /// how "discard the call's effects" (settled decision 10) is enforced.
    pub fn take(&self) -> Vec<ActionCmd> {
        std::mem::take(&mut self.0.lock().expect("effect sink lock"))
    }

    /// Number of buffered commands (test/introspection helper).
    pub fn len(&self) -> usize {
        self.0.lock().expect("effect sink lock").len()
    }

    /// Whether the buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Register the effect vocabulary on a runtime engine.
///
/// Each function is a method on the `Effects` custom type, so a script calls
/// them as `ctx.effects.complete_objective("obj1")`. Every one pushes an
/// existing `ActionCmd`.
pub fn register_effects(engine: &mut Engine) {
    engine.register_type_with_name::<EffectSink>("Effects");

    engine.register_fn(
        "complete_objective",
        |sink: &mut EffectSink, id: ImmutableString| {
            sink.push(ActionCmd::CompleteObjective { id: id.to_string() });
        },
    );
    engine.register_fn(
        "fail_objective",
        |sink: &mut EffectSink, id: ImmutableString| {
            sink.push(ActionCmd::FailObjective { id: id.to_string() });
        },
    );
    engine.register_fn(
        "reset_trigger",
        |sink: &mut EffectSink, id: ImmutableString| {
            sink.push(ActionCmd::ResetTrigger { id: id.to_string() });
        },
    );
    engine.register_fn(
        "load_world",
        |sink: &mut EffectSink, path: ImmutableString| {
            // `loader_path` is `None` here: a script-issued load is authored at base
            // scope in M1 (no sub-world layer origin to thread yet). Mirrors
            // `dispatch_action`'s `LoadWorld` when `origin_layer` is `None`.
            sink.push(ActionCmd::LoadWorld {
                path: path.to_string(),
                loader_path: None,
            });
        },
    );
    engine.register_fn(
        "unload_world",
        |sink: &mut EffectSink, path: ImmutableString| {
            sink.push(ActionCmd::UnloadWorld {
                path: path.to_string(),
            });
        },
    );
    engine.register_fn(
        "game_over",
        |sink: &mut EffectSink, reason: ImmutableString| {
            // Reason first, then the transition — `OnEnter(GamePhase::GameOver)`
            // reads the reason, so the ordering is load-bearing. Mirrors
            // `dispatch_state_action`'s `GameOver` handling. `outcome` is `None`:
            // an undeclared scripted end (the headless classifier defaults it to
            // victory), matching `TriggerAction::GameOver { outcome: None }`.
            sink.push(ActionCmd::SetGameOverReason {
                reason: reason.to_string(),
                outcome: None,
            });
            sink.push(ActionCmd::SetNextState {
                phase: crate::messages::GamePhase::GameOver,
            });
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::script::engine::runtime_engine;
    use crate::world::script::flags::Flags;
    use rhai::{Dynamic, Map};

    /// Compile `source` on a runtime engine and call `fn_name`, returning the
    /// drained effect buffer. A local harness so this module's tests don't
    /// depend on `RuntimeHost`'s failure-mode wrapper.
    fn run(source: &str, fn_name: &str) -> Vec<ActionCmd> {
        let engine = runtime_engine();
        let ast = engine.compile(source).expect("compiles");
        let sink = EffectSink::new();
        let mut ctx = Map::new();
        ctx.insert("effects".into(), Dynamic::from(sink.clone()));
        // Flags share the one ordered buffer (issue #981), so a flag write lands
        // in `sink` alongside effects; these tests write no flags.
        ctx.insert(
            "flags".into(),
            Dynamic::from(Flags::new(
                &crate::world::flags::FlagStore::new(),
                sink.clone(),
            )),
        );
        let _ = vellum_script::call_fn(&engine, &ast, "t.rhai", fn_name, ctx).expect("calls");
        sink.take()
    }

    #[test]
    fn complete_objective_drains_to_action_cmd() {
        let cmds = run(
            r#"fn on_x(ctx) { ctx.effects.complete_objective("obj1"); }"#,
            "on_x",
        );
        assert_eq!(
            cmds,
            vec![ActionCmd::CompleteObjective {
                id: "obj1".to_string()
            }]
        );
    }

    #[test]
    fn game_over_emits_reason_then_transition_in_order() {
        let cmds = run(
            r#"fn end(ctx) { ctx.effects.game_over("hull breach"); }"#,
            "end",
        );
        assert_eq!(
            cmds,
            vec![
                ActionCmd::SetGameOverReason {
                    reason: "hull breach".to_string(),
                    outcome: None,
                },
                ActionCmd::SetNextState {
                    phase: crate::messages::GamePhase::GameOver,
                },
            ]
        );
    }

    #[test]
    fn multiple_effects_buffer_in_call_order() {
        let cmds = run(
            r#"fn on_x(ctx) {
                ctx.effects.fail_objective("a");
                ctx.effects.reset_trigger("b");
                ctx.effects.unload_world("w.toml");
            }"#,
            "on_x",
        );
        assert_eq!(
            cmds,
            vec![
                ActionCmd::FailObjective {
                    id: "a".to_string()
                },
                ActionCmd::ResetTrigger {
                    id: "b".to_string()
                },
                ActionCmd::UnloadWorld {
                    path: "w.toml".to_string()
                },
            ]
        );
    }

    #[test]
    fn take_empties_the_buffer() {
        let sink = EffectSink::new();
        sink.push(ActionCmd::CompleteObjective {
            id: "x".to_string(),
        });
        assert_eq!(sink.len(), 1);
        let _ = sink.take();
        assert!(sink.is_empty());
    }
}
