// Unified world parser — single-pass deserialization for the merged
// map/scenario world TOML (PRD #337, slice 1).
//
// This module introduces a NEW `WorldConfig` type and `parse_world` function
// that deserialize the entire world TOML in one parse pass into a `RawWorld`
// and project it into typed fields.
//
// During the merger transition (slices 1–4 of PRD #337) this type coexists
// with the older `crate::world::content::WorldConfig` shim. Existing callers
// keep using the old shim; new code routed through `wasm_load_world` reads
// from this `WorldConfig`. After the merger completes, this becomes the only
// `WorldConfig`.
//
// Slice 1 scope: anchors + `[[entity]]` blocks. Other sections (`[[spawn]]`,
// `[[trigger]]`, `[[comms]]`) are intentionally ignored here — they continue
// to flow through `parse_scenario` until later slices fold them in.

use serde::Deserialize;
use std::collections::HashMap;

use crate::map_config::{EntityInstance, GlobalConfig};

// ── TOML-facing raw types ──────────────────────────────────────────────────

/// Raw single-pass deserialization of a world TOML.
///
/// All fields are `serde(default)` so any partial subset of the schema
/// parses cleanly (the legacy `[[spawn]]`, `[[trigger]]`, `[[comms]]`
/// blocks are silently ignored at this layer).
#[derive(Debug, Default, Deserialize)]
pub struct RawWorld {
    #[serde(default)]
    pub global: GlobalConfig,
    #[serde(default)]
    pub anchors: HashMap<String, Vec<f32>>,
    #[serde(default, rename = "entity")]
    pub entities: Vec<EntityInstance>,
}

// ── Public typed config ────────────────────────────────────────────────────

/// Parsed unified world configuration.
///
/// Carries the anchor table and the `[[entity]]` instances. Anchors are
/// normalised to fixed-size `[f32; 3]` arrays at parse time so downstream
/// consumers (e.g. AI patrol path lookups, region positioning) don't have
/// to re-validate length on every read.
#[derive(Clone, Debug, Default, bevy::prelude::Resource)]
pub struct WorldConfig {
    pub global: GlobalConfig,
    pub anchors: HashMap<String, [f32; 3]>,
    pub entities: Vec<EntityInstance>,
}

impl WorldConfig {
    /// Borrow the anchor table.
    ///
    /// Returned values are normalised `[x, y, z]` arrays; 2-element anchors
    /// from the source TOML are widened to 3 elements by inserting `0.0` at
    /// the Y component (mirrors the historical `ai/server.rs` behaviour).
    pub fn anchors(&self) -> &HashMap<String, [f32; 3]> {
        &self.anchors
    }

    /// Borrow the unified `[[entity]]` instance list.
    pub fn entities(&self) -> &[EntityInstance] {
        &self.entities
    }
}

// ── Parser ─────────────────────────────────────────────────────────────────

/// Parse a unified world TOML string in a single pass.
///
/// Validates that every anchor position has 2 or 3 components and normalises
/// to `[x, y, z]`. Returns an `Err` with a human-readable message on TOML
/// parse errors or invalid anchor shapes.
pub fn parse_world(toml_str: &str) -> Result<WorldConfig, String> {
    let raw: RawWorld = toml::from_str(toml_str).map_err(|e| e.to_string())?;

    let mut anchors: HashMap<String, [f32; 3]> = HashMap::with_capacity(raw.anchors.len());
    for (name, pos) in raw.anchors {
        let normalised = match pos.len() {
            3 => [pos[0], pos[1], pos[2]],
            2 => [pos[0], 0.0, pos[1]],
            other => {
                return Err(format!(
                    "Anchor '{name}' has invalid position array length: {other} (expected 2 or 3)"
                ));
            }
        };
        anchors.insert(name, normalised);
    }

    Ok(WorldConfig {
        global: raw.global,
        anchors,
        entities: raw.entities,
    })
}

