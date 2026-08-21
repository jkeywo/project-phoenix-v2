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

/// A one-line valid inline script block that compiles with zero findings.
const INLINE_SCRIPT_WORLD: &str = "[global]\nseed = 5\n[script]\non_alpha = \"fn on_alpha(ctx) { }\"\n";

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
    let loaded = load(request("w.toml", &reader, &NoScriptResolver, LoadPolicy::Inspect))
        .expect("inspect should load");

    assert_eq!(loaded.config.global.seed, Some(7));
    assert!(loaded.scripts.is_none(), "inspect compiles no scripts");
    assert!(loaded.children.is_empty(), "inspect recurses no children");
    assert!(loaded.findings.is_empty(), "inspect runs no composition gate");
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
    let loaded = load(request("root.toml", &reader, &NoScriptResolver, LoadPolicy::Inspect))
        .expect("inspect ignores extra_worlds");
    assert_eq!(loaded.config.extra_worlds, vec!["missing.toml".to_string()]);
    assert!(loaded.children.is_empty());
}

#[test]
fn inspect_read_failure_is_a_load_error() {
    let reader = MemoryReader::new(std::iter::empty::<(String, String)>());
    let err = load(request("absent.toml", &reader, &NoScriptResolver, LoadPolicy::Inspect))
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
    let err = load(request("bad.toml", &reader, &NoScriptResolver, LoadPolicy::Inspect))
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
    let loaded = load(request("layer.toml", &reader, &NoScriptResolver, LoadPolicy::Merge))
        .expect("merge should load");

    assert_eq!(loaded.config.global.seed, Some(9));
    assert!(loaded.scripts.is_none(), "a script-free world carries no scripts");
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
    assert_eq!(loaded.ledger.records.len(), 1, "merge records the world once");
}

// ── Activate ────────────────────────────────────────────────────────────────

#[test]
fn activate_records_the_root_with_no_children() {
    crate::content_ledger::reset();
    let text = "[global]\nseed = 3\n";
    let reader = MemoryReader::new([("root.toml", text)]);
    let loaded = load(request("root.toml", &reader, &NoScriptResolver, LoadPolicy::Activate))
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

    let loaded = load(request("root.toml", &reader, &NoScriptResolver, LoadPolicy::Activate))
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
fn activate_missing_child_is_a_load_error() {
    let reader = MemoryReader::new([("root.toml", "extra_worlds = [\"gone.toml\"]\n")]);
    let err = load(request("root.toml", &reader, &NoScriptResolver, LoadPolicy::Activate))
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
    let err = load(request("root.toml", &reader, &NoScriptResolver, LoadPolicy::Activate))
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
    let req = request("w.toml", &reader, &NoScriptResolver, LoadPolicy::Merge).with_transform(&inject);
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
    let req = request("w.toml", &reader, &NoScriptResolver, LoadPolicy::Merge).with_transform(&boom);
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
    };
    plan.apply();

    assert_eq!(
        crate::content_ledger::snapshot().len(),
        2,
        "apply records both files into the live ledger"
    );
    crate::content_ledger::reset();
}
