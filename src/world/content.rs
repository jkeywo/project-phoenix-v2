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
    /// A `Hail` message arrived for `target_uuid`.
    Hailed { target_uuid: String },
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
    /// Fires when a `Hail` message arrives for the named entity.
    OnHailed { entity_name: String },
}

/// An action to execute when a trigger fires.
#[derive(Clone, Debug, PartialEq)]
pub enum TriggerAction {
    /// Load and activate a new scenario from `path`. Parameters (`$name`) are
    /// substituted before dispatch.
    LoadScenario { path: String },
    /// Unload an active scenario identified by `path`. On execution:
    /// (1) AI entities carrying `ScenarioOwner(path)` receive `on_scenario_unloaded`,
    /// (2) comms messages owned by the scenario are orphaned,
    /// (3) the scenario is removed from the active map. Owned entities persist.
    UnloadScenario { path: String },
    /// Add a new mission objective owned by the current scenario.
    AddObjective { id: String, text: String, mandatory: bool },
    /// Mark an active objective as completed.
    CompleteObjective { id: String },
    /// Mark an active objective as failed.
    FailObjective { id: String },
    /// Force a named AI entity into a specific state.
    ///
    /// - `entity`: spawn name (resolved to UUID via `name_to_uuid`).
    /// - `state`: state name as declared in the entity's `[behaviour]` config.
    /// - `target`: optional spawn name to write into the blackboard `target`.
    ///
    /// Blackboard semantics:
    /// - `state_entered_at` is reset to the current sim time.
    /// - `target` is overwritten only when `target` is `Some`.
    /// - `last_attacker` and `waypoint_index` are preserved.
    ///
    /// Load-time validation should confirm the state name is known in the
    /// entity's `BehaviourConfig`; runtime errors are logged and ignored.
    SetAiState { entity: String, state: String, target: Option<String> },
    /// Apply a float modifier to the target entity's `ShipModifiers`.
    ///
    /// - `entity`: spawn name resolved to UUID at runtime.
    /// - `tag`: identity tag; paired with the scenario ID forms `ModifierSource::Scenario { id, tag }`.
    /// - `slot`: which `ModifierSlot` to modify.
    /// - `bonus`: additive bonus (positive = buff, negative = debuff).
    ApplyModifier { entity: String, tag: String, slot: crate::messages::ModifierSlot, bonus: f32 },
    /// Remove a float modifier previously applied by this scenario+tag pair.
    ///
    /// - `entity`: spawn name resolved to UUID at runtime.
    /// - `tag`: identity tag matching the one used in `ApplyModifier`.
    /// - `slot`: which `ModifierSlot` to remove.
    RemoveModifier { entity: String, tag: String, slot: crate::messages::ModifierSlot },
    /// Set a boolean flag on the target entity's `ShipModifiers`.
    ///
    /// - `entity`: spawn name resolved to UUID at runtime.
    /// - `tag`: identity tag; paired with the scenario ID forms `ModifierSource::Scenario { id, tag }`.
    /// - `kind`: which `FlagKind` to set.
    ApplyFlag { entity: String, tag: String, kind: crate::flag_kind::FlagKind },
    /// Clear a boolean flag previously set by this scenario+tag pair.
    ///
    /// - `entity`: spawn name resolved to UUID at runtime.
    /// - `tag`: identity tag matching the one used in `ApplyFlag`.
    /// - `kind`: which `FlagKind` to clear.
    RemoveFlag { entity: String, tag: String, kind: crate::flag_kind::FlagKind },
    /// Apply an integer modifier to the target entity's `ShipModifiers`.
    ///
    /// - `entity`: spawn name resolved to UUID at runtime.
    /// - `tag`: identity tag; paired with the scenario ID forms `ModifierSource::Scenario { id, tag }`.
    /// - `slot`: which `IntModifierSlot` to modify.
    /// - `bonus`: additive integer bonus.
    ApplyIntModifier { entity: String, tag: String, slot: crate::modifiers::IntModifierSlot, bonus: i32 },
    /// Remove an integer modifier previously applied by this scenario+tag pair.
    ///
    /// - `entity`: spawn name resolved to UUID at runtime.
    /// - `tag`: identity tag matching the one used in `ApplyIntModifier`.
    /// - `slot`: which `IntModifierSlot` to remove.
    RemoveIntModifier { entity: String, tag: String, slot: crate::modifiers::IntModifierSlot },
    /// Transition the game to the GameOver phase with an optional message.
    GameOver { message: Option<String> },
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
    /// Path of the scenario that owns this trigger (e.g. `"scenarios/alpha.toml"`).
    pub scenario_path: String,
}

/// Result of evaluating triggers against a batch of world events.
#[derive(Clone, Debug, PartialEq)]
pub struct FiredTrigger {
    pub actions: Vec<TriggerAction>,
    /// Path of the scenario that owned the trigger that fired.
    pub scenario_path: String,
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
            results.push(FiredTrigger { actions, scenario_path: state.scenario_path.clone() });
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
        (TriggerCondition::OnHailed { entity_name }, WorldEvent::Hailed { target_uuid }) => {
            name_to_uuid.get(entity_name).map(|u| u == target_uuid).unwrap_or(false)
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
        TriggerAction::UnloadScenario { path } => {
            let resolved = substitute_params(path, name_to_uuid);
            TriggerAction::UnloadScenario { path: resolved }
        }
        // Objective, AI-state, modifier, and flag actions carry no path parameters — pass through unchanged.
        TriggerAction::AddObjective { .. }
        | TriggerAction::CompleteObjective { .. }
        | TriggerAction::FailObjective { .. }
        | TriggerAction::SetAiState { .. }
        | TriggerAction::ApplyModifier { .. }
        | TriggerAction::RemoveModifier { .. }
        | TriggerAction::ApplyFlag { .. }
        | TriggerAction::RemoveFlag { .. }
        | TriggerAction::ApplyIntModifier { .. }
        | TriggerAction::RemoveIntModifier { .. }
        | TriggerAction::GameOver { .. } => action.clone(),
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
    /// Used by `set_ai_state`: the spawn entity name to force into a new state.
    /// Also used by modifier/flag actions: the target spawn entity name.
    #[serde(default)]
    entity: Option<String>,
    /// Used by `set_ai_state`: the state name to transition into.
    #[serde(default)]
    state: Option<String>,
    /// Used by `set_ai_state`: optional spawn entity name to write into blackboard `target`.
    #[serde(default)]
    target: Option<String>,
    /// Used by modifier/flag actions: identity tag for `ModifierSource::Scenario`.
    #[serde(default)]
    tag: Option<String>,
    /// Used by `apply_modifier` / `remove_modifier`: which `ModifierSlot` to affect.
    #[serde(default)]
    slot: Option<String>,
    /// Used by `apply_modifier`: the additive float bonus value.
    #[serde(default)]
    bonus: Option<f32>,
    /// Used by `apply_int_modifier`: the additive integer bonus value.
    #[serde(default)]
    int_bonus: Option<i32>,
    /// Used by `apply_flag` / `remove_flag`: which `FlagKind` to affect.
    /// Named `flag_kind` in TOML to avoid shadowing the action's own `kind` field.
    #[serde(default, rename = "kind")]
    flag_kind: Option<String>,
    /// Used by `game_over`: the optional message to display.
    #[serde(default)]
    message: Option<String>,
}

// ── TOML-facing deserialization for comms blocks ──────────────────────────

#[derive(Deserialize)]
struct RawCommsFollowUp {
    message: String,
    #[serde(default, rename = "response")]
    responses: Vec<RawCommsResponse>,
}

#[derive(Deserialize)]
struct RawCommsResponse {
    text: String,
    #[serde(default, rename = "action")]
    actions: Vec<RawActionEntry>,
    #[serde(default)]
    follow_up: Option<RawCommsFollowUp>,
}

#[derive(Deserialize)]
struct RawCommsEntry {
    from: String,
    message: String,
    trigger: String,
    #[serde(default)]
    entity: Option<String>,
    #[serde(default, rename = "response")]
    responses: Vec<RawCommsResponse>,
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
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default, rename = "spawn")]
    spawns: Vec<RawSpawnEntry>,
    #[serde(default, rename = "trigger")]
    triggers: Vec<RawTriggerEntry>,
    #[serde(default, rename = "comms")]
    comms: Vec<RawCommsEntry>,
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
    /// Scenario title for display in the lobby.
    pub title: String,
    /// Scenario description / body text for display in the lobby.
    pub description: String,
    /// Ordered list of spawn entries.
    pub spawns: Vec<SpawnEntry>,
    /// Map from spawn `name` to its assigned runtime UUID.
    pub name_to_uuid: HashMap<String, String>,
    /// Ordered list of triggers declared in the scenario.
    pub triggers: Vec<Trigger>,
    /// Ordered list of comms dialogue templates declared in the scenario.
    pub comms: Vec<CommsTemplate>,
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