/// Collect the deduplicated entity template paths referenced by a `WorldConfig`.
///
/// Used by `wasm_load_world` to queue entity TOML fetches via the JS preload
/// callback (PRD #338). Returned in stable iteration order so the queue
/// sequence is deterministic across runs.
pub fn entity_template_paths(world: &WorldConfig) -> Vec<String> {
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut out: Vec<String> = Vec::new();
    for ent in &world.entities {
        if seen.insert(ent.template_path.clone()) {
            out.push(ent.template_path.clone());
        }
    }
    out
}

/// Partition immediate-spawn entity instances into (asteroid_field, other).
///
/// The classifier closure inspects the resolved template (typically by looking
/// it up in the config cache and checking `EntityConfig.asteroid_field`) and
/// returns `true` for asteroid-field templates.
///
/// During PRD #337/#338 slice 1 the asteroid-field instances flow through the
/// new `spawn_world_entities` Bevy system, while every other immediate-spawn
/// instance continues to flow through the legacy `setup_world_from_config`
/// path. Keeping the partitioning logic pure means both call sites consult
/// the same source of truth and double-spawn is impossible.
///
/// Only `EntityInstanceSpawnOn::Immediate` entries are considered; `GameStart`
/// entries are returned in neither bucket (they're handled by
/// `spawn_game_start_entities`).
pub fn partition_immediate_entities<F>(
    world: &WorldConfig,
    is_asteroid_field: F,
) -> (Vec<&crate::map_config::EntityInstance>, Vec<&crate::map_config::EntityInstance>)
where
    F: Fn(&str) -> bool,
{
    use crate::map_config::EntityInstanceSpawnOn;
    let mut fields = Vec::new();
    let mut others = Vec::new();
    for ent in &world.entities {
        if ent.spawn_on != EntityInstanceSpawnOn::Immediate {
            continue;
        }
        if is_asteroid_field(&ent.template_path) {
            fields.push(ent);
        } else {
            others.push(ent);
        }
    }
    (fields, others)
}

