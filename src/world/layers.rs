// Pure world-layer load/unload decision layer (issue #821).
//
// Pure Rust module — no Bevy. `LoadWorld` / `UnloadWorld` effects queue
// `WorldLayerChange`s; the `apply_world_layer_changes` applier in
// `world::server` performs the I/O (TOML read, entity spawn/despawn) and
// resource mutation, while every decision — de-duplication, parse handling,
// name→UUID assignment — lives here as plain functions over plain data.
//
// # A layer contributes ENTITIES, and (until #1045) nothing else
//
// A loaded layer used to merge its own `[[trigger]]` blocks into the live
// `WorldContentRuntime` — origin-tagged with the layer path so `UnloadWorld`
// could take exactly them back out — and that was the ONLY way a layer carried
// scenario logic: scripts compile on the standalone/base-world path only.
// Issue #985 deleted the `[[trigger]]` parser, so a layer TOML has no way to
// author a trigger at all, `evaluate_layer_load` has none to return, and
// `evaluate_layer_unload` had nothing left to compute. Both went.
//
// No shipped layer is affected — `reinforcements.toml`, the one layer any
// shipped world loads, authors none — and the capability comes back through
// script-in-layers (#1045), which will hand the applier `ScriptTrigger`s to
// merge rather than parsed `Trigger`s.
//
// # The supporting-world script ROUTE (present) vs. its EFFECT (deferred, #1045)
//
// This evaluation now runs the layer through the one world-load sequence
// ([`crate::world::load::load`]) under [`LoadPolicy::Merge`], which compiles the
// layer's `[script]` block just as the base-world path compiles the base world's.
// The compiled [`CompiledScripts`] is carried out on
// [`LayerLoadOutcome::Loaded::scripts`] — the ROUTE is present end to end.
//
// The applier (`world::server::apply_world_layer_changes`) still DROPS that set:
// a supporting world's `[script]` does not yet take effect. That remaining gap is
// a ledger-FREEZE POLICY question owned by issue #1045, NOT a plumbing omission.
// A layer loads at runtime, after boot has already `freeze()`d the content
// ledger; deciding whether a layer's compiled-script digest joins the frozen
// content set (and how a save reconciles a layer that arrived mid-session) is the
// policy #1045 settles. Merge under a MemoryReader leaves the frozen digest
// untouched — the `LoadedWorld.ledger` records are dropped here (the applier
// already recorded the TOML text via `load_scenario_toml`), and no shipped layer
// authors a script for the loader to digest — so wiring the route in now moves no
// behaviour; only #1045's policy will make a layered script fire.
// # Purity boundaries
//
// * **TOML loading is I/O and stays in the applier.** `load_scenario_toml`
//   reads the filesystem (native) or the WASM pending-world queue (side
//   effect), so the applier resolves it first and passes the result in as
//   `Option<&str>`; this module only decides what the `None` / parse-failure /
//   success branches mean.
// * **UUID generation is injected.** Named `[[entity]]` blocks receive UUIDs
//   from the caller-supplied `uuid_source` (production passes
//   `entity_loader::assign_uuid`; tests pass a counter).
// * **Entities are never held.** Spawned `Entity` handles, `WorldLayerMap`
//   insertion, comms merge/removal, and despawning stay in the applier; the
//   unload evaluation returns *indices* of live trigger states to drop.
// * **Logging becomes data.** Failure paths push onto `warnings`; the applier
//   logs them.

use crate::world::config::{assign_named_entity_uuids, WorldConfig};
use crate::world::load::{load, LoadError, LoadPolicy, LoadRequest, MemoryReader};
use crate::world::script::load::{CompiledScripts, NoSiblingScripts};

/// Decision produced by [`evaluate_layer_load`].
#[derive(Debug)]
pub struct LayerLoadResult {
    pub outcome: LayerLoadOutcome,
    /// Failure-path messages for the applier to log (`error` level).
    pub warnings: Vec<String>,
}

