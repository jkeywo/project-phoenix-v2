//! The one script loader: source lifting, compilation, and the content hash
//! (issue #979, milestone M1).
//!
//! Scripts reach a world two ways, and this module funnels both into a single
//! sorted `BTreeMap<String, AST>` so there is one loader, one AST map, and one
//! span-offset mapping:
//!
//! * **Sibling files** — a top-level `script = "combat.rhai"` in the world TOML
//!   resolves to a path beside the world file and is read through a
//!   [`ScriptResolver`] (injected, like `world::dispatch`'s `TemplateLoader`, so
//!   this module touches neither the filesystem nor the WASM config cache).
//! * **Inline blocks** — a `[script]` table whose entries are strings lifts each
//!   entry to a virtual path `world.toml#script.<key>`, with the string as the
//!   source. The virtual path is a real key in the AST map and content hash, so
//!   inline and sibling scripts are indistinguishable downstream.
//!
//! Sources are sorted by path before hashing, so `vellum_script::content_hash`
//! is a property of the content set and not of table or directory iteration
//! order. Parse and top-level runtime errors surface as
//! [`WorldFinding`](crate::world::validate::WorldFinding)s, joining the
//! cross-reference findings from [`validate`](super::validate) so the atomic
//! activation gate (`world::validate::has_error`) sees them all.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use rhai::AST;
use vellum_script::ScriptSource;

use crate::world::script::engine::{loading_engine, BuilderState, Registration, ScriptTrigger};
use crate::world::script::validate::{
    validate_deadline_handlers, validate_flag_opassign, validate_on_pick_fns,
    validate_registrations, validate_script_triggers,
};
use crate::world::validate::{Severity, SourceLocation, WorldFinding};

/// Reads the source of a sibling script path (already resolved relative to the
/// world file). Native uses the filesystem, WASM the config cache; tests use a
/// fake. `None` means missing or unreadable.
pub trait ScriptResolver {
    /// Read the script at `path`, or `None` if it cannot be read.
    fn read(&self, path: &str) -> Option<String>;
}

/// A [`ScriptResolver`] that resolves no sibling files.
///
/// Inline `[script]` blocks lift straight from a world's TOML and never consult a
/// resolver, so this is the resolver for every load that has no sibling-`.rhai`
/// source to reach for: the pure additive-merge decisions (`world::layers`'
/// `evaluate_layer_load` and the scenario applier, which route through
/// `world::load` under `Merge`) and the test fixtures (`world::script::fixture`).
/// A top-level `script = "file.rhai"` resolved through it is reported as missing —
/// resolving a *merged* world's sibling scripts is a ledger-freeze policy concern
/// for issue #1045, not something these pure/decision paths do.
pub struct NoSiblingScripts;

impl ScriptResolver for NoSiblingScripts {
    fn read(&self, _path: &str) -> Option<String> {
        None
    }
}

/// A compiled, validated script content set for one world.
pub struct CompiledScripts {
    /// Retained ASTs keyed by content-relative (or virtual) path, sorted.
    pub asts: BTreeMap<String, AST>,
    /// Every named function defined across all units — the resolution set the
    /// cross-reference pass checks handler names against.
    pub defined_fns: BTreeSet<String>,
    /// Registrations collected while running each unit's top level.
    pub registrations: Vec<Registration>,
    /// Triggers built by the Rhai front-end (`on_destroyed`, …) while running
    /// each unit's top level (issue #980, M2). Feed the existing pipeline exactly
    /// as TOML-authored triggers do.
    pub script_triggers: Vec<ScriptTrigger>,
    /// `on_deadline("id", "handler")` declarations collected while running each
    /// unit's top level (issue #1024). Paired with the world's `[[deadline]]`
    /// blocks by `arm_mission_deadlines`.
    pub deadline_handlers: Vec<crate::world::deadlines::DeadlineHandler>,
    /// Content hash binding a save to the exact scripts it was recorded against.
    pub content_hash: u64,
    /// The content-ledger write this set implies, returned as DATA for the
    /// caller to apply (issue #1241) — `None` for a world that lifted no source
    /// at all, which is what "the whole shipped set pays nothing" means here.
    ///
    /// [`load_world_scripts`] used to write it itself. That made the ledger a
    /// side effect of compiling, and it was the one thing
    /// [`crate::world::load::load`] could not hand back as data even though its
    /// whole contract says it does — see [`crate::world::load::LedgerPlan`].
    pub ledger_digest: Option<crate::content_ledger::LedgerDigest>,
    /// All findings: source lifting, compilation, and cross-reference. Any
    /// error blocks activation.
    pub findings: Vec<WorldFinding>,
}