// ── Internal helpers ───────────────────────────────────────────────────────

fn parse_modifier_slot(s: &str) -> Result<crate::messages::ModifierSlot, String> {
    use crate::messages::ModifierSlot;
    match s {
        "MaxSpeed" => Ok(ModifierSlot::MaxSpeed),
        "MaxYawRate" => Ok(ModifierSlot::MaxYawRate),
        "RadarRange" => Ok(ModifierSlot::RadarRange),
        "PhaserDamage" => Ok(ModifierSlot::PhaserDamage),
        "HullDamageTaken" => Ok(ModifierSlot::HullDamageTaken),
        "RepairRate" => Ok(ModifierSlot::RepairRate),
        other => Err(format!("Unknown slot '{}'; valid values: MaxSpeed, MaxYawRate, RadarRange, PhaserDamage, HullDamageTaken, RepairRate", other)),
    }
}

fn parse_int_modifier_slot(s: &str) -> Result<crate::modifiers::IntModifierSlot, String> {
    use crate::modifiers::IntModifierSlot;
    match s {
        "RepairTeams" => Ok(IntModifierSlot::RepairTeams),
        other => Err(format!("Unknown int slot '{}'; valid values: RepairTeams", other)),
    }
}

fn parse_flag_kind(s: &str) -> Result<crate::flag_kind::FlagKind, String> {
    use crate::flag_kind::FlagKind;
    match s {
        "CommsJammed" => Ok(FlagKind::CommsJammed),
        "SensorBlind" => Ok(FlagKind::SensorBlind),
        other => Err(format!("Unknown kind '{}'; valid values: CommsJammed, SensorBlind", other)),
    }
}

fn parse_raw_actions(raw_actions: &[RawActionEntry]) -> Result<Vec<TriggerAction>, String> {
    let mut actions = Vec::new();
    for raw_action in raw_actions {
        let action = match raw_action.kind.as_str() {
            "load_scenario" => {
                let path = raw_action.path.clone().ok_or_else(|| {
                    "Action 'load_scenario' requires a 'path' field".to_string()
                })?;
                TriggerAction::LoadScenario { path }
            }
            "unload_scenario" => {
                let path = raw_action.path.clone().ok_or_else(|| {
                    "Action 'unload_scenario' requires a 'path' field".to_string()
                })?;
                TriggerAction::UnloadScenario { path }
            }
            "add_objective" => {
                let id = raw_action.id.clone().ok_or_else(|| {
                    "Action 'add_objective' requires an 'id' field".to_string()
                })?;
                let text = raw_action.text.clone().ok_or_else(|| {
                    "Action 'add_objective' requires a 'text' field".to_string()
                })?;
                let mandatory = raw_action.mandatory.unwrap_or(false);
                TriggerAction::AddObjective { id, text, mandatory }
            }
            "complete_objective" => {
                let id = raw_action.id.clone().ok_or_else(|| {
                    "Action 'complete_objective' requires an 'id' field".to_string()
                })?;
                TriggerAction::CompleteObjective { id }
            }
            "fail_objective" => {
                let id = raw_action.id.clone().ok_or_else(|| {
                    "Action 'fail_objective' requires an 'id' field".to_string()
                })?;
                TriggerAction::FailObjective { id }
            }
            "set_ai_state" => {
                let entity = raw_action.entity.clone().ok_or_else(|| {
                    "Action 'set_ai_state' requires an 'entity' field".to_string()
                })?;
                let state = raw_action.state.clone().ok_or_else(|| {
                    "Action 'set_ai_state' requires a 'state' field".to_string()
                })?;
                let target = raw_action.target.clone();
                TriggerAction::SetAiState { entity, state, target }
            }
            "apply_modifier" => {
                let entity = raw_action.entity.clone().ok_or_else(|| {
                    "Action 'apply_modifier' requires an 'entity' field".to_string()
                })?;
                let tag = raw_action.tag.clone().ok_or_else(|| {
                    "Action 'apply_modifier' requires a 'tag' field".to_string()
                })?;
                let slot_str = raw_action.slot.as_deref().ok_or_else(|| {
                    "Action 'apply_modifier' requires a 'slot' field".to_string()
                })?;
                let slot = parse_modifier_slot(slot_str)?;
                let bonus = raw_action.bonus.ok_or_else(|| {
                    "Action 'apply_modifier' requires a 'bonus' field".to_string()
                })?;
                TriggerAction::ApplyModifier { entity, tag, slot, bonus }
            }
            "remove_modifier" => {
                let entity = raw_action.entity.clone().ok_or_else(|| {
                    "Action 'remove_modifier' requires an 'entity' field".to_string()
                })?;
                let tag = raw_action.tag.clone().ok_or_else(|| {
                    "Action 'remove_modifier' requires a 'tag' field".to_string()
                })?;
                let slot_str = raw_action.slot.as_deref().ok_or_else(|| {
                    "Action 'remove_modifier' requires a 'slot' field".to_string()
                })?;
                let slot = parse_modifier_slot(slot_str)?;
                TriggerAction::RemoveModifier { entity, tag, slot }
            }
            "apply_flag" => {
                let entity = raw_action.entity.clone().ok_or_else(|| {
                    "Action 'apply_flag' requires an 'entity' field".to_string()
                })?;
                let tag = raw_action.tag.clone().ok_or_else(|| {
                    "Action 'apply_flag' requires a 'tag' field".to_string()
                })?;
                let kind_str = raw_action.flag_kind.as_deref().ok_or_else(|| {
                    "Action 'apply_flag' requires a 'kind' field".to_string()
                })?;
                let kind = parse_flag_kind(kind_str)?;
                TriggerAction::ApplyFlag { entity, tag, kind }
            }
            "remove_flag" => {
                let entity = raw_action.entity.clone().ok_or_else(|| {
                    "Action 'remove_flag' requires an 'entity' field".to_string()
                })?;
                let tag = raw_action.tag.clone().ok_or_else(|| {
                    "Action 'remove_flag' requires a 'tag' field".to_string()
                })?;
                let kind_str = raw_action.flag_kind.as_deref().ok_or_else(|| {
                    "Action 'remove_flag' requires a 'kind' field".to_string()
                })?;
                let kind = parse_flag_kind(kind_str)?;
                TriggerAction::RemoveFlag { entity, tag, kind }
            }
            "apply_int_modifier" => {
                let entity = raw_action.entity.clone().ok_or_else(|| {
                    "Action 'apply_int_modifier' requires an 'entity' field".to_string()
                })?;
                let tag = raw_action.tag.clone().ok_or_else(|| {
                    "Action 'apply_int_modifier' requires a 'tag' field".to_string()
                })?;
                let slot_str = raw_action.slot.as_deref().ok_or_else(|| {
                    "Action 'apply_int_modifier' requires a 'slot' field".to_string()
                })?;
                let slot = parse_int_modifier_slot(slot_str)?;
                let bonus = raw_action.int_bonus.ok_or_else(|| {
                    "Action 'apply_int_modifier' requires an 'int_bonus' field".to_string()
                })?;
                TriggerAction::ApplyIntModifier { entity, tag, slot, bonus }
            }
            "remove_int_modifier" => {
                let entity = raw_action.entity.clone().ok_or_else(|| {
                    "Action 'remove_int_modifier' requires an 'entity' field".to_string()
                })?;
                let tag = raw_action.tag.clone().ok_or_else(|| {
                    "Action 'remove_int_modifier' requires a 'tag' field".to_string()
                })?;
                let slot_str = raw_action.slot.as_deref().ok_or_else(|| {
                    "Action 'remove_int_modifier' requires a 'slot' field".to_string()
                })?;
                let slot = parse_int_modifier_slot(slot_str)?;
                TriggerAction::RemoveIntModifier { entity, tag, slot }
            }
            "game_over" => {
                TriggerAction::GameOver { message: raw_action.message.clone() }
            }
            other => {
                return Err(format!("Unknown trigger action '{}'", other));
            }
        };
        actions.push(action);
    }
    Ok(actions)
}