/// The branch the applier must take for one `WorldLayerChange::Load`.
///
/// `Debug` is hand-rolled (not derived) because [`Loaded::scripts`] holds a
/// [`CompiledScripts`], which carries Rhai `AST`s and is not `Debug`; it prints
/// as a presence marker, exactly as [`crate::world::load::LoadedWorld`] does.
pub enum LayerLoadOutcome {
    /// Path already present in `WorldLayerMap` — de-duplicate, no-op.
    AlreadyLoaded,
    /// TOML not yet available (WASM fetch in flight) — re-queue the change.
    TomlUnavailable,
    /// TOML parse failed — insert an empty `WorldRuntime` entry so the broken
    /// file is not retried. The parse error is in `warnings`.
    ParseFailed,
    /// Layer is loadable; the applier merges/spawns from these decisions.
    Loaded {
        /// Named-entity `name → uuid` registrations for the live runtime's
        /// `name_to_uuid` map (already inserted into `scenario_config`).
        name_to_uuid_inserts: Vec<(String, String)>,
        /// Parsed layer config (with `name_to_uuid` filled in) for the
        /// impure steps: entity spawning and the anchor snapshot.
        scenario_config: Box<WorldConfig>,
        /// Emit `WorldEvent::WorldLoaded` so a base-world `on_world_loaded`
        /// handler can react to this layer arriving (issue #415).
        emit_world_loaded: bool,
        /// The layer's compiled `[script]` set, carried out of the `Merge` load
        /// (`None` for the entire shipped set — no shipped layer authors a
        /// script). The applier DROPS this today: a supporting world's script
        /// taking effect is a ledger-freeze policy question for issue #1045 (see
        /// the module docs). The route is present; only #1045 makes it fire.
        scripts: Option<CompiledScripts>,
    },
}

impl std::fmt::Debug for LayerLoadOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LayerLoadOutcome::AlreadyLoaded => f.write_str("AlreadyLoaded"),
            LayerLoadOutcome::TomlUnavailable => f.write_str("TomlUnavailable"),
            LayerLoadOutcome::ParseFailed => f.write_str("ParseFailed"),
            LayerLoadOutcome::Loaded {
                name_to_uuid_inserts,
                scenario_config,
                emit_world_loaded,
                scripts,
            } => f
                .debug_struct("Loaded")
                .field("name_to_uuid_inserts", name_to_uuid_inserts)
                .field("scenario_config", scenario_config)
                .field("emit_world_loaded", emit_world_loaded)
                .field("scripts", &scripts.as_ref().map(|_| "<compiled>"))
                .finish(),
        }
    }
}

