//! Unit tests for [`super::load`] over a [`MemoryReader`] fixture (issue #1213).
//!
//! One test per [`LoadPolicy`] plus the error and hook edges. Everything runs off
//! an in-memory `path -> TOML` map, so no filesystem or bridge is involved.

use super::*;
use crate::world::validate::has_error;

/// A resolver that serves no sibling scripts — every test authors its scripts
/// inline (`[script]` blocks are lifted from the TOML directly and never reach a
/// resolver), so this stays `None` throughout.
struct NoScriptResolver;

impl ScriptResolver for NoScriptResolver {
    fn read(&self, _path: &str) -> Option<String> {
        None
    }
}

/// One resolver-backed sibling with an observable read count, so the root
/// regression proves the reorder compiles once rather than validating through a
/// second compile.
struct CountingScriptResolver {
    expected_path: &'static str,
    source: &'static str,
    reads: std::cell::Cell<usize>,
}

impl ScriptResolver for CountingScriptResolver {
    fn read(&self, path: &str) -> Option<String> {
        if path != self.expected_path {
            return None;
        }
        self.reads.set(self.reads.get() + 1);
        Some(self.source.to_string())
    }
}

/// A one-line valid inline script block that compiles with zero findings.
const INLINE_SCRIPT_WORLD: &str =
    "[global]\nseed = 5\n[script]\non_alpha = \"fn on_alpha(ctx) { }\"\n";

fn request<'a>(
    path: &str,
    reader: &'a dyn WorldReader,
    resolver: &'a dyn ScriptResolver,
    policy: LoadPolicy,
) -> LoadRequest<'a> {
    LoadRequest::new(path.to_string(), reader, resolver, policy)
}

// ── Inspect ─────────────────────────────────────────────────────────────────

#[test]
fn inspect_parses_config_and_leaves_everything_else_empty() {
    let reader = MemoryReader::new([("w.toml", "[global]\nseed = 7\n")]);
    let loaded = load(request(
        "w.toml",
        &reader,
        &NoScriptResolver,
        LoadPolicy::Inspect,
    ))
    .expect("inspect should load");

    assert_eq!(loaded.config.global.seed, Some(7));
    assert!(loaded.scripts.is_none(), "inspect compiles no scripts");
    assert!(loaded.children.is_empty(), "inspect recurses no children");
    assert!(
        loaded.findings.is_empty(),
        "inspect runs no composition gate"
    );
    assert!(
        loaded.ledger.records.is_empty(),
        "inspect records nothing in the ledger plan"
    );
}

#[test]
fn inspect_ignores_extra_worlds_children() {
    // A child the reader does not carry — Inspect must not read it, so the load
    // still succeeds (unlike Activate, which would fail with ChildReadFailed).
    let reader = MemoryReader::new([("root.toml", "extra_worlds = [\"missing.toml\"]\n")]);
    let loaded = load(request(
        "root.toml",
        &reader,
        &NoScriptResolver,
        LoadPolicy::Inspect,
    ))
    .expect("inspect ignores extra_worlds");
    assert_eq!(loaded.config.extra_worlds, vec!["missing.toml".to_string()]);
    assert!(loaded.children.is_empty());
}

#[test]
fn inspect_read_failure_is_a_load_error() {
    let reader = MemoryReader::new(std::iter::empty::<(String, String)>());
    let err = load(request(
        "absent.toml",
        &reader,
        &NoScriptResolver,
        LoadPolicy::Inspect,
    ))
    .expect_err("a missing world must be a LoadError");
    assert_eq!(
        err,
        LoadError::ReadFailed {
            path: "absent.toml".to_string()
        }
    );
}

#[test]
fn inspect_parse_failure_is_a_load_error() {
    let reader = MemoryReader::new([("bad.toml", "this is not [ valid toml")]);
    let err = load(request(
        "bad.toml",
        &reader,
        &NoScriptResolver,
        LoadPolicy::Inspect,
    ))
    .expect_err("a broken world must be a LoadError");
    assert!(
        matches!(err, LoadError::ParseFailed { ref path, .. } if path == "bad.toml"),
        "expected ParseFailed for bad.toml, got {err:?}"
    );
}

// ── Merge ───────────────────────────────────────────────────────────────────

