//! Pure world-layer load/unload decision layer (issue #821).
//!
//! Pure Rust module — no Bevy. `LoadWorld` / `UnloadWorld` effects queue
//! `WorldLayerChange`s; the `apply_world_layer_changes` applier in
//! `world::server` performs the I/O (TOML read, entity spawn/despawn) and
//! resource mutation, while every decision — de-duplication, parse handling,
//! name→UUID assignment — lives here as plain functions over plain data.
//!
//! # A layer contributes ENTITIES and SCRIPTS, and nothing else
//!
//! A loaded layer used to merge its own `[[trigger]]` blocks into the live
//! `WorldContentRuntime` — origin-tagged with the layer path so `UnloadWorld`
//! could take exactly them back out — and that was the ONLY way a layer carried
//! scenario logic: scripts compiled on the standalone/base-world path only.
//! Issue #985 deleted the `[[trigger]]` parser, so a layer TOML has no way to
//! author a trigger at all, `evaluate_layer_load` has none to return, and
//! `evaluate_layer_unload` had nothing left to compute. Both went.
//!
//! Issue #1045 gives the capability back the way the deletion note said it would:
//! through the layer's `[script]` block, which compiles here and hands the applier
//! `ScriptTrigger`s to merge rather than parsed `Trigger`s. No shipped layer is
//! affected — `reinforcements.toml`, the one layer any shipped world loads,
//! authors neither a trigger nor a script.
//!
//! # The supporting-world script route, end to end
//!
//! This evaluation runs the layer through the one world-load sequence
//! ([`crate::world::load::load`]) under [`LoadPolicy::Merge`], which compiles the
//! layer's `[script]` block just as the base-world path compiles the base world's.
//! The compiled [`CompiledScripts`] is carried out on
//! [`LoadedLayer::scripts`], and the applier
//! (`world::server::apply_world_layer_changes`) merges it into the live
//! `WorldScriptRuntime` — creating that resource when the base world authored no
//! script of its own. A layer whose scripts carry an ERROR finding is refused
//! outright ([`LayerLoadOutcome::ParseFailed`]) rather than half-merged: the
//! boot-time `SCRIPT_ACTIVATION_BLOCKED` gate belongs to base-world activation and
//! must not be tripped by a layer arriving mid-run.
//!
//! ## What the merge does NOT touch
//!
//! The `LoadedWorld.ledger`'s TOML records are dropped here — the applier already
//! recorded the layer's text via `load_scenario_toml`. Its script DIGEST is not:
//! since issue #1241 that write comes back as data instead of being made from
//! inside `load_world_scripts`, so it rides out on
//! [`LayerLoadResult::ledger`] for the applier to apply at the same moment the
//! eager write used to happen. Whether a layer that arrives *after* `freeze()`
//! should move the frozen content set at all is a ledger-policy question owned by
//! issue #1047, not something this route decides — and it cannot, because the
//! frozen snapshot is what the digest folds. No shipped layer authors a script, so
//! the frozen digest of every shipped world is untouched either way.
//!
//! # Purity boundaries
//!
//! * **TOML loading is I/O and stays in the applier.** `load_scenario_toml`
//!   reads the filesystem (native) or the WASM pending-world queue (side
//!   effect), so the applier resolves it first and passes the result in as
//!   `Option<&str>`; this module only decides what the `None` / parse-failure /
//!   success branches mean.
//! * **UUID generation is injected.** Named `[[entity]]` blocks receive UUIDs
//!   from the caller-supplied `uuid_source` (production passes
//!   `entity_loader::assign_uuid`; tests pass a counter).
//! * **Sibling-script reading is injected too.** A layer's top-level
//!   `script = "wave.rhai"` resolves through the caller-supplied
//!   [`ScriptResolver`] (production passes
//!   [`crate::entities::config_cache::production_script_resolver`]; tests pass a
//!   fake or [`NoSiblingScripts`](crate::world::script::load::NoSiblingScripts)),
//!   so this module still touches neither filesystem nor bridge.
//! * **Entities are never held.** Spawned `Entity` handles, `WorldLayerMap`
//!   insertion, comms merge/removal, and despawning stay in the applier.
//! * **Logging becomes data.** Failure paths push onto `warnings`; the applier
//!   logs them.

use crate::world::config::{assign_named_entity_uuids, WorldConfig};
use crate::world::load::{load, LedgerPlan, LoadError, LoadPolicy, LoadRequest, MemoryReader};
use crate::world::script::load::{CompiledScripts, ScriptResolver};

