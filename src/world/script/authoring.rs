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
//!   verbs), so the editor's autocomplete stays in step with what actually
//!   resolves at load and runtime. Rhai's own `gen_fn_signatures` needs the
//!   `metadata` feature, which phoenix's `rhai` build deliberately does not
//!   enable (it would bloat the wasm and break vellum's single-`rhai`
//!   unification). Rather than a hand-maintained mirror, [`host_fns`] is now
//!   DERIVED: each editor-exposed verb is declared with the
//!   [`host_fn!`](super::registry::host_fn) macro, which emits both the engine
//!   registration and its [`HostFn`] descriptor from one site, and
//!   [`host_fns`] runs the same registration the engines run and returns the
//!   descriptors it collected (issue #1238). A descriptor therefore cannot name
//!   a verb the engine never registered, and an exposed verb cannot be
//!   registered without being described.
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

use std::sync::{Arc, Mutex, OnceLock};

use crate::world::script::engine::{collect_host_fn_descriptors, loading_engine, BuilderState};

/// One entry in the host-fn signature registry.
///
/// `receiver` is the `ctx` sub-object a method hangs off (`"effects"`,
/// `"flags"`, `"schedule"`), `"delay"` for the delay-builder verbs reached via
/// `ctx.schedule.in_seconds(n).<verb>(…)`, `"trigger"` for a modifier chained
/// onto a registration's own handle (`on_timer(…).when(…)`), or `""` for a
/// top-level call (`on_destroyed(…)`, `on(…)`). Everything is `&'static` — a
/// descriptor is emitted by the [`host_fn!`](super::registry::host_fn)
/// registration that defines the verb, so the registry is derived, not
/// hand-mirrored (issue #1238).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize)]
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
    /// receiver method, `effects.complete_objective(id, instance_id)`.
    pub fn signature(&self) -> String {
        let call = format!("{}({})", self.name, self.params.join(", "));
        if self.receiver.is_empty() {
            call
        } else {
            format!("{}.{}", self.receiver, call)
        }
    }
}

/// The host-fn vocabulary the editor offers for autocomplete, DERIVED from the
/// registration sites rather than hand-mirrored (issue #1238).
///
/// Every entry is produced by the [`host_fn!`](super::registry::host_fn) call
/// that registers its verb on one of the two engines, harvested by
/// [`collect_host_fn_descriptors`]: the loading-engine builders (`on`, the
/// trigger builders, and the `when` modifier) come first, then the runtime verbs
/// (`ctx.flags` / `ctx.effects` / `ctx.schedule` and the `in_seconds(n).<verb>`
/// delay builder, plus `ctx.dossier.append`), grouped as the engines register
/// them. Memoised: the descriptors are collected once and kept for the process
/// lifetime, so the returned slice is `&'static` and callers keep the borrow the
/// old `HOST_FNS` const gave them.
pub fn host_fns() -> &'static [HostFn] {
    static REGISTRY: OnceLock<Vec<HostFn>> = OnceLock::new();
    REGISTRY.get_or_init(collect_host_fn_descriptors).as_slice()
}

/// One editor diagnostic: a message pinned to a 1-based line and column.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
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
#[path = "authoring_tests.rs"]
mod tests;
