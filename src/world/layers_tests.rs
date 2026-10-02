use super::*;

/// Minimal loadable layer: one named entity, and nothing else a layer can
/// author. It carried two `[[trigger]]` blocks until issue #985 deleted the
/// parser; the assertions those blocks fed are re-homed onto the new
/// reality below.
const LAYER_TOML: &str = r#"
[global]
seed = 1

[[entity]]
template_path = "assets/entities/ship_harrow_destroyer.toml"
name = "raider_alpha"
"#;

fn counter_uuids() -> impl FnMut() -> String {
    let mut n = 0u32;
    move || {
        n += 1;
        format!("uuid-{n}")
    }
}

/// A [`ScriptResolver`] over an in-memory `path -> source` map, for the
/// sibling-`.rhai` cases below. The production resolver reads the filesystem
/// / config cache; every test here injects this instead.
struct FakeSiblings(Vec<(String, String)>);

impl ScriptResolver for FakeSiblings {
    fn read(&self, path: &str) -> Option<String> {
        self.0
            .iter()
            .find(|(p, _)| p == path)
            .map(|(_, src)| src.clone())
    }
}

/// The default evaluation these tests make: no sibling scripts, counter UUIDs.
fn evaluate(path: &str, already_loaded: bool, toml_str: Option<&str>) -> LayerLoadResult {
    let templates = crate::entities::loader::WasmTemplateLoader;
    let fragments = crate::entities::include_resolve::HostFragmentSource;
    let validation = LayerValidationContext::new(&templates, &fragments, Vec::new());
    evaluate_layer_load(
        path,
        already_loaded,
        toml_str,
        &crate::world::script::load::NoSiblingScripts,
        &validation,
        counter_uuids(),
    )
}

#[test]
fn load_registers_named_entities_in_config_and_insert_list() {
    let result = evaluate("worlds/l1.toml", false, Some(LAYER_TOML));
    let LayerLoadOutcome::Loaded(layer) = result.outcome else {
        panic!("expected Loaded, got {:?}", result.outcome);
    };
    let LoadedLayer {
        name_to_uuid_inserts,
        scenario_config,
        ..
    } = *layer;
    assert_eq!(
        name_to_uuid_inserts,
        vec![("raider_alpha".to_string(), "uuid-1".to_string())]
    );
    assert_eq!(
        scenario_config.name_to_uuid.get("raider_alpha"),
        Some(&"uuid-1".to_string()),
        "the returned config must carry the same registration for spawning"
    );
}

/// A scriptless layer (the entire shipped set) carries no compiled scripts:
/// the `Merge` load's `compile_scripts` short-circuits on the absent `script`
/// key, so nothing is recorded into the content ledger and nothing reaches the
/// applier to merge.
#[test]
fn a_scriptless_layer_carries_no_scripts() {
    let result = evaluate("worlds/l1.toml", false, Some(LAYER_TOML));
    let LayerLoadOutcome::Loaded(layer) = result.outcome else {
        panic!("expected Loaded, got {:?}", result.outcome);
    };
    assert!(
        layer.scripts.is_none(),
        "a layer with no [script] block compiles no scripts"
    );
}

/// The supporting-world script route (#1215 plumbing, #1045 effect): a layer
/// that authors an inline `[script]` block has it compiled by the `Merge` load
/// and carried out on `Loaded { scripts }` for the applier to merge.
#[test]
fn a_layer_authoring_a_script_carries_the_compiled_set_through() {
    const WITH_SCRIPT: &str = r#"
[global]
seed = 1

[script]
setup = "fn on_noop(ctx) { }"
"#;
    let result = evaluate("worlds/l1.toml", false, Some(WITH_SCRIPT));
    let LayerLoadOutcome::Loaded(layer) = result.outcome else {
        panic!("expected Loaded, got {:?}", result.outcome);
    };
    let scripts = layer
        .scripts
        .expect("the layer's compiled [script] set is carried through");
    assert!(
        scripts.asts.contains_key("worlds/l1.toml#script.setup"),
        "the inline block lifts to its virtual path in the carried set"
    );
}

