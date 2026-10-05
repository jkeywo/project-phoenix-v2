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
fn source_ledger_record_matches_compiled_hash_without_mutating_content() {
    let sources = vec![
        ScriptSource {
            path: "world.toml#script.zeta".into(),
            source: "fn zeta(ctx) {}".into(),
        },
        ScriptSource {
            path: "world.toml#script.alpha".into(),
            source: "fn alpha(ctx) {}".into(),
        },
    ];
    let before = crate::content_ledger::snapshot();
    let record = script_source_ledger_digest("world.toml", &sources).unwrap();
    assert_eq!(record.key, "world.toml#scripts");
    assert_eq!(record.digest, compile_scripts(&sources).content_hash);
    let reversed: Vec<_> = sources.into_iter().rev().collect();
    assert_eq!(
        Some(record),
        script_source_ledger_digest("world.toml", &reversed)
    );
    assert!(script_source_ledger_digest("world.toml", &[]).is_none());
    assert_eq!(crate::content_ledger::snapshot(), before);
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
fn resolved_spawn_refs_are_sorted_by_unit_and_keep_exact_lines() {
    let sources = vec![
            ScriptSource {
                path: "worlds/zeta.rhai".to_string(),
                source: "fn z(ctx) {\n    ctx.effects.spawn_entity(#{ template_path: \"z.toml\" });\n}"
                    .to_string(),
            },
            ScriptSource {
                path: "worlds/alpha.rhai".to_string(),
                source: "fn a(ctx) {\n    let ignored = #{ template_path: \"data.toml\" };\n    ctx.effects.spawn_entity(#{ template_path: \"a.toml\" });\n}"
                    .to_string(),
            },
        ];

    let compiled = compile_scripts(&sources);
    assert!(compiled.findings.is_empty(), "{:?}", compiled.findings);
    assert_eq!(
        compiled
            .spawned_templates
            .iter()
            .map(|spawn| (
                spawn.source_path.as_str(),
                spawn.line,
                spawn.template_path.as_str()
            ))
            .collect::<Vec<_>>(),
        vec![
            ("worlds/alpha.rhai", 3, "a.toml"),
            ("worlds/zeta.rhai", 2, "z.toml"),
        ],
        "the resolved unit path is the primary order and data maps are not spawns"
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
