//! Editor authoring support: the host-fn signature registry and a script
//! diagnostics pass (issue #983, Rhai milestone M5).
//!
//! The scenario editor becomes a `.rhai` script editor, and it needs two things
//! the running host already knows and JS would only duplicate (and drift from):
//!
//! * **A host-fn signature registry** — the vocabulary a scenario author can
//!   call. It is the *same* vocabulary [`engine::loading_engine`] and
//!   [`engine::runtime_engine`] register (the trigger builders, `on(..)`, the
//!   `ctx.effects` / `ctx.flags` / `ctx.schedule` methods, and the delay-builder
//!   verbs), enumerated once here so the editor's autocomplete stays in step with
//!   what actually resolves at load and runtime. Rhai's own `gen_fn_signatures`
//!   needs the `metadata` feature, which phoenix's `rhai` build deliberately does
//!   not enable (it would bloat the wasm and break vellum's single-`rhai`
//!   unification), so this is a hand-maintained mirror with a test that pins it
//!   against the registration sites.
//!
//! * **A diagnostics pass** — compile a `.rhai` source (a sibling file's whole
//!   text, or a lifted inline `[script.*]` block) on the *loading* engine (so the
//!   trigger builders resolve as known functions) and run its top level once,
//!   exactly as [`load::compile_scripts`] does, surfacing the first parse or
//!   top-level runtime error with its line and column. A `line_offset` is added
//!   to every reported line so an inline block edited inside its host TOML lands
//!   on the correct *document* line — the editor passes the block's start line;
//!   a standalone `.rhai` file passes `0`. This is the span mapping the M1 loader
//!   set up by lifting inline blocks to virtual paths (see [`load`]).
//!
//! Both are pure and native-testable; the wasm bridge (`server::bridge`) is a
//! thin marshalling layer over them.
//!
//! [`engine::loading_engine`]: super::engine::loading_engine
//! [`engine::runtime_engine`]: super::engine::runtime_engine
//! [`load`]: super::load
//! [`load::compile_scripts`]: super::load::compile_scripts

use std::sync::{Arc, Mutex};

use crate::world::script::engine::{loading_engine, BuilderState};

/// One entry in the host-fn signature registry.
///
/// `receiver` is the `ctx` sub-object a method hangs off (`"effects"`,
/// `"flags"`, `"schedule"`), `"delay"` for the delay-builder verbs reached via
/// `ctx.schedule.in_seconds(n).<verb>(…)`, or `""` for a top-level call
/// (`on_destroyed(…)`, `on(…)`). Everything is `&'static` — the registry is a
/// compile-time constant list.
#[derive(Clone, Copy, Debug)]
pub struct HostFn {
    /// The callable name, exactly as it is registered on the engine.
    pub name: &'static str,
    /// The `ctx` sub-object the method hangs off, or `""` for a top-level call.
    pub receiver: &'static str,
    /// Parameter names, for the autocomplete signature. Overloads collapse to
    /// the fullest form; optional trailing params are noted in `summary`.
    pub params: &'static [&'static str],
    /// One of `register` / `trigger` / `effect` / `flag` / `schedule` / `delay`.
    pub category: &'static str,
    /// A one-line human description for the completion popup.
    pub summary: &'static str,
}

impl HostFn {
    /// The display signature, e.g. `on_destroyed(entity, handler)` or, for a
    /// receiver method, `effects.complete_objective(id)`.
    pub fn signature(&self) -> String {
        let call = format!("{}({})", self.name, self.params.join(", "));
        if self.receiver.is_empty() {
            call
        } else {
            format!("{}.{}", self.receiver, call)
        }
    }
}