fn parse_comms_responses(raw_responses: &[RawCommsResponse]) -> Result<Vec<CommsResponse>, String> {
    let mut responses = Vec::new();
    for raw_resp in raw_responses {
        let actions = parse_raw_actions(&raw_resp.actions)?;
        let follow_up = if let Some(ref raw_fu) = raw_resp.follow_up {
            let fu_responses = parse_comms_responses(&raw_fu.responses)?;
            Some(CommsDialogueNode { body: raw_fu.message.clone(), responses: fu_responses })
        } else {
            None
        };
        responses.push(CommsResponse { text: raw_resp.text.clone(), actions, follow_up });
    }
    Ok(responses)
}

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
            "on_hailed" => {
                let entity_name = raw_trigger.entity.ok_or_else(|| {
                    "Trigger 'on_hailed' requires an 'entity' field".to_string()
                })?;
                TriggerCondition::OnHailed { entity_name }
            }
            other => {
                return Err(format!("Unknown trigger condition '{}'", other));
            }
        };

        let actions = parse_raw_actions(&raw_trigger.actions)?;

        triggers.push(Trigger { condition, actions });
    }

    // Parse comms blocks.
    let mut comms = Vec::new();
    for raw_comms in raw.comms {
        let trigger = match raw_comms.trigger.as_str() {
            "on_hailed" => {
                let entity_name = raw_comms.entity.ok_or_else(|| {
                    "Comms block 'on_hailed' requires an 'entity' field".to_string()
                })?;
                TriggerCondition::OnHailed { entity_name }
            }
            "on_destroyed" => {
                let entity_name = raw_comms.entity.ok_or_else(|| {
                    "Comms block 'on_destroyed' requires an 'entity' field".to_string()
                })?;
                TriggerCondition::OnDestroyed { entity_name }
            }
            "on_attacked" => {
                let entity_name = raw_comms.entity.ok_or_else(|| {
                    "Comms block 'on_attacked' requires an 'entity' field".to_string()
                })?;
                TriggerCondition::OnAttacked { entity_name }
            }
            other => {
                return Err(format!("Unknown comms trigger '{}'", other));
            }
        };

        let responses = parse_comms_responses(&raw_comms.responses)?;
        let node = CommsDialogueNode { body: raw_comms.message, responses };
        comms.push(CommsTemplate { from: raw_comms.from, trigger, node });
    }

    Ok(ScenarioConfig { title: raw.title.unwrap_or_default(), description: raw.description.unwrap_or_default(), spawns, name_to_uuid, triggers, comms })
}

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
pub fn trigger_states_from_config(config: &ScenarioConfig, scenario_path: &str) -> Vec<TriggerState> {
    config
        .triggers
        .iter()
        .map(|t| TriggerState { trigger: t.clone(), fired: false, scenario_path: scenario_path.to_string() })
        .collect()
}

// ── ScenarioManager ────────────────────────────────────────────────────────

/// Per-path runtime state for one active scenario.
#[derive(Clone, Debug, Default)]
pub struct ScenarioRuntime {
    /// Mutable per-trigger runtime state (fired flag).
    pub trigger_states: Vec<TriggerState>,
    /// Mutable per-template runtime state (fired flag).
    pub comms_template_states: Vec<CommsTemplateState>,
    /// Active in-flight dialogues keyed by CommsMessage id.
    pub active_dialogues: HashMap<String, ActiveDialogue>,
    /// Named-spawn → UUID mapping (populated from ScenarioConfig).
    pub name_to_uuid: HashMap<String, String>,
    /// Hailable contacts derived from scenario spawns.
    pub contacts: Vec<crate::messages::CommsContact>,
}

/// Additive multi-scenario registry.
///
/// Scenarios are keyed by their path string. Loading the same path twice is a
/// no-op. Unloading a path removes it from the map and triggers cleanup of
/// comms and objectives in the associated resources.
#[derive(Default)]
pub struct ScenarioManager {
    /// Active scenarios keyed by scenario path.
    pub scenarios: HashMap<String, ScenarioRuntime>,
}

impl ScenarioManager {
    /// Load a scenario if not already active. Returns `true` if newly inserted, `false` if already present.
    pub fn load_scenario(&mut self, path: impl Into<String>) -> bool {
        let path = path.into();
        if self.scenarios.contains_key(&path) {
            return false;
        }
        self.scenarios.insert(path, ScenarioRuntime::default());
        true
    }

    /// Unload a scenario. Returns the removed `ScenarioRuntime` if it was present, or `None`.
    pub fn unload_scenario(&mut self, path: &str) -> Option<ScenarioRuntime> {
        self.scenarios.remove(path)
    }

    /// Returns `true` if the scenario at `path` is currently active.
    pub fn is_active(&self, path: &str) -> bool {
        self.scenarios.contains_key(path)
    }

    /// Number of currently active scenarios.
    pub fn len(&self) -> usize {
        self.scenarios.len()
    }

    /// `true` if no scenarios are active.
    pub fn is_empty(&self) -> bool {
        self.scenarios.is_empty()
    }
}

// ── Comms dialogue types ───────────────────────────────────────────────────

/// A single response option within a comms dialogue node.
#[derive(Clone, Debug, PartialEq)]
pub struct CommsResponse {
    /// Display text shown on the response button.
    pub text: String,
    /// Actions executed when this response is chosen.
    pub actions: Vec<TriggerAction>,
    /// Optional inline follow-up message displayed after this response is chosen.
    pub follow_up: Option<CommsDialogueNode>,
}

/// A single node in an inline dialogue tree: a message body with response options.
#[derive(Clone, Debug, PartialEq)]
pub struct CommsDialogueNode {
    /// The message body text.
    pub body: String,
    /// Available response options. Empty if no response is expected.
    pub responses: Vec<CommsResponse>,
}

/// A comms template: a root dialogue node associated with a trigger condition.
/// Parsed from a `[[comms]]` block in the scenario TOML.
#[derive(Clone, Debug, PartialEq)]
pub struct CommsTemplate {
    /// Spawn entity name whose UUID is the sender of the comms message.
    pub from: String,
    /// The trigger condition that fires this template.
    pub trigger: TriggerCondition,
    /// The root dialogue node.
    pub node: CommsDialogueNode,
}

/// Runtime state for one active dialogue conversation.
#[derive(Clone, Debug)]
pub struct ActiveDialogue {
    /// ID of the `CommsMessage` that was sent to the Comms inbox for this dialogue.
    pub message_id: String,
    /// The current dialogue node being presented.
    pub current_node: CommsDialogueNode,
    /// Path of the scenario that owns this dialogue (for scoping inbox/objectives).
    pub scenario_path: String,
}

/// Runtime state for one comms template — tracks whether it has already fired.
#[derive(Clone, Debug)]
pub struct CommsTemplateState {
    pub template: CommsTemplate,
    pub fired: bool,
    /// Path of the scenario that owns this template.
    pub scenario_path: String,
}

/// A comms template that fired in response to world events.
#[derive(Clone, Debug, PartialEq)]
pub struct FiredCommsTemplate {
    /// The sender entity name from the template.
    pub from: String,
    /// The root dialogue node to inject into the inbox.
    pub node: CommsDialogueNode,
    /// Path of the scenario that owns this template.
    pub scenario_path: String,
}

/// Create a `Vec<CommsTemplateState>` from a parsed `ScenarioConfig`.
pub fn comms_template_states_from_config(config: &ScenarioConfig, scenario_path: &str) -> Vec<CommsTemplateState> {
    config
        .comms
        .iter()
        .map(|t| CommsTemplateState { template: t.clone(), fired: false, scenario_path: scenario_path.to_string() })
        .collect()
}

/// Evaluate all comms templates in `states` against the given `events`.
///
/// Each template fires at most once (single-shot). When a template fires its
/// `fired` flag is set to `true` and a `FiredCommsTemplate` is collected.
///
/// Returns the list of `FiredCommsTemplate` values produced in this call.
pub fn evaluate_comms_templates(
    states: &mut Vec<CommsTemplateState>,
    events: &[WorldEvent],
    name_to_uuid: &HashMap<String, String>,
) -> Vec<FiredCommsTemplate> {
    let mut results = Vec::new();

    for state in states.iter_mut() {
        if state.fired {
            continue;
        }

        let fires = events.iter().any(|event| {
            condition_matches(&state.template.trigger, event, name_to_uuid)
        });

        if fires {
            state.fired = true;
            results.push(FiredCommsTemplate {
                from: state.template.from.clone(),
                node: state.template.node.clone(),
                scenario_path: state.scenario_path.clone(),
            });
        }
    }

    results
}

/// Result returned by `process_response`.
#[derive(Clone, Debug, PartialEq)]
pub struct ProcessResponseResult {
    /// Actions from the chosen response to execute.
    pub actions: Vec<TriggerAction>,
    /// Follow-up dialogue node to present next, if any.
    pub follow_up: Option<CommsDialogueNode>,
}

