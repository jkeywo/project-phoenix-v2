// Pure Rust module for parsing scenario TOML files.
// No Bevy dependency. Owns all deserialization for scenario configuration.
//
// A scenario file declares `[[spawn]]` blocks that reference entity template
// paths and supply a position specification (absolute, anchor-relative, or
// entity-relative). Each named spawn is assigned a stable runtime UUID on
// parse. `resolve_positions` then resolves position specs against a map's
// anchor table, returning a flat list of `ResolvedSpawn` values ready for
// the entity-spawn pipeline.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

// ── Position specification ─────────────────────────────────────────────────

/// How the position of a spawn is determined.
#[derive(Clone, Debug, PartialEq)]
pub enum PositionSpec {
    /// Absolute world-space position [x, y, z].
    Absolute([f32; 3]),
    /// Position taken from a named anchor in the map's `[anchors]` table.
    Anchor(String),
    /// Position relative to another named spawn in this scenario, plus offset.
    RelativeTo { entity_name: String, offset: [f32; 3] },
}

// ── TOML-facing deserialization ────────────────────────────────────────────

/// Raw TOML representation of a single `[[spawn]]` block.
#[derive(Clone, Debug, Deserialize)]
struct RawSpawnEntry {
    /// Stable human-readable name for this spawn instance.
    pub name: String,
    /// Path to the entity template TOML (relative to assets/).
    pub entity_path: String,
    /// Absolute position [x, y, z]. Mutually exclusive with `anchor` / `relative_to`.
    #[serde(default)]
    pub position: Option<[f32; 3]>,
    /// Named anchor from the map's `[anchors]` table. Mutually exclusive with `position` / `relative_to`.
    #[serde(default)]
    pub anchor: Option<String>,
    /// Name of another spawn in this scenario to position relative to. Mutually exclusive with `position` / `anchor`.
    #[serde(default)]
    pub relative_to: Option<String>,
    /// Offset applied on top of `relative_to` position.
    #[serde(default)]
    pub offset: Option<[f32; 3]>,
}

#[derive(Deserialize)]
struct RawScenario {
    #[serde(default, rename = "spawn")]
    spawns: Vec<RawSpawnEntry>,
}

// ── Public types ───────────────────────────────────────────────────────────

/// A single spawn entry with a resolved position spec and an assigned UUID.
#[derive(Clone, Debug, PartialEq)]
pub struct SpawnEntry {
    /// Stable human-readable name for this spawn instance.
    pub name: String,
    /// Path to the entity template TOML.
    pub entity_path: String,
    /// How the position of this spawn is determined.
    pub position_spec: PositionSpec,
    /// Runtime UUID assigned at parse time. Stable for the lifetime of this
    /// `ScenarioConfig` value.
    pub uuid: String,
}

/// Parsed scenario configuration. Created by `parse_scenario`.
#[derive(Clone, Debug)]
pub struct ScenarioConfig {
    /// Ordered list of spawn entries.
    pub spawns: Vec<SpawnEntry>,
    /// Map from spawn `name` to its assigned runtime UUID.
    pub name_to_uuid: HashMap<String, String>,
}

/// A spawn entry with its world-space position fully resolved.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedSpawn {
    /// Runtime UUID for this entity.
    pub uuid: String,
    /// Human-readable name.
    pub name: String,
    /// Path to the entity template TOML.
    pub entity_path: String,
    /// Resolved world-space position [x, y, z].
    pub position: [f32; 3],
}

// ── Parsing ────────────────────────────────────────────────────────────────

