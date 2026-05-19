// Unified world parser — single-pass deserialization for the merged
// map/scenario world TOML (PRD #337/#341).
//
// This module owns the entire world TOML schema: anchors, `[[entity]]`
// instances, `[[trigger]]` blocks, and `[[comms]]` templates. `parse_world`
// produces a `WorldConfig` in one parse pass.
//
// Pure module — no Bevy systems, only the `Resource` derive for the
// `WorldConfig` type. Runtime types (`TriggerState`, `CommsTemplateState`,
// `ActiveDialogue`, `WorldEvent`, etc.) live in `world::content` and import
// the pure config types from here.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::entity_config::GlobalConfig;

// ── World-tree entity instance types ───────────────────────────────────────

/// When to spawn a `WorldEntity` declared in the world TOML.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum WorldEntitySpawnOn {
    /// Spawn immediately when the world is loaded (lobby phase).
    Immediate,
    /// Spawn when the game starts (in-progress phase).
    GameStart,
}

impl Default for WorldEntitySpawnOn {
    fn default() -> Self {
        WorldEntitySpawnOn::Immediate
    }
}

/// A concrete entity instance declared in the world TOML — a reference to an
/// entity template (under `assets/entities/`) plus instance-level metadata
/// (position / anchor, spawn timing, optional name for trigger/comms binding,
/// inline overrides).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct WorldEntity {
    /// Path to the entity template TOML (relative to assets/).
    pub template_path: String,
    /// Optional human-readable identifier for this instance.
    #[serde(default)]
    pub id: Option<String>,
    /// Optional named identity for the entity. When present, the entity
    /// becomes trigger- and comms-eligible: `spawn_world_entities` assigns
    /// it a stable UUID and registers `name → uuid` in `WorldConfig.name_to_uuid`.
    #[serde(default)]
    pub name: Option<String>,
    /// World-space position [x, y, z].
    #[serde(default)]
    pub position: Vec<f32>,
    /// Named anchor reference (from `[anchors]` in the world TOML). Mutually
    /// exclusive with `position`. Resolved by `resolve_entity_position` at
    /// spawn time.
    #[serde(default)]
    pub anchor: Option<String>,
    /// Named reference to another `[[entity]]` (by `name`) whose resolved
    /// position this entity is positioned relative to. Combined with
    /// `offset`. Resolved by `resolve_entity_position_with` at spawn time
    /// after sibling positions are known. Takes precedence over `anchor` /
    /// `position` when set.
    #[serde(default)]
    pub relative_to: Option<String>,
    /// Offset [x, y, z] added to the `relative_to` entity's resolved position.
    /// Ignored unless `relative_to` is set. Missing components default to 0.
    #[serde(default)]
    pub offset: Vec<f32>,
    /// When this entity should be spawned.
    #[serde(default)]
    pub spawn_on: WorldEntitySpawnOn,
    /// Optional inline TOML overrides merged on top of the template.
    #[serde(default)]
    pub overrides: Option<toml::Value>,
}

// ── TOML-facing raw types ──────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct RawTriggerEntry {
    condition: String,
    #[serde(default)]
    entity: Option<String>,
    #[serde(default)]
    after_secs: Option<f32>,
    #[serde(default, rename = "action")]
    actions: Vec<RawActionEntry>,
}