fn finding(
    severity: Severity,
    category: &'static str,
    file: &str,
    reference: &str,
    message: String,
) -> WorldFinding {
    WorldFinding {
        severity,
        category,
        message,
        source: SourceLocation {
            file: file.to_string(),
            // Best-effort: the loader does not carry the raw world TOML text, so
            // no line is derived (the existing `WorldFinding` allows `None`).
            line: None,
            reference: reference.to_string(),
        },
    }
}

/// Resolve a sibling script path relative to the world file's directory,
/// normalising to forward slashes. No `..` normalisation in M1.
fn sibling_path(world_path: &str, rel: &str) -> String {
    let rel = rel.replace('\\', "/");
    match world_path.rfind(['/', '\\']) {
        Some(i) => format!("{}/{}", world_path[..i].replace('\\', "/"), rel),
        None => rel,
    }
}

/// Return the resolved sibling script path declared by a raw world document.
/// Inline `[script]` tables and script-free worlds return `None`.
///
/// The browser layer loader uses this small preflight before compilation so it
/// can retain the already-fetched TOML while the sibling request is pending,
/// without treating "not delivered yet" as a missing-file authoring error.
#[cfg(any(target_arch = "wasm32", test))]
pub(crate) fn declared_sibling_script_path(world_path: &str, world_toml: &str) -> Option<String> {
    let value: toml::Value = toml::from_str(world_toml).ok()?;
    value
        .get("script")
        .and_then(toml::Value::as_str)
        .map(|rel| sibling_path(world_path, rel))
}

/// Lift a world's `script` declaration into sorted [`ScriptSource`]s.
///
/// Handles a top-level `script = "file.rhai"` (sibling, read via `resolver`) or
/// a `[script]` table of inline string blocks. A world with no `script` key
/// yields nothing. Returns the sources (sorted by path) and any lifting
/// findings.
pub fn lift_world_scripts(
    world_path: &str,
    world_toml: &toml::Value,
    resolver: &dyn ScriptResolver,
) -> (Vec<ScriptSource>, Vec<WorldFinding>) {
    let mut sources = Vec::new();
    let mut findings = Vec::new();

    let Some(script_val) = world_toml.get("script") else {
        return (sources, findings);
    };

    match script_val {
        toml::Value::String(rel) => {
            let path = sibling_path(world_path, rel);
            match resolver.read(&path) {
                Some(source) => sources.push(ScriptSource { path, source }),
                None => findings.push(finding(
                    Severity::Error,
                    "script-file-missing",
                    world_path,
                    rel,
                    format!("script file '{path}' could not be read"),
                )),
            }
        }
        toml::Value::Table(table) => {
            for (key, value) in table {
                match value {
                    toml::Value::String(source) => sources.push(ScriptSource {
                        path: format!("{world_path}#script.{key}"),
                        source: source.clone(),
                    }),
                    _ => findings.push(finding(
                        Severity::Error,
                        "script-inline-invalid",
                        world_path,
                        key,
                        format!("inline [script.{key}] must be a string of Rhai source"),
                    )),
                }
            }
        }
        _ => findings.push(finding(
            Severity::Error,
            "script-source-invalid",
            world_path,
            "script",
            "`script` must be a path string or a [script] table of inline blocks".to_string(),
        )),
    }

    sources.sort_by(|a, b| a.path.cmp(&b.path));
    (sources, findings)
}