/// The host-fn vocabulary the editor offers for autocomplete.
///
/// Kept in step with the registration sites by the
/// `registry_covers_every_registration_site` test below: the `""`-receiver
/// entries come from [`super::engine::loading_engine`] and [`super::triggers`];
/// the receiver entries come from [`super::effects`], [`super::flags`], and
/// [`super::schedule`].
pub const HOST_FNS: &[HostFn] = &[
    // ── top-level registration + trigger builders (loading engine) ───────────
    HostFn {
        name: "on",
        receiver: "",
        params: &["event", "handler"],
        category: "register",
        summary: "Register a named handler fn for a generic event string.",
    },
    HostFn {
        name: "on_destroyed",
        receiver: "",
        params: &["entity", "handler"],
        category: "trigger",
        summary: "Fire when the named entity is destroyed.",
    },
    HostFn {
        name: "on_all_destroyed",
        receiver: "",
        params: &["group", "handler"],
        category: "trigger",
        summary: "Fire when every entity in a group is destroyed. Optional \
                  middle arg `after_secs` gates the fire.",
    },
    HostFn {
        name: "on_attacked",
        receiver: "",
        params: &["entity", "handler"],
        category: "trigger",
        summary: "Fire when the named entity is attacked.",
    },
    HostFn {
        name: "on_timer",
        receiver: "",
        params: &["after_secs", "handler"],
        category: "trigger",
        summary: "Fire once, `after_secs` seconds after the world loads.",
    },
    HostFn {
        name: "on_hailed",
        receiver: "",
        params: &["entity", "handler"],
        category: "trigger",
        summary: "Fire when the named entity is hailed over comms.",
    },
    HostFn {
        name: "on_flag_set",
        receiver: "",
        params: &["name", "handler"],
        category: "trigger",
        summary: "Fire when the named flag transitions to set.",
    },
    HostFn {
        name: "on_flag_cleared",
        receiver: "",
        params: &["name", "handler"],
        category: "trigger",
        summary: "Fire when the named flag transitions to cleared.",
    },
    HostFn {
        name: "on_world_loaded",
        receiver: "",
        params: &["handler"],
        category: "trigger",
        summary: "Fire once when this world finishes loading.",
    },
    HostFn {
        name: "on_entered_region",
        receiver: "",
        params: &["entity", "handler"],
        category: "trigger",
        summary: "Fire when the named entity enters a region.",
    },
    HostFn {
        name: "on_exited_region",
        receiver: "",
        params: &["entity", "handler"],
        category: "trigger",
        summary: "Fire when the named entity exits a region.",
    },
    HostFn {
        name: "on_waypoint_reached",
        receiver: "",
        params: &["entity", "handler"],
        category: "trigger",
        summary: "Fire when the named entity reaches a waypoint. Optional middle \
                  arg `waypoint` pins a specific anchor.",
    },
    // ── ctx.effects.* (runtime engine) ───────────────────────────────────────
    HostFn {
        name: "complete_objective",
        receiver: "effects",
        params: &["id"],
        category: "effect",
        summary: "Mark the objective complete.",
    },
    HostFn {
        name: "fail_objective",
        receiver: "effects",
        params: &["id"],
        category: "effect",
        summary: "Mark the objective failed.",
    },
    HostFn {
        name: "reset_trigger",
        receiver: "effects",
        params: &["id"],
        category: "effect",
        summary: "Re-arm a fired trigger by id.",
    },
    HostFn {
        name: "load_world",
        receiver: "effects",
        params: &["path"],
        category: "effect",
        summary: "Load the world layer at `path`.",
    },
    HostFn {
        name: "unload_world",
        receiver: "effects",
        params: &["path"],
        category: "effect",
        summary: "Unload the world layer at `path`.",
    },
    HostFn {
        name: "game_over",
        receiver: "effects",
        params: &["reason"],
        category: "effect",
        summary: "End the game with a reason string.",
    },
    // ── ctx.flags.* (runtime engine) ─────────────────────────────────────────
    HostFn {
        name: "increment",
        receiver: "flags",
        params: &["name", "by"],
        category: "flag",
        summary: "Composably add `by` to a counter flag. Use over `flags.x += n`.",
    },
    // ── ctx.schedule.* (runtime engine) ──────────────────────────────────────
    HostFn {
        name: "in_seconds",
        receiver: "schedule",
        params: &["secs"],
        category: "schedule",
        summary: "Start a delayed effect: `in_seconds(n).<verb>(…)`.",
    },
    HostFn {
        name: "after",
        receiver: "schedule",
        params: &["secs", "callback"],
        category: "schedule",
        summary: "Defer a `|ctx| { … }` callback by `secs` seconds.",
    },
    // ── ctx.schedule.in_seconds(n).* delay-builder verbs (runtime engine) ────
    HostFn {
        name: "complete_objective",
        receiver: "delay",
        params: &["id"],
        category: "delay",
        summary: "Delayed: mark the objective complete.",
    },
    HostFn {
        name: "fail_objective",
        receiver: "delay",
        params: &["id"],
        category: "delay",
        summary: "Delayed: mark the objective failed.",
    },
    HostFn {
        name: "reset_trigger",
        receiver: "delay",
        params: &["id"],
        category: "delay",
        summary: "Delayed: re-arm a fired trigger by id.",
    },
    HostFn {
        name: "load_world",
        receiver: "delay",
        params: &["path"],
        category: "delay",
        summary: "Delayed: load the world layer at `path`.",
    },
    HostFn {
        name: "unload_world",
        receiver: "delay",
        params: &["path"],
        category: "delay",
        summary: "Delayed: unload the world layer at `path`.",
    },
    HostFn {
        name: "game_over",
        receiver: "delay",
        params: &["reason"],
        category: "delay",
        summary: "Delayed: end the game with a reason string.",
    },
];