/// The already-active composition facts needed to validate one additive layer
/// before it can mutate the runtime (issue #1046).
///
/// The two sources are injected to preserve the browser's pending-cache
/// authority rule (`absence_is_final = false` while content may still arrive).
/// Anchor names are owned, sorted, and deduplicated because the Bevy caller
/// rebuilds this context for every load from the root plus all currently-active
/// layers; a layer applied earlier in the same drain can therefore satisfy a
/// later layer deterministically.
pub struct LayerValidationContext<'a> {
    pub template_loader: &'a dyn crate::entities::loader::TemplateLoader,
    pub fragment_source: &'a dyn crate::entities::include_resolve::FragmentSource,
    pub declared_anchors: Vec<String>,
}

impl<'a> LayerValidationContext<'a> {
    pub fn new(
        template_loader: &'a dyn crate::entities::loader::TemplateLoader,
        fragment_source: &'a dyn crate::entities::include_resolve::FragmentSource,
        declared_anchors: impl IntoIterator<Item = String>,
    ) -> Self {
        let mut declared_anchors: Vec<String> = declared_anchors.into_iter().collect();
        declared_anchors.sort();
        declared_anchors.dedup();
        Self {
            template_loader,
            fragment_source,
            declared_anchors,
        }
    }
}

/// Decision produced by [`evaluate_layer_load`].
#[derive(Debug)]
pub struct LayerLoadResult {
    pub outcome: LayerLoadOutcome,
    /// Failure-path messages for the applier to log (`error` level).
    pub warnings: Vec<String>,
    /// Content-ledger writes this evaluation gathered, for the applier to apply
    /// (issue #1241) — the compiled script set's digest, and nothing else.
    ///
    /// The load's TOML `records` are deliberately NOT carried: the applier read
    /// that text itself through `load_scenario_toml`, which recorded it, so
    /// passing them on would be a second write of a record it already owns. What
    /// it could not own is the digest, because only the compile knows it.
    ///
    /// Carried on EVERY outcome, not just [`LayerLoadOutcome::Loaded`]. A layer
    /// refused for a script error still compiled its sources, and the eager write
    /// this replaced happened at compile time regardless of what the evaluation
    /// then decided — so a refused layer recorded its digest before, and records
    /// it now.
    pub ledger: crate::world::load::LedgerPlan,
}

/// The branch the applier must take for one `WorldLayerChange::Load`.
#[derive(Debug)]
pub enum LayerLoadOutcome {
    /// Path already present in `WorldLayerMap` — de-duplicate, no-op.
    AlreadyLoaded,
    /// TOML not yet available (WASM fetch in flight) — re-queue the change.
    TomlUnavailable,
    /// The layer is REFUSED — insert an empty `WorldRuntime` entry so the broken
    /// file is not retried. The reason is in `warnings`.
    ///
    /// Named for its first cause and since widened to every refusal this
    /// evaluation can reach, because the applier's response is the same for all
    /// of them: a TOML that will not parse, a root-world-only key a supporting
    /// world may not author (`scenario_detail_floor`), and — since issue #1045 —
    /// a `[script]` block that compiles with an error finding.
    ParseFailed,
    /// Layer is loadable; the applier merges/spawns from this decision.
    ///
    /// Boxed because it is also what
    /// [`WorldLayerChange::DeferredApply`](crate::world::server::WorldLayerChange::DeferredApply)
    /// carries across a tick (issue #1045): a layer whose scripts need a
    /// `WorldScriptRuntime` that does not exist yet is evaluated ONCE and its
    /// decision stashed, never re-evaluated. Re-evaluating would be wrong on
    /// wasm, where reading the TOML CONSUMES it from the pending-fetch queue and
    /// the fetch guard refuses to ask for it twice — a second evaluation would
    /// find nothing and re-queue forever.
    Loaded(Box<LoadedLayer>),
}