#[test]
fn merge_records_the_world_and_carries_no_scripts_for_a_script_free_world() {
    crate::content_ledger::reset();
    let text = "[global]\nseed = 9\n";
    let reader = MemoryReader::new([("layer.toml", text)]);
    let loaded = load(request(
        "layer.toml",
        &reader,
        &NoScriptResolver,
        LoadPolicy::Merge,
    ))
    .expect("merge should load");

    assert_eq!(loaded.config.global.seed, Some(9));
    assert!(
        loaded.scripts.is_none(),
        "a script-free world carries no scripts"
    );
    assert!(loaded.children.is_empty(), "merge recurses no children");
    assert!(loaded.findings.is_empty(), "merge runs no composition gate");
    assert_eq!(
        loaded.ledger.records,
        vec![LedgerRecord {
            path: "layer.toml".to_string(),
            text: text.to_string(),
        }],
        "merge records exactly the world TOML it read"
    );
}

#[test]
fn merge_compiles_inline_scripts_and_carries_them() {
    crate::content_ledger::reset();
    let reader = MemoryReader::new([("scripted.toml", INLINE_SCRIPT_WORLD)]);
    let loaded = load(request(
        "scripted.toml",
        &reader,
        &NoScriptResolver,
        LoadPolicy::Merge,
    ))
    .expect("merge should load a scripted world");

    let scripts = loaded.scripts.expect("inline scripts must be carried");
    assert!(
        !has_error(&scripts.findings),
        "a well-formed inline script compiles cleanly: {:?}",
        scripts.findings
    );
    assert!(
        !scripts.asts.is_empty(),
        "the inline block must produce an AST"
    );
    assert_eq!(
        loaded.ledger.records.len(),
        1,
        "merge records the world once"
    );
}

// ── Activate ────────────────────────────────────────────────────────────────

#[test]
fn activate_records_the_root_with_no_children() {
    crate::content_ledger::reset();
    let text = "[global]\nseed = 3\n";
    let reader = MemoryReader::new([("root.toml", text)]);
    let loaded = load(request(
        "root.toml",
        &reader,
        &NoScriptResolver,
        LoadPolicy::Activate,
    ))
    .expect("activate should load");

    assert!(loaded.children.is_empty());
    assert!(
        loaded.findings.is_empty(),
        "an entity-free world validates clean: {:?}",
        loaded.findings
    );
    assert_eq!(
        loaded.ledger.records,
        vec![LedgerRecord {
            path: "root.toml".to_string(),
            text: text.to_string(),
        }]
    );
}

#[test]
fn activate_loads_children_and_records_the_whole_composition() {
    crate::content_ledger::reset();
    let root = "extra_worlds = [\"child.toml\"]\n[global]\nseed = 1\n";
    let child = "[global]\nseed = 2\n";
    let reader = MemoryReader::new([("root.toml", root), ("child.toml", child)]);

    let loaded = load(request(
        "root.toml",
        &reader,
        &NoScriptResolver,
        LoadPolicy::Activate,
    ))
    .expect("activate should load the composition");

    assert_eq!(loaded.children.len(), 1, "the one extra_world is loaded");
    assert_eq!(loaded.children[0].config.global.seed, Some(2));
    assert!(
        loaded.children[0].ledger.records.is_empty(),
        "a child's own ledger plan stays empty; its record rides on the root"
    );
    assert!(
        !has_error(&loaded.findings),
        "an entity-free composition validates clean: {:?}",
        loaded.findings
    );
    assert_eq!(
        loaded.ledger.records,
        vec![
            LedgerRecord {
                path: "root.toml".to_string(),
                text: root.to_string(),
            },
            LedgerRecord {
                path: "child.toml".to_string(),
                text: child.to_string(),
            },
        ],
        "the ledger plan records the root then each child, in order"
    );
}