// ── Unit Tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map_config::EntityInstanceSpawnOn;

    #[test]
    fn parse_world_empty_string_returns_empty_config() {
        let cfg = parse_world("").expect("empty TOML should parse");
        assert!(cfg.anchors.is_empty());
        assert!(cfg.entities.is_empty());
        assert_eq!(cfg.global.seed, 42);
    }

    #[test]
    fn parse_world_reads_anchors_as_three_element_arrays() {
        let toml = r#"
[anchors]
alpha = [10.0, 0.0, 20.0]
beta  = [-5.0, 1.5, 30.0]
"#;
        let cfg = parse_world(toml).expect("must parse");
        assert_eq!(cfg.anchors.len(), 2);
        assert_eq!(cfg.anchors.get("alpha"), Some(&[10.0, 0.0, 20.0]));
        assert_eq!(cfg.anchors.get("beta"), Some(&[-5.0, 1.5, 30.0]));
    }

    #[test]
    fn parse_world_widens_two_element_anchor_to_three() {
        // Historic AI code widened 2-element anchors by inserting 0.0 at Y.
        let toml = r#"
[anchors]
flat = [100.0, 200.0]
"#;
        let cfg = parse_world(toml).expect("must parse");
        assert_eq!(cfg.anchors.get("flat"), Some(&[100.0, 0.0, 200.0]));
    }

    #[test]
    fn parse_world_rejects_one_element_anchor() {
        let toml = r#"
[anchors]
busted = [1.0]
"#;
        let err = parse_world(toml).expect_err("one-element anchor must error");
        assert!(err.contains("busted"), "error must mention anchor name: {err}");
    }

    #[test]
    fn parse_world_reads_entity_blocks_with_template_path_and_position() {
        let toml = r#"
[[entity]]
template_path = "assets/entities/star_sun.toml"
position = [0.0, 0.0, 0.0]

[[entity]]
template_path = "assets/entities/asteroid_field_main.toml"
position = [100.0, 0.0, -200.0]
"#;
        let cfg = parse_world(toml).expect("must parse");
        assert_eq!(cfg.entities.len(), 2);
        assert_eq!(cfg.entities[0].template_path, "assets/entities/star_sun.toml");
        assert_eq!(cfg.entities[1].template_path, "assets/entities/asteroid_field_main.toml");
        assert_eq!(cfg.entities[1].position, vec![100.0, 0.0, -200.0]);
    }

    #[test]
    fn parse_world_entity_spawn_on_defaults_to_immediate() {
        let toml = r#"
[[entity]]
template_path = "assets/entities/asteroid_field_main.toml"
position = [0.0, 0.0, 0.0]
"#;
        let cfg = parse_world(toml).expect("must parse");
        assert_eq!(cfg.entities[0].spawn_on, EntityInstanceSpawnOn::Immediate);
    }

    #[test]
    fn parse_world_entity_spawn_on_game_start_recognised() {
        let toml = r#"
[[entity]]
template_path = "assets/entities/player_ship.toml"
id = "player-ship"
position = [0.0, 0.0, 0.0]
spawn_on = "game_start"
"#;
        let cfg = parse_world(toml).expect("must parse");
        assert_eq!(cfg.entities[0].spawn_on, EntityInstanceSpawnOn::GameStart);
        assert_eq!(cfg.entities[0].id.as_deref(), Some("player-ship"));
    }

    #[test]
    fn parse_world_silently_ignores_legacy_spawn_trigger_comms_blocks() {
        // The unified parser owns [[entity]]; the legacy [[spawn]] / [[trigger]] /
        // [[comms]] blocks continue to flow through parse_scenario until later
        // slices fold them in. They must not error here.
        let toml = r#"
[anchors]
alpha = [0.0, 0.0, 0.0]

[[entity]]
template_path = "assets/entities/star_sun.toml"
position = [0.0, 0.0, 0.0]

[[spawn]]
name = "raider"
entity_path = "assets/entities/pirate_raider.toml"
anchor = "alpha"

[[trigger]]
condition = "on_destroyed"
entity = "raider"

[[comms]]
from = "raider"
trigger = "on_attacked"
entity = "raider"
message = "Mayday"
"#;
        let cfg = parse_world(toml).expect("legacy blocks must be ignored, not errored");
        assert_eq!(cfg.entities.len(), 1);
        assert_eq!(cfg.anchors.len(), 1);
    }

    // ── Shipped-world smoke parses ────────────────────────────────────────

    #[test]
    fn parse_world_handles_shipped_default_toml_in_one_pass() {
        let toml = include_str!("../../assets/worlds/default.toml");
        let cfg = parse_world(toml).expect("default.toml must parse via new pipeline");
        // Anchors declared in default.toml.
        assert!(cfg.anchors.contains_key("starbase_alpha"));
        assert!(cfg.anchors.contains_key("patrol_alpha"));
        // [[entity]] blocks: star, planet, asteroid_field, player_ship, region_nebula.
        assert_eq!(
            cfg.entities.len(),
            5,
            "default.toml must contain 5 [[entity]] blocks"
        );
        // Asteroid field must be present so spawn_world_entities can route it.
        assert!(
            cfg.entities
                .iter()
                .any(|e| e.template_path.contains("asteroid_field")),
            "asteroid_field [[entity]] must be visible to the unified pipeline"
        );
    }

    #[test]
    fn parse_world_handles_shipped_patrol_toml_in_one_pass() {
        let toml = include_str!("../../assets/worlds/patrol.toml");
        let cfg = parse_world(toml).expect("patrol.toml must parse via new pipeline");
        assert!(cfg.anchors.contains_key("patrol_alpha"));
        // [[entity]] blocks: star, asteroid_field, player_ship.
        assert_eq!(
            cfg.entities.len(),
            3,
            "patrol.toml must contain 3 [[entity]] blocks"
        );
        assert!(
            cfg.entities
                .iter()
                .any(|e| e.template_path.contains("asteroid_field")),
            "asteroid_field [[entity]] must be visible to the unified pipeline"
        );
    }

    // ── entity_template_paths ─────────────────────────────────────────────

    #[test]
    fn entity_template_paths_returns_empty_for_no_entities() {
        let world = WorldConfig::default();
        assert!(entity_template_paths(&world).is_empty());
    }

    #[test]
    fn entity_template_paths_deduplicates_repeated_paths() {
        let toml = r#"
[[entity]]
template_path = "assets/entities/asteroid_large.toml"
position = [0.0, 0.0, 0.0]

[[entity]]
template_path = "assets/entities/asteroid_large.toml"
position = [10.0, 0.0, 10.0]

[[entity]]
template_path = "assets/entities/star_sun.toml"
position = [100.0, 0.0, 0.0]
"#;
        let cfg = parse_world(toml).expect("must parse");
        let paths = entity_template_paths(&cfg);
        assert_eq!(paths.len(), 2, "duplicates must be collapsed");
        assert!(paths.contains(&"assets/entities/asteroid_large.toml".to_string()));
        assert!(paths.contains(&"assets/entities/star_sun.toml".to_string()));
    }

    #[test]
    fn entity_template_paths_preserves_first_occurrence_order() {
        let toml = r#"
[[entity]]
template_path = "first.toml"
position = [0.0, 0.0, 0.0]

[[entity]]
template_path = "second.toml"
position = [0.0, 0.0, 0.0]

[[entity]]
template_path = "first.toml"
position = [0.0, 0.0, 0.0]

[[entity]]
template_path = "third.toml"
position = [0.0, 0.0, 0.0]
"#;
        let cfg = parse_world(toml).expect("must parse");
        let paths = entity_template_paths(&cfg);
        assert_eq!(
            paths,
            vec!["first.toml".to_string(), "second.toml".to_string(), "third.toml".to_string()],
            "iteration order must follow first-occurrence in the entity list"
        );
    }

    // ── partition_immediate_entities ──────────────────────────────────────

    #[test]
    fn partition_immediate_entities_routes_asteroid_fields_separately() {
        let toml = r#"
[[entity]]
template_path = "assets/entities/asteroid_field_main.toml"
position = [0.0, 0.0, 0.0]

[[entity]]
template_path = "assets/entities/star_sun.toml"
position = [100.0, 0.0, 0.0]

[[entity]]
template_path = "assets/entities/asteroid_field_outer.toml"
position = [500.0, 0.0, 500.0]
"#;
        let cfg = parse_world(toml).expect("must parse");
        let (fields, others) = partition_immediate_entities(&cfg, |path| {
            path.contains("asteroid_field")
        });
        assert_eq!(fields.len(), 2);
        assert_eq!(others.len(), 1);
        assert_eq!(others[0].template_path, "assets/entities/star_sun.toml");
    }

    #[test]
    fn partition_immediate_entities_excludes_game_start_entries() {
        let toml = r#"
[[entity]]
template_path = "assets/entities/asteroid_field_main.toml"
position = [0.0, 0.0, 0.0]

[[entity]]
template_path = "assets/entities/player_ship.toml"
position = [0.0, 0.0, 0.0]
spawn_on = "game_start"
"#;
        let cfg = parse_world(toml).expect("must parse");
        let (fields, others) = partition_immediate_entities(&cfg, |path| {
            path.contains("asteroid_field")
        });
        assert_eq!(fields.len(), 1);
        assert!(others.is_empty(), "game_start entries must NOT appear in the 'other' bucket");
    }

    #[test]
    fn partition_immediate_entities_empty_world_yields_two_empty_buckets() {
        let cfg = WorldConfig::default();
        let (fields, others) = partition_immediate_entities(&cfg, |_| true);
        assert!(fields.is_empty());
        assert!(others.is_empty());
    }

    #[test]
    fn partition_immediate_entities_classifier_returning_false_for_all_keeps_everything_in_other() {
        let toml = r#"
[[entity]]
template_path = "a.toml"
position = [0.0, 0.0, 0.0]

[[entity]]
template_path = "b.toml"
position = [0.0, 0.0, 0.0]
"#;
        let cfg = parse_world(toml).expect("must parse");
        let (fields, others) = partition_immediate_entities(&cfg, |_| false);
        assert!(fields.is_empty());
        assert_eq!(others.len(), 2);
    }
}