/// Parse a scenario TOML string into a `ScenarioConfig`.
///
/// Each spawn entry is assigned a UUID v4 on parse. Position specs are stored
/// as-is; call `resolve_positions` to turn them into concrete world coordinates.
pub fn parse_scenario(toml_str: &str) -> Result<ScenarioConfig, String> {
    let raw: RawScenario = toml::from_str(toml_str).map_err(|e| e.to_string())?;

    let mut spawns = Vec::new();
    let mut name_to_uuid = HashMap::new();

    for raw_spawn in raw.spawns {
        // Validate exactly one position spec is present.
        let spec_count = raw_spawn.position.is_some() as u8
            + raw_spawn.anchor.is_some() as u8
            + raw_spawn.relative_to.is_some() as u8;

        if spec_count == 0 {
            return Err(format!(
                "Spawn '{}' has no position specification (need position, anchor, or relative_to)",
                raw_spawn.name
            ));
        }
        if spec_count > 1 {
            return Err(format!(
                "Spawn '{}' has multiple position specifications (only one of position, anchor, relative_to allowed)",
                raw_spawn.name
            ));
        }

        let position_spec = if let Some(pos) = raw_spawn.position {
            PositionSpec::Absolute(pos)
        } else if let Some(anchor) = raw_spawn.anchor {
            PositionSpec::Anchor(anchor)
        } else {
            let entity_name = raw_spawn.relative_to.unwrap();
            let offset = raw_spawn.offset.unwrap_or([0.0, 0.0, 0.0]);
            PositionSpec::RelativeTo { entity_name, offset }
        };

        let uuid = Uuid::new_v4().to_string();
        name_to_uuid.insert(raw_spawn.name.clone(), uuid.clone());

        spawns.push(SpawnEntry {
            name: raw_spawn.name,
            entity_path: raw_spawn.entity_path,
            position_spec,
            uuid,
        });
    }

    Ok(ScenarioConfig { spawns, name_to_uuid })
}

// ── Position resolution ────────────────────────────────────────────────────

/// Resolve all spawn positions against the given anchor table.
///
/// Returns an ordered `Vec<ResolvedSpawn>` in the same order as the original
/// `[[spawn]]` blocks.
///
/// # Errors
/// - Unknown anchor name → `Err` describing the missing anchor.
/// - Unknown `relative_to` entity name → `Err` describing the missing entity.
pub fn resolve_positions(
    scenario: &ScenarioConfig,
    anchors: &HashMap<String, Vec<f32>>,
) -> Result<Vec<ResolvedSpawn>, String> {
    // First pass: build a name→position map for entities with non-relative positions.
    // Second pass: resolve relative-to entries.
    // We process in order and allow relative_to to reference any previously
    // resolved entry. Circular or forward references produce an error.

    let mut resolved_positions: HashMap<String, [f32; 3]> = HashMap::new();
    let mut result = Vec::new();

    for spawn in &scenario.spawns {
        let position = match &spawn.position_spec {
            PositionSpec::Absolute(pos) => *pos,
            PositionSpec::Anchor(anchor_name) => {
                let anchor_pos = anchors.get(anchor_name).ok_or_else(|| {
                    format!(
                        "Spawn '{}' references unknown anchor '{}'",
                        spawn.name, anchor_name
                    )
                })?;
                [anchor_pos[0], anchor_pos[1], anchor_pos[2]]
            }
            PositionSpec::RelativeTo { entity_name, offset } => {
                let base = resolved_positions.get(entity_name).ok_or_else(|| {
                    format!(
                        "Spawn '{}' references unknown or unresolved entity '{}'",
                        spawn.name, entity_name
                    )
                })?;
                [base[0] + offset[0], base[1] + offset[1], base[2] + offset[2]]
            }
        };

        resolved_positions.insert(spawn.name.clone(), position);

        result.push(ResolvedSpawn {
            uuid: spawn.uuid.clone(),
            name: spawn.name.clone(),
            entity_path: spawn.entity_path.clone(),
            position,
        });
    }

    Ok(result)
}