/// Compile a set of sources: run each unit's top level once on the loading
/// engine, collect ASTs, defined function names, and registrations, and hash
/// the (re-sorted) content. Parse/runtime errors become findings; the offending
/// unit is skipped.
pub fn compile_scripts(sources: &[ScriptSource]) -> CompiledScripts {
    let state = Arc::new(Mutex::new(BuilderState::default()));
    let engine = loading_engine(state.clone());

    // Hash and iterate in sorted path order regardless of caller order.
    let mut sorted: Vec<ScriptSource> = sources.to_vec();
    sorted.sort_by(|a, b| a.path.cmp(&b.path));

    let mut asts: BTreeMap<String, AST> = BTreeMap::new();
    let mut defined_fns: BTreeSet<String> = BTreeSet::new();
    let mut findings: Vec<WorldFinding> = Vec::new();

    for src in &sorted {
        if asts.contains_key(&src.path) {
            findings.push(finding(
                Severity::Error,
                "duplicate-script-path",
                &src.path,
                &src.path,
                format!("duplicate script path '{}'", src.path),
            ));
            continue;
        }
        state.lock().expect("builder state lock").current_path = src.path.clone();

        let ast = match engine.compile(&src.source) {
            Ok(ast) => ast,
            Err(err) => {
                findings.push(finding(
                    Severity::Error,
                    "script-parse-error",
                    &src.path,
                    &src.path,
                    format!("parse error: {err}"),
                ));
                continue;
            }
        };
        if let Err(err) = engine.run_ast(&ast) {
            findings.push(finding(
                Severity::Error,
                "script-runtime-error",
                &src.path,
                &src.path,
                format!("top-level runtime error: {err}"),
            ));
            continue;
        }

        for f in ast.iter_functions() {
            defined_fns.insert(f.name.to_string());
        }
        asts.insert(src.path.clone(), ast);
    }

    // Drop the engine so it releases its clone of the builder-state handle,
    // leaving `state` as the sole owner.
    drop(engine);
    let builder = Arc::try_unwrap(state)
        .map(|m| m.into_inner().expect("builder state lock"))
        .unwrap_or_default();
    let registrations = builder.registrations;
    let script_triggers = builder.script_triggers;
    let deadline_handlers = builder.deadline_handlers;

    let content_hash = vellum_script::content_hash(&sorted);

    CompiledScripts {
        asts,
        defined_fns,
        registrations,
        script_triggers,
        deadline_handlers,
        content_hash,
        // `compile_scripts` is handed sources with no world path to key a ledger
        // entry by; `load_world_scripts` is what knows the world and fills this.
        ledger_digest: None,
        findings,
    }
}

/// The content-ledger key one world's compiled script set is recorded under
/// (issue #864).
///
/// `<world path>#scripts`, in the same `path#part` shape the loader already uses
/// for an inline block's virtual path (`world.toml#script.<key>`) — so a
/// designer reading a refusal sees a key that names the world it belongs to, and
/// two worlds' script sets can never collide on one entry.
pub fn script_ledger_key(world_path: &str) -> String {
    format!("{world_path}#scripts")
}