/// The OTHER half of "sibling `.rhai` or inline" (issue #1045): a layer's
/// top-level `script = "…"` resolves through the injected resolver, relative to
/// the layer file's own directory, and its registrations reach the carried set.
#[test]
fn a_layer_authoring_a_sibling_script_resolves_it_through_the_injected_resolver() {
    const WITH_SIBLING: &str = r#"
script = "wave.rhai"

[global]
seed = 1
"#;
    let resolver = FakeSiblings(vec![(
        "worlds/wave.rhai".to_string(),
        "on_world_loaded(\"wave_in\"); fn wave_in(ctx) { }".to_string(),
    )]);
    let templates = crate::entities::loader::WasmTemplateLoader;
    let fragments = crate::entities::include_resolve::HostFragmentSource;
    let validation = LayerValidationContext::new(&templates, &fragments, Vec::new());
    let result = evaluate_layer_load(
        "worlds/l1.toml",
        false,
        Some(WITH_SIBLING),
        &resolver,
        &validation,
        counter_uuids(),
    );
    let LayerLoadOutcome::Loaded(layer) = result.outcome else {
        panic!("expected Loaded, got {:?}", result.outcome);
    };
    let scripts = layer
        .scripts
        .expect("the sibling unit compiles into the carried set");
    assert!(
        scripts.asts.contains_key("worlds/wave.rhai"),
        "the sibling resolves beside the layer file: {:?}",
        scripts.asts.keys().collect::<Vec<_>>()
    );
    assert_eq!(
        scripts.script_triggers.len(),
        1,
        "and its top-level registration built the layer's one trigger"
    );
}

#[test]
fn inline_and_sibling_script_spawns_are_composition_gated_with_unit_provenance() {
    const INLINE: &str = r#"
[global]
seed = 1

[script]
setup = """
fn wave(ctx) {
    ctx.effects.spawn_entity(#{ template_path: "assets/entities/missing_inline.toml" });
}
"""
"#;
    const SIBLING: &str = r#"
script = "wave.rhai"

[global]
seed = 1
"#;
    let sibling_resolver = FakeSiblings(vec![(
            "worlds/wave.rhai".to_string(),
            "fn wave(ctx) {\n    ctx.effects.spawn_entity(#{ template_path: \"assets/entities/missing_sibling.toml\" });\n}"
                .to_string(),
        )]);
    let templates = crate::world::load::MemoryTemplateLoader::authoritative_empty();
    let fragments = std::collections::HashMap::<String, String>::new();
    let validation = LayerValidationContext::new(&templates, &fragments, Vec::new());

    let inline = evaluate_layer_load(
        "worlds/inline.toml",
        false,
        Some(INLINE),
        &crate::world::script::load::NoSiblingScripts,
        &validation,
        counter_uuids(),
    );
    assert!(matches!(inline.outcome, LayerLoadOutcome::ParseFailed));
    assert!(
        inline.warnings[0].contains("worlds/inline.toml#script.setup:2"),
        "the inline virtual unit and exact call line own the finding: {}",
        inline.warnings[0]
    );
    assert!(inline.warnings[0].contains("unresolvable-template"));

    let sibling = evaluate_layer_load(
        "worlds/layer.toml",
        false,
        Some(SIBLING),
        &sibling_resolver,
        &validation,
        counter_uuids(),
    );
    assert!(matches!(sibling.outcome, LayerLoadOutcome::ParseFailed));
    assert!(
        sibling.warnings[0].contains("worlds/wave.rhai:2"),
        "the resolved sibling file and exact call line own the finding: {}",
        sibling.warnings[0]
    );
    assert!(sibling.warnings[0].contains("missing_sibling.toml"));
}