// ── Unit Tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Cycle 1: parse empty scenario ─────────────────────────────────────

    #[test]
    fn parse_empty_scenario_returns_empty_spawns() {
        let config = parse_scenario("").unwrap();
        assert!(config.spawns.is_empty());
        assert!(config.name_to_uuid.is_empty());
    }

    // ── Cycle 2: parse [[spawn]] with absolute position ────────────────────

    #[test]
    fn parse_spawn_with_absolute_position() {
        let toml = r#"
[[spawn]]
name = "asteroid_alpha"
entity_path = "entities/asteroid_large.toml"
position = [100.0, 0.0, 200.0]
"#;
        let config = parse_scenario(toml).unwrap();
        assert_eq!(config.spawns.len(), 1);
        let spawn = &config.spawns[0];
        assert_eq!(spawn.name, "asteroid_alpha");
        assert_eq!(spawn.entity_path, "entities/asteroid_large.toml");
        assert_eq!(spawn.position_spec, PositionSpec::Absolute([100.0, 0.0, 200.0]));
    }

    #[test]
    fn parse_multiple_spawns_preserves_order() {
        let toml = r#"
[[spawn]]
name = "alpha"
entity_path = "entities/a.toml"
position = [0.0, 0.0, 0.0]

[[spawn]]
name = "beta"
entity_path = "entities/b.toml"
position = [1.0, 0.0, 0.0]
"#;
        let config = parse_scenario(toml).unwrap();
        assert_eq!(config.spawns.len(), 2);
        assert_eq!(config.spawns[0].name, "alpha");
        assert_eq!(config.spawns[1].name, "beta");
    }

    // ── Cycle 3: parse [[spawn]] with anchor ──────────────────────────────

    #[test]
    fn parse_spawn_with_anchor_position() {
        let toml = r#"
[[spawn]]
name = "station_beta"
entity_path = "entities/station.toml"
anchor = "waypoint_alpha"
"#;
        let config = parse_scenario(toml).unwrap();
        assert_eq!(config.spawns.len(), 1);
        assert_eq!(
            config.spawns[0].position_spec,
            PositionSpec::Anchor("waypoint_alpha".to_string())
        );
    }

    // ── Cycle 4: resolve positions - anchor ───────────────────────────────

    #[test]
    fn resolve_positions_anchor_returns_anchor_position() {
        let toml = r#"
[[spawn]]
name = "station_beta"
entity_path = "entities/station.toml"
anchor = "waypoint_alpha"
"#;
        let config = parse_scenario(toml).unwrap();
        let mut anchors = HashMap::new();
        anchors.insert("waypoint_alpha".to_string(), vec![50.0, 0.0, 100.0]);

        let resolved = resolve_positions(&config, &anchors).unwrap();
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].position, [50.0, 0.0, 100.0]);
        assert_eq!(resolved[0].name, "station_beta");
    }

    #[test]
    fn resolve_positions_absolute_unchanged() {
        let toml = r#"
[[spawn]]
name = "asteroid_alpha"
entity_path = "entities/asteroid_large.toml"
position = [100.0, 5.0, 200.0]
"#;
        let config = parse_scenario(toml).unwrap();
        let anchors = HashMap::new();
        let resolved = resolve_positions(&config, &anchors).unwrap();
        assert_eq!(resolved[0].position, [100.0, 5.0, 200.0]);
    }

    // ── Cycle 5: error on unknown anchor ──────────────────────────────────

    #[test]
    fn resolve_positions_errors_on_unknown_anchor() {
        let toml = r#"
[[spawn]]
name = "station"
entity_path = "entities/station.toml"
anchor = "nonexistent_anchor"
"#;
        let config = parse_scenario(toml).unwrap();
        let anchors = HashMap::new();
        let result = resolve_positions(&config, &anchors);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("nonexistent_anchor"), "error should mention missing anchor: {err}");
    }

    // ── Cycle 6: entity-relative position ────────────────────────────────

    #[test]
    fn parse_spawn_with_relative_to() {
        let toml = r#"
[[spawn]]
name = "patrol_drone"
entity_path = "entities/drone.toml"
relative_to = "station_beta"
offset = [10.0, 0.0, 5.0]
"#;
        let config = parse_scenario(toml).unwrap();
        assert_eq!(
            config.spawns[0].position_spec,
            PositionSpec::RelativeTo {
                entity_name: "station_beta".to_string(),
                offset: [10.0, 0.0, 5.0]
            }
        );
    }

    #[test]
    fn resolve_positions_relative_to_adds_offset() {
        let toml = r#"
[[spawn]]
name = "station_beta"
entity_path = "entities/station.toml"
position = [100.0, 0.0, 0.0]

[[spawn]]
name = "patrol_drone"
entity_path = "entities/drone.toml"
relative_to = "station_beta"
offset = [10.0, 0.0, 5.0]
"#;
        let config = parse_scenario(toml).unwrap();
        let anchors = HashMap::new();
        let resolved = resolve_positions(&config, &anchors).unwrap();
        assert_eq!(resolved[1].position, [110.0, 0.0, 5.0]);
    }

    #[test]
    fn resolve_positions_errors_on_unknown_relative_to() {
        let toml = r#"
[[spawn]]
name = "drone"
entity_path = "entities/drone.toml"
relative_to = "missing_entity"
offset = [0.0, 0.0, 0.0]
"#;
        let config = parse_scenario(toml).unwrap();
        let result = resolve_positions(&config, &HashMap::new());
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("missing_entity"), "error should mention missing entity: {err}");
    }

    #[test]
    fn relative_to_without_offset_defaults_to_zero() {
        let toml = r#"
[[spawn]]
name = "base"
entity_path = "entities/base.toml"
position = [50.0, 0.0, 50.0]

[[spawn]]
name = "drone"
entity_path = "entities/drone.toml"
relative_to = "base"
"#;
        let config = parse_scenario(toml).unwrap();
        let anchors = HashMap::new();
        let resolved = resolve_positions(&config, &anchors).unwrap();
        assert_eq!(resolved[1].position, [50.0, 0.0, 50.0]);
    }

    // ── Cycle 7: name → UUID resolution ──────────────────────────────────

    #[test]
    fn each_spawn_gets_a_uuid() {
        let toml = r#"
[[spawn]]
name = "asteroid_alpha"
entity_path = "entities/asteroid.toml"
position = [0.0, 0.0, 0.0]
"#;
        let config = parse_scenario(toml).unwrap();
        assert!(!config.spawns[0].uuid.is_empty());
    }

    #[test]
    fn name_to_uuid_map_contains_all_spawn_names() {
        let toml = r#"
[[spawn]]
name = "alpha"
entity_path = "entities/a.toml"
position = [0.0, 0.0, 0.0]

[[spawn]]
name = "beta"
entity_path = "entities/b.toml"
position = [1.0, 0.0, 0.0]
"#;
        let config = parse_scenario(toml).unwrap();
        assert_eq!(config.name_to_uuid.len(), 2);
        assert!(config.name_to_uuid.contains_key("alpha"));
        assert!(config.name_to_uuid.contains_key("beta"));
    }

    #[test]
    fn uuid_in_spawn_matches_name_to_uuid_map() {
        let toml = r#"
[[spawn]]
name = "alpha"
entity_path = "entities/a.toml"
position = [0.0, 0.0, 0.0]
"#;
        let config = parse_scenario(toml).unwrap();
        let spawn = &config.spawns[0];
        assert_eq!(
            config.name_to_uuid.get("alpha").unwrap(),
            &spawn.uuid
        );
    }

    #[test]
    fn resolved_spawn_uuid_matches_scenario_uuid() {
        let toml = r#"
[[spawn]]
name = "alpha"
entity_path = "entities/a.toml"
position = [10.0, 0.0, 20.0]
"#;
        let config = parse_scenario(toml).unwrap();
        let anchors = HashMap::new();
        let resolved = resolve_positions(&config, &anchors).unwrap();
        assert_eq!(resolved[0].uuid, config.spawns[0].uuid);
    }

    // ── Error cases ────────────────────────────────────────────────────────

    #[test]
    fn parse_spawn_without_position_spec_returns_error() {
        let toml = r#"
[[spawn]]
name = "orphan"
entity_path = "entities/a.toml"
"#;
        let result = parse_scenario(toml);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("orphan") || err.contains("position"), "error: {err}");
    }

    #[test]
    fn parse_spawn_with_multiple_position_specs_returns_error() {
        let toml = r#"
[[spawn]]
name = "conflicted"
entity_path = "entities/a.toml"
position = [0.0, 0.0, 0.0]
anchor = "some_anchor"
"#;
        let result = parse_scenario(toml);
        assert!(result.is_err());
    }

    #[test]
    fn parse_invalid_toml_returns_error() {
        let toml = "[[spawn\nbroken";
        let result = parse_scenario(toml);
        assert!(result.is_err());
    }
}
