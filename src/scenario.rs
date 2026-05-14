// Pure Rust module for parsing scenario TOML files.
// No Bevy dependency. Owns all deserialization for scenario configuration.
//
// A scenario file declares `[[spawn]]` blocks that reference entity template
// paths and supply a position specification (absolute, anchor-relative, or
// entity-relative). Each named spawn is assigned a stable runtime UUID on
// parse. `resolve_positions` then resolves position specs against a map's
// anchor table, returning a flat list of `ResolvedSpawn` values ready for
// the entity-spawn pipeline.

use serde::Deserialize;
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

// ── Trigger types ─────────────────────────────────────────────────────────

/// A world event that triggers can react to.
#[derive(Clone, Debug, PartialEq)]
pub enum WorldEvent {
    /// An entity (by UUID) was destroyed.
    Destroyed { uuid: String },
    /// An entity (by UUID) was attacked; `attacker_uuid` is the attacker.
    Attacked { uuid: String, attacker_uuid: String },
    /// Simulation time has advanced. `elapsed_secs` is total elapsed time.
    TimerElapsed { elapsed_secs: f32 },
}

/// A condition that a trigger can check against incoming world events.
#[derive(Clone, Debug, PartialEq)]
pub enum TriggerCondition {
    /// Fires when the named entity (by spawn name, resolved to UUID at runtime) is destroyed.
    OnDestroyed { entity_name: String },
    /// Fires when the named entity is attacked.
    OnAttacked { entity_name: String },
    /// Fires once when `elapsed_secs` crosses `after_secs`.
    OnTimer { after_secs: f32 },
}

/// An action to execute when a trigger fires.
#[derive(Clone, Debug, PartialEq)]
pub enum TriggerAction {
    /// Load and activate a new scenario from `path`. Parameters (`$name`) are
    /// substituted before dispatch.
    LoadScenario { path: String },
    /// Add a new mission objective owned by the current scenario.
    AddObjective { id: String, text: String, mandatory: bool },
    /// Mark an active objective as completed.
    CompleteObjective { id: String },
    /// Mark an active objective as failed.
    FailObjective { id: String },
}

/// A single trigger: a condition plus an ordered list of actions.
#[derive(Clone, Debug, PartialEq)]
pub struct Trigger {
    pub condition: TriggerCondition,
    pub actions: Vec<TriggerAction>,
}

/// Runtime state for one trigger within an active scenario.
#[derive(Clone, Debug)]
pub struct TriggerState {
    pub trigger: Trigger,
    /// Whether this trigger has already fired (single-shot semantics).
    pub fired: bool,
}

/// Result of evaluating triggers against a batch of world events.
#[derive(Clone, Debug, PartialEq)]
pub struct FiredTrigger {
    pub actions: Vec<TriggerAction>,
}

/// Evaluate all triggers in `states` against the given `events`.
///
/// Each trigger fires at most once (single-shot). When a trigger fires its
/// `fired` flag is set to `true` and its actions (with `$name` parameters
/// substituted using `name_to_uuid`) are collected into a `FiredTrigger`.
///
/// Returns the list of `FiredTrigger` values produced in this call.
pub fn evaluate_triggers(
    states: &mut Vec<TriggerState>,
    events: &[WorldEvent],
    name_to_uuid: &HashMap<String, String>,
) -> Vec<FiredTrigger> {
    let mut results = Vec::new();

    for state in states.iter_mut() {
        if state.fired {
            continue;
        }

        let fires = events.iter().any(|event| {
            condition_matches(&state.trigger.condition, event, name_to_uuid)
        });

        if fires {
            state.fired = true;
            let actions = state
                .trigger
                .actions
                .iter()
                .map(|a| substitute_action(a, name_to_uuid))
                .collect();
            results.push(FiredTrigger { actions });
        }
    }

    results
}

