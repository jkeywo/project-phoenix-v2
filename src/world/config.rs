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
    /// Map of `name → uuid` for entities spawned via `[[entity]] name = "..."`.
    /// Populated by `spawn_world_entities` (PRD #337/#339 slice 2); read by
    /// trigger and comms lookup paths that resolve a name to a live UUID.
    pub name_to_uuid: HashMap<String, String>,
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
        name_to_uuid: HashMap::new(),
    })
}

/// Build a `name → uuid` map for the named entries in an `[[entity]]` slice.
///
/// PRD #337/#339 slice 2: anonymous `[[entity]]` instances stay unaddressable;
/// `[[entity]]` instances carrying a `name` field become trigger- and
/// comms-eligible. The UUID generator is supplied by the caller so this
/// helper stays a pure function (tests pass a counter; production passes
/// `|| Uuid::new_v4().to_string()`).
pub fn assign_named_entity_uuids<F>(
    entities: &[EntityInstance],
    mut gen_uuid: F,
) -> HashMap<String, String>
where
    F: FnMut() -> String,
{
    let mut out = HashMap::new();
    for entity in entities {
        if let Some(name) = entity.name.as_ref() {
            out.insert(name.clone(), gen_uuid());
        }
    }
    out
}

/// Predicate: is this `[[entity]]` instance owned by the unified pipeline
/// (`spawn_world_entities`), rather than the legacy
/// `setup_world_from_config` path?
///
/// PRD #337 routes two kinds of entries through the unified pipeline:
/// * **Slice 1**: any entry whose resolved template is an asteroid field.
/// * **Slice 2**: any entry carrying a `name` field — the unified pipeline
///   assigns the UUID so `name → uuid` is single-sourced.
///
/// Both call sites (legacy + unified) call this helper with the same
/// `is_asteroid_field` lookup to guarantee no entry is spawned twice.
pub fn is_owned_by_unified_pipeline<F>(
    entity_inst: &EntityInstance,
    is_asteroid_field: F,
) -> bool
where
    F: Fn(&str) -> bool,
{
    if entity_inst.name.is_some() {
        return true;
    }
    is_asteroid_field(&entity_inst.template_path)
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

/// Resolve the spawn position of an `[[entity]]` instance against the
/// world's anchor table.
///
/// Precedence:
/// 1. `anchor = "name"` — look up the anchor; error if missing.
/// 2. `position = [x, y, z]` — return as-is when length ≥ 3.
/// 3. Neither — `[0, 0, 0]`.
///
/// `anchor` and `position` are not strictly mutually exclusive at parse
/// time; when both are supplied the anchor wins (matches the legacy
/// `[[spawn]]` semantics where anchor lookups happened first).
///
/// PRD #337 slice 3: lifts anchor positioning from the scenario half into
/// the unified `[[entity]]` pipeline so NPCs can be migrated off
/// `[[spawn]]`. Pure function — tested without Bevy.
pub fn resolve_entity_position(
    entity_inst: &crate::map_config::EntityInstance,
    anchors: &HashMap<String, [f32; 3]>,
) -> Result<[f32; 3], String> {
    if let Some(name) = entity_inst.anchor.as_ref() {
        let pos = anchors.get(name).ok_or_else(|| {
            format!(
                "Entity (template '{}') references unknown anchor '{}'",
                entity_inst.template_path, name
            )
        })?;
        return Ok(*pos);
    }
    if entity_inst.position.len() >= 3 {
        return Ok([
            entity_inst.position[0],
            entity_inst.position[1],
            entity_inst.position[2],
        ]);
    }
    Ok([0.0, 0.0, 0.0])
}

/// Three-way partition of immediate `[[entity]]` instances.
///
/// PRD #339 slice 2: the unified pipeline owns BOTH asteroid-field templates
/// AND any entry carrying a `name` field (so the entity that triggers / comms
/// resolve through `name → uuid` is actually spawned with that UUID). The
/// legacy `setup_world_from_config` path only spawns the third bucket
/// (anonymous non-asteroid entries).
///
/// Returns `(asteroid_fields, named_non_asteroid, anonymous_non_asteroid)`.
/// `GameStart` entries are returned in none of the three buckets.
pub fn partition_immediate_entities_three_way<F>(
    world: &WorldConfig,
    is_asteroid_field: F,
) -> (
    Vec<&crate::map_config::EntityInstance>,
    Vec<&crate::map_config::EntityInstance>,
    Vec<&crate::map_config::EntityInstance>,
)
where
    F: Fn(&str) -> bool,
{
    use crate::map_config::EntityInstanceSpawnOn;
    let mut fields = Vec::new();
    let mut named = Vec::new();
    let mut anon = Vec::new();
    for ent in &world.entities {
        if ent.spawn_on != EntityInstanceSpawnOn::Immediate {
            continue;
        }
        if is_asteroid_field(&ent.template_path) {
            fields.push(ent);
        } else if ent.name.is_some() {
            named.push(ent);
        } else {
            anon.push(ent);
        }
    }
    (fields, named, anon)
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
    fn world_config_default_has_empty_name_to_uuid() {
        // PRD #337/#339 slice 2: the unified WorldConfig owns the
        // `name → uuid` map that `spawn_world_entities` populates and
        // trigger/comms lookup reads. Starts empty.
        let cfg = WorldConfig::default();
        assert!(cfg.name_to_uuid.is_empty());
        assert_eq!(cfg.name_to_uuid.len(), 0);
    }

    #[test]
    fn assign_named_entity_uuids_collects_named_only_with_stable_uuids() {
        // PRD #337/#339 slice 2: a pure helper builds the `name → uuid`
        // map from a slice of EntityInstance. Anonymous entries are skipped;
        // a caller-supplied generator yields the UUIDs (so tests can be
        // deterministic without dragging real RNG in).
        let entities = vec![
            EntityInstance {
                template_path: "assets/entities/station_outpost.toml".into(),
                name: Some("starbase_alpha".into()),
                ..Default::default()
            },
            EntityInstance {
                template_path: "assets/entities/star_sun.toml".into(),
                name: None,
                ..Default::default()
            },
            EntityInstance {
                template_path: "assets/entities/planet_earth.toml".into(),
                name: Some("earth".into()),
                ..Default::default()
            },
        ];
        let mut counter = 0u32;
        let map = assign_named_entity_uuids(&entities, || {
            counter += 1;
            format!("uuid-{counter}")
        });
        assert_eq!(map.len(), 2, "only named entities get a uuid");
        assert_eq!(map.get("starbase_alpha").map(String::as_str), Some("uuid-1"));
        assert_eq!(map.get("earth").map(String::as_str), Some("uuid-2"));
    }

    #[test]
    fn is_owned_by_unified_pipeline_routes_asteroid_fields_and_named_entries() {
        // PRD #337/#339 slice 2: the legacy `setup_world_from_config` path
        // must skip both asteroid-field templates (slice 1) AND any entry
        // carrying a `name` field (slice 2 — owned by `spawn_world_entities`).
        let asteroid = EntityInstance {
            template_path: "assets/entities/asteroid_field_dense.toml".into(),
            ..Default::default()
        };
        let named = EntityInstance {
            template_path: "assets/entities/station_outpost.toml".into(),
            name: Some("starbase_alpha".into()),
            ..Default::default()
        };
        let anon = EntityInstance {
            template_path: "assets/entities/star_sun.toml".into(),
            ..Default::default()
        };

        let is_field = |p: &str| p.contains("asteroid_field");
        assert!(is_owned_by_unified_pipeline(&asteroid, is_field));
        assert!(is_owned_by_unified_pipeline(&named, is_field));
        assert!(
            !is_owned_by_unified_pipeline(&anon, is_field),
            "anonymous non-asteroid entries stay on the legacy path"
        );
    }

    #[test]
    fn partition_immediate_entities_three_buckets_separates_fields_named_anonymous() {
        // PRD #339 slice 2: named non-asteroid entries are owned by the
        // unified pipeline (and must be spawned by it). The partition
        // helper now produces three buckets so `spawn_world_entities` can
        // iterate both fields AND named entries while
        // `setup_world_from_config` keeps anonymous ones.
        let mut cfg = WorldConfig::default();
        cfg.entities.push(EntityInstance {
            template_path: "assets/entities/asteroid_field_main.toml".into(),
            ..Default::default()
        });
        cfg.entities.push(EntityInstance {
            template_path: "assets/entities/station_outpost.toml".into(),
            name: Some("starbase_alpha".into()),
            ..Default::default()
        });
        cfg.entities.push(EntityInstance {
            template_path: "assets/entities/star_sun.toml".into(),
            ..Default::default()
        });
        // game_start entries are in no bucket
        cfg.entities.push(EntityInstance {
            template_path: "assets/entities/player_ship.toml".into(),
            spawn_on: crate::map_config::EntityInstanceSpawnOn::GameStart,
            ..Default::default()
        });

        let is_field = |p: &str| p.contains("asteroid_field");
        let (fields, named, anon) =
            partition_immediate_entities_three_way(&cfg, is_field);

        assert_eq!(fields.len(), 1);
        assert_eq!(named.len(), 1);
        assert_eq!(named[0].name.as_deref(), Some("starbase_alpha"));
        assert_eq!(anon.len(), 1);
        assert_eq!(anon[0].template_path, "assets/entities/star_sun.toml");
    }

    #[test]
    fn parse_world_entity_accepts_optional_name_field() {
        // PRD #337/#339 slice 2: named [[entity]] blocks become the unified
        // replacement for [[spawn]] — they get a UUID at spawn time and
        // become eligible for trigger / comms lookups.
        let toml = r#"
[[entity]]
template_path = "assets/entities/station_outpost.toml"
name = "Starbase Alpha"
position = [500.0, 0.0, 0.0]

[[entity]]
template_path = "assets/entities/star_sun.toml"
position = [0.0, 0.0, 0.0]
"#;
        let cfg = parse_world(toml).expect("must parse");
        assert_eq!(cfg.entities.len(), 2);
        assert_eq!(cfg.entities[0].name.as_deref(), Some("Starbase Alpha"));
        assert_eq!(
            cfg.entities[1].name, None,
            "entity without a name field must deserialize as None"
        );
    }

    #[test]
    fn parse_world_entity_accepts_anchor_field() {
        // PRD #337 slice 3: `[[entity]]` now supports `anchor = "..."` so NPC
        // patrols (formerly `[[spawn]]`) can be migrated into the unified
        // pipeline without inlining anchor coordinates.
        let toml = r#"
[anchors]
patrol_alpha = [300.0, 0.0, -300.0]

[[entity]]
template_path = "assets/entities/pirate_raider.toml"
name = "raider_alpha"
anchor = "patrol_alpha"
"#;
        let cfg = parse_world(toml).expect("must parse");
        assert_eq!(cfg.entities.len(), 1);
        assert_eq!(cfg.entities[0].anchor.as_deref(), Some("patrol_alpha"));
        assert!(
            cfg.entities[0].position.is_empty(),
            "no inline position when anchor is supplied"
        );
    }

    // ── resolve_entity_position (PRD #337 slice 3) ────────────────────────

    fn anchor_table(entries: &[(&str, [f32; 3])]) -> HashMap<String, [f32; 3]> {
        entries
            .iter()
            .map(|(k, v)| ((*k).to_string(), *v))
            .collect()
    }

    #[test]
    fn resolve_entity_position_uses_anchor_when_set() {
        let entity = EntityInstance {
            template_path: "assets/entities/pirate_raider.toml".into(),
            anchor: Some("patrol_alpha".into()),
            ..Default::default()
        };
        let anchors = anchor_table(&[("patrol_alpha", [300.0, 0.0, -300.0])]);
        let pos = resolve_entity_position(&entity, &anchors).unwrap();
        assert_eq!(pos, [300.0, 0.0, -300.0]);
    }

    #[test]
    fn resolve_entity_position_falls_back_to_inline_position() {
        let entity = EntityInstance {
            template_path: "assets/entities/star_sun.toml".into(),
            position: vec![10.0, 0.0, 20.0],
            ..Default::default()
        };
        let pos = resolve_entity_position(&entity, &HashMap::new()).unwrap();
        assert_eq!(pos, [10.0, 0.0, 20.0]);
    }

    #[test]
    fn resolve_entity_position_errors_on_unknown_anchor() {
        let entity = EntityInstance {
            template_path: "assets/entities/pirate_raider.toml".into(),
            anchor: Some("ghost".into()),
            ..Default::default()
        };
        let err = resolve_entity_position(&entity, &HashMap::new()).unwrap_err();
        assert!(err.contains("ghost"), "error must mention missing anchor: {err}");
    }

    #[test]
    fn resolve_entity_position_anchor_wins_over_inline_position() {
        let entity = EntityInstance {
            template_path: "x.toml".into(),
            anchor: Some("a".into()),
            position: vec![999.0, 999.0, 999.0],
            ..Default::default()
        };
        let anchors = anchor_table(&[("a", [1.0, 2.0, 3.0])]);
        let pos = resolve_entity_position(&entity, &anchors).unwrap();
        assert_eq!(pos, [1.0, 2.0, 3.0]);
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
        // [[entity]] blocks: star, planet (named "earth"), asteroid_field,
        // player_ship, region_nebula, Starbase Alpha (named, was [[spawn]]
        // — migrated in PRD #339 slice 2), raider_alpha (named NPC, was
        // [[spawn]] — migrated in PRD #337 slice 3).
        assert_eq!(
            cfg.entities.len(),
            7,
            "default.toml must contain 7 [[entity]] blocks after raider migration"
        );
        // Asteroid field must be present so spawn_world_entities can route it.
        assert!(
            cfg.entities
                .iter()
                .any(|e| e.template_path.contains("asteroid_field")),
            "asteroid_field [[entity]] must be visible to the unified pipeline"
        );
        // PRD #339 slice 2: named entries are now [[entity]] with `name`.
        assert!(
            cfg.entities.iter().any(|e| e.name.as_deref() == Some("Starbase Alpha")),
            "Starbase Alpha must be a named [[entity]] after slice 2 migration"
        );
        assert!(
            cfg.entities.iter().any(|e| e.name.as_deref() == Some("earth")),
            "earth must carry a `name` field after slice 2 migration"
        );
        // PRD #337 slice 3: the default-world raider migrated from
        // [[spawn]] to a named [[entity]] with `anchor = "patrol_alpha"`.
        let raider = cfg
            .entities
            .iter()
            .find(|e| e.name.as_deref() == Some("raider_alpha"))
            .expect("raider_alpha must be a named [[entity]] after slice 3 migration");
        assert_eq!(
            raider.anchor.as_deref(),
            Some("patrol_alpha"),
            "default-world raider must use anchor positioning"
        );
        assert!(
            raider.position.is_empty(),
            "default-world raider must have no inline position when anchor is supplied"
        );
    }

    #[test]
    fn parse_world_handles_shipped_patrol_toml_in_one_pass() {
        let toml = include_str!("../../assets/worlds/patrol.toml");
        let cfg = parse_world(toml).expect("patrol.toml must parse via new pipeline");
        assert!(cfg.anchors.contains_key("patrol_alpha"));
        // PRD #337 slice 3: the raider migrated from [[spawn]] to a named
        // [[entity]] with `anchor = "patrol_alpha"`. Total [[entity]] blocks:
        // star, asteroid_field, player_ship, raider_alpha.
        assert_eq!(
            cfg.entities.len(),
            4,
            "patrol.toml must contain 4 [[entity]] blocks after raider migration"
        );
        assert!(
            cfg.entities
                .iter()
                .any(|e| e.template_path.contains("asteroid_field")),
            "asteroid_field [[entity]] must be visible to the unified pipeline"
        );
        let raider = cfg
            .entities
            .iter()
            .find(|e| e.name.as_deref() == Some("raider_alpha"))
            .expect("raider_alpha must be a named [[entity]] after slice 3 migration");
        assert_eq!(
            raider.anchor.as_deref(),
            Some("patrol_alpha"),
            "raider must use anchor positioning"
        );
        assert!(
            raider.position.is_empty(),
            "raider must have no inline position when anchor is supplied"
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