/// The host-fn registry (a `&'static` slice — no allocation).
pub fn host_fns() -> &'static [HostFn] {
    HOST_FNS
}

/// One editor diagnostic: a message pinned to a 1-based line and column.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScriptDiagnostic {
    /// Human-readable message (no position suffix).
    pub message: String,
    /// 1-based line in the edited buffer, already shifted by `line_offset`.
    pub line: usize,
    /// 1-based column, or `0` when Rhai reports no column.
    pub column: usize,
    /// Always `"error"` in M5 (Rhai stops at the first parse/runtime error).
    pub severity: &'static str,
}

/// Compile `source` on the loading engine and run its top level once, returning
/// the first parse or top-level runtime error (if any) as a diagnostic.
///
/// `line_offset` is added to the reported line so an inline `[script.*]` block
/// edited inside its host TOML lands on the correct document line; pass `0` for
/// a standalone `.rhai` file. Mirrors [`super::load::compile_scripts`]'s per-unit
/// `compile` + `run_ast`.
///
/// # Scope: the compile subset of load-time findings
///
/// This pass is the *compile* half of activation only. It runs the same parse +
/// top-level `run_ast` [`super::load::compile_scripts`] runs, but **not** the
/// cross-reference `validate` stage [`super::load::load_world_scripts`] runs
/// afterwards (`validate_registrations` / `validate_script_triggers` and the TOML
/// front-end validators). A source is therefore "editor-clean" here iff it
/// *compiles and its top level runs* — which is a strict subset of "activates".
/// The gap is unresolved-handler / cross-reference findings: e.g.
/// `on_destroyed("x", "missing_fn")` compiles and its top level runs fine (the
/// trigger builder only records the handler name; proving it resolves is
/// `validate`'s job), so it shows **no** editor diagnostic yet still blocks the
/// world from activating. Editor diagnostics catch syntax and top-level runtime
/// errors; a handler naming a function that is never defined surfaces only at
/// activation, not here.
///
/// The vector holds at most one diagnostic (Rhai halts at the first error); it
/// is a `Vec` so later milestones can surface warnings without a signature
/// change.
pub fn script_diagnostics(source: &str, line_offset: usize) -> Vec<ScriptDiagnostic> {
    // A throwaway builder state: `run_ast` executes top-level `on_*(..)` calls,
    // which push into it. We only care whether they *resolve and run*, not what
    // they built, so the state is discarded.
    let state = Arc::new(Mutex::new(BuilderState::default()));
    let engine = loading_engine(state);

    let ast = match engine.compile(source) {
        Ok(ast) => ast,
        Err(err) => {
            let (line, column) = position_of(&err.1, line_offset);
            return vec![ScriptDiagnostic {
                // `ParseErrorType`'s Display carries no position suffix.
                message: err.0.to_string(),
                line,
                column,
                severity: "error",
            }];
        }
    };

    if let Err(err) = engine.run_ast(&ast) {
        let pos = err.position();
        let (line, column) = position_of(&pos, line_offset);
        return vec![ScriptDiagnostic {
            message: strip_position_suffix(&err.to_string()),
            line,
            column,
            severity: "error",
        }];
    }

    Vec::new()
}

/// Map a Rhai [`Position`](rhai::Position) to a 1-based `(line, column)`,
/// shifting the line by `line_offset`. A position with no line (rare — most
/// errors carry one) defaults to line `1 + line_offset`, column `0`.
fn position_of(pos: &rhai::Position, line_offset: usize) -> (usize, usize) {
    let line = pos.line().unwrap_or(1) + line_offset;
    let column = pos.position().unwrap_or(0);
    (line, column)
}