/// Returns true if `condition` matches `event`.
fn condition_matches(
    condition: &TriggerCondition,
    event: &WorldEvent,
    name_to_uuid: &HashMap<String, String>,
) -> bool {
    match (condition, event) {
        (TriggerCondition::OnDestroyed { entity_name }, WorldEvent::Destroyed { uuid }) => {
            name_to_uuid.get(entity_name).map(|u| u == uuid).unwrap_or(false)
        }
        (TriggerCondition::OnAttacked { entity_name }, WorldEvent::Attacked { uuid, .. }) => {
            name_to_uuid.get(entity_name).map(|u| u == uuid).unwrap_or(false)
        }
        (TriggerCondition::OnTimer { after_secs }, WorldEvent::TimerElapsed { elapsed_secs }) => {
            elapsed_secs >= after_secs
        }
        _ => false,
    }
}

/// Substitute `$name` parameters in an action using `name_to_uuid`.
fn substitute_action(
    action: &TriggerAction,
    name_to_uuid: &HashMap<String, String>,
) -> TriggerAction {
    match action {
        TriggerAction::LoadScenario { path } => {
            let resolved = substitute_params(path, name_to_uuid);
            TriggerAction::LoadScenario { path: resolved }
        }
        // Objective actions carry no path parameters — pass through unchanged.
        TriggerAction::AddObjective { .. }
        | TriggerAction::CompleteObjective { .. }
        | TriggerAction::FailObjective { .. } => action.clone(),
    }
}

/// Replace `$name` tokens in a string with their UUID values from `name_to_uuid`.
/// Tokens that have no matching entry are left unchanged.
fn substitute_params(s: &str, name_to_uuid: &HashMap<String, String>) -> String {
    let mut result = s.to_string();
    for (name, uuid) in name_to_uuid {
        let token = format!("${}", name);
        result = result.replace(&token, uuid);
    }
    result
}

// ── TOML-facing deserialization for triggers ──────────────────────────────

#[derive(Deserialize)]
struct RawTriggerEntry {
    condition: String,
    #[serde(default)]
    entity: Option<String>,
    #[serde(default)]
    after_secs: Option<f32>,
    #[serde(default, rename = "action")]
    actions: Vec<RawActionEntry>,
}

#[derive(Deserialize)]
struct RawActionEntry {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    mandatory: Option<bool>,
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
    #[serde(default, rename = "trigger")]
    triggers: Vec<RawTriggerEntry>,
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
    /// Ordered list of triggers declared in the scenario.
    pub triggers: Vec<Trigger>,
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

    // Parse triggers.
    let mut triggers = Vec::new();
    for raw_trigger in raw.triggers {
        let condition = match raw_trigger.condition.as_str() {
            "on_destroyed" => {
                let entity_name = raw_trigger.entity.ok_or_else(|| {
                    "Trigger 'on_destroyed' requires an 'entity' field".to_string()
                })?;
                TriggerCondition::OnDestroyed { entity_name }
            }
            "on_attacked" => {
                let entity_name = raw_trigger.entity.ok_or_else(|| {
                    "Trigger 'on_attacked' requires an 'entity' field".to_string()
                })?;
                TriggerCondition::OnAttacked { entity_name }
            }
            "on_timer" => {
                let after_secs = raw_trigger.after_secs.ok_or_else(|| {
                    "Trigger 'on_timer' requires an 'after_secs' field".to_string()
                })?;
                TriggerCondition::OnTimer { after_secs }
            }
            other => {
                return Err(format!("Unknown trigger condition '{}'", other));
            }
        };

        let mut actions = Vec::new();
        for raw_action in raw_trigger.actions {
            let action = match raw_action.kind.as_str() {
                "load_scenario" => {
                    let path = raw_action.path.ok_or_else(|| {
                        "Action 'load_scenario' requires a 'path' field".to_string()
                    })?;
                    TriggerAction::LoadScenario { path }
                }
                "add_objective" => {
                    let id = raw_action.id.ok_or_else(|| {
                        "Action 'add_objective' requires an 'id' field".to_string()
                    })?;
                    let text = raw_action.text.ok_or_else(|| {
                        "Action 'add_objective' requires a 'text' field".to_string()
                    })?;
                    let mandatory = raw_action.mandatory.unwrap_or(false);
                    TriggerAction::AddObjective { id, text, mandatory }
                }
                "complete_objective" => {
                    let id = raw_action.id.ok_or_else(|| {
                        "Action 'complete_objective' requires an 'id' field".to_string()
                    })?;
                    TriggerAction::CompleteObjective { id }
                }
                "fail_objective" => {
                    let id = raw_action.id.ok_or_else(|| {
                        "Action 'fail_objective' requires an 'id' field".to_string()
                    })?;
                    TriggerAction::FailObjective { id }
                }
                other => {
                    return Err(format!("Unknown trigger action '{}'", other));
                }
            };
            actions.push(action);
        }

        triggers.push(Trigger { condition, actions });
    }