/// The full load path for one world: lift → compile → cross-reference validate,
/// folding every finding into one [`CompiledScripts`]. No caller wires this into
/// activation in M1 (no shipped world authors scripts yet); it exists so the
/// seam is complete and tested.
pub fn load_world_scripts(
    world_path: &str,
    world_toml: &toml::Value,
    resolver: &dyn ScriptResolver,
) -> CompiledScripts {
    let (sources, lift_findings) = lift_world_scripts(world_path, world_toml, resolver);
    let mut compiled = compile_scripts(&sources);
    compiled.findings.extend(lift_findings);
    // Bind a save to the exact script set it was recorded against (issue #864).
    //
    // The compiled set's own `content_hash` — the same u64 that becomes
    // `WorldScriptRuntime::content_hash` — belongs in the content ledger, so
    // `snapshot::content_digest` folds it and `Versions::check` refuses a save
    // whose scripts have since moved.
    //
    // RETURNED, not written (issue #1241). It used to be recorded right here, on
    // the theory that doing it at the one shared site covered every call site by
    // construction. What that actually did was make the ledger a side effect of
    // compiling: `world::load::load` promises its caller that every ledger write
    // comes back as a `LedgerPlan` to apply, and this one silently did not, so a
    // caller that deliberately dropped the plan — the pure layer decision, the
    // `Inspect` linters — still moved global state. Every call site now applies
    // it explicitly, which is one line each and visible in the diff.
    //
    // Belt AND braces, deliberately: an inline `[script]` block already rides in
    // the world TOML's own recorded text, so an edit to it moves the digest
    // twice. What this closes is the sibling-`.rhai` case, where the *text* is
    // recorded by `OverlayScriptResolver` but nothing recorded the compiled set,
    // and the case where two different source sets compile to the same text
    // shape. On wasm the ledger is frozen in `wasm_init` before `Startup` runs,
    // so a browser save binds through the world TOML's inline blocks (the whole
    // shipped set) rather than through this record — moving that freeze past
    // `Startup` is `server/bridge.rs`'s call, not this loader's.
    compiled.ledger_digest = (!sources.is_empty()).then(|| crate::content_ledger::LedgerDigest {
        key: script_ledger_key(world_path),
        digest: compiled.content_hash,
    });
    // Cross-reference every handler name against the defined-function set: the
    // generic `on(..)` registrations, the Rhai trigger front-end
    // (`on_destroyed`, …). Every unresolved name is an error finding, so the
    // atomic activation gate keeps working (issue #980, M2).
    //
    // The TOML `[[trigger]] script = "fn"` / `[[comms]] script = "fn"` front-ends
    // had their own cross-reference passes here; issue #985 deleted both blocks,
    // so a world can no longer name a handler from TOML at all.
    compiled.findings.extend(validate_registrations(
        &compiled.registrations,
        &compiled.defined_fns,
    ));
    compiled.findings.extend(validate_script_triggers(
        &compiled.script_triggers,
        &compiled.defined_fns,
    ));
    // And the named-deadline pairing (issue #1024): every `on_deadline(id, fn)`
    // must name a `[[deadline]]` this world authored AND a fn that exists, and
    // every authored `[[deadline]]` must have exactly one handler. All three are
    // error findings on the same channel, so the atomic activation gate blocks a
    // world whose deadlines could never fire — the failure that would otherwise
    // surface as a countdown running to zero and nothing happening.
    compiled.findings.extend(validate_deadline_handlers(
        world_path,
        world_toml,
        &compiled.deadline_handlers,
        &compiled.defined_fns,
    ));
    // Walk each script body for a compound assignment on the `flags` accessor
    // (`flags.x += n`), which Rhai desugars to an absolute get-then-set and so
    // silently drains as a clobber-prone `SetValue` instead of a composable
    // increment. Each is a blocking finding, so the atomic activation gate keeps a
    // world that reaches for the old `+=` idiom from spawning (issue #994).
    compiled.findings.extend(validate_flag_opassign(&sources));
    // And the dialogue tree's OWN cross-reference (issue #984): every
    // `on_pick: "fn"` literal inside a node fn's returned map, checked against the
    // same defined-function set. Every node past a thread's root is named by a
    // string literal nothing else in the load path reads, so without this a typo
    // three responses deep survived load and surfaced as a refused pick
    // mid-mission.
    compiled
        .findings
        .extend(validate_on_pick_fns(&sources, &compiled.defined_fns));
    compiled
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// Fake resolver serving sibling scripts from a map.
    #[derive(Default)]
    struct FakeResolver {
        files: HashMap<String, String>,
    }

    impl ScriptResolver for FakeResolver {
        fn read(&self, path: &str) -> Option<String> {
            self.files.get(path).cloned()
        }
    }

    fn toml_of(src: &str) -> toml::Value {
        toml::from_str(src).expect("valid toml")
    }

    #[test]
    fn lifts_inline_blocks_to_virtual_paths_sorted() {
        // Authored out of sorted order to prove the loader sorts.
        let world = toml_of(
            r#"
            [script]
            on_zulu = "fn on_zulu(ctx) { }"
            on_alpha = "fn on_alpha(ctx) { }"
            "#,
        );
        let (sources, findings) =
            lift_world_scripts("assets/worlds/w.toml", &world, &FakeResolver::default());
        assert!(findings.is_empty());
        let paths: Vec<&str> = sources.iter().map(|s| s.path.as_str()).collect();
        assert_eq!(
            paths,
            vec![
                "assets/worlds/w.toml#script.on_alpha",
                "assets/worlds/w.toml#script.on_zulu",
            ]
        );
    }

    #[test]
    fn lifts_a_sibling_file_relative_to_the_world() {
        let mut resolver = FakeResolver::default();
        resolver.files.insert(
            "assets/worlds/combat.rhai".to_string(),
            "fn on_x(ctx) { }".to_string(),
        );
        let world = toml_of(r#"script = "combat.rhai""#);
        let (sources, findings) =
            lift_world_scripts("assets/worlds/combat_test.toml", &world, &resolver);
        assert!(findings.is_empty());
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].path, "assets/worlds/combat.rhai");
        assert_eq!(sources[0].source, "fn on_x(ctx) { }");
    }

    #[test]
    fn discovers_only_a_top_level_sibling_declaration_for_async_prefetch() {
        assert_eq!(
            declared_sibling_script_path(
                "assets/worlds/layer.toml",
                "script = \"logic.rhai\"\n[global]\nseed = 1",
            )
            .as_deref(),
            Some("assets/worlds/logic.rhai")
        );
        assert!(declared_sibling_script_path(
            "assets/worlds/layer.toml",
            "[script]\nsetup = \"fn f(ctx) {}\"",
        )
        .is_none());
    }

    #[test]
    fn a_missing_sibling_file_is_an_error_finding() {
        let world = toml_of(r#"script = "missing.rhai""#);
        let (sources, findings) =
            lift_world_scripts("assets/worlds/w.toml", &world, &FakeResolver::default());
        assert!(sources.is_empty());
        assert_eq!(findings.len(), 1);
        assert!(findings[0].is_error());
        assert_eq!(findings[0].category, "script-file-missing");
    }

    #[test]
    fn no_script_key_yields_nothing() {
        let world = toml_of(r#"name = "w""#);
        let (sources, findings) = lift_world_scripts("w.toml", &world, &FakeResolver::default());
        assert!(sources.is_empty());
        assert!(findings.is_empty());
    }

    #[test]
    fn compile_collects_fns_and_hashes_stably() {
        let sources = vec![
            ScriptSource {
                path: "b.rhai".to_string(),
                source: "fn beta(ctx) { }".to_string(),
            },
            ScriptSource {
                path: "a.rhai".to_string(),
                source: "fn alpha(ctx) { }".to_string(),
            },
        ];
        let compiled = compile_scripts(&sources);
        assert!(compiled.findings.is_empty());
        assert!(compiled.defined_fns.contains("alpha"));
        assert!(compiled.defined_fns.contains("beta"));
        // Keys are the sorted paths.
        let keys: Vec<&String> = compiled.asts.keys().collect();
        assert_eq!(keys, vec!["a.rhai", "b.rhai"]);
        // The hash is order-independent (both orders hash the same sorted set).
        let mut reordered = sources.clone();
        reordered.reverse();
        assert_eq!(
            compiled.content_hash,
            compile_scripts(&reordered).content_hash
        );
    }

    #[test]
    fn a_parse_error_is_a_finding_not_a_panic() {
        let sources = vec![ScriptSource {
            path: "bad.rhai".to_string(),
            source: "fn oops(ctx) { let x = ; }".to_string(),
        }];
        let compiled = compile_scripts(&sources);
        assert!(compiled.asts.is_empty());
        assert_eq!(compiled.findings.len(), 1);
        assert_eq!(compiled.findings[0].category, "script-parse-error");
    }

    #[test]
    fn full_load_flags_an_unresolved_registration() {
        // Top level registers a handler that no function defines.
        let world = toml_of(
            r#"
            [script]
            setup = """
            on("flag_set:armed", "handle_armed");
            fn other(ctx) { }
            """
            "#,
        );
        let compiled = load_world_scripts("w.toml", &world, &FakeResolver::default());
        assert!(
            crate::world::validate::has_error(&compiled.findings),
            "an unresolved handler must block activation"
        );
        assert!(compiled
            .findings
            .iter()
            .any(|f| f.category == "unresolved-script-fn"));
    }

    #[test]
    fn full_load_of_a_resolved_registration_is_clean() {
        let world = toml_of(
            r#"
            [script]
            setup = """
            on("flag_set:armed", "handle_armed");
            fn handle_armed(ctx) { }
            """
            "#,
        );
        let compiled = load_world_scripts("w.toml", &world, &FakeResolver::default());
        assert!(
            !crate::world::validate::has_error(&compiled.findings),
            "findings: {:?}",
            compiled.findings
        );
    }

    // ── `flags` compound-assignment lint (issue #994) ─────────────────────────

    #[test]
    fn full_load_blocks_a_flag_opassign() {
        // A `flags.x += n` body must block activation via the atomic gate, exactly
        // like an unresolved handler.
        let world = toml_of(
            r#"
            [script]
            setup = "fn on_x(ctx) { ctx.flags.score += 50; }"
            "#,
        );
        let compiled = load_world_scripts("w.toml", &world, &FakeResolver::default());
        assert!(
            crate::world::validate::has_error(&compiled.findings),
            "a flag compound-assignment must block activation"
        );
        assert!(compiled
            .findings
            .iter()
            .any(|f| f.category == "flag-opassign-not-composable"));
    }

    #[test]
    fn full_load_of_the_increment_verb_is_clean() {
        let world = toml_of(
            r#"
            [script]
            setup = "fn on_x(ctx) { ctx.flags.increment(\"score\", 50); }"
            "#,
        );
        let compiled = load_world_scripts("w.toml", &world, &FakeResolver::default());
        assert!(
            !crate::world::validate::has_error(&compiled.findings),
            "findings: {:?}",
            compiled.findings
        );
    }

    #[test]
    fn full_load_of_a_plain_flag_assign_is_clean() {
        let world = toml_of(
            r#"
            [script]
            setup = "fn on_x(ctx) { ctx.flags.armed = 1; }"
            "#,
        );
        let compiled = load_world_scripts("w.toml", &world, &FakeResolver::default());
        assert!(
            !crate::world::validate::has_error(&compiled.findings),
            "findings: {:?}",
            compiled.findings
        );
    }

    // ── dialogue `on_pick` resolution lint (issue #984) ───────────────────────

    #[test]
    fn full_load_blocks_a_dialogue_node_naming_a_missing_on_pick_fn() {
        // The gap the root-fn check could not reach: `hail_axiom` resolves, but
        // the response inside the node it returns names a fn that does not
        // exist. Without this lint the typo survives load and surfaces as a
        // refused pick mid-mission.
        let world = toml_of(
            r#"
            [[comms]]
            from = "axiom"
            trigger = "on_hailed"
            entity = "axiom"
            script = "hail_axiom"

            [script]
            setup = """
            fn hail_axiom(ctx) {
                #{ message: "Go ahead.", responses: [
                    #{ text: "Acknowledge", on_pick: "on_ack" },
                    #{ text: "Decline",     on_pick: "on_declien" },
                ] }
            }
            fn on_ack(ctx)     { }
            fn on_decline(ctx) { }
            """
            "#,
        );
        let compiled = load_world_scripts("w.toml", &world, &FakeResolver::default());
        assert!(
            crate::world::validate::has_error(&compiled.findings),
            "an unresolved on_pick must block activation: {:?}",
            compiled.findings
        );
        let f = compiled
            .findings
            .iter()
            .find(|f| f.category == "unresolved-on-pick-fn")
            .expect("the on_pick finding");
        assert_eq!(f.source.reference, "on_declien", "it names the fn");
        assert_eq!(f.source.file, "w.toml#script.setup", "and the file");
    }

    #[test]
    fn full_load_of_a_resolved_dialogue_tree_is_clean() {
        let world = toml_of(
            r#"
            [[comms]]
            from = "axiom"
            trigger = "on_hailed"
            entity = "axiom"
            script = "hail_axiom"

            [script]
            setup = """
            fn hail_axiom(ctx) {
                #{ message: "Go ahead.", responses: [
                    #{ text: "Acknowledge", on_pick: "on_ack" },
                    #{ text: "Decline",     on_pick: "on_decline" },
                ] }
            }
            fn on_ack(ctx)     { #{ message: "Cleared.", responses: [] } }
            fn on_decline(ctx) { }
            """
            "#,
        );
        let compiled = load_world_scripts("w.toml", &world, &FakeResolver::default());
        assert!(
            !crate::world::validate::has_error(&compiled.findings),
            "findings: {:?}",
            compiled.findings
        );
    }

    #[test]
    fn full_load_does_not_flag_a_dynamically_built_on_pick() {
        // The documented limitation of a lexical pass, pinned at the load
        // boundary: a computed name is left to the runtime rather than blocking
        // a legitimate world.
        let world = toml_of(
            r#"
            [script]
            setup = """
            fn root(ctx) {
                let kind = "ack";
                #{ message: "Go ahead.", responses: [
                    #{ text: "Yes", on_pick: "on_" + kind },
                ] }
            }
            """
            "#,
        );
        let compiled = load_world_scripts("w.toml", &world, &FakeResolver::default());
        assert!(
            !crate::world::validate::has_error(&compiled.findings),
            "findings: {:?}",
            compiled.findings
        );
    }
}