/// Everything the applier needs to bring one evaluated layer into the world.
///
/// `Debug` is hand-rolled (not derived) because [`scripts`](Self::scripts) holds
/// a [`CompiledScripts`], which carries Rhai `AST`s and is not `Debug`; it prints
/// as a presence marker, exactly as [`crate::world::load::LoadedWorld`] does.
pub struct LoadedLayer {
    /// Named-entity `name → uuid` registrations for the live runtime's
    /// `name_to_uuid` map (already inserted into `scenario_config`).
    pub name_to_uuid_inserts: Vec<(String, String)>,
    /// Parsed layer config (with `name_to_uuid` filled in) for the impure steps:
    /// entity spawning and the anchor snapshot.
    pub scenario_config: WorldConfig,
    /// Emit `WorldEvent::WorldLoaded` so a base-world `on_world_loaded` handler
    /// can react to this layer arriving (issue #415).
    pub emit_world_loaded: bool,
    /// The layer's compiled `[script]` set, carried out of the `Merge` load
    /// (`None` for the entire shipped set — no shipped layer authors a script).
    /// The applier merges it into the live `WorldScriptRuntime` (issue #1045),
    /// creating that resource if the base world authored no script of its own.
    /// Guaranteed free of ERROR findings: a layer whose scripts do not compile
    /// never reaches this variant.
    pub scripts: Option<CompiledScripts>,
}

impl std::fmt::Debug for LoadedLayer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoadedLayer")
            .field("name_to_uuid_inserts", &self.name_to_uuid_inserts)
            .field("scenario_config", &self.scenario_config)
            .field("emit_world_loaded", &self.emit_world_loaded)
            .field("scripts", &self.scripts.as_ref().map(|_| "<compiled>"))
            .finish()
    }
}