#[test]
fn activate_retains_a_childs_sibling_scripts_digest_and_spawn_provenance() {
    const CHILD_PATH: &str = "assets/worlds/child_scripted.toml";
    const SCRIPT_PATH: &str = "assets/worlds/child_scripted.rhai";
    const MISSING: &str = "assets/entities/missing_child_sibling_1047.toml";
    const ROOT: &str =
        "extra_worlds = [\"assets/worlds/child_scripted.toml\"]\n[global]\nseed = 1\n";
    const CHILD: &str = "script = \"child_scripted.rhai\"\n[global]\nseed = 2\n";
    const SCRIPT: &str = "fn release(ctx) {\n    ctx.effects.spawn_entity(#{ template_path: \"assets/entities/missing_child_sibling_1047.toml\", name: \"missing\", position: [0, 0, 0] });\n}\n";

    let reader = MemoryReader::new([("assets/worlds/root.toml", ROOT), (CHILD_PATH, CHILD)]);
    let resolver = CountingScriptResolver {
        expected_path: SCRIPT_PATH,
        source: SCRIPT,
        reads: std::cell::Cell::new(0),
    };
    let loaded = load(request(
        "assets/worlds/root.toml",
        &reader,
        &resolver,
        LoadPolicy::Activate,
    ))
    .expect("the scripted child is retained as pre-freeze load data");

    assert!(loaded.scripts.is_none(), "the root itself is script-free");
    assert_eq!(resolver.reads.get(), 1, "the child sibling compiles once");
    let child_scripts = loaded.children[0]
        .scripts
        .as_ref()
        .expect("the child's compiled set must not be discarded");
    assert_eq!(child_scripts.spawned_templates.len(), 1);
    assert_eq!(child_scripts.spawned_templates[0].source_path, SCRIPT_PATH);
    assert_eq!(child_scripts.spawned_templates[0].template_path, MISSING);

    let child_digest = child_scripts
        .ledger_digest
        .clone()
        .expect("a scripted child carries a content-ledger digest");
    assert_eq!(child_digest.key, format!("{CHILD_PATH}#scripts"));
    assert_eq!(loaded.ledger.digests, vec![child_digest]);

    let unresolved: Vec<_> = loaded
        .findings
        .iter()
        .filter(|finding| {
            finding.category == "unresolvable-template" && finding.source.reference == MISSING
        })
        .collect();
    assert_eq!(unresolved.len(), 1, "findings: {:?}", loaded.findings);
    assert_eq!(unresolved[0].source.file, SCRIPT_PATH);
    assert_eq!(unresolved[0].source.line, Some(2));
}

#[test]
fn activate_applies_the_raw_transform_to_the_root_not_static_children() {
    let root = "extra_worlds = [\"child.toml\"]\n[global]\nseed = 1\n";
    let child = "[global]\nseed = 2\n[script]\nsetup = \"fn child_handler(ctx) { }\"\n";
    let reader = MemoryReader::new([("root.toml", root), ("child.toml", child)]);
    let calls = std::cell::Cell::new(0usize);
    let root_only = |value: toml::Value| -> Result<toml::Value, String> {
        calls.set(calls.get() + 1);
        Ok(value)
    };

    let loaded = load(
        request(
            "root.toml",
            &reader,
            &NoScriptResolver,
            LoadPolicy::Activate,
        )
        .with_transform(&root_only),
    )
    .expect("the root and child load");

    assert_eq!(
        calls.get(),
        1,
        "the harness transform belongs to the root and must not run over a supporting world"
    );
    assert!(loaded.scripts.is_none());
    assert!(
        loaded.children[0].scripts.is_some(),
        "the child still compiles its own untouched inline set"
    );
}

#[test]
fn activate_validates_a_root_sibling_scripts_spawn_with_exact_provenance_once() {
    const ROOT: &str = "script = \"root_wave.rhai\"\n[global]\nseed = 1\n";
    const SCRIPT_PATH: &str = "assets/worlds/root_wave.rhai";
    const MISSING: &str = "assets/entities/missing_root_sibling_1046.toml";
    const SCRIPT: &str = "fn release(ctx) {\n    ctx.effects.spawn_entity(#{ template_path: \"assets/entities/missing_root_sibling_1046.toml\", name: \"missing\", position: [0, 0, 0] });\n}\n";

    let reader = MemoryReader::new([("assets/worlds/root.toml", ROOT)]);
    let resolver = CountingScriptResolver {
        expected_path: SCRIPT_PATH,
        source: SCRIPT,
        reads: std::cell::Cell::new(0),
    };
    let loaded = load(request(
        "assets/worlds/root.toml",
        &reader,
        &resolver,
        LoadPolicy::Activate,
    ))
    .expect("the load returns source-located composition findings");

    assert_eq!(resolver.reads.get(), 1, "the root sibling compiles once");
    let unresolved: Vec<_> = loaded
        .findings
        .iter()
        .filter(|finding| {
            finding.category == "unresolvable-template" && finding.source.reference == MISSING
        })
        .collect();
    assert_eq!(unresolved.len(), 1, "findings: {:?}", loaded.findings);
    assert_eq!(unresolved[0].source.file, SCRIPT_PATH);
    assert_eq!(unresolved[0].source.line, Some(2));
}