/// Process a player's response to an active dialogue.
///
/// Looks up `message_id` in `dialogues`. If found and `response_index` is
/// valid, returns the response's actions and optional follow-up node.
/// Returns `None` if the message is not found or the index is out of range.
pub fn process_response(
    dialogues: &[ActiveDialogue],
    message_id: &str,
    response_index: usize,
) -> Option<ProcessResponseResult> {
    let dialogue = dialogues.iter().find(|d| d.message_id == message_id)?;
    let response = dialogue.current_node.responses.get(response_index)?;
    Some(ProcessResponseResult {
        actions: response.actions.clone(),
        follow_up: response.follow_up.clone(),
    })
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

    #[test]
    fn parse_scenario_with_title_and_description() {
        let toml = r#"
title = "First Contact"
description = "A mysterious signal has been detected near Starbase Alpha."

[[spawn]]
name = "station_alpha"
entity_path = "entities/station.toml"
position = [0.0, 0.0, 0.0]
"#;
        let config = parse_scenario(toml).unwrap();
        assert_eq!(config.title, "First Contact");
        assert_eq!(config.description, "A mysterious signal has been detected near Starbase Alpha.");
    }

    #[test]
    fn parse_scenario_without_title_uses_default() {
        let toml = r#"
[[spawn]]
name = "station_alpha"
entity_path = "entities/station.toml"
position = [0.0, 0.0, 0.0]
"#;
        let config = parse_scenario(toml).unwrap();
        assert_eq!(config.title, "");
        assert_eq!(config.description, "");
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
        let mut states = vec![TriggerState { trigger, fired: false, scenario_path: "test".to_string() }];

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
        let mut states = vec![TriggerState { trigger, fired: false, scenario_path: "test".to_string() }];

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
        let mut states = vec![TriggerState { trigger, fired: false, scenario_path: "test".to_string() }];

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
        let mut states = vec![TriggerState { trigger, fired: false, scenario_path: "test".to_string() }];

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
        let mut states = vec![TriggerState { trigger, fired: false, scenario_path: "test".to_string() }];

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
        let mut states = vec![TriggerState { trigger, fired: false, scenario_path: "test".to_string() }];

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
            TriggerState { trigger: trigger_a, fired: false, scenario_path: "test".to_string() },
            TriggerState { trigger: trigger_b, fired: false, scenario_path: "test".to_string() },
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
        let mut states = vec![TriggerState { trigger, fired: false, scenario_path: "test".to_string() }];

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
        let states = trigger_states_from_config(&config, "test");
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
        let mut states = vec![TriggerState { trigger, fired: false, scenario_path: "test".to_string() }];
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

    // ── Cycles 26-28: on_hailed trigger condition ─────────────────────────

    // Cycle 26: on_hailed fires when entity is hailed
    #[test]
    fn on_hailed_fires_when_entity_hailed() {
        let mut name_to_uuid = HashMap::new();
        name_to_uuid.insert("station_alpha".to_string(), "uuid-alpha".to_string());

        let trigger = Trigger {
            condition: TriggerCondition::OnHailed { entity_name: "station_alpha".to_string() },
            actions: vec![TriggerAction::AddObjective {
                id: "greet".to_string(),
                text: "Respond to hail".to_string(),
                mandatory: false,
            }],
        };
        let mut states = vec![TriggerState { trigger, fired: false, scenario_path: "test".to_string() }];

        let events = vec![WorldEvent::Hailed { target_uuid: "uuid-alpha".to_string() }];
        let fired = evaluate_triggers(&mut states, &events, &name_to_uuid);

        assert_eq!(fired.len(), 1);
    }

    // Cycle 27: on_hailed does not fire for a different entity
    #[test]
    fn on_hailed_does_not_fire_for_different_entity() {
        let mut name_to_uuid = HashMap::new();
        name_to_uuid.insert("station_alpha".to_string(), "uuid-alpha".to_string());

        let trigger = Trigger {
            condition: TriggerCondition::OnHailed { entity_name: "station_alpha".to_string() },
            actions: vec![],
        };
        let mut states = vec![TriggerState { trigger, fired: false, scenario_path: "test".to_string() }];

        let events = vec![WorldEvent::Hailed { target_uuid: "uuid-other".to_string() }];
        let fired = evaluate_triggers(&mut states, &events, &name_to_uuid);

        assert_eq!(fired.len(), 0);
    }

    // Cycle 28: on_hailed parses from TOML
    #[test]
    fn parse_on_hailed_trigger() {
        let toml = r#"
[[spawn]]
name = "starbase_one"
entity_path = "entities/station.toml"
position = [0.0, 0.0, 0.0]

[[trigger]]
condition = "on_hailed"
entity = "starbase_one"

[[trigger.action]]
type = "add_objective"
id = "hailed-obj"
text = "Respond to starbase"
"#;
        let config = parse_scenario(toml).unwrap();
        assert_eq!(config.triggers.len(), 1);
        assert_eq!(
            config.triggers[0].condition,
            TriggerCondition::OnHailed { entity_name: "starbase_one".to_string() }
        );
    }

    // ── Cycles 29-35: [[comms]] block parsing ─────────────────────────────

    // Cycle 29: parse basic [[comms]] block
    #[test]
    fn parse_comms_block_with_from_and_message() {
        let toml = r#"
[[spawn]]
name = "starbase_one"
entity_path = "entities/station.toml"
position = [0.0, 0.0, 0.0]

[[comms]]
from = "starbase_one"
message = "Greetings, Captain."
trigger = "on_hailed"
entity = "starbase_one"
"#;
        let config = parse_scenario(toml).unwrap();
        assert_eq!(config.comms.len(), 1);
        let tmpl = &config.comms[0];
        assert_eq!(tmpl.from, "starbase_one");
        assert_eq!(tmpl.node.body, "Greetings, Captain.");
        assert_eq!(
            tmpl.trigger,
            TriggerCondition::OnHailed { entity_name: "starbase_one".to_string() }
        );
    }

    // Cycle 30: empty scenario has no comms blocks
    #[test]
    fn empty_scenario_has_no_comms_blocks() {
        let config = parse_scenario("").unwrap();
        assert!(config.comms.is_empty());
    }

    // Cycle 31: parse [[comms]] with responses
    #[test]
    fn parse_comms_block_with_responses() {
        let toml = r#"
[[spawn]]
name = "starbase_one"
entity_path = "entities/station.toml"
position = [0.0, 0.0, 0.0]

[[comms]]
from = "starbase_one"
message = "Greetings, Captain. How can we help?"
trigger = "on_hailed"
entity = "starbase_one"

[[comms.response]]
text = "We need supplies"

[[comms.response]]
text = "Just checking in"
"#;
        let config = parse_scenario(toml).unwrap();
        let tmpl = &config.comms[0];
        assert_eq!(tmpl.node.responses.len(), 2);
        assert_eq!(tmpl.node.responses[0].text, "We need supplies");
        assert_eq!(tmpl.node.responses[1].text, "Just checking in");
    }

    // Cycle 32: parse response actions
    #[test]
    fn parse_comms_response_with_actions() {
        let toml = r#"
[[spawn]]
name = "starbase_one"
entity_path = "entities/station.toml"
position = [0.0, 0.0, 0.0]

[[comms]]
from = "starbase_one"
message = "Greetings, Captain."
trigger = "on_hailed"
entity = "starbase_one"

[[comms.response]]
text = "We need supplies"

[[comms.response.action]]
type = "add_objective"
id = "supplies"
text = "Get supplies from starbase"
mandatory = true
"#;
        let config = parse_scenario(toml).unwrap();
        let resp = &config.comms[0].node.responses[0];
        assert_eq!(resp.actions.len(), 1);
        assert_eq!(
            resp.actions[0],
            TriggerAction::AddObjective {
                id: "supplies".to_string(),
                text: "Get supplies from starbase".to_string(),
                mandatory: true,
            }
        );
    }

    // Cycle 33: parse inline follow_up branching
    #[test]
    fn parse_comms_response_with_follow_up() {
        let toml = r#"
[[spawn]]
name = "starbase_one"
entity_path = "entities/station.toml"
position = [0.0, 0.0, 0.0]

[[comms]]
from = "starbase_one"
message = "Greetings, Captain."
trigger = "on_hailed"
entity = "starbase_one"

[[comms.response]]
text = "We need supplies"

[comms.response.follow_up]
message = "We'll have them ready in 5 minutes."
"#;
        let config = parse_scenario(toml).unwrap();
        let resp = &config.comms[0].node.responses[0];
        assert!(resp.follow_up.is_some());
        let follow_up = resp.follow_up.as_ref().unwrap();
        assert_eq!(follow_up.body, "We'll have them ready in 5 minutes.");
        assert!(follow_up.responses.is_empty());
    }

    // Cycle 34: follow_up can itself have responses (deep branching)
    #[test]
    fn parse_follow_up_with_nested_responses() {
        let toml = r#"
[[spawn]]
name = "starbase_one"
entity_path = "entities/station.toml"
position = [0.0, 0.0, 0.0]

[[comms]]
from = "starbase_one"
message = "Greetings, Captain."
trigger = "on_hailed"
entity = "starbase_one"

[[comms.response]]
text = "We need supplies"

[comms.response.follow_up]
message = "What kind of supplies?"

[[comms.response.follow_up.response]]
text = "Food and water"

[[comms.response.follow_up.response]]
text = "Fuel cells"
"#;
        let config = parse_scenario(toml).unwrap();
        let follow_up = config.comms[0].node.responses[0].follow_up.as_ref().unwrap();
        assert_eq!(follow_up.responses.len(), 2);
        assert_eq!(follow_up.responses[0].text, "Food and water");
        assert_eq!(follow_up.responses[1].text, "Fuel cells");
    }

    // ── Cycles 35-39: process_response dialogue state machine ─────────────

    // Cycle 35: process_response returns actions for valid message and index
    #[test]
    fn process_response_executes_response_actions() {
        let node = CommsDialogueNode {
            body: "How can we help?".to_string(),
            responses: vec![
                CommsResponse {
                    text: "Supplies".to_string(),
                    actions: vec![TriggerAction::AddObjective {
                        id: "supplies".to_string(),
                        text: "Get supplies".to_string(),
                        mandatory: false,
                    }],
                    follow_up: None,
                },
            ],
        };
        let dialogues = vec![ActiveDialogue {
            message_id: "msg-1".to_string(),
            current_node: node,
            scenario_path: "test".to_string(),
        }];

        let result = process_response(&dialogues, "msg-1", 0).unwrap();
        assert_eq!(result.actions.len(), 1);
        assert_eq!(
            result.actions[0],
            TriggerAction::AddObjective {
                id: "supplies".to_string(),
                text: "Get supplies".to_string(),
                mandatory: false,
            }
        );
        assert!(result.follow_up.is_none());
    }

    // Cycle 36: process_response returns follow_up node when present
    #[test]
    fn process_response_returns_follow_up_when_present() {
        let follow_up_node = CommsDialogueNode {
            body: "We'll have them ready.".to_string(),
            responses: vec![],
        };
        let node = CommsDialogueNode {
            body: "How can we help?".to_string(),
            responses: vec![
                CommsResponse {
                    text: "Supplies".to_string(),
                    actions: vec![],
                    follow_up: Some(follow_up_node.clone()),
                },
            ],
        };
        let dialogues = vec![ActiveDialogue {
            message_id: "msg-1".to_string(),
            current_node: node,
            scenario_path: "test".to_string(),
        }];

        let result = process_response(&dialogues, "msg-1", 0).unwrap();
        assert_eq!(result.follow_up, Some(follow_up_node));
    }

    // Cycle 37: process_response returns None for unknown message_id
    #[test]
    fn process_response_returns_none_for_unknown_message_id() {
        let dialogues: Vec<ActiveDialogue> = vec![];
        let result = process_response(&dialogues, "ghost-id", 0);
        assert!(result.is_none());
    }

    // Cycle 38: process_response returns None for out-of-bounds response index
    #[test]
    fn process_response_returns_none_for_out_of_bounds_index() {
        let node = CommsDialogueNode {
            body: "Hello".to_string(),
            responses: vec![
                CommsResponse { text: "Hi".to_string(), actions: vec![], follow_up: None },
            ],
        };
        let dialogues = vec![ActiveDialogue {
            message_id: "msg-1".to_string(),
            current_node: node,
            scenario_path: "test".to_string(),
        }];

        let result = process_response(&dialogues, "msg-1", 99);
        assert!(result.is_none());
    }

    // Cycle 39: process_response with no follow_up returns None follow_up
    #[test]
    fn process_response_no_follow_up_returns_none() {
        let node = CommsDialogueNode {
            body: "Hello".to_string(),
            responses: vec![
                CommsResponse { text: "Ack".to_string(), actions: vec![], follow_up: None },
            ],
        };
        let dialogues = vec![ActiveDialogue {
            message_id: "msg-1".to_string(),
            current_node: node,
            scenario_path: "test".to_string(),
        }];

        let result = process_response(&dialogues, "msg-1", 0).unwrap();
        assert!(result.follow_up.is_none());
    }

    // ── Cycle 40: evaluate_comms_templates fires on on_attacked ──────────
    #[test]
    fn evaluate_comms_templates_fires_on_attacked() {
        let mut name_to_uuid = HashMap::new();
        name_to_uuid.insert("convoy".to_string(), "uuid-convoy".to_string());

        let template = CommsTemplate {
            from: "convoy".to_string(),
            trigger: TriggerCondition::OnAttacked { entity_name: "convoy".to_string() },
            node: CommsDialogueNode {
                body: "We are under attack! Please help!".to_string(),
                responses: vec![],
            },
        };
        let mut states = vec![CommsTemplateState { template, fired: false, scenario_path: "test".to_string() }];

        let events = vec![WorldEvent::Attacked {
            uuid: "uuid-convoy".to_string(),
            attacker_uuid: "uuid-player".to_string(),
        }];
        let fired = evaluate_comms_templates(&mut states, &events, &name_to_uuid);
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].from, "convoy");
        assert_eq!(fired[0].node.body, "We are under attack! Please help!");
    }

    // Cycle 41: comms template fires at most once (single-shot)
    #[test]
    fn evaluate_comms_templates_fires_at_most_once() {
        let mut name_to_uuid = HashMap::new();
        name_to_uuid.insert("convoy".to_string(), "uuid-convoy".to_string());

        let template = CommsTemplate {
            from: "convoy".to_string(),
            trigger: TriggerCondition::OnAttacked { entity_name: "convoy".to_string() },
            node: CommsDialogueNode { body: "Help!".to_string(), responses: vec![] },
        };
        let mut states = vec![CommsTemplateState { template, fired: false, scenario_path: "test".to_string() }];

        let events = vec![WorldEvent::Attacked {
            uuid: "uuid-convoy".to_string(),
            attacker_uuid: "uuid-player".to_string(),
        }];
        let first = evaluate_comms_templates(&mut states, &events, &name_to_uuid);
        let second = evaluate_comms_templates(&mut states, &events, &name_to_uuid);
        assert_eq!(first.len(), 1);
        assert_eq!(second.len(), 0);
    }

    // Cycle 42: non-matching event does not fire comms template
    #[test]
    fn evaluate_comms_templates_does_not_fire_for_unrelated_entity() {
        let mut name_to_uuid = HashMap::new();
        name_to_uuid.insert("convoy".to_string(), "uuid-convoy".to_string());

        let template = CommsTemplate {
            from: "convoy".to_string(),
            trigger: TriggerCondition::OnAttacked { entity_name: "convoy".to_string() },
            node: CommsDialogueNode { body: "Help!".to_string(), responses: vec![] },
        };
        let mut states = vec![CommsTemplateState { template, fired: false, scenario_path: "test".to_string() }];

        let events = vec![WorldEvent::Attacked {
            uuid: "uuid-other".to_string(),
            attacker_uuid: "uuid-player".to_string(),
        }];
        let fired = evaluate_comms_templates(&mut states, &events, &name_to_uuid);
        assert_eq!(fired.len(), 0);
    }

    // ── Cycles 43-48: SetAiState action ───────────────────────────────────

    // Cycle 43: parse set_ai_state action from TOML
    #[test]
    fn parse_set_ai_state_action_from_toml() {
        let toml = r#"
[[spawn]]
name = "raider_x"
entity_path = "entities/raider.toml"
position = [0.0, 0.0, 0.0]

[[trigger]]
condition = "on_destroyed"
entity = "raider_x"

[[trigger.action]]
type = "set_ai_state"
entity = "raider_x"
state = "pursuing"
"#;
        let config = parse_scenario(toml).unwrap();
        assert_eq!(config.triggers[0].actions.len(), 1);
        assert_eq!(
            config.triggers[0].actions[0],
            TriggerAction::SetAiState {
                entity: "raider_x".to_string(),
                state: "pursuing".to_string(),
                target: None,
            }
        );
    }

    // Cycle 44: parse set_ai_state action with optional target
    #[test]
    fn parse_set_ai_state_action_with_target() {
        let toml = r#"
[[spawn]]
name = "ambusher"
entity_path = "entities/raider.toml"
position = [0.0, 0.0, 0.0]

[[trigger]]
condition = "on_timer"
after_secs = 5.0

[[trigger.action]]
type = "set_ai_state"
entity = "ambusher"
state = "pursuing"
target = "player_ship"
"#;
        let config = parse_scenario(toml).unwrap();
        assert_eq!(
            config.triggers[0].actions[0],
            TriggerAction::SetAiState {
                entity: "ambusher".to_string(),
                state: "pursuing".to_string(),
                target: Some("player_ship".to_string()),
            }
        );
    }

    // Cycle 45: parse set_ai_state without entity field → error
    #[test]
    fn parse_set_ai_state_without_entity_returns_error() {
        let toml = r#"
[[trigger]]
condition = "on_timer"
after_secs = 1.0

[[trigger.action]]
type = "set_ai_state"
state = "pursuing"
"#;
        let result = parse_scenario(toml);
        assert!(result.is_err(), "missing entity should return error");
        let err = result.unwrap_err();
        assert!(err.contains("entity"), "error should mention 'entity': {err}");
    }

    // Cycle 46: parse set_ai_state without state field → error
    #[test]
    fn parse_set_ai_state_without_state_returns_error() {
        let toml = r#"
[[spawn]]
name = "raider"
entity_path = "entities/raider.toml"
position = [0.0, 0.0, 0.0]

[[trigger]]
condition = "on_timer"
after_secs = 1.0

[[trigger.action]]
type = "set_ai_state"
entity = "raider"
"#;
        let result = parse_scenario(toml);
        assert!(result.is_err(), "missing state should return error");
        let err = result.unwrap_err();
        assert!(err.contains("state"), "error should mention 'state': {err}");
    }

    // Cycle 47: evaluate_triggers returns SetAiState action
    #[test]
    fn evaluate_triggers_returns_set_ai_state_action() {
        let mut name_to_uuid = HashMap::new();
        name_to_uuid.insert("ambusher".to_string(), "uuid-ambusher".to_string());

        let trigger = Trigger {
            condition: TriggerCondition::OnTimer { after_secs: 5.0 },
            actions: vec![TriggerAction::SetAiState {
                entity: "ambusher".to_string(),
                state: "pursuing".to_string(),
                target: Some("player_ship".to_string()),
            }],
        };
        let mut states = vec![TriggerState { trigger, fired: false, scenario_path: "test".to_string() }];
        let events = vec![WorldEvent::TimerElapsed { elapsed_secs: 5.0 }];
        let fired = evaluate_triggers(&mut states, &events, &name_to_uuid);

        assert_eq!(fired.len(), 1);
        assert_eq!(
            fired[0].actions[0],
            TriggerAction::SetAiState {
                entity: "ambusher".to_string(),
                state: "pursuing".to_string(),
                target: Some("player_ship".to_string()),
            }
        );
    }

    // Cycle 48: on_entity_attacked trigger fires once (single-shot semantics)
    #[test]
    fn on_attacked_trigger_fires_only_once_single_shot() {
        let mut name_to_uuid = HashMap::new();
        name_to_uuid.insert("raider".to_string(), "uuid-raider".to_string());

        let trigger = Trigger {
            condition: TriggerCondition::OnAttacked { entity_name: "raider".to_string() },
            actions: vec![TriggerAction::SetAiState {
                entity: "raider".to_string(),
                state: "attacking".to_string(),
                target: None,
            }],
        };
        let mut states = vec![TriggerState { trigger, fired: false, scenario_path: "test".to_string() }];

        let events = vec![WorldEvent::Attacked {
            uuid: "uuid-raider".to_string(),
            attacker_uuid: "uuid-player".to_string(),
        }];

        // First evaluation fires.
        let fired1 = evaluate_triggers(&mut states, &events, &name_to_uuid);
        assert_eq!(fired1.len(), 1);

        // Second evaluation with same events: must not fire again.
        let fired2 = evaluate_triggers(&mut states, &events, &name_to_uuid);
        assert_eq!(fired2.len(), 0);
    }

    // ── Cycles 49-56: modifier and flag actions ───────────────────────────────

    // Cycle 49: parse apply_modifier action from TOML
    #[test]
    fn parse_apply_modifier_action_from_toml() {
        let toml = r#"
[[trigger]]
condition = "on_timer"
after_secs = 5.0

[[trigger.action]]
type = "apply_modifier"
entity = "player_ship"
tag = "speed_boost"
slot = "MaxSpeed"
bonus = 0.5
"#;
        let config = parse_scenario(toml).unwrap();
        assert_eq!(config.triggers[0].actions.len(), 1);
        assert_eq!(
            config.triggers[0].actions[0],
            TriggerAction::ApplyModifier {
                entity: "player_ship".to_string(),
                tag: "speed_boost".to_string(),
                slot: crate::messages::ModifierSlot::MaxSpeed,
                bonus: 0.5,
            }
        );
    }

    // Cycle 50: parse remove_modifier action from TOML
    #[test]
    fn parse_remove_modifier_action_from_toml() {
        let toml = r#"
[[trigger]]
condition = "on_timer"
after_secs = 10.0

[[trigger.action]]
type = "remove_modifier"
entity = "player_ship"
tag = "speed_boost"
slot = "MaxSpeed"
"#;
        let config = parse_scenario(toml).unwrap();
        assert_eq!(
            config.triggers[0].actions[0],
            TriggerAction::RemoveModifier {
                entity: "player_ship".to_string(),
                tag: "speed_boost".to_string(),
                slot: crate::messages::ModifierSlot::MaxSpeed,
            }
        );
    }

    // Cycle 51: parse apply_flag action from TOML
    #[test]
    fn parse_apply_flag_action_from_toml() {
        let toml = r#"
[[trigger]]
condition = "on_timer"
after_secs = 1.0

[[trigger.action]]
type = "apply_flag"
entity = "player_ship"
tag = "jammer"
kind = "CommsJammed"
"#;
        let config = parse_scenario(toml).unwrap();
        assert_eq!(
            config.triggers[0].actions[0],
            TriggerAction::ApplyFlag {
                entity: "player_ship".to_string(),
                tag: "jammer".to_string(),
                kind: crate::flag_kind::FlagKind::CommsJammed,
            }
        );
    }

    // Cycle 52: parse remove_flag action from TOML
    #[test]
    fn parse_remove_flag_action_from_toml() {
        let toml = r#"
[[trigger]]
condition = "on_timer"
after_secs = 2.0

[[trigger.action]]
type = "remove_flag"
entity = "player_ship"
tag = "jammer"
kind = "CommsJammed"
"#;
        let config = parse_scenario(toml).unwrap();
        assert_eq!(
            config.triggers[0].actions[0],
            TriggerAction::RemoveFlag {
                entity: "player_ship".to_string(),
                tag: "jammer".to_string(),
                kind: crate::flag_kind::FlagKind::CommsJammed,
            }
        );
    }

    // Cycle 53: all four new variants are passed through substitute_action unchanged
    #[test]
    fn substitute_action_passes_through_modifier_and_flag_variants() {
        let name_to_uuid = {
            let mut m = HashMap::new();
            m.insert("player_ship".to_string(), "uuid-player-001".to_string());
            m
        };

        let actions = vec![
            TriggerAction::ApplyModifier {
                entity: "player_ship".to_string(),
                tag: "tag1".to_string(),
                slot: crate::messages::ModifierSlot::MaxSpeed,
                bonus: 0.3,
            },
            TriggerAction::RemoveModifier {
                entity: "player_ship".to_string(),
                tag: "tag1".to_string(),
                slot: crate::messages::ModifierSlot::MaxSpeed,
            },
            TriggerAction::ApplyFlag {
                entity: "player_ship".to_string(),
                tag: "jammer".to_string(),
                kind: crate::flag_kind::FlagKind::SensorBlind,
            },
            TriggerAction::RemoveFlag {
                entity: "player_ship".to_string(),
                tag: "jammer".to_string(),
                kind: crate::flag_kind::FlagKind::SensorBlind,
            },
        ];

        let trigger = Trigger {
            condition: TriggerCondition::OnTimer { after_secs: 0.0 },
            actions: actions.clone(),
        };
        let mut states = vec![TriggerState { trigger, fired: false, scenario_path: "test".to_string() }];
        let events = vec![WorldEvent::TimerElapsed { elapsed_secs: 1.0 }];
        let fired = evaluate_triggers(&mut states, &events, &name_to_uuid);

        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].actions, actions);
    }

    // Cycle 54: apply_modifier requires entity field
    #[test]
    fn parse_apply_modifier_without_entity_returns_error() {
        let toml = r#"
[[trigger]]
condition = "on_timer"
after_secs = 1.0

[[trigger.action]]
type = "apply_modifier"
tag = "boost"
slot = "MaxSpeed"
bonus = 0.5
"#;
        let result = parse_scenario(toml);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("entity"), "error should mention 'entity': {err}");
    }

    // Cycle 55: apply_modifier with invalid slot returns error
    #[test]
    fn parse_apply_modifier_with_invalid_slot_returns_error() {
        let toml = r#"
[[trigger]]
condition = "on_timer"
after_secs = 1.0

[[trigger.action]]
type = "apply_modifier"
entity = "player_ship"
tag = "boost"
slot = "NotASlot"
bonus = 0.5
"#;
        let result = parse_scenario(toml);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("slot") || err.contains("NotASlot"), "error should mention bad slot: {err}");
    }

    // Cycle 56: apply_flag with unknown kind returns error
    #[test]
    fn parse_apply_flag_with_unknown_kind_returns_error() {
        let toml = r#"
[[trigger]]
condition = "on_timer"
after_secs = 1.0

[[trigger.action]]
type = "apply_flag"
entity = "player_ship"
tag = "jammer"
kind = "UnknownFlag"
"#;
        let result = parse_scenario(toml);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("kind") || err.contains("UnknownFlag"), "error should mention bad kind: {err}");
    }

    // ── default.toml compile-time template tests ────────────────────────────

    #[test]
    fn default_scenario_toml_parses_without_error() {
        let toml_str = include_str!("../../assets/scenarios/default.toml");
        parse_scenario(toml_str).expect("default.toml must parse");
    }

    #[test]
    fn default_scenario_has_starbase_alpha_spawn() {
        let toml_str = include_str!("../../assets/scenarios/default.toml");
        let config = parse_scenario(toml_str).expect("default.toml must parse");
        let starbase = config.spawns.iter().find(|s| s.name == "Starbase Alpha");
        assert!(starbase.is_some(), "default scenario must have a spawn named 'Starbase Alpha'");
    }

    #[test]
    fn default_scenario_has_starbase_alpha_comms_with_responses_and_actions() {
        let toml_str = include_str!("../../assets/scenarios/default.toml");
        let config = parse_scenario(toml_str).expect("default.toml must parse");
        // Find the on_hailed template specifically (there is also an on_attacked distress template).
        let tmpl = config.comms.iter().find(|c| {
            c.from == "Starbase Alpha"
                && matches!(&c.trigger, TriggerCondition::OnHailed { .. })
        });
        assert!(tmpl.is_some(), "default scenario must have an on_hailed comms template from 'Starbase Alpha'");
        let tmpl = tmpl.unwrap();
        assert!(
            !tmpl.node.responses.is_empty(),
            "Starbase Alpha on_hailed comms must have at least one response"
        );
        let has_add_objective = tmpl.node.responses.iter().any(|r| {
            r.actions.iter().any(|a| matches!(a, TriggerAction::AddObjective { .. }))
        });
        assert!(has_add_objective, "Starbase Alpha comms response must have an add_objective action");
    }

    // ── patrol.toml compile-time template tests ─────────────────────────────

    #[test]
    fn patrol_scenario_toml_parses_without_error() {
        let toml_str = include_str!("../../assets/scenarios/patrol.toml");
        parse_scenario(toml_str).expect("patrol.toml must parse");
    }

    #[test]
    fn patrol_scenario_has_one_named_raider_spawn() {
        let toml_str = include_str!("../../assets/scenarios/patrol.toml");
        let config = parse_scenario(toml_str).expect("patrol.toml must parse");
        let raider = config.spawns.iter().find(|s| s.name == "raider");
        assert!(raider.is_some(), "patrol scenario must have a spawn named 'raider'");
        let raider = raider.unwrap();
        assert!(
            raider.entity_path.contains("pirate_raider"),
            "raider spawn must reference pirate_raider template, got: {}",
            raider.entity_path
        );
    }

    #[test]
    fn patrol_scenario_has_on_entity_destroyed_trigger_with_add_objective() {
        let toml_str = include_str!("../../assets/scenarios/patrol.toml");
        let config = parse_scenario(toml_str).expect("patrol.toml must parse");
        let has_destroyed_trigger = config.triggers.iter().any(|t| {
            matches!(&t.condition, TriggerCondition::OnDestroyed { .. })
                && t.actions.iter().any(|a| matches!(a, TriggerAction::AddObjective { .. }))
        });
        assert!(
            has_destroyed_trigger,
            "patrol scenario must have an on_entity_destroyed trigger with add_objective action"
        );
    }

    // ── Cycles 57-61: apply_int_modifier / remove_int_modifier ───────────────

    // Cycle 57: parse apply_int_modifier action from TOML
    #[test]
    fn parse_apply_int_modifier_action_from_toml() {
        let toml = r#"
[[trigger]]
condition = "on_timer"
after_secs = 5.0

[[trigger.action]]
type = "apply_int_modifier"
entity = "player_ship"
tag = "extra_team"
slot = "RepairTeams"
int_bonus = 1
"#;
        let config = parse_scenario(toml).unwrap();
        assert_eq!(config.triggers[0].actions.len(), 1);
        assert_eq!(
            config.triggers[0].actions[0],
            TriggerAction::ApplyIntModifier {
                entity: "player_ship".to_string(),
                tag: "extra_team".to_string(),
                slot: crate::modifiers::IntModifierSlot::RepairTeams,
                bonus: 1,
            }
        );
    }

    // Cycle 58: parse remove_int_modifier action from TOML
    #[test]
    fn parse_remove_int_modifier_action_from_toml() {
        let toml = r#"
[[trigger]]
condition = "on_timer"
after_secs = 10.0

[[trigger.action]]
type = "remove_int_modifier"
entity = "player_ship"
tag = "extra_team"
slot = "RepairTeams"
"#;
        let config = parse_scenario(toml).unwrap();
        assert_eq!(
            config.triggers[0].actions[0],
            TriggerAction::RemoveIntModifier {
                entity: "player_ship".to_string(),
                tag: "extra_team".to_string(),
                slot: crate::modifiers::IntModifierSlot::RepairTeams,
            }
        );
    }

    // Cycle 59: apply_int_modifier with unknown slot returns error
    #[test]
    fn parse_apply_int_modifier_with_invalid_slot_returns_error() {
        let toml = r#"
[[trigger]]
condition = "on_timer"
after_secs = 1.0

[[trigger.action]]
type = "apply_int_modifier"
entity = "player_ship"
tag = "boost"
slot = "NotAnIntSlot"
int_bonus = 2
"#;
        let result = parse_scenario(toml);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.contains("slot") || err.contains("NotAnIntSlot"),
            "error should mention bad slot: {err}"
        );
    }

    // Cycle 60: apply_int_modifier missing int_bonus returns error
    #[test]
    fn parse_apply_int_modifier_without_int_bonus_returns_error() {
        let toml = r#"
[[trigger]]
condition = "on_timer"
after_secs = 1.0

[[trigger.action]]
type = "apply_int_modifier"
entity = "player_ship"
tag = "boost"
slot = "RepairTeams"
"#;
        let result = parse_scenario(toml);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.contains("int_bonus"),
            "error should mention 'int_bonus': {err}"
        );
    }

    // Cycle 61: new variants pass through substitute_action unchanged
    #[test]
    fn substitute_action_passes_through_int_modifier_variants() {
        let name_to_uuid = HashMap::new();
        let actions = vec![
            TriggerAction::ApplyIntModifier {
                entity: "player_ship".to_string(),
                tag: "extra_team".to_string(),
                slot: crate::modifiers::IntModifierSlot::RepairTeams,
                bonus: 1,
            },
            TriggerAction::RemoveIntModifier {
                entity: "player_ship".to_string(),
                tag: "extra_team".to_string(),
                slot: crate::modifiers::IntModifierSlot::RepairTeams,
            },
        ];
        let trigger = Trigger {
            condition: TriggerCondition::OnTimer { after_secs: 0.0 },
            actions: actions.clone(),
        };
        let mut states = vec![TriggerState { trigger, fired: false, scenario_path: "test".to_string() }];
        let events = vec![WorldEvent::TimerElapsed { elapsed_secs: 1.0 }];
        let fired = evaluate_triggers(&mut states, &events, &name_to_uuid);
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].actions, actions);
    }

    // ── Cycle 36: unload_scenario TOML parse ────────────────────────────────

    // Cycle 36a: unload_scenario parses from TOML to TriggerAction::UnloadScenario
    #[test]
    fn parse_unload_scenario_action() {
        let toml = r#"
[[spawn]]
name = "station_alpha"
entity_path = "entities/station.toml"
position = [0.0, 0.0, 0.0]

[[trigger]]
condition = "on_destroyed"
entity = "station_alpha"

[[trigger.action]]
type = "unload_scenario"
path = "scenarios/phase1.toml"
"#;
        let config = parse_scenario(toml).unwrap();
        assert_eq!(config.triggers.len(), 1);
        assert_eq!(config.triggers[0].actions.len(), 1);
        assert_eq!(
            config.triggers[0].actions[0],
            TriggerAction::UnloadScenario { path: "scenarios/phase1.toml".to_string() }
        );
    }

    // Cycle 36b: unload_scenario requires path field
    #[test]
    fn parse_unload_scenario_requires_path() {
        let toml = r#"
[[spawn]]
name = "e"
entity_path = "entities/station.toml"
position = [0.0, 0.0, 0.0]

[[trigger]]
condition = "on_destroyed"
entity = "e"

[[trigger.action]]
type = "unload_scenario"
"#;
        let result = parse_scenario(toml);
        assert!(result.is_err(), "unload_scenario without path should fail");
        assert!(result.unwrap_err().contains("path"), "error should mention 'path'");
    }

    // ── Cycle 37: ScenarioManager additive load ──────────────────────────────

    // Cycle 37a: loading a new scenario inserts it into the map
    #[test]
    fn load_scenario_inserts_new_path() {
        let mut mgr = ScenarioManager::default();
        let inserted = mgr.load_scenario("scenarios/phase1.toml");
        assert!(inserted, "first load should return true");
        assert_eq!(mgr.len(), 1);
        assert!(mgr.is_active("scenarios/phase1.toml"));
    }

    // Cycle 37b: loading the same path twice is a no-op
    #[test]
    fn load_scenario_second_time_is_noop() {
        let mut mgr = ScenarioManager::default();
        mgr.load_scenario("scenarios/phase1.toml");
        let inserted = mgr.load_scenario("scenarios/phase1.toml");
        assert!(!inserted, "second load should return false");
        assert_eq!(mgr.len(), 1, "map size must not grow");
    }

    // Cycle 37c: loading two different paths results in both being active
    #[test]
    fn load_two_different_scenarios_both_active() {
        let mut mgr = ScenarioManager::default();
        mgr.load_scenario("scenarios/a.toml");
        mgr.load_scenario("scenarios/b.toml");
        assert_eq!(mgr.len(), 2);
        assert!(mgr.is_active("scenarios/a.toml"));
        assert!(mgr.is_active("scenarios/b.toml"));
    }

    // Cycle 37d: unloading removes the scenario and returns its runtime
    #[test]
    fn unload_scenario_removes_from_map() {
        let mut mgr = ScenarioManager::default();
        mgr.load_scenario("scenarios/a.toml");
        mgr.load_scenario("scenarios/b.toml");
        let removed = mgr.unload_scenario("scenarios/a.toml");
        assert!(removed.is_some(), "should return the removed runtime");
        assert_eq!(mgr.len(), 1);
        assert!(!mgr.is_active("scenarios/a.toml"));
        assert!(mgr.is_active("scenarios/b.toml"), "b must remain active");
    }

    // Cycle 37e: unloading a non-active path returns None
    #[test]
    fn unload_nonexistent_scenario_returns_none() {
        let mut mgr = ScenarioManager::default();
        let removed = mgr.unload_scenario("scenarios/ghost.toml");
        assert!(removed.is_none());
    }

    // ── default.toml integration tests ────────────────────────────────────────

    /// The default scenario file parses without error.
    #[test]
    fn default_scenario_parses_without_error() {
        let toml = include_str!("../../assets/scenarios/default.toml");
        let result = parse_scenario(toml);
        assert!(result.is_ok(), "default.toml must parse without error: {:?}", result.err());
    }

    /// default.toml must declare an `on_attacked` trigger on the raider that
    /// chains `load_scenario` to `assets/scenarios/patrol.toml`.
    #[test]
    fn default_scenario_raider_on_attacked_trigger_loads_patrol_scenario() {
        let toml = include_str!("../../assets/scenarios/default.toml");
        let config = parse_scenario(toml).expect("default.toml must parse");

        let attacked_trigger = config.triggers.iter().find(|t| {
            matches!(&t.condition, TriggerCondition::OnAttacked { entity_name } if entity_name == "raider")
        });
        assert!(
            attacked_trigger.is_some(),
            "default.toml must have an on_attacked trigger for the raider"
        );

        let has_load_patrol = attacked_trigger.unwrap().actions.iter().any(|a| {
            matches!(a, TriggerAction::LoadScenario { path } if path == "assets/scenarios/patrol.toml")
        });
        assert!(has_load_patrol, "raider on_attacked trigger must include load_scenario for patrol.toml");
    }

    /// default.toml must declare an `on_attacked` comms template on the raider
    /// that broadcasts a distress message with no player response options.
    #[test]
    fn default_scenario_raider_on_attacked_comms_is_broadcast_distress() {
        let toml = include_str!("../../assets/scenarios/default.toml");
        let config = parse_scenario(toml).expect("default.toml must parse");

        let raider_distress = config.comms.iter().find(|c| {
            c.from == "raider"
                && matches!(&c.trigger, TriggerCondition::OnAttacked { entity_name } if entity_name == "raider")
        });
        assert!(
            raider_distress.is_some(),
            "default.toml must have an on_attacked comms template from the raider"
        );
        let template = raider_distress.unwrap();
        assert!(
            template.node.responses.is_empty(),
            "raider distress comms must have no player responses (broadcast-only)"
        );
        assert!(
            !template.node.body.is_empty(),
            "raider distress comms must have a non-empty message body"
        );
    }

    /// default.toml must declare an `on_attacked` comms template for Starbase Alpha.
    #[test]
    fn default_scenario_starbase_on_attacked_comms_is_broadcast_distress() {
        let toml = include_str!("../../assets/scenarios/default.toml");
        let config = parse_scenario(toml).expect("default.toml must parse");

        let station_distress = config.comms.iter().find(|c| {
            c.from == "Starbase Alpha"
                && matches!(&c.trigger, TriggerCondition::OnAttacked { entity_name } if entity_name == "Starbase Alpha")
        });
        assert!(
            station_distress.is_some(),
            "default.toml must have an on_attacked comms template from Starbase Alpha"
        );
        let template = station_distress.unwrap();
        assert!(
            template.node.responses.is_empty(),
            "Starbase Alpha distress comms must have no player responses (broadcast-only)"
        );
    }

    /// The `evaluate_triggers` + `evaluate_comms_templates` functions both fire
    /// their respective entries when an `on_attacked` world event matches.
    #[test]
    fn default_scenario_on_attacked_fires_both_trigger_and_comms_template() {
        let toml = include_str!("../../assets/scenarios/default.toml");
        let config = parse_scenario(toml).expect("default.toml must parse");

        // Build trigger and comms template states from the parsed config.
        let mut trigger_states = trigger_states_from_config(&config, "assets/scenarios/default.toml");
        let mut comms_states = comms_template_states_from_config(&config, "assets/scenarios/default.toml");

        let raider_uuid = config.name_to_uuid.get("raider").expect("raider spawn must exist").clone();
        let name_to_uuid = config.name_to_uuid.clone();

        let world_events = vec![WorldEvent::Attacked {
            uuid: raider_uuid.clone(),
            attacker_uuid: "player-ship".to_string(),
        }];

        let fired_triggers = evaluate_triggers(&mut trigger_states, &world_events, &name_to_uuid);
        let fired_comms = evaluate_comms_templates(&mut comms_states, &world_events, &name_to_uuid);

        // The on_attacked trigger (load_scenario) must fire.
        let has_load_patrol = fired_triggers.iter().any(|ft| {
            ft.actions.iter().any(|a| {
                matches!(a, TriggerAction::LoadScenario { path } if path == "assets/scenarios/patrol.toml")
            })
        });
        assert!(has_load_patrol, "on_attacked trigger must produce LoadScenario action for patrol.toml");

        // The raider on_attacked comms template must fire.
        let raider_comms = fired_comms.iter().any(|fc| fc.from == "raider");
        assert!(raider_comms, "on_attacked comms template from raider must fire");
    }
}
