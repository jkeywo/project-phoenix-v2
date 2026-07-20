// Pure world-layer load/unload decision layer (issue #821).
//
// Pure Rust module — no Bevy. `LoadWorld` / `UnloadWorld` trigger actions queue
// `WorldLayerChange`s; the `apply_world_layer_changes` applier in
// `world::server` performs the I/O (TOML read, entity spawn/despawn, comms
// merge) and resource mutation, while every decision — de-duplication, parse
// handling, origin tagging, name→UUID assignment, the trigger-removal set —
// lives here as plain functions over plain data.
//
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

use std::collections::HashSet;

use crate::world::config::{assign_named_entity_uuids, parse_world, WorldConfig};
use crate::world::content::{trigger_states_from_world, TriggerState};

/// Parse a world TOML string and derive its trigger states.
///
/// Shared core of the layer-load and scenario-load paths (both do
/// `parse_world` + `trigger_states_from_world` before merging into the live
/// runtime). Returns the parse error message on failure.
pub fn parse_world_triggers(toml_str: &str) -> Result<(WorldConfig, Vec<TriggerState>), String> {
    let config = parse_world(toml_str)?;
    let trigger_states = trigger_states_from_world(&config);
    Ok((config, trigger_states))
}

/// Decision produced by [`evaluate_layer_load`].
#[derive(Debug)]
pub struct LayerLoadResult {
    pub outcome: LayerLoadOutcome,
    /// Failure-path messages for the applier to log (`error` level).
    pub warnings: Vec<String>,
}

/// The branch the applier must take for one `WorldLayerChange::Load`.
#[derive(Debug)]
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
        /// This layer's trigger states, origin-tagged with the layer path so
        /// `spawn_entity` actions attach new entities to the right
        /// `WorldLayerMap` entry (issue #417). The applier extends the live
        /// runtime with a clone and snapshots them into the `WorldRuntime`.
        trigger_states: Vec<TriggerState>,
        /// Named-entity `name → uuid` registrations for the live runtime's
        /// `name_to_uuid` map (already inserted into `scenario_config`).
        name_to_uuid_inserts: Vec<(String, String)>,
        /// Parsed layer config (with `name_to_uuid` filled in) for the
        /// impure steps: entity spawning, comms merge, anchor snapshot.
        scenario_config: Box<WorldConfig>,
        /// Emit `WorldEvent::WorldLoaded` so `on_world_loaded` triggers
        /// declared inside this sub-world fire on the next tick (issue #415).
        emit_world_loaded: bool,
    },
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
    match parse_world_triggers(toml_str) {
        Err(e) => LayerLoadResult {
            outcome: LayerLoadOutcome::ParseFailed,
            warnings: vec![format!("failed to parse {path}: {e}")],
        },
        Ok((mut scenario_config, mut trigger_states)) => {
            // Tag every trigger state from this layer with its origin path.
            for ts in trigger_states.iter_mut() {
                ts.origin_layer = Some(path.to_string());
            }

            // Assign UUIDs to named entities in this layer's config; the
            // registrations go both into the returned config (for spawning /
            // comms) and to the applier (for the live runtime map).
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
                    trigger_states,
                    name_to_uuid_inserts,
                    scenario_config: Box::new(scenario_config),
                    emit_world_loaded: true,
                },
                warnings: Vec::new(),
            }
        }
    }
}

/// Decision produced by [`evaluate_layer_unload`].
#[derive(Debug)]
pub struct LayerUnloadResult {
    /// Indices into the live runtime's `trigger_states` vec to remove (the
    /// states this layer contributed, matched by trigger equality). The
    /// applier retains everything else.
    pub triggers_to_remove: HashSet<usize>,
    pub warnings: Vec<String>,
}

/// Evaluate one `WorldLayerChange::Unload`: compute which live trigger states
/// belong to the unloaded layer's snapshot.
///
/// Matching is by `Trigger` equality against the snapshot taken at load time —
/// the same first-match `position` scan the inline code used, so duplicate
/// triggers collapse to a single index exactly as before.
pub fn evaluate_layer_unload(
    layer_trigger_states: &[TriggerState],
    runtime_trigger_states: &[TriggerState],
) -> LayerUnloadResult {
    let triggers_to_remove: HashSet<usize> = layer_trigger_states
        .iter()
        .filter_map(|ls| {
            runtime_trigger_states
                .iter()
                .position(|rs| rs.trigger == ls.trigger)
        })
        .collect();
    LayerUnloadResult {
        triggers_to_remove,
        warnings: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal loadable world: one named entity, two triggers.
    const LAYER_TOML: &str = r#"
[global]
seed = 1

[[entity]]
template_path = "assets/entities/pirate_raider.toml"
name = "raider_alpha"

[[trigger]]
condition = "on_world_loaded"

  [[trigger.action]]
  type = "set_flag"
  name = "layer_armed"

[[trigger]]
condition = "on_destroyed"
entity = "raider_alpha"

  [[trigger.action]]
  type = "set_flag"
  name = "raider_down"
"#;

    fn counter_uuids() -> impl FnMut() -> String {
        let mut n = 0u32;
        move || {
            n += 1;
            format!("uuid-{n}")
        }
    }

    #[test]
    fn load_merges_triggers_and_tags_origin_layer() {
        let result =
            evaluate_layer_load("worlds/l1.toml", false, Some(LAYER_TOML), counter_uuids());
        assert!(result.warnings.is_empty());
        let LayerLoadOutcome::Loaded { trigger_states, .. } = result.outcome else {
            panic!("expected Loaded, got {:?}", result.outcome);
        };
        assert_eq!(trigger_states.len(), 2);
        assert!(
            trigger_states
                .iter()
                .all(|ts| ts.origin_layer.as_deref() == Some("worlds/l1.toml")),
            "every trigger state must be origin-tagged with the layer path"
        );
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
            "the returned config must carry the same registration for spawn/comms"
        );
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

    #[test]
    fn unload_removes_exactly_the_layer_triggers() {
        // Pure half of the App-boot unload test: base runtime holds a base
        // trigger plus the layer's two; the removal set names only the
        // layer's indices.
        let base =
            evaluate_layer_load("worlds/base.toml", false, Some(LAYER_TOML), counter_uuids());
        let LayerLoadOutcome::Loaded {
            trigger_states: layer_states,
            ..
        } = base.outcome
        else {
            panic!("fixture must load");
        };

        // Live runtime: an unrelated trigger at index 0, then the layer's two.
        let other_toml = r#"
[global]
seed = 2

[[trigger]]
condition = "on_timer"
after_secs = 30.0

  [[trigger.action]]
  type = "set_flag"
  name = "base_flag"
"#;
        let (_, base_states) = parse_world_triggers(other_toml).expect("base fixture parses");
        let mut runtime_states = base_states;
        runtime_states.extend(layer_states.clone());

        let result = evaluate_layer_unload(&layer_states, &runtime_states);
        assert_eq!(
            result.triggers_to_remove,
            HashSet::from([1usize, 2usize]),
            "only the layer's trigger indices are removed; the base trigger survives"
        );
    }

    #[test]
    fn unload_of_unknown_triggers_removes_nothing() {
        let loaded =
            evaluate_layer_load("worlds/l1.toml", false, Some(LAYER_TOML), counter_uuids());
        let LayerLoadOutcome::Loaded { trigger_states, .. } = loaded.outcome else {
            panic!("fixture must load");
        };
        // Runtime no longer holds this layer's triggers (e.g. already cleared).
        let result = evaluate_layer_unload(&trigger_states, &[]);
        assert!(result.triggers_to_remove.is_empty());
    }
}