#[test]
fn activate_uses_resolved_inline_refs_instead_of_scanning_them_twice() {
    const MISSING: &str = "assets/entities/missing_root_inline_1046.toml";
    const ROOT: &str = r#"
[script]
setup = """
fn release(ctx) {
    ctx.effects.spawn_entity(#{ template_path: "assets/entities/missing_root_inline_1046.toml", name: "missing", position: [0, 0, 0] });
}
"""
"#;
    let reader = MemoryReader::new([("assets/worlds/root_inline.toml", ROOT)]);
    let loaded = load(request(
        "assets/worlds/root_inline.toml",
        &reader,
        &NoScriptResolver,
        LoadPolicy::Activate,
    ))
    .expect("the load returns source-located composition findings");

    let unresolved: Vec<_> = loaded
        .findings
        .iter()
        .filter(|finding| {
            finding.category == "unresolvable-template" && finding.source.reference == MISSING
        })
        .collect();
    assert_eq!(unresolved.len(), 1, "findings: {:?}", loaded.findings);
    assert_eq!(
        unresolved[0].source.file,
        "assets/worlds/root_inline.toml#script.setup"
    );
    assert_eq!(unresolved[0].source.line, Some(2));
}

#[test]
fn activate_missing_child_is_a_load_error() {
    let reader = MemoryReader::new([("root.toml", "extra_worlds = [\"gone.toml\"]\n")]);
    let err = load(request(
        "root.toml",
        &reader,
        &NoScriptResolver,
        LoadPolicy::Activate,
    ))
    .expect_err("a missing child must abort the load");
    assert_eq!(
        err,
        LoadError::ChildReadFailed {
            path: "gone.toml".to_string()
        }
    );
}

#[test]
fn activate_broken_child_is_a_load_error() {
    let reader = MemoryReader::new([
        ("root.toml", "extra_worlds = [\"child.toml\"]\n"),
        ("child.toml", "not [ valid"),
    ]);
    let err = load(request(
        "root.toml",
        &reader,
        &NoScriptResolver,
        LoadPolicy::Activate,
    ))
    .expect_err("a broken child must abort the load");
    assert!(
        matches!(err, LoadError::ChildParseFailed { ref path, .. } if path == "child.toml"),
        "expected ChildParseFailed for child.toml, got {err:?}"
    );
}

// ── The raw_transform hook ──────────────────────────────────────────────────

#[test]
fn raw_transform_runs_before_script_compile_and_leaves_the_config_untouched() {
    crate::content_ledger::reset();
    // The original text has no `[script]` key; the transform injects one, so the
    // compiled scripts must appear even though the parsed config is derived from
    // the untouched text (proving the hook touches only the raw script value).
    let inject = |value: toml::Value| -> Result<toml::Value, String> {
        let mut table = value.as_table().cloned().unwrap_or_default();
        let script: toml::Value =
            toml::from_str("on_alpha = \"fn on_alpha(ctx) { }\"").map_err(|e| e.to_string())?;
        table.insert("script".to_string(), script);
        Ok(toml::Value::Table(table))
    };

    let reader = MemoryReader::new([("w.toml", "[global]\nseed = 4\n")]);
    let req =
        request("w.toml", &reader, &NoScriptResolver, LoadPolicy::Merge).with_transform(&inject);
    let loaded = load(req).expect("transform load should succeed");

    assert_eq!(
        loaded.config.global.seed,
        Some(4),
        "the config is parsed from the untouched text, not the transformed value"
    );
    let scripts = loaded
        .scripts
        .expect("the injected [script] block must compile into carried scripts");
    assert!(!has_error(&scripts.findings), "{:?}", scripts.findings);
}

#[test]
fn raw_transform_error_aborts_the_load() {
    let boom = |_value: toml::Value| -> Result<toml::Value, String> { Err("boom".to_string()) };
    let reader = MemoryReader::new([("w.toml", "[global]\nseed = 4\n")]);
    let req =
        request("w.toml", &reader, &NoScriptResolver, LoadPolicy::Merge).with_transform(&boom);
    let err = load(req).expect_err("a failing transform must abort");
    assert_eq!(
        err,
        LoadError::TransformFailed {
            message: "boom".to_string()
        }
    );
}

// ── LedgerPlan application ──────────────────────────────────────────────────

#[test]
fn ledger_plan_apply_writes_the_records_to_the_content_ledger() {
    crate::content_ledger::reset();
    assert!(crate::content_ledger::snapshot().is_empty());

    let plan = LedgerPlan {
        records: vec![
            LedgerRecord {
                path: "a.toml".to_string(),
                text: "[global]\n".to_string(),
            },
            LedgerRecord {
                path: "b.toml".to_string(),
                text: "[global]\nseed = 1\n".to_string(),
            },
        ],
        digests: vec![crate::content_ledger::LedgerDigest {
            key: "a.toml#scripts".to_string(),
            digest: 0xfeed_beef,
        }],
    };
    plan.apply();

    let after = crate::content_ledger::snapshot();
    assert_eq!(
        after.len(),
        3,
        "apply writes both text records AND the digest half into the live ledger"
    );
    assert_eq!(
        after.get("a.toml#scripts"),
        Some(0xfeed_beef),
        "a digest record is stored verbatim, not re-hashed"
    );
    crate::content_ledger::reset();
}