/// Strip a trailing ` (line N, position M)` suffix that `EvalAltResult`'s
/// Display appends — the structured `line`/`column` fields carry it instead.
fn strip_position_suffix(msg: &str) -> String {
    match msg.rfind(" (line ") {
        Some(i) if msg.ends_with(')') => msg[..i].to_string(),
        _ => msg.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::flags::FlagStore;
    use crate::world::script::engine::RuntimeHost;
    use crate::world::script::schedule::SchedClock;
    use rhai::{Engine, Map};
    use std::collections::BTreeSet;

    /// A syntactically valid, correctly-*typed* argument list for a top-level
    /// builder, so a call that resolves never trips on arity/type — only a
    /// genuinely missing (phantom) fn yields "Function not found".
    fn top_level_args(name: &str) -> &'static str {
        match name {
            "on" => "\"evt\", \"h\"",     // on(event, handler)
            "on_timer" => "0, \"h\"",     // on_timer(after_secs: INT, handler)
            "on_world_loaded" => "\"h\"", // on_world_loaded(handler)
            // Every other trigger builder is (name/entity/group, handler).
            _ => "\"e\", \"h\"",
        }
    }

    /// A `ctx.<receiver>.<name>(…)` expression that exercises `hf` correctly, so
    /// a resolving method runs clean and only a phantom yields "Function not
    /// found". Panics on an unrecognised receiver/verb so a NEW registration with
    /// a different shape must extend this probe rather than silently mis-call.
    fn receiver_expr(hf: &HostFn) -> String {
        match hf.receiver {
            // Every effect / delay verb takes a single string (id / path / reason).
            "effects" => format!("ctx.effects.{}(\"x\")", hf.name),
            "delay" => format!("ctx.schedule.in_seconds(0).{}(\"x\")", hf.name),
            "flags" => match hf.name {
                "increment" => "ctx.flags.increment(\"n\", 0)".to_string(),
                other => panic!("add a probe for the new flags.{other}"),
            },
            "schedule" => match hf.name {
                "in_seconds" => "ctx.schedule.in_seconds(0)".to_string(),
                "after" => "ctx.schedule.after(0, |ctx| { })".to_string(),
                other => panic!("add a probe for the new schedule.{other}"),
            },
            other => panic!("unhandled receiver {other:?} for {}", hf.name),
        }
    }

    /// Probe that `hf` actually resolves on its engine. A resolving fn runs clean
    /// (or fails for some *other* reason); only a phantom — a HOST_FNS entry the
    /// engine never registered — answers "Function not found".
    fn assert_hostfn_resolves(hf: &HostFn, loading: &Engine, host: &RuntimeHost) {
        let is_not_found = |msg: &str| msg.contains("Function not found");
        if hf.receiver.is_empty() {
            // Top-level builder / `on`: run it on the loading engine's top level,
            // exactly as the loader's `run_ast` does.
            let src = format!("{}({});", hf.name, top_level_args(hf.name));
            let ast = loading.compile(&src).expect("probe compiles");
            if let Err(err) = loading.run_ast(&ast) {
                assert!(
                    !is_not_found(&err.to_string()),
                    "HOST_FNS lists top-level `{}` but the loading engine never \
                     registers it: {err}",
                    hf.name
                );
            }
        } else {
            // Receiver method: run it through the runtime host, which injects
            // ctx.effects / ctx.flags / ctx.schedule exactly as a real call does.
            let src = format!("fn probe(ctx) {{ {}; }}", receiver_expr(hf));
            let ast = host.engine().compile(&src).expect("probe compiles");
            let res = host.try_call(
                &SchedClock::ZERO,
                &ast,
                "probe.rhai",
                "probe",
                &FlagStore::new(),
                Map::new(),
            );
            if let Err(err) = res {
                assert!(
                    !is_not_found(&err.to_string()),
                    "HOST_FNS lists `{}.{}` but the runtime engine never registers \
                     it: {err}",
                    hf.receiver,
                    hf.name
                );
            }
        }
    }

    #[test]
    fn signature_formats_top_level_and_receiver_calls() {
        let on_destroyed = HOST_FNS.iter().find(|h| h.name == "on_destroyed").unwrap();
        assert_eq!(on_destroyed.signature(), "on_destroyed(entity, handler)");
        let complete = HOST_FNS
            .iter()
            .find(|h| h.name == "complete_objective" && h.receiver == "effects")
            .unwrap();
        assert_eq!(complete.signature(), "effects.complete_objective(id)");
    }

    #[test]
    fn registry_covers_every_registration_site() {
        // Pins the hand-maintained registry against the actual registration
        // sites, so adding a builder or effect without listing it here fails.
        let names: BTreeSet<(&str, &str)> = HOST_FNS.iter().map(|h| (h.receiver, h.name)).collect();

        // Top-level: `on` + one per TriggerCondition variant (triggers.rs).
        for expected in [
            "on",
            "on_destroyed",
            "on_all_destroyed",
            "on_attacked",
            "on_timer",
            "on_hailed",
            "on_flag_set",
            "on_flag_cleared",
            "on_world_loaded",
            "on_entered_region",
            "on_exited_region",
            "on_waypoint_reached",
        ] {
            assert!(
                names.contains(&("", expected)),
                "missing top-level {expected}"
            );
        }
        // ctx.effects.* (effects.rs) and the same verbs as delay-builder methods
        // (schedule.rs).
        for verb in [
            "complete_objective",
            "fail_objective",
            "reset_trigger",
            "load_world",
            "unload_world",
            "game_over",
        ] {
            assert!(names.contains(&("effects", verb)), "missing effects.{verb}");
            assert!(names.contains(&("delay", verb)), "missing delay.{verb}");
        }
        // ctx.flags.* (flags.rs) and ctx.schedule.* (schedule.rs).
        assert!(names.contains(&("flags", "increment")));
        assert!(names.contains(&("schedule", "in_seconds")));
        assert!(names.contains(&("schedule", "after")));

        // ── Phantom guard: every HOST_FN must actually resolve on its engine ──
        // The membership check above pins the registry against a curated list,
        // but a HOST_FNS entry naming a fn the engine never registers (a
        // *phantom*) would still slip through — the list is duplicated, not
        // derived. Rhai's `metadata` feature is off (see the module doc), so the
        // engines are not enumerable and `gen_fn_signatures` is unavailable;
        // instead we compile+call each entry correctly and assert the engine does
        // NOT answer "Function not found". That catches every phantom.
        //
        // The *other* direction — an engine fn HOST_FNS omits — is still caught
        // only by the curated membership check above: without metadata the engine
        // cannot be walked, so a newly registered builder/effect that nobody adds
        // to HOST_FNS is detected by its missing entry in the lists above, not by
        // enumeration here.
        let state = Arc::new(Mutex::new(BuilderState::default()));
        let loading = loading_engine(state);
        let host = RuntimeHost::new();
        for hf in HOST_FNS {
            assert_hostfn_resolves(hf, &loading, &host);
        }
    }

    #[test]
    fn clean_script_has_no_diagnostics() {
        let src = r#"
            on_destroyed("raider", "on_raider_dead");
            fn on_raider_dead(ctx) {
                ctx.effects.complete_objective("obj-x");
            }
        "#;
        assert!(script_diagnostics(src, 0).is_empty());
    }

    #[test]
    fn a_syntax_error_lands_on_its_line() {
        // The `let x = ;` is on line 3 (1-based) of the buffer.
        let src = "fn a(ctx) {\n    let ok = 1;\n    let x = ;\n}\n";
        let diags = script_diagnostics(src, 0);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].line, 3, "diagnostic must land on the error line");
        assert_eq!(diags[0].severity, "error");
        assert!(!diags[0].message.is_empty());
    }

    #[test]
    fn line_offset_shifts_an_inline_block_to_the_document_line() {
        // The identical error, but the block starts at document line 12 (11
        // lines precede its content), so the diagnostic must read line 14.
        let src = "fn a(ctx) {\n    let ok = 1;\n    let x = ;\n}\n";
        let diags = script_diagnostics(src, 11);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].line, 3 + 11);
    }

    #[test]
    fn a_top_level_call_to_an_undefined_fn_is_a_runtime_diagnostic() {
        // `run_ast` executes the top level; calling a function that neither the
        // engine nor the unit defines is a runtime error the editor should show.
        let src = "no_such_builder(\"x\", \"h\");\n";
        let diags = script_diagnostics(src, 0);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].line, 1);
        // No redundant position suffix left in the message.
        assert!(
            !diags[0].message.ends_with(')') || !diags[0].message.contains("(line "),
            "position suffix should be stripped: {:?}",
            diags[0].message
        );
    }

    #[test]
    fn a_registered_trigger_builder_call_resolves_clean() {
        // `on_timer` is a registered loading-engine host fn, so a top-level call
        // to it (with a defined handler) runs without a diagnostic — proving the
        // diagnostics pass uses the same vocabulary the loader does.
        let src = "on_timer(30, \"tick\");\nfn tick(ctx) { }\n";
        assert!(script_diagnostics(src, 0).is_empty());
    }
}