/// Evaluate one `WorldLayerChange::Load` for `path`.
///
/// `already_loaded` is the applier's `WorldLayerMap.contains_key` check (done
/// before TOML resolution so a duplicate load never touches the WASM fetch
/// queue). `toml_str` is `None` when the TOML is not yet available.
/// `script_resolver` reads a top-level `script = "…"` sibling file for the layer
/// (issue #1045) — the same injected seam the boot path uses, so this function
/// stays free of filesystem and bridge access.
pub fn evaluate_layer_load<F>(
    path: &str,
    already_loaded: bool,
    toml_str: Option<&str>,
    script_resolver: &dyn ScriptResolver,
    validation: &LayerValidationContext,
    uuid_source: F,
) -> LayerLoadResult
where
    F: FnMut() -> String,
{
    if already_loaded {
        return LayerLoadResult {
            outcome: LayerLoadOutcome::AlreadyLoaded,
            warnings: Vec::new(),
            ledger: LedgerPlan::default(),
        };
    }
    let Some(toml_str) = toml_str else {
        return LayerLoadResult {
            outcome: LayerLoadOutcome::TomlUnavailable,
            warnings: Vec::new(),
            ledger: LedgerPlan::default(),
        };
    };

    // Route the parse and the script compile through the one world-load sequence
    // ([`load`]) under [`LoadPolicy::Merge`]. A [`MemoryReader`] seeded with the
    // TOML the applier already read — and already recorded into the content ledger
    // via `load_scenario_toml` — keeps this a PURE decision: no filesystem, no
    // bridge. The returned [`LedgerPlan`](crate::world::load::LedgerPlan) is
    // therefore dropped (the applier owns the one recording); a sibling `.rhai`
    // resolves through the INJECTED `script_resolver` for the same reason (issue
    // #1045), so the impurity stays the caller's.
    let reader = MemoryReader::new([(path.to_string(), toml_str.to_string())]);
    let request = LoadRequest::new(path, &reader, script_resolver, LoadPolicy::Merge);
    let loaded = match load(request) {
        Ok(loaded) => loaded,
        Err(LoadError::ParseFailed { message, .. }) => {
            return LayerLoadResult {
                outcome: LayerLoadOutcome::ParseFailed,
                warnings: vec![format!("failed to parse {path}: {message}")],
                ledger: LedgerPlan::default(),
            };
        }
        // Unreachable with a MemoryReader under Merge — the read cannot fail (the
        // text is in hand), the raw re-parse cannot fail once `parse_world`
        // succeeded, and there is no transform or child recursion — but map any
        // load failure to the same broken-file outcome rather than panic.
        Err(other) => {
            return LayerLoadResult {
                outcome: LayerLoadOutcome::ParseFailed,
                warnings: vec![format!("failed to load {path}: {other}")],
                ledger: LedgerPlan::default(),
            };
        }
    };

    let mut scenario_config = loaded.config;
    // The compiled `[script]` set the Merge load carried. Threaded out to the
    // applier on `Loaded`, which merges it into the live runtime (#1045).
    let scripts = loaded.scripts;
    // And the ledger writes it gathered (issue #1241). Only the digests: the
    // applier already recorded this layer's TOML itself — see the field docs.
    let ledger = LedgerPlan {
        records: Vec::new(),
        digests: loaded.ledger.digests,
    };

    // A layer whose scripts carry an ERROR finding is REFUSED whole, entities and
    // all, rather than merged with its logic missing — the same all-or-nothing the
    // boot path applies to a base world, expressed the one way a mid-run load can.
    //
    // It deliberately does NOT trip `SCRIPT_ACTIVATION_BLOCKED`: that atomic gate
    // stops the base world's Startup SPAWN pass, which is long finished by the time
    // a layer arrives, so setting it here would be a global refusal for one broken
    // supporting file. Refusing this layer (and marking it broken so it is not
    // retried) is the whole of the blast radius.
    if let Some(compiled) = &scripts {
        if crate::world::validate::has_error(&compiled.findings) {
            let detail = compiled
                .findings
                .iter()
                .filter(|f| f.is_error())
                .map(|f| format!("[{}] {}", f.category, f.message))
                .collect::<Vec<_>>()
                .join("; ");
            // A missing sibling is the one refusal a correctly-authored layer can
            // still hit, and only in the browser: nothing prefetches a layer's
            // `script = "wave.rhai"` into the wasm config cache, so the resolver
            // reads `None` and the layer is refused whole. Name that explicitly
            // rather than leave a designer reading "file could not be read" about
            // a file that is plainly there in the repo.
            let hint = if compiled
                .findings
                .iter()
                .any(|f| f.is_error() && f.category == "script-file-missing")
            {
                " (a supporting world's sibling .rhai is not prefetched in the \
                 browser — author the layer's script as an inline [script] table)"
            } else {
                ""
            };
            return LayerLoadResult {
                outcome: LayerLoadOutcome::ParseFailed,
                warnings: vec![format!(
                    "failed to load {path}: script error: {detail}{hint}"
                )],
                ledger,
            };
        }
    }

    if !scenario_config.scenario_detail_floor.is_empty() {
        return LayerLoadResult {
            outcome: LayerLoadOutcome::ParseFailed,
            warnings: vec![format!(
                "failed to load {path}: scenario_detail_floor is root-world-only; supporting worlds cannot override the selected scenario's crew detail floor"
            )],
            ledger,
        };
    }

    // Composition-gate the layer before UUID minting or any value can reach the
    // applier. The resolved compiled spawn set is authoritative when present:
    // it includes sibling `.rhai` units and replaces the config's inline-only
    // scan, avoiding duplicate inline findings.
    let composition_findings = {
        let mut source = crate::world::validate::WorldSource::new(path, toml_str, &scenario_config);
        if let Some(compiled) = scripts.as_ref() {
            source = source.with_resolved_script_spawns(&compiled.spawned_templates);
        }
        crate::world::validate::validate_supporting_world(
            &source,
            validation.template_loader,
            validation.fragment_source,
            &validation.declared_anchors,
        )
    };
    if crate::world::validate::has_error(&composition_findings) {
        let detail = composition_findings
            .iter()
            .filter(|finding| finding.is_error())
            .map(|finding| {
                let line = finding
                    .source
                    .line
                    .map(|line| format!(":{line}"))
                    .unwrap_or_default();
                format!(
                    "[{}] {}{}: {}",
                    finding.category, finding.source.file, line, finding.message
                )
            })
            .collect::<Vec<_>>()
            .join("; ");
        return LayerLoadResult {
            outcome: LayerLoadOutcome::ParseFailed,
            warnings: vec![format!(
                "failed to load {path}: composition error: {detail}"
            )],
            ledger,
        };
    }

    // Assign UUIDs to named entities in this layer's config; the registrations go
    // both into the returned config (for spawning) and to the applier (for the
    // live runtime map).
    let new_names = assign_named_entity_uuids(&scenario_config.entities, uuid_source);
    let mut name_to_uuid_inserts: Vec<(String, String)> = new_names.into_iter().collect();
    name_to_uuid_inserts.sort();
    for (name, uuid) in &name_to_uuid_inserts {
        scenario_config
            .name_to_uuid
            .insert(name.clone(), uuid.clone());
    }

    LayerLoadResult {
        outcome: LayerLoadOutcome::Loaded(Box::new(LoadedLayer {
            name_to_uuid_inserts,
            scenario_config,
            emit_world_loaded: true,
            scripts,
        })),
        warnings: Vec::new(),
        ledger,
    }
}

#[cfg(test)]
mod tests {
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
}