#[test]
fn sibling_spawn_doctrine_may_use_an_active_root_anchor_but_not_an_undeclared_one() {
    const LAYER: &str = r#"
script = "wave.rhai"

[global]
seed = 1
"#;
    let resolver = FakeSiblings(vec![(
            "worlds/wave.rhai".to_string(),
            "fn wave(ctx) {\n    ctx.effects.spawn_entity(#{ template_path: \"assets/entities/ship_harrow_patrol.toml\" });\n}"
                .to_string(),
        )]);
    let templates = crate::entities::loader::WasmTemplateLoader;
    let fragments = crate::entities::include_resolve::HostFragmentSource;

    let undeclared = LayerValidationContext::new(&templates, &fragments, Vec::new());
    let rejected = evaluate_layer_load(
        "worlds/layer.toml",
        false,
        Some(LAYER),
        &resolver,
        &undeclared,
        counter_uuids(),
    );
    assert!(matches!(rejected.outcome, LayerLoadOutcome::ParseFailed));
    assert!(
        rejected.warnings[0].contains("unresolved-anchor"),
        "the template's undeclared patrol anchors must block the layer: {}",
        rejected.warnings[0]
    );

    let root_declared = LayerValidationContext::new(
        &templates,
        &fragments,
        [
            "ironveil_patrol_a".to_string(),
            "ironveil_patrol_b".to_string(),
        ],
    );
    let accepted = evaluate_layer_load(
        "worlds/layer.toml",
        false,
        Some(LAYER),
        &resolver,
        &root_declared,
        counter_uuids(),
    );
    assert!(
        matches!(accepted.outcome, LayerLoadOutcome::Loaded(_)),
        "the active root's anchors are visible to its child layer: {:?}",
        accepted.outcome
    );
}

#[test]
fn composition_refusal_happens_before_named_entity_uuid_minting() {
    use std::cell::Cell;

    const BROKEN: &str = r#"
[global]
seed = 1

[[entity]]
template_path = "assets/entities/ship_harrow_destroyer.toml"
name = "must_not_receive_a_uuid"

[script]
setup = """
fn wave(ctx) {
    ctx.effects.spawn_entity(#{ template_path: "assets/entities/missing.toml" });
}
"""
"#;
    let templates = crate::world::load::MemoryTemplateLoader::authoritative_empty();
    let fragments = std::collections::HashMap::<String, String>::new();
    let validation = LayerValidationContext::new(&templates, &fragments, Vec::new());
    let minted = Cell::new(0usize);

    let result = evaluate_layer_load(
        "worlds/broken.toml",
        false,
        Some(BROKEN),
        &crate::world::script::load::NoSiblingScripts,
        &validation,
        || {
            minted.set(minted.get() + 1);
            format!("uuid-{}", minted.get())
        },
    );

    assert!(matches!(result.outcome, LayerLoadOutcome::ParseFailed));
    assert_eq!(
        minted.get(),
        0,
        "composition rejection must precede every named-entity UUID mint"
    );
}

/// A layer whose `[script]` names a handler nothing defines is REFUSED whole
/// (issue #1045) — entities included — rather than merged with its logic
/// missing. The same all-or-nothing the boot gate applies to a base world.
#[test]
fn a_layer_whose_script_does_not_compile_is_refused() {
    const BROKEN_SCRIPT: &str = r#"
[global]
seed = 1

[[entity]]
template_path = "assets/entities/ship_harrow_destroyer.toml"
name = "raider_alpha"

[script]
setup = "on_world_loaded(\"nope\"); fn other(ctx) { }"
"#;
    let result = evaluate("worlds/l1.toml", false, Some(BROKEN_SCRIPT));
    assert!(
        matches!(result.outcome, LayerLoadOutcome::ParseFailed),
        "got {:?}",
        result.outcome
    );
    assert_eq!(result.warnings.len(), 1);
    assert!(
        result.warnings[0].contains("script error"),
        "the refusal must say the scripts are why: {}",
        result.warnings[0]
    );
    assert!(
        result.warnings[0].contains("unresolved-script-fn"),
        "and carry the finding category: {}",
        result.warnings[0]
    );
}