#[derive(Debug, Deserialize)]
struct RawActionEntry {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    mandatory: Option<bool>,
    #[serde(default)]
    entity: Option<String>,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    target: Option<String>,
    #[serde(default)]
    tag: Option<String>,
    #[serde(default)]
    slot: Option<String>,
    #[serde(default)]
    bonus: Option<f32>,
    #[serde(default)]
    int_bonus: Option<i32>,
    #[serde(default, rename = "kind")]
    flag_kind: Option<String>,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    load_scenario: Option<String>,
    #[serde(default)]
    path: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawCommsFollowUp {
    message: String,
    #[serde(default, rename = "response")]
    responses: Vec<RawCommsResponse>,
}

#[derive(Debug, Deserialize)]
struct RawCommsResponse {
    text: String,
    #[serde(default, rename = "action")]
    actions: Vec<RawActionEntry>,
    #[serde(default)]
    follow_up: Option<RawCommsFollowUp>,
}

#[derive(Debug, Deserialize)]
struct RawCommsEntry {
    from: String,
    message: String,
    trigger: String,
    #[serde(default)]
    entity: Option<String>,
    #[serde(default, rename = "response")]
    responses: Vec<RawCommsResponse>,
}

/// Raw single-pass deserialization of a world TOML.
#[derive(Debug, Default, Deserialize)]
pub struct RawWorld {
    #[serde(default)]
    pub global: GlobalConfig,
    #[serde(default)]
    pub anchors: HashMap<String, Vec<f32>>,
    #[serde(default, rename = "entity")]
    pub entities: Vec<WorldEntity>,
    #[serde(default, rename = "trigger")]
    triggers: Vec<RawTriggerEntry>,
    #[serde(default, rename = "comms")]
    comms: Vec<RawCommsEntry>,
    /// Paths to additional world TOML files to load additively at startup.
    #[serde(default)]
    pub extra_worlds: Vec<String>,
}

// ── Trigger / comms pure config types ──────────────────────────────────────

/// A condition that a trigger can check against incoming world events.
#[derive(Clone, Debug, PartialEq)]
pub enum TriggerCondition {
    /// Fires when the named entity (by name, resolved to UUID at runtime) is destroyed.
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
    AddObjective { id: String, text: String, mandatory: bool },
    CompleteObjective { id: String },
    FailObjective { id: String },
    SetAiState { entity: String, state: String, target: Option<String> },
    ApplyModifier { entity: String, tag: String, slot: crate::messages::ModifierSlot, bonus: f32 },
    RemoveModifier { entity: String, tag: String, slot: crate::messages::ModifierSlot },
    ApplyFlag { entity: String, tag: String, kind: crate::flag_kind::FlagKind },
    RemoveFlag { entity: String, tag: String, kind: crate::flag_kind::FlagKind },
    ApplyIntModifier { entity: String, tag: String, slot: crate::modifiers::IntModifierSlot, bonus: i32 },
    RemoveIntModifier { entity: String, tag: String, slot: crate::modifiers::IntModifierSlot },
    GameOver { message: Option<String> },
    /// Load a new scenario, replacing the current world.
    LoadScenario { path: String },
    /// Additively load a sub-world from `path` into the running world layer map.
    LoadWorld { path: String },
    /// Unload a previously loaded sub-world identified by `path`.
    UnloadWorld { path: String },
}

/// A single trigger: a condition plus an ordered list of actions.
#[derive(Clone, Debug, PartialEq)]
pub struct Trigger {
    pub condition: TriggerCondition,
    pub actions: Vec<TriggerAction>,
}

/// A single response option within a comms dialogue node.
#[derive(Clone, Debug, PartialEq)]
pub struct CommsResponse {
    pub text: String,
    pub actions: Vec<TriggerAction>,
    pub follow_up: Option<CommsDialogueNode>,
}

/// A single node in an inline dialogue tree.
#[derive(Clone, Debug, PartialEq)]
pub struct CommsDialogueNode {
    pub body: String,
    pub responses: Vec<CommsResponse>,
}

/// A comms template: a root dialogue node associated with a trigger condition.
#[derive(Clone, Debug, PartialEq)]
pub struct CommsTemplate {
    /// Entity name whose UUID is the sender of the comms message.
    pub from: String,
    /// The trigger condition that fires this template.
    pub trigger: TriggerCondition,
    /// The root dialogue node.
    pub node: CommsDialogueNode,
}

// ── Parser helpers ─────────────────────────────────────────────────────────

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
            "add_objective" => TriggerAction::AddObjective {
                id: raw_action.id.clone().ok_or_else(|| "Action 'add_objective' requires an 'id' field".to_string())?,
                text: raw_action.text.clone().ok_or_else(|| "Action 'add_objective' requires a 'text' field".to_string())?,
                mandatory: raw_action.mandatory.unwrap_or(false),
            },
            "complete_objective" => TriggerAction::CompleteObjective {
                id: raw_action.id.clone().ok_or_else(|| "Action 'complete_objective' requires an 'id' field".to_string())?,
            },
            "fail_objective" => TriggerAction::FailObjective {
                id: raw_action.id.clone().ok_or_else(|| "Action 'fail_objective' requires an 'id' field".to_string())?,
            },
            "set_ai_state" => TriggerAction::SetAiState {
                entity: raw_action.entity.clone().ok_or_else(|| "Action 'set_ai_state' requires an 'entity' field".to_string())?,
                state: raw_action.state.clone().ok_or_else(|| "Action 'set_ai_state' requires a 'state' field".to_string())?,
                target: raw_action.target.clone(),
            },
            "apply_modifier" => {
                let slot_str = raw_action.slot.as_deref().ok_or_else(|| "Action 'apply_modifier' requires a 'slot' field".to_string())?;
                TriggerAction::ApplyModifier {
                    entity: raw_action.entity.clone().ok_or_else(|| "Action 'apply_modifier' requires an 'entity' field".to_string())?,
                    tag: raw_action.tag.clone().ok_or_else(|| "Action 'apply_modifier' requires a 'tag' field".to_string())?,
                    slot: parse_modifier_slot(slot_str)?,
                    bonus: raw_action.bonus.ok_or_else(|| "Action 'apply_modifier' requires a 'bonus' field".to_string())?,
                }
            }
            "remove_modifier" => {
                let slot_str = raw_action.slot.as_deref().ok_or_else(|| "Action 'remove_modifier' requires a 'slot' field".to_string())?;
                TriggerAction::RemoveModifier {
                    entity: raw_action.entity.clone().ok_or_else(|| "Action 'remove_modifier' requires an 'entity' field".to_string())?,
                    tag: raw_action.tag.clone().ok_or_else(|| "Action 'remove_modifier' requires a 'tag' field".to_string())?,
                    slot: parse_modifier_slot(slot_str)?,
                }
            }
            "apply_flag" => {
                let kind_str = raw_action.flag_kind.as_deref().ok_or_else(|| "Action 'apply_flag' requires a 'kind' field".to_string())?;
                TriggerAction::ApplyFlag {
                    entity: raw_action.entity.clone().ok_or_else(|| "Action 'apply_flag' requires an 'entity' field".to_string())?,
                    tag: raw_action.tag.clone().ok_or_else(|| "Action 'apply_flag' requires a 'tag' field".to_string())?,
                    kind: parse_flag_kind(kind_str)?,
                }
            }
            "remove_flag" => {
                let kind_str = raw_action.flag_kind.as_deref().ok_or_else(|| "Action 'remove_flag' requires a 'kind' field".to_string())?;
                TriggerAction::RemoveFlag {
                    entity: raw_action.entity.clone().ok_or_else(|| "Action 'remove_flag' requires an 'entity' field".to_string())?,
                    tag: raw_action.tag.clone().ok_or_else(|| "Action 'remove_flag' requires a 'tag' field".to_string())?,
                    kind: parse_flag_kind(kind_str)?,
                }
            }
            "apply_int_modifier" => {
                let slot_str = raw_action.slot.as_deref().ok_or_else(|| "Action 'apply_int_modifier' requires a 'slot' field".to_string())?;
                TriggerAction::ApplyIntModifier {
                    entity: raw_action.entity.clone().ok_or_else(|| "Action 'apply_int_modifier' requires an 'entity' field".to_string())?,
                    tag: raw_action.tag.clone().ok_or_else(|| "Action 'apply_int_modifier' requires a 'tag' field".to_string())?,
                    slot: parse_int_modifier_slot(slot_str)?,
                    bonus: raw_action.int_bonus.ok_or_else(|| "Action 'apply_int_modifier' requires an 'int_bonus' field".to_string())?,
                }
            }
            "remove_int_modifier" => {
                let slot_str = raw_action.slot.as_deref().ok_or_else(|| "Action 'remove_int_modifier' requires a 'slot' field".to_string())?;
                TriggerAction::RemoveIntModifier {
                    entity: raw_action.entity.clone().ok_or_else(|| "Action 'remove_int_modifier' requires an 'entity' field".to_string())?,
                    tag: raw_action.tag.clone().ok_or_else(|| "Action 'remove_int_modifier' requires a 'tag' field".to_string())?,
                    slot: parse_int_modifier_slot(slot_str)?,
                }
            }
            "game_over" => TriggerAction::GameOver { message: raw_action.message.clone() },
            "load_scenario" => TriggerAction::LoadScenario {
                path: raw_action.load_scenario.clone().ok_or_else(|| "Action 'load_scenario' requires a 'load_scenario' field".to_string())?,
            },
            "load_world" => TriggerAction::LoadWorld {
                path: raw_action.path.clone().ok_or_else(|| "Action 'load_world' requires a 'path' field".to_string())?,
            },
            "unload_world" => TriggerAction::UnloadWorld {
                path: raw_action.path.clone().ok_or_else(|| "Action 'unload_world' requires a 'path' field".to_string())?,
            },
            other => return Err(format!("Unknown trigger action '{}'", other)),
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

fn parse_trigger_condition_from_string(
    name: &str,
    entity: Option<String>,
    after_secs: Option<f32>,
    ctx: &str,
) -> Result<TriggerCondition, String> {
    match name {
        "on_destroyed" => Ok(TriggerCondition::OnDestroyed {
            entity_name: entity.ok_or_else(|| format!("{ctx} 'on_destroyed' requires an 'entity' field"))?,
        }),
        "on_attacked" => Ok(TriggerCondition::OnAttacked {
            entity_name: entity.ok_or_else(|| format!("{ctx} 'on_attacked' requires an 'entity' field"))?,
        }),
        "on_timer" => Ok(TriggerCondition::OnTimer {
            after_secs: after_secs.ok_or_else(|| format!("{ctx} 'on_timer' requires an 'after_secs' field"))?,
        }),
        "on_hailed" => Ok(TriggerCondition::OnHailed {
            entity_name: entity.ok_or_else(|| format!("{ctx} 'on_hailed' requires an 'entity' field"))?,
        }),
        other => Err(format!("Unknown {ctx} condition '{}'", other)),
    }
}

// ── Public typed config ────────────────────────────────────────────────────

/// Parsed unified world configuration.
///
/// Carries the anchor table, `[[entity]]` instances, `[[trigger]]` blocks,
/// and `[[comms]]` templates. Anchors are normalised to fixed-size `[f32; 3]`
/// arrays at parse time so downstream consumers (e.g. AI patrol path lookups,
/// region positioning) don't have to re-validate length on every read.
#[derive(Clone, Debug, Default, bevy::prelude::Resource)]
pub struct WorldConfig {
    pub global: GlobalConfig,
    pub anchors: HashMap<String, [f32; 3]>,
    pub entities: Vec<WorldEntity>,
    /// Ordered list of triggers declared in the world.
    pub triggers: Vec<Trigger>,
    /// Ordered list of comms dialogue templates declared in the world.
    pub comms: Vec<CommsTemplate>,
    /// Map of `name → uuid` for entities spawned via `[[entity]] name = "..."`.
    /// Populated by `spawn_world_entities` (PRD #337/#339 slice 2); read by
    /// trigger and comms lookup paths that resolve a name to a live UUID.
    pub name_to_uuid: HashMap<String, String>,
    /// Paths of additional world TOML files to load additively at startup
    /// (issue #352 — `extra_worlds` field).
    pub extra_worlds: Vec<String>,
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
    pub fn entities(&self) -> &[WorldEntity] {
        &self.entities
    }
}

// ── Parser ─────────────────────────────────────────────────────────────────

/// Parse a unified world TOML string in a single pass.
///
/// Validates that every anchor position has 2 or 3 components and normalises
/// to `[x, y, z]`. Returns an `Err` with a human-readable message on TOML
/// parse errors, invalid anchor shapes, unknown trigger conditions, or
/// invalid trigger actions.
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