/// The lift itself (issue #1241): compiling a world's scripts writes NOTHING to
/// the global ledger — the digest comes back on the compiled set for a caller to
/// apply.
///
/// This is the regression guard for the defect, not just for the plumbing: before
/// #1241 `load_world_scripts` recorded from inside itself, so a caller that
/// deliberately dropped the plan still moved global state.
#[test]
fn compiling_a_worlds_scripts_records_nothing_until_the_caller_applies_it() {
    use crate::world::script::load::{load_world_scripts, NoSiblingScripts};

    const WORLD: &str = r#"
[script]
setup = "fn on_x(ctx) { }"
"#;
    let value: toml::Value = toml::from_str(WORLD).expect("valid toml");

    crate::content_ledger::reset();
    let compiled = load_world_scripts("w.toml", &value, &NoSiblingScripts);
    assert!(
        crate::content_ledger::snapshot().is_empty(),
        "the compile must not touch the ledger"
    );

    let digest = compiled
        .ledger_digest
        .as_ref()
        .expect("a world with sources carries its digest record");
    assert_eq!(digest.key, "w.toml#scripts");
    assert_eq!(
        digest.digest, compiled.content_hash,
        "the record carries the set's own content hash — the #988 save binding"
    );

    digest.apply();
    assert_eq!(
        crate::content_ledger::snapshot().get("w.toml#scripts"),
        Some(compiled.content_hash),
        "and applying it lands exactly what the eager write used to land"
    );
    crate::content_ledger::reset();
}

/// A script-free world carries no digest record at all — the "shipped set pays
/// nothing" property, preserved verbatim across the lift.
#[test]
fn a_script_free_world_carries_no_ledger_digest() {
    use crate::world::script::load::{load_world_scripts, NoSiblingScripts};

    let value: toml::Value = toml::from_str("[global]\nseed = 1\n").expect("valid toml");
    let compiled = load_world_scripts("w.toml", &value, &NoSiblingScripts);
    assert!(compiled.ledger_digest.is_none());
}

/// THE equivalence the lift has to preserve (issue #1241): the ledger a load
/// leaves behind is identical whether the script digest was written from inside
/// the loader (the shape before) or applied by the caller off the returned plan
/// (the shape after).
///
/// Reconstructs the OLD shape exactly — the digest written at compile time, then
/// the caller applying only the text records — and compares the whole resulting
/// ledger, not just its fold, against the new one-call `plan.apply()`. Over a
/// composition with a child, so the interleaving of root text / child text /
/// digest is exercised rather than assumed.
#[test]
fn the_returned_plan_lands_the_same_ledger_the_eager_write_did() {
    let root = "extra_worlds = [\"child.toml\"]\n[global]\nseed = 1\n\
                [script]\nsetup = \"fn on_x(ctx) { }\"\n";
    let child = "[global]\nseed = 2\n";
    let reader = MemoryReader::new([("root.toml", root), ("child.toml", child)]);

    // NEW: one `apply()` covering both halves.
    crate::content_ledger::reset();
    let loaded = load(request(
        "root.toml",
        &reader,
        &NoScriptResolver,
        LoadPolicy::Activate,
    ))
    .expect("activate should load");
    loaded.ledger.apply();
    let after_new = crate::content_ledger::snapshot();

    // OLD: the digest written where `load_world_scripts` used to write it —
    // during the compile, before the caller applied anything — then the text
    // records applied by the caller.
    crate::content_ledger::reset();
    let loaded_again = load(request(
        "root.toml",
        &reader,
        &NoScriptResolver,
        LoadPolicy::Activate,
    ))
    .expect("activate should load");
    let eager = loaded_again
        .scripts
        .as_ref()
        .and_then(|s| s.ledger_digest.clone())
        .expect("the scripted root carries a digest");
    crate::content_ledger::record_digest(&eager.key, eager.digest);
    for record in &loaded_again.ledger.records {
        crate::content_ledger::record(&record.path, &record.text);
    }
    let after_old = crate::content_ledger::snapshot();

    assert_eq!(
        after_new, after_old,
        "every (key, digest) pair must match, not merely fold to the same number"
    );
    assert_eq!(after_new.fold(), after_old.fold());

    // And the comparison is not vacuous: the digest really is in there, keyed to
    // the root world, alongside both TOML texts.
    assert_eq!(after_new.len(), 3);
    assert_eq!(
        after_new.get("root.toml#scripts"),
        Some(loaded.scripts.as_ref().expect("compiled").content_hash),
    );
    crate::content_ledger::reset();
}