/// A layer whose `script = "…"` sibling cannot be read is refused the same
/// way: `script-file-missing` is an error finding, so the layer does not load
/// with its logic quietly absent.
#[test]
fn a_layer_whose_sibling_script_is_unreadable_is_refused() {
    const WITH_SIBLING: &str = r#"
script = "wave.rhai"

[global]
seed = 1
"#;
    let result = evaluate("worlds/l1.toml", false, Some(WITH_SIBLING));
    assert!(
        matches!(result.outcome, LayerLoadOutcome::ParseFailed),
        "got {:?}",
        result.outcome
    );
    assert!(
        result.warnings[0].contains("script-file-missing"),
        "{}",
        result.warnings[0]
    );
}

/// The layer contract since issue #985: a layer's `[[entity]]` blocks merge and
/// its `[script]` block carries scenario logic (#1045). A layer that still
/// authors the retired `[[trigger]]` is REFUSED by the parser rather than
/// loading with its logic silently absent.
#[test]
fn a_layer_that_still_authors_a_trigger_block_is_refused() {
    const WITH_TRIGGER: &str = r#"
[global]
seed = 1

[[trigger]]
condition = "on_world_loaded"

  [[trigger.action]]
  type = "set_flag"
  name = "layer_armed"
"#;
    let result = evaluate("worlds/l1.toml", false, Some(WITH_TRIGGER));
    assert!(matches!(result.outcome, LayerLoadOutcome::ParseFailed));
    assert_eq!(result.warnings.len(), 1);
    assert!(
        result.warnings[0].contains("[[trigger]]"),
        "the refusal must name the retired block: {}",
        result.warnings[0]
    );
}

#[test]
fn a_supporting_world_cannot_author_a_scenario_detail_floor() {
    const WITH_FLOOR: &str = r#"
scenario_detail_floor = ["navigation"]
[global]
seed = 1
"#;
    let result = evaluate("worlds/support.toml", false, Some(WITH_FLOOR));
    assert!(matches!(result.outcome, LayerLoadOutcome::ParseFailed));
    assert_eq!(result.warnings.len(), 1);
    assert!(result.warnings[0].contains("root-world-only"));
    assert!(result.warnings[0].contains("scenario_detail_floor"));
}

#[test]
fn load_emits_world_loaded_on_success() {
    let result = evaluate("worlds/l1.toml", false, Some(LAYER_TOML));
    let LayerLoadOutcome::Loaded(layer) = result.outcome else {
        panic!("expected Loaded, got {:?}", result.outcome);
    };
    assert!(layer.emit_world_loaded);
}

#[test]
fn load_is_deduped_when_already_in_layer_map() {
    // Pure half of the App-boot dedup test: same path, second evaluation
    // sees `already_loaded = true` and contributes nothing.
    let result = evaluate("worlds/l1.toml", true, Some(LAYER_TOML));
    assert!(matches!(result.outcome, LayerLoadOutcome::AlreadyLoaded));
    assert!(result.warnings.is_empty());
}

#[test]
fn load_requeues_when_toml_unavailable() {
    let result = evaluate("worlds/l1.toml", false, None);
    assert!(matches!(result.outcome, LayerLoadOutcome::TomlUnavailable));
    assert!(result.warnings.is_empty());
}

#[test]
fn load_parse_failure_warns_and_marks_broken() {
    let result = evaluate("worlds/broken.toml", false, Some("not [ valid"));
    assert!(matches!(result.outcome, LayerLoadOutcome::ParseFailed));
    assert_eq!(result.warnings.len(), 1);
    assert!(
        result.warnings[0].starts_with("failed to parse worlds/broken.toml:"),
        "warning must name the broken path: {}",
        result.warnings[0]
    );
}