    // Triggers.
    let mut triggers = Vec::with_capacity(raw.triggers.len());
    for raw_trigger in raw.triggers {
        let condition = parse_trigger_condition_from_string(
            &raw_trigger.condition,
            raw_trigger.entity,
            raw_trigger.after_secs,
            "Trigger",
        )?;
        let actions = parse_raw_actions(&raw_trigger.actions)?;
        triggers.push(Trigger { condition, actions });
    }

    // Comms templates.
    let mut comms = Vec::with_capacity(raw.comms.len());
    for raw_comms in raw.comms {
        let trigger = parse_trigger_condition_from_string(
            &raw_comms.trigger,
            raw_comms.entity,
            None,
            "Comms block",
        )?;
        let responses = parse_comms_responses(&raw_comms.responses)?;
        let node = CommsDialogueNode { body: raw_comms.message, responses };
        comms.push(CommsTemplate { from: raw_comms.from, trigger, node });
    }

    // Validate extra_worlds: every entry must be a non-empty string.
    for (i, path) in raw.extra_worlds.iter().enumerate() {
        if path.trim().is_empty() {
            return Err(format!(
                "extra_worlds[{i}] is an empty string; all paths must be non-empty"
            ));
        }
    }

    Ok(WorldConfig {
        global: raw.global,
        anchors,
        entities: raw.entities,
        triggers,
        comms,
        name_to_uuid: HashMap::new(),
        extra_worlds: raw.extra_worlds,
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
    entities: &[WorldEntity],
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
/// (`spawn_world_entities`), rather than the complementary `setup_world`
/// path in `server_app.rs`?
///
/// PRD #337 routes two kinds of entries through the unified pipeline:
/// * **Slice 1**: any entry whose resolved template is an asteroid field.
/// * **Slice 2**: any entry carrying a `name` field — the unified pipeline
///   assigns the UUID so `name → uuid` is single-sourced.
///
/// Both call sites (legacy + unified) call this helper with the same
/// `is_asteroid_field` lookup to guarantee no entry is spawned twice.
pub fn is_owned_by_unified_pipeline<F>(
    entity_inst: &WorldEntity,
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
/// Asteroid-field instances and any `[[entity]]` carrying a `name` field flow
/// through the `spawn_world_entities` Bevy system; every other immediate-spawn
/// instance flows through the complementary `setup_world` system in
/// `server_app.rs`. Keeping the partitioning logic pure means both call sites
/// consult the same source of truth and double-spawn is impossible.
///
/// Only `WorldEntitySpawnOn::Immediate` entries are considered; `GameStart`
/// entries are returned in neither bucket (they're handled by
/// `spawn_game_start_entities`).
pub fn partition_immediate_entities<F>(
    world: &WorldConfig,
    is_asteroid_field: F,
) -> (Vec<&crate::world::config::WorldEntity>, Vec<&crate::world::config::WorldEntity>)
where
    F: Fn(&str) -> bool,
{
    use crate::world::config::WorldEntitySpawnOn;
    let mut fields = Vec::new();
    let mut others = Vec::new();
    for ent in &world.entities {
        if ent.spawn_on != WorldEntitySpawnOn::Immediate {
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
/// Build a map of `name → resolved_position` for every named `[[entity]]`
/// instance in the world, used as the lookup table for `relative_to`
/// references during spawning.
///
/// Named entities are resolved using anchor/inline `position` only (NOT
/// `relative_to`), which means relative-to-relative chains are not supported.
/// This is intentional: it keeps resolution single-pass and avoids cycle
/// detection complexity for a feature whose primary use is "spawn an enemy
/// 10 units off this landmark".
///
/// Resolution failures (missing anchor) are silently skipped — the affected
/// entity will produce its own error when its position is resolved at spawn
/// time, so this helper doesn't need to duplicate error reporting.
pub fn build_named_entity_positions(world: &WorldConfig) -> HashMap<String, [f32; 3]> {
    let mut out = HashMap::new();
    for ent in &world.entities {
        let Some(name) = ent.name.as_ref() else { continue };
        // Skip entities whose own position is relative_to-based — their
        // position isn't valid as a base for further relative_to lookups.
        if ent.relative_to.is_some() {
            continue;
        }
        if let Ok(pos) = resolve_entity_position(ent, &world.anchors) {
            out.insert(name.clone(), pos);
        }
    }
    out
}

pub fn resolve_entity_position(
    entity_inst: &crate::world::config::WorldEntity,
    anchors: &HashMap<String, [f32; 3]>,
) -> Result<[f32; 3], String> {
    resolve_entity_position_with(entity_inst, anchors, &HashMap::new())
}

/// Extended position resolver supporting `relative_to`+`offset`.
///
/// `entities_by_name` maps already-resolved named entity positions; callers
/// performing two-pass spawning populate it with results from the first pass.
/// Resolution precedence: `relative_to` → `anchor` → inline `position` →
/// origin. A `relative_to` that doesn't appear in `entities_by_name` (forward
/// reference or typo) is an error.
pub fn resolve_entity_position_with(
    entity_inst: &crate::world::config::WorldEntity,
    anchors: &HashMap<String, [f32; 3]>,
    entities_by_name: &HashMap<String, [f32; 3]>,
) -> Result<[f32; 3], String> {
    if let Some(name) = entity_inst.relative_to.as_ref() {
        let base = entities_by_name.get(name).ok_or_else(|| {
            format!(
                "Entity (template '{}') references unknown relative_to entity '{}' \
                 (must be a previously-declared named entity in the same world)",
                entity_inst.template_path, name
            )
        })?;
        let ox = entity_inst.offset.first().copied().unwrap_or(0.0);
        let oy = entity_inst.offset.get(1).copied().unwrap_or(0.0);
        let oz = entity_inst.offset.get(2).copied().unwrap_or(0.0);
        return Ok([base[0] + ox, base[1] + oy, base[2] + oz]);
    }
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
/// complementary `setup_world` path in `server_app.rs` only spawns the third
/// bucket (anonymous non-asteroid entries).
///
/// Returns `(asteroid_fields, named_non_asteroid, anonymous_non_asteroid)`.
/// `GameStart` entries are returned in none of the three buckets.
pub fn partition_immediate_entities_three_way<F>(
    world: &WorldConfig,
    is_asteroid_field: F,
) -> (
    Vec<&crate::world::config::WorldEntity>,
    Vec<&crate::world::config::WorldEntity>,
    Vec<&crate::world::config::WorldEntity>,
)
where
    F: Fn(&str) -> bool,
{
    use crate::world::config::WorldEntitySpawnOn;
    let mut fields = Vec::new();
    let mut named = Vec::new();
    let mut anon = Vec::new();
    for ent in &world.entities {
        if ent.spawn_on != WorldEntitySpawnOn::Immediate {
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
    use crate::world::config::WorldEntitySpawnOn;

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
        assert_eq!(cfg.entities[0].spawn_on, WorldEntitySpawnOn::Immediate);
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
        // map from a slice of WorldEntity. Anonymous entries are skipped;
        // a caller-supplied generator yields the UUIDs (so tests can be
        // deterministic without dragging real RNG in).
        let entities = vec![
            WorldEntity {
                template_path: "assets/entities/station_outpost.toml".into(),
                name: Some("starbase_alpha".into()),
                ..Default::default()
            },
            WorldEntity {
                template_path: "assets/entities/star_sun.toml".into(),
                name: None,
                ..Default::default()
            },
            WorldEntity {
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
        // The complementary `setup_world` path in `server_app.rs` must skip
        // both asteroid-field templates AND any entry carrying a `name` field
        // (owned by `spawn_world_entities`).
        let asteroid = WorldEntity {
            template_path: "assets/entities/asteroid_field_dense.toml".into(),
            ..Default::default()
        };
        let named = WorldEntity {
            template_path: "assets/entities/station_outpost.toml".into(),
            name: Some("starbase_alpha".into()),
            ..Default::default()
        };
        let anon = WorldEntity {
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
        // iterate both fields AND named entries while the complementary
        // `setup_world` in `server_app.rs` keeps anonymous ones.
        let mut cfg = WorldConfig::default();
        cfg.entities.push(WorldEntity {
            template_path: "assets/entities/asteroid_field_main.toml".into(),
            ..Default::default()
        });
        cfg.entities.push(WorldEntity {
            template_path: "assets/entities/station_outpost.toml".into(),
            name: Some("starbase_alpha".into()),
            ..Default::default()
        });
        cfg.entities.push(WorldEntity {
            template_path: "assets/entities/star_sun.toml".into(),
            ..Default::default()
        });
        // game_start entries are in no bucket
        cfg.entities.push(WorldEntity {
            template_path: "assets/entities/player_ship.toml".into(),
            spawn_on: crate::world::config::WorldEntitySpawnOn::GameStart,
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
        let entity = WorldEntity {
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
        let entity = WorldEntity {
            template_path: "assets/entities/star_sun.toml".into(),
            position: vec![10.0, 0.0, 20.0],
            ..Default::default()
        };
        let pos = resolve_entity_position(&entity, &HashMap::new()).unwrap();
        assert_eq!(pos, [10.0, 0.0, 20.0]);
    }

    #[test]
    fn resolve_entity_position_errors_on_unknown_anchor() {
        let entity = WorldEntity {
            template_path: "assets/entities/pirate_raider.toml".into(),
            anchor: Some("ghost".into()),
            ..Default::default()
        };
        let err = resolve_entity_position(&entity, &HashMap::new()).unwrap_err();
        assert!(err.contains("ghost"), "error must mention missing anchor: {err}");
    }

    #[test]
    fn resolve_entity_position_anchor_wins_over_inline_position() {
        let entity = WorldEntity {
            template_path: "x.toml".into(),
            anchor: Some("a".into()),
            position: vec![999.0, 999.0, 999.0],
            ..Default::default()
        };
        let anchors = anchor_table(&[("a", [1.0, 2.0, 3.0])]);
        let pos = resolve_entity_position(&entity, &anchors).unwrap();
        assert_eq!(pos, [1.0, 2.0, 3.0]);
    }

    // ── relative_to + offset (PRD #337 — closing AC) ──────────────────────

    fn resolved_table(entries: &[(&str, [f32; 3])]) -> HashMap<String, [f32; 3]> {
        entries
            .iter()
            .map(|(k, v)| ((*k).to_string(), *v))
            .collect()
    }

    #[test]
    fn resolve_entity_position_relative_to_adds_offset_to_referenced_entity() {
        let entity = WorldEntity {
            template_path: "assets/entities/pirate_raider.toml".into(),
            relative_to: Some("starbase_alpha".into()),
            offset: vec![10.0, 0.0, -5.0],
            ..Default::default()
        };
        let resolved = resolved_table(&[("starbase_alpha", [100.0, 0.0, 200.0])]);
        let pos = resolve_entity_position_with(&entity, &HashMap::new(), &resolved).unwrap();
        assert_eq!(pos, [110.0, 0.0, 195.0]);
    }

    #[test]
    fn resolve_entity_position_relative_to_with_missing_offset_uses_zero() {
        let entity = WorldEntity {
            template_path: "x.toml".into(),
            relative_to: Some("origin".into()),
            ..Default::default()
        };
        let resolved = resolved_table(&[("origin", [5.0, 6.0, 7.0])]);
        let pos = resolve_entity_position_with(&entity, &HashMap::new(), &resolved).unwrap();
        assert_eq!(pos, [5.0, 6.0, 7.0]);
    }

    #[test]
    fn resolve_entity_position_relative_to_errors_on_unknown_reference() {
        let entity = WorldEntity {
            template_path: "x.toml".into(),
            relative_to: Some("ghost".into()),
            ..Default::default()
        };
        let err = resolve_entity_position_with(&entity, &HashMap::new(), &HashMap::new())
            .unwrap_err();
        assert!(
            err.contains("ghost") && err.contains("relative_to"),
            "error must mention missing reference and relative_to: {err}"
        );
    }

    #[test]
    fn resolve_entity_position_relative_to_wins_over_anchor_and_position() {
        let entity = WorldEntity {
            template_path: "x.toml".into(),
            anchor: Some("a".into()),
            position: vec![999.0, 999.0, 999.0],
            relative_to: Some("base".into()),
            offset: vec![1.0, 1.0, 1.0],
            ..Default::default()
        };
        let anchors = anchor_table(&[("a", [50.0, 50.0, 50.0])]);
        let resolved = resolved_table(&[("base", [10.0, 0.0, 0.0])]);
        let pos = resolve_entity_position_with(&entity, &anchors, &resolved).unwrap();
        assert_eq!(pos, [11.0, 1.0, 1.0]);
    }

    #[test]
    fn parse_world_accepts_relative_to_and_offset_on_entity() {
        let toml = r#"
[[entity]]
template_path = "assets/entities/starbase_alpha.toml"
name          = "starbase_alpha"
position      = [100.0, 0.0, 200.0]

[[entity]]
template_path = "assets/entities/pirate_raider.toml"
relative_to   = "starbase_alpha"
offset        = [10.0, 0.0, -5.0]
"#;
        let world = parse_world(toml).expect("parse");
        assert_eq!(world.entities.len(), 2);
        let raider = &world.entities[1];
        assert_eq!(raider.relative_to.as_deref(), Some("starbase_alpha"));
        assert_eq!(raider.offset, vec![10.0, 0.0, -5.0]);
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
        assert_eq!(cfg.entities[0].spawn_on, WorldEntitySpawnOn::GameStart);
        assert_eq!(cfg.entities[0].id.as_deref(), Some("player-ship"));
    }

    #[test]
    fn parse_world_silently_ignores_legacy_spawn_blocks() {
        // PRD #341: the unified parser owns [[entity]], [[trigger]], and
        // [[comms]]. Legacy [[spawn]] blocks (no longer used by any shipped
        // world) are silently ignored — they must not error.
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
"#;
        let cfg = parse_world(toml).expect("legacy [[spawn]] blocks must be ignored, not errored");
        assert_eq!(cfg.entities.len(), 1);
        assert_eq!(cfg.anchors.len(), 1);
    }

    // ── Triggers & comms (PRD #341) ───────────────────────────────────────

    #[test]
    fn parse_world_reads_on_destroyed_trigger_with_actions() {
        let toml = r#"
[[trigger]]
condition = "on_destroyed"
entity    = "raider_alpha"

  [[trigger.action]]
  type      = "add_objective"
  id        = "obj-raider-destroyed"
  text      = "Pirate raider eliminated."
  mandatory = false
"#;
        let cfg = parse_world(toml).expect("must parse");
        assert_eq!(cfg.triggers.len(), 1);
        assert_eq!(
            cfg.triggers[0].condition,
            TriggerCondition::OnDestroyed { entity_name: "raider_alpha".to_string() }
        );
        assert_eq!(cfg.triggers[0].actions.len(), 1);
        match &cfg.triggers[0].actions[0] {
            TriggerAction::AddObjective { id, text, mandatory } => {
                assert_eq!(id, "obj-raider-destroyed");
                assert_eq!(text, "Pirate raider eliminated.");
                assert_eq!(*mandatory, false);
            }
            other => panic!("expected AddObjective, got {other:?}"),
        }
    }

    #[test]
    fn parse_world_reads_on_attacked_comms_template() {
        let toml = r#"
[[comms]]
from    = "raider_alpha"
trigger = "on_attacked"
entity  = "raider_alpha"
message = "MAYDAY!"
"#;
        let cfg = parse_world(toml).expect("must parse");
        assert_eq!(cfg.comms.len(), 1);
        assert_eq!(cfg.comms[0].from, "raider_alpha");
        assert_eq!(
            cfg.comms[0].trigger,
            TriggerCondition::OnAttacked { entity_name: "raider_alpha".to_string() }
        );
        assert_eq!(cfg.comms[0].node.body, "MAYDAY!");
        assert!(cfg.comms[0].node.responses.is_empty());
    }

    #[test]
    fn parse_world_reads_comms_response_tree_with_actions() {
        let toml = r#"
[[comms]]
from    = "Starbase Alpha"
trigger = "on_hailed"
entity  = "Starbase Alpha"
message = "Please state your business."

  [[comms.response]]
  text = "We are on a survey mission."
    [[comms.response.action]]
    type = "add_objective"
    id   = "obj-survey"
    text = "Complete the survey."

  [[comms.response]]
  text = "We require docking clearance."
    [[comms.response.action]]
    type      = "add_objective"
    id        = "obj-dock"
    text      = "Dock at Starbase Alpha."
    mandatory = true
"#;
        let cfg = parse_world(toml).expect("must parse");
        assert_eq!(cfg.comms.len(), 1);
        let tmpl = &cfg.comms[0];
        assert_eq!(tmpl.node.responses.len(), 2);
        assert_eq!(tmpl.node.responses[0].text, "We are on a survey mission.");
        assert_eq!(tmpl.node.responses[0].actions.len(), 1);
        assert_eq!(tmpl.node.responses[1].text, "We require docking clearance.");
        match &tmpl.node.responses[1].actions[0] {
            TriggerAction::AddObjective { mandatory, .. } => assert!(*mandatory),
            _ => panic!("expected mandatory AddObjective"),
        }
    }

    #[test]
    fn parse_world_unknown_trigger_condition_errors() {
        let toml = r#"
[[trigger]]
condition = "on_zombie"
"#;
        let err = parse_world(toml).expect_err("unknown trigger condition must error");
        assert!(err.contains("on_zombie"), "error must mention the bad condition: {err}");
    }

    #[test]
    fn parse_world_default_toml_loads_triggers_and_comms() {
        let toml = include_str!("../../assets/worlds/default.toml");
        let cfg = parse_world(toml).expect("default.toml must parse");
        assert_eq!(cfg.triggers.len(), 2, "default.toml has 2 [[trigger]]s");
        assert_eq!(cfg.comms.len(), 3, "default.toml has 3 [[comms]] templates");
    }

    #[test]
    fn parse_world_patrol_toml_loads_triggers_with_no_comms() {
        let toml = include_str!("../../assets/worlds/patrol.toml");
        let cfg = parse_world(toml).expect("patrol.toml must parse");
        assert_eq!(cfg.triggers.len(), 1);
        assert!(cfg.comms.is_empty());
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

    // ── extra_worlds (issue #352) ─────────────────────────────────────────

    #[test]
    fn parse_world_extra_worlds_defaults_to_empty() {
        let cfg = parse_world("").expect("empty TOML should parse");
        assert!(cfg.extra_worlds.is_empty());
    }

    #[test]
    fn parse_world_reads_extra_worlds_list() {
        let toml = r#"
extra_worlds = ["assets/worlds/patrol.toml", "assets/worlds/side_mission.toml"]
"#;
        let cfg = parse_world(toml).expect("must parse");
        assert_eq!(cfg.extra_worlds.len(), 2);
        assert_eq!(cfg.extra_worlds[0], "assets/worlds/patrol.toml");
        assert_eq!(cfg.extra_worlds[1], "assets/worlds/side_mission.toml");
    }

    #[test]
    fn parse_world_rejects_empty_string_in_extra_worlds() {
        let toml = r#"
extra_worlds = ["assets/worlds/patrol.toml", ""]
"#;
        let err = parse_world(toml).expect_err("empty path in extra_worlds must error");
        assert!(
            err.contains("extra_worlds"),
            "error must mention extra_worlds: {err}"
        );
    }

    #[test]
    fn parse_world_rejects_whitespace_only_string_in_extra_worlds() {
        let toml = r#"
extra_worlds = ["   "]
"#;
        let err = parse_world(toml).expect_err("whitespace-only path in extra_worlds must error");
        assert!(
            err.contains("extra_worlds"),
            "error must mention extra_worlds: {err}"
        );
    }

    #[test]
    fn parse_world_extra_worlds_round_trips_via_worldconfig() {
        let toml = r#"
extra_worlds = ["assets/worlds/patrol.toml"]
"#;
        let cfg = parse_world(toml).expect("must parse");
        assert_eq!(cfg.extra_worlds, vec!["assets/worlds/patrol.toml".to_string()]);
    }

    // ── LoadWorld / UnloadWorld trigger actions (issue #352) ─────────────

    #[test]
    fn parse_world_load_world_action_parses() {
        let toml = r#"
[[trigger]]
condition = "on_timer"
after_secs = 10.0

  [[trigger.action]]
  type = "load_world"
  path = "assets/worlds/patrol.toml"
"#;
        let cfg = parse_world(toml).expect("must parse");
        assert_eq!(cfg.triggers.len(), 1);
        assert_eq!(cfg.triggers[0].actions.len(), 1);
        match &cfg.triggers[0].actions[0] {
            TriggerAction::LoadWorld { path } => {
                assert_eq!(path, "assets/worlds/patrol.toml");
            }
            other => panic!("expected LoadWorld, got {other:?}"),
        }
    }

    #[test]
    fn parse_world_unload_world_action_parses() {
        let toml = r#"
[[trigger]]
condition = "on_timer"
after_secs = 20.0

  [[trigger.action]]
  type = "unload_world"
  path = "assets/worlds/patrol.toml"
"#;
        let cfg = parse_world(toml).expect("must parse");
        assert_eq!(cfg.triggers.len(), 1);
        match &cfg.triggers[0].actions[0] {
            TriggerAction::UnloadWorld { path } => {
                assert_eq!(path, "assets/worlds/patrol.toml");
            }
            other => panic!("expected UnloadWorld, got {other:?}"),
        }
    }

    #[test]
    fn parse_world_load_world_action_requires_path_field() {
        let toml = r#"
[[trigger]]
condition = "on_timer"
after_secs = 10.0

  [[trigger.action]]
  type = "load_world"
"#;
        let err = parse_world(toml).expect_err("load_world without path must error");
        assert!(
            err.contains("load_world") && err.contains("path"),
            "error must mention load_world and path: {err}"
        );
    }

    #[test]
    fn parse_world_unload_world_action_requires_path_field() {
        let toml = r#"
[[trigger]]
condition = "on_timer"
after_secs = 10.0

  [[trigger.action]]
  type = "unload_world"
"#;
        let err = parse_world(toml).expect_err("unload_world without path must error");
        assert!(
            err.contains("unload_world") && err.contains("path"),
            "error must mention unload_world and path: {err}"
        );
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