/// Evaluate one `WorldLayerChange::Load` for `path`.
///
/// `already_loaded` is the applier's `WorldLayerMap.contains_key` check (done
/// before TOML resolution so a duplicate load never touches the WASM fetch
/// queue). `toml_str` is `None` when the TOML is not yet available.
pub fn evaluate_layer_load<F>(
    path: &str,
    already_loaded: bool,
    toml_str: Option<&str>,
    uuid_source: F,
) -> LayerLoadResult
where
    F: FnMut() -> String,
{
    if already_loaded {
        return LayerLoadResult {
            outcome: LayerLoadOutcome::AlreadyLoaded,
            warnings: Vec::new(),
        };
    }
    let Some(toml_str) = toml_str else {
        return LayerLoadResult {
            outcome: LayerLoadOutcome::TomlUnavailable,
            warnings: Vec::new(),
        };
    };

    // Route the parse and the script-carry through the one world-load sequence
    // ([`load`]) under [`LoadPolicy::Merge`]. A [`MemoryReader`] seeded with the
    // TOML the applier already read — and already recorded into the content ledger
    // via `load_scenario_toml` — keeps this a PURE decision: no filesystem, no
    // bridge. The returned [`LedgerPlan`](crate::world::load::LedgerPlan) is
    // therefore dropped (the applier owns the one recording), and sibling-`.rhai`
    // resolution is left to #1045, so a [`NoSiblingScripts`] resolver is passed.
    let reader = MemoryReader::new([(path.to_string(), toml_str.to_string())]);
    let request = LoadRequest::new(path, &reader, &NoSiblingScripts, LoadPolicy::Merge);
    let loaded = match load(request) {
        Ok(loaded) => loaded,
        Err(LoadError::ParseFailed { message, .. }) => {
            return LayerLoadResult {
                outcome: LayerLoadOutcome::ParseFailed,
                warnings: vec![format!("failed to parse {path}: {message}")],
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
            };
        }
    };

    let mut scenario_config = loaded.config;
    // The compiled `[script]` set the Merge load carried. Threaded out to the
    // applier on `Loaded`, which drops it — see the module docs (#1045).
    let scripts = loaded.scripts;

    if !scenario_config.scenario_detail_floor.is_empty() {
        return LayerLoadResult {
            outcome: LayerLoadOutcome::ParseFailed,
            warnings: vec![format!(
                "failed to load {path}: scenario_detail_floor is root-world-only; supporting worlds cannot override the selected scenario's crew detail floor"
            )],
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
        outcome: LayerLoadOutcome::Loaded {
            name_to_uuid_inserts,
            scenario_config: Box::new(scenario_config),
            emit_world_loaded: true,
            scripts,
        },
        warnings: Vec::new(),
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

    #[test]
    fn load_registers_named_entities_in_config_and_insert_list() {
        let result =
            evaluate_layer_load("worlds/l1.toml", false, Some(LAYER_TOML), counter_uuids());
        let LayerLoadOutcome::Loaded {
            name_to_uuid_inserts,
            scenario_config,
            ..
        } = result.outcome
        else {
            panic!("expected Loaded, got {:?}", result.outcome);
        };
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
    /// applier to drop.
    #[test]
    fn a_scriptless_layer_carries_no_scripts() {
        let result =
            evaluate_layer_load("worlds/l1.toml", false, Some(LAYER_TOML), counter_uuids());
        let LayerLoadOutcome::Loaded { scripts, .. } = result.outcome else {
            panic!("expected Loaded, got {:?}", result.outcome);
        };
        assert!(
            scripts.is_none(),
            "a layer with no [script] block compiles no scripts"
        );
    }

    /// The supporting-world script ROUTE is present (issue #1215): a layer that
    /// authors an inline `[script]` block has it compiled by the `Merge` load and
    /// carried out on `Loaded { scripts }`. The applier drops it today — a layered
    /// script TAKING EFFECT is a ledger-freeze policy question for #1045 — but the
    /// plumbing that would deliver it is exercised here.
    #[test]
    fn a_layer_authoring_a_script_carries_the_compiled_set_through() {
        const WITH_SCRIPT: &str = r#"
[global]
seed = 1

[script]
setup = "fn on_noop(ctx) { }"
"#;
        let result =
            evaluate_layer_load("worlds/l1.toml", false, Some(WITH_SCRIPT), counter_uuids());
        let LayerLoadOutcome::Loaded { scripts, .. } = result.outcome else {
            panic!("expected Loaded, got {:?}", result.outcome);
        };
        let scripts = scripts.expect("the layer's compiled [script] set is carried through");
        assert!(
            scripts.asts.contains_key("worlds/l1.toml#script.setup"),
            "the inline block lifts to its virtual path in the carried set"
        );
    }

    /// The NEW layer contract (issue #985): a layer's `[[entity]]` blocks merge,
    /// and scenario logic does not exist in a layer at all until script-in-layers
    /// (#1045) lands. A layer that still authors `[[trigger]]` is REFUSED by the
    /// parser rather than loading with its logic silently absent.
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
        let result =
            evaluate_layer_load("worlds/l1.toml", false, Some(WITH_TRIGGER), counter_uuids());
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
        let result = evaluate_layer_load(
            "worlds/support.toml",
            false,
            Some(WITH_FLOOR),
            counter_uuids(),
        );
        assert!(matches!(result.outcome, LayerLoadOutcome::ParseFailed));
        assert_eq!(result.warnings.len(), 1);
        assert!(result.warnings[0].contains("root-world-only"));
        assert!(result.warnings[0].contains("scenario_detail_floor"));
    }

    #[test]
    fn load_emits_world_loaded_on_success() {
        let result =
            evaluate_layer_load("worlds/l1.toml", false, Some(LAYER_TOML), counter_uuids());
        let LayerLoadOutcome::Loaded {
            emit_world_loaded, ..
        } = result.outcome
        else {
            panic!("expected Loaded, got {:?}", result.outcome);
        };
        assert!(emit_world_loaded);
    }

    #[test]
    fn load_is_deduped_when_already_in_layer_map() {
        // Pure half of the App-boot dedup test: same path, second evaluation
        // sees `already_loaded = true` and contributes nothing.
        let result = evaluate_layer_load("worlds/l1.toml", true, Some(LAYER_TOML), counter_uuids());
        assert!(matches!(result.outcome, LayerLoadOutcome::AlreadyLoaded));
        assert!(result.warnings.is_empty());
    }

    #[test]
    fn load_requeues_when_toml_unavailable() {
        let result = evaluate_layer_load("worlds/l1.toml", false, None, counter_uuids());
        assert!(matches!(result.outcome, LayerLoadOutcome::TomlUnavailable));
        assert!(result.warnings.is_empty());
    }

    #[test]
    fn load_parse_failure_warns_and_marks_broken() {
        let result = evaluate_layer_load(
            "worlds/broken.toml",
            false,
            Some("not [ valid"),
            counter_uuids(),
        );
        assert!(matches!(result.outcome, LayerLoadOutcome::ParseFailed));
        assert_eq!(result.warnings.len(), 1);
        assert!(
            result.warnings[0].starts_with("failed to parse worlds/broken.toml:"),
            "warning must name the broken path: {}",
            result.warnings[0]
        );
    }
}