    Ok(ScenarioConfig { spawns, name_to_uuid, triggers })
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

/// Create a `Vec<TriggerState>` from a parsed `ScenarioConfig`, with all triggers unfired.
/// Call this when a scenario is first activated to get its mutable runtime state.
pub fn trigger_states_from_config(config: &ScenarioConfig) -> Vec<TriggerState> {
    config
        .triggers
        .iter()
        .map(|t| TriggerState { trigger: t.clone(), fired: false })
        .collect()
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

    // ── Trigger parsing ────────────────────────────────────────────────────

    // Cycle 8: parse on_destroyed trigger
    #[test]
    fn parse_on_destroyed_trigger() {
        let toml = r#"
[[spawn]]
name = "station_alpha"
entity_path = "entities/station.toml"
position = [0.0, 0.0, 0.0]

[[trigger]]
condition = "on_destroyed"
entity = "station_alpha"

[[trigger.action]]
type = "load_scenario"
path = "scenarios/phase2.toml"
"#;
        let config = parse_scenario(toml).unwrap();
        assert_eq!(config.triggers.len(), 1);
        assert_eq!(
            config.triggers[0].condition,
            TriggerCondition::OnDestroyed { entity_name: "station_alpha".to_string() }
        );
        assert_eq!(config.triggers[0].actions.len(), 1);
        assert_eq!(
            config.triggers[0].actions[0],
            TriggerAction::LoadScenario { path: "scenarios/phase2.toml".to_string() }
        );
    }

    // Cycle 9: parse on_timer trigger
    #[test]
    fn parse_on_timer_trigger() {
        let toml = r#"
[[trigger]]
condition = "on_timer"
after_secs = 30.0

[[trigger.action]]
type = "load_scenario"
path = "scenarios/timeout.toml"
"#;
        let config = parse_scenario(toml).unwrap();
        assert_eq!(config.triggers.len(), 1);
        assert_eq!(
            config.triggers[0].condition,
            TriggerCondition::OnTimer { after_secs: 30.0 }
        );
    }

    // Cycle 10: parse on_attacked trigger
    #[test]
    fn parse_on_attacked_trigger() {
        let toml = r#"
[[spawn]]
name = "convoy"
entity_path = "entities/station.toml"
position = [0.0, 0.0, 0.0]

[[trigger]]
condition = "on_attacked"
entity = "convoy"

[[trigger.action]]
type = "load_scenario"
path = "scenarios/reinforcements.toml"
"#;
        let config = parse_scenario(toml).unwrap();
        assert_eq!(
            config.triggers[0].condition,
            TriggerCondition::OnAttacked { entity_name: "convoy".to_string() }
        );
    }

    // Cycle 11: on_destroyed fires when matching entity destroyed
    #[test]
    fn on_destroyed_fires_when_entity_destroyed() {
        let station_uuid = "uuid-station-1".to_string();
        let mut name_to_uuid = HashMap::new();
        name_to_uuid.insert("station_alpha".to_string(), station_uuid.clone());

        let trigger = Trigger {
            condition: TriggerCondition::OnDestroyed { entity_name: "station_alpha".to_string() },
            actions: vec![TriggerAction::LoadScenario { path: "scenarios/phase2.toml".to_string() }],
        };
        let mut states = vec![TriggerState { trigger, fired: false }];

        let events = vec![WorldEvent::Destroyed { uuid: station_uuid.clone() }];
        let fired = evaluate_triggers(&mut states, &events, &name_to_uuid);

        assert_eq!(fired.len(), 1);
        assert_eq!(
            fired[0].actions[0],
            TriggerAction::LoadScenario { path: "scenarios/phase2.toml".to_string() }
        );
    }

    // Cycle 12: on_destroyed does NOT fire for a different UUID
    #[test]
    fn on_destroyed_does_not_fire_for_different_entity() {
        let mut name_to_uuid = HashMap::new();
        name_to_uuid.insert("station_alpha".to_string(), "uuid-alpha".to_string());

        let trigger = Trigger {
            condition: TriggerCondition::OnDestroyed { entity_name: "station_alpha".to_string() },
            actions: vec![TriggerAction::LoadScenario { path: "scenarios/phase2.toml".to_string() }],
        };
        let mut states = vec![TriggerState { trigger, fired: false }];

        let events = vec![WorldEvent::Destroyed { uuid: "uuid-beta".to_string() }];
        let fired = evaluate_triggers(&mut states, &events, &name_to_uuid);

        assert_eq!(fired.len(), 0);
    }

    // Cycle 13: single-shot — trigger fires only once even given repeated events
    #[test]
    fn trigger_fires_only_once_single_shot() {
        let mut name_to_uuid = HashMap::new();
        name_to_uuid.insert("station_alpha".to_string(), "uuid-alpha".to_string());

        let trigger = Trigger {
            condition: TriggerCondition::OnDestroyed { entity_name: "station_alpha".to_string() },
            actions: vec![TriggerAction::LoadScenario { path: "scenarios/phase2.toml".to_string() }],
        };
        let mut states = vec![TriggerState { trigger, fired: false }];

        let events = vec![WorldEvent::Destroyed { uuid: "uuid-alpha".to_string() }];

        // First evaluation fires.
        let fired1 = evaluate_triggers(&mut states, &events, &name_to_uuid);
        assert_eq!(fired1.len(), 1);

        // Second evaluation with same events: must not fire again.
        let fired2 = evaluate_triggers(&mut states, &events, &name_to_uuid);
        assert_eq!(fired2.len(), 0);
    }

    // Cycle 14: on_timer fires when elapsed time exceeds threshold
    #[test]
    fn on_timer_fires_when_elapsed_exceeds_threshold() {
        let name_to_uuid = HashMap::new();
        let trigger = Trigger {
            condition: TriggerCondition::OnTimer { after_secs: 10.0 },
            actions: vec![TriggerAction::LoadScenario { path: "scenarios/late.toml".to_string() }],
        };
        let mut states = vec![TriggerState { trigger, fired: false }];

        // Below threshold — no fire.
        let events_before = vec![WorldEvent::TimerElapsed { elapsed_secs: 9.99 }];
        let fired = evaluate_triggers(&mut states, &events_before, &name_to_uuid);
        assert_eq!(fired.len(), 0);

        // At/above threshold — fires.
        let events_after = vec![WorldEvent::TimerElapsed { elapsed_secs: 10.0 }];
        let fired = evaluate_triggers(&mut states, &events_after, &name_to_uuid);
        assert_eq!(fired.len(), 1);
    }

    // Cycle 15: on_attacked fires when entity is attacked
    #[test]
    fn on_attacked_fires_when_entity_attacked() {
        let mut name_to_uuid = HashMap::new();
        name_to_uuid.insert("convoy".to_string(), "uuid-convoy".to_string());

        let trigger = Trigger {
            condition: TriggerCondition::OnAttacked { entity_name: "convoy".to_string() },
            actions: vec![TriggerAction::LoadScenario { path: "scenarios/reinforcements.toml".to_string() }],
        };
        let mut states = vec![TriggerState { trigger, fired: false }];

        let events = vec![WorldEvent::Attacked {
            uuid: "uuid-convoy".to_string(),
            attacker_uuid: "uuid-player".to_string(),
        }];
        let fired = evaluate_triggers(&mut states, &events, &name_to_uuid);
        assert_eq!(fired.len(), 1);
    }

    // Cycle 16: parameter substitution in load_scenario path
    #[test]
    fn load_scenario_path_substitutes_entity_name_params() {
        let mut name_to_uuid = HashMap::new();
        name_to_uuid.insert("station_alpha".to_string(), "uuid-abc-123".to_string());

        let trigger = Trigger {
            condition: TriggerCondition::OnDestroyed { entity_name: "station_alpha".to_string() },
            actions: vec![TriggerAction::LoadScenario { path: "scenarios/$station_alpha.toml".to_string() }],
        };
        let mut states = vec![TriggerState { trigger, fired: false }];

        let events = vec![WorldEvent::Destroyed { uuid: "uuid-abc-123".to_string() }];
        let fired = evaluate_triggers(&mut states, &events, &name_to_uuid);

        assert_eq!(fired.len(), 1);
        assert_eq!(
            fired[0].actions[0],
            TriggerAction::LoadScenario { path: "scenarios/uuid-abc-123.toml".to_string() }
        );
    }

    // Cycle 17: scenario with mixed spawns and triggers parses correctly
    #[test]
    fn scenario_with_spawns_and_triggers_parses_correctly() {
        let toml = r#"
[[spawn]]
name = "station_bravo"
entity_path = "entities/station.toml"
position = [50.0, 0.0, 50.0]

[[trigger]]
condition = "on_destroyed"
entity = "station_bravo"

[[trigger.action]]
type = "load_scenario"
path = "scenarios/follow_on.toml"
"#;
        let config = parse_scenario(toml).unwrap();
        assert_eq!(config.spawns.len(), 1);
        assert_eq!(config.triggers.len(), 1);
    }

    // Cycle 18: multiple triggers, only matching one fires
    #[test]
    fn only_matching_trigger_fires() {
        let mut name_to_uuid = HashMap::new();
        name_to_uuid.insert("station_a".to_string(), "uuid-a".to_string());
        name_to_uuid.insert("station_b".to_string(), "uuid-b".to_string());

        let trigger_a = Trigger {
            condition: TriggerCondition::OnDestroyed { entity_name: "station_a".to_string() },
            actions: vec![TriggerAction::LoadScenario { path: "scenarios/a.toml".to_string() }],
        };
        let trigger_b = Trigger {
            condition: TriggerCondition::OnDestroyed { entity_name: "station_b".to_string() },
            actions: vec![TriggerAction::LoadScenario { path: "scenarios/b.toml".to_string() }],
        };
        let mut states = vec![
            TriggerState { trigger: trigger_a, fired: false },
            TriggerState { trigger: trigger_b, fired: false },
        ];

        let events = vec![WorldEvent::Destroyed { uuid: "uuid-a".to_string() }];
        let fired = evaluate_triggers(&mut states, &events, &name_to_uuid);

        assert_eq!(fired.len(), 1);
        assert_eq!(
            fired[0].actions[0],
            TriggerAction::LoadScenario { path: "scenarios/a.toml".to_string() }
        );
    }

    // Cycle 19: unknown entity in on_destroyed condition doesn't fire (name not in map)
    #[test]
    fn on_destroyed_does_not_fire_if_entity_name_unknown() {
        let name_to_uuid = HashMap::new(); // empty — name not registered

        let trigger = Trigger {
            condition: TriggerCondition::OnDestroyed { entity_name: "ghost".to_string() },
            actions: vec![TriggerAction::LoadScenario { path: "scenarios/ghost.toml".to_string() }],
        };
        let mut states = vec![TriggerState { trigger, fired: false }];

        let events = vec![WorldEvent::Destroyed { uuid: "some-uuid".to_string() }];
        let fired = evaluate_triggers(&mut states, &events, &name_to_uuid);

        assert_eq!(fired.len(), 0);
    }

    // Cycle 20: trigger_states_from_config helper creates unfired states
    #[test]
    fn trigger_states_from_config_creates_unfired_states() {
        let toml = r#"
[[trigger]]
condition = "on_timer"
after_secs = 5.0

[[trigger.action]]
type = "load_scenario"
path = "scenarios/next.toml"
"#;
        let config = parse_scenario(toml).unwrap();
        let states = trigger_states_from_config(&config);
        assert_eq!(states.len(), 1);
        assert!(!states[0].fired);
    }

    // ── Cycles 21-23: objective actions parsed from TOML ──────────────────

    // Cycle 21: parse add_objective action
    #[test]
    fn parse_add_objective_action() {
        let toml = r#"
[[trigger]]
condition = "on_timer"
after_secs = 5.0

[[trigger.action]]
type = "add_objective"
id = "obj-1"
text = "Destroy the convoy"
mandatory = true
"#;
        let config = parse_scenario(toml).unwrap();
        assert_eq!(config.triggers[0].actions.len(), 1);
        assert_eq!(
            config.triggers[0].actions[0],
            TriggerAction::AddObjective {
                id: "obj-1".to_string(),
                text: "Destroy the convoy".to_string(),
                mandatory: true,
            }
        );
    }

    #[test]
    fn parse_add_objective_defaults_mandatory_to_false() {
        let toml = r#"
[[trigger]]
condition = "on_timer"
after_secs = 1.0

[[trigger.action]]
type = "add_objective"
id = "opt-1"
text = "Scan the debris"
"#;
        let config = parse_scenario(toml).unwrap();
        assert_eq!(
            config.triggers[0].actions[0],
            TriggerAction::AddObjective {
                id: "opt-1".to_string(),
                text: "Scan the debris".to_string(),
                mandatory: false,
            }
        );
    }

    // Cycle 22: parse complete_objective action
    #[test]
    fn parse_complete_objective_action() {
        let toml = r#"
[[spawn]]
name = "convoy"
entity_path = "entities/station.toml"
position = [0.0, 0.0, 0.0]

[[trigger]]
condition = "on_destroyed"
entity = "convoy"

[[trigger.action]]
type = "complete_objective"
id = "obj-1"
"#;
        let config = parse_scenario(toml).unwrap();
        assert_eq!(
            config.triggers[0].actions[0],
            TriggerAction::CompleteObjective { id: "obj-1".to_string() }
        );
    }

    // Cycle 23: parse fail_objective action
    #[test]
    fn parse_fail_objective_action() {
        let toml = r#"
[[trigger]]
condition = "on_timer"
after_secs = 60.0

[[trigger.action]]
type = "fail_objective"
id = "obj-2"
"#;
        let config = parse_scenario(toml).unwrap();
        assert_eq!(
            config.triggers[0].actions[0],
            TriggerAction::FailObjective { id: "obj-2".to_string() }
        );
    }

    // Cycle 24: evaluate_triggers returns AddObjective action
    #[test]
    fn evaluate_triggers_returns_add_objective_action() {
        let name_to_uuid = HashMap::new();
        let trigger = Trigger {
            condition: TriggerCondition::OnTimer { after_secs: 5.0 },
            actions: vec![TriggerAction::AddObjective {
                id: "obj-1".to_string(),
                text: "Destroy convoy".to_string(),
                mandatory: true,
            }],
        };
        let mut states = vec![TriggerState { trigger, fired: false }];
        let events = vec![WorldEvent::TimerElapsed { elapsed_secs: 5.0 }];
        let fired = evaluate_triggers(&mut states, &events, &name_to_uuid);

        assert_eq!(fired.len(), 1);
        assert_eq!(
            fired[0].actions[0],
            TriggerAction::AddObjective {
                id: "obj-1".to_string(),
                text: "Destroy convoy".to_string(),
                mandatory: true,
            }
        );
    }

    // Cycle 25: tracer end-to-end — add, complete, unload cleans active
    // This test exercises the full objective lifecycle across module boundaries.
    #[test]
    fn full_objective_lifecycle_add_complete_unload() {
        use crate::objectives::ObjectiveManager;
        use crate::messages::ObjectiveStatus;

        let mut mgr = ObjectiveManager::new();

        // Trigger fires: add_objective
        mgr.add("obj-1", "Destroy the convoy", true, "scenario-a");
        assert_eq!(mgr.sorted_snapshots().len(), 1);
        assert_eq!(mgr.sorted_snapshots()[0].status, ObjectiveStatus::Active);

        // Second trigger fires: complete_objective
        mgr.complete("obj-1");
        assert_eq!(mgr.sorted_snapshots()[0].status, ObjectiveStatus::Completed);

        // Scenario unloads: active objectives removed, completed retained
        mgr.unload_scenario("scenario-a");
        assert_eq!(mgr.sorted_snapshots().len(), 1, "completed objective should be retained");
        assert_eq!(mgr.sorted_snapshots()[0].status, ObjectiveStatus::Completed);
    }
}
