// Pure trigger-action dispatch table for the world engine (issue #708).
//
// Pure Rust module — no Bevy. This module owns the single `match` over every
// `TriggerAction` variant. It reads a read-only `DispatchContext` and returns a
// `DispatchResult` describing *what should happen*, without performing any of
// it. The Bevy layer (`world::server`) builds the context, calls
// `dispatch_action`, and applies the result (issue #710 wires it up).
//
// # Why a pure table
//
// The dispatch table used to be inlined twice inside `world::server` — once in
// `tick_trigger_pipeline` for immediate actions and once in `dispatch_single_action`
// for delayed ones. Both copies mixed decision logic (name lookups, `parent:`
// walking, anchor resolution, transition detection) with ECS mutation, so none
// of it could be tested without spinning up a Bevy `App`. Splitting the
// decision out makes every arm — including every failure path — a plain
// function call over plain data.
//
// # Purity boundaries
//
// * **Entities are UUID strings, never `Entity`.** The six modifier arms
//   ultimately write a `ShipModifiers` component, which is irreducibly impure.
//   `dispatch_action` resolves entity *name* → *UUID* from
//   `DispatchContext::name_to_uuid` and stops there; the applier does
//   UUID → `Entity` → component.
// * **Logging becomes data.** Failure paths push a message onto
//   `DispatchResult::warnings` instead of calling the Bevy log macros. The
//   applier logs them at warn level; tests assert on them.
// * **Flag mutation is previewed, not performed.** The `parent:` layer walk and
//   the before/after transition are computed here (mirroring `FlagStore`'s own
//   semantics exactly); the applier performs the write. This is the idiom
//   `world::flags` already documents. The preview reads the *live* stores, so
//   the applier must apply each `DispatchResult` before dispatching the next
//   action or flag idempotence breaks — see `DispatchContext::base_flags`.
// * **New events are destination-agnostic.** `DispatchResult::new_events` are
//   returned, not routed. The immediate path feeds them into the next chaining
//   pass (same tick); the delayed path queues them for the next tick. The
//   caller decides.
// * **Non-determinism is injected.** `SpawnEntity` takes its UUID from
//   `DispatchContext::uuid_source` so tests can stub it.
// * **Template loading is injected** (issue #715). `SpawnEntity` resolves its
//   template through `DispatchContext::template_loader`, so this module never
//   touches the filesystem or the WASM config cache itself — and the
//   failed-spawn contingency gate lives *here*: a template that fails to
//   resolve returns a warning-only result with no command and no name/group
//   inserts, so the applier applies `DispatchResult`s unconditionally.

use std::collections::HashMap;
use uuid::Uuid;

use crate::entity_config::EntityConfig;
use crate::entity_loader::TemplateLoader;
use crate::faction::FactionRegistry;
use crate::messages::FlagKind;
use crate::messages::{AiDirective, GamePhase, ModifierSlot, ObjectiveSource};
use crate::modifiers::IntModifierSlot;
use crate::objectives::UtilityConfig;
use crate::world::config::TriggerAction;
use crate::world::content::WorldEvent;
use crate::world::flags::FlagStore;

/// The `ModifierSource::World { id }` discriminator used by every modifier and
/// flag arm. This is a source *identity*, not a tunable gameplay value: it is
/// what lets `RemoveModifier` find the modifier a previous `ApplyModifier`
/// added. The applier reconstructs `ModifierSource::World { id, tag }` from
/// this const plus the `ActionCmd`'s `tag`.
pub const WORLD_MODIFIER_SOURCE_ID: &str = "world";

// ── Context ───────────────────────────────────────────────────────────────

/// Read-only view of one additively-loaded world layer.
///
/// Mirrors the fields of `world::server::WorldRuntime` that the dispatch table
/// reads. The Bevy layer projects `WorldLayerMap` into these. Freshness is
/// per-field, not per-struct — see each field, and `DispatchContext`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LayerView {
    /// The layer's own flag store. **Must be the live store** — same rule, and
    /// same reason, as `DispatchContext::base_flags`.
    pub flags: FlagStore,
    /// Path of the layer whose trigger loaded this one (`None` = base world).
    /// Each `parent:` prefix on a flag name walks one step along these.
    pub loader_path: Option<String>,
    /// The layer's anchor table, used to resolve `SpawnEntity { anchor }` for
    /// triggers authored in this layer.
    pub anchors: HashMap<String, [f32; 3]>,
}

/// Everything `dispatch_action` is allowed to read.
///
/// Built fresh per fired trigger by the Bevy caller. **Freshness is specified
/// per field, not for the struct as a whole** — the rules genuinely differ, so
/// read the field docs rather than assuming one blanket rule. In short:
/// `name_to_uuid` is rebuilt once per chaining pass, whereas `base_flags` and
/// `layers[].flags` must be the *live* stores, never a per-pass snapshot.
pub struct DispatchContext<'a> {
    /// Authoring layer of the trigger that fired (`None` = base world).
    /// Anchors `parent:` walks, `LoadWorld`'s `loader_path`, and
    /// `SpawnEntity`'s anchor lookup.
    pub origin_layer: Option<String>,
    /// The trigger condition's entity, if any. `AddObjective` falls back to
    /// this when the action lists no explicit `targets`.
    pub entity_name: Option<String>,
    /// Live entity name → UUID map.
    ///
    /// Rebuilt once per chaining pass (as is the runtime's `entity_groups`
    /// map, which this module only writes to via
    /// `DispatchResult::entity_group_inserts`) — a deliberate decision, so an
    /// action resolves names a `SpawnEntity` earlier in the pass registered.
    /// Issue #708: the old code read a tick-stale snapshot in the modifier arms
    /// but the live map in `DestroyEntity`.
    pub name_to_uuid: &'a HashMap<String, String>,
    /// The base world's flag store (the root of every `parent:` walk).
    ///
    /// **MUST be the live store**, reflecting every command already applied
    /// this pass: the applier applies each `DispatchResult` before dispatching
    /// the next action. `before`/`after` — and hence whether a transition event
    /// is emitted at all — are computed against it.
    ///
    /// Condition evaluation in `server::tick_trigger_pipeline` borrows these
    /// same live stores, which is safe only because every condition of a pass
    /// is evaluated before any of its actions is dispatched. What MUST NOT be
    /// passed here is a store copied before the pass's dispatches began — the
    /// tempting wrong move when refactoring the call site. It breaks
    /// flag idempotence: two triggers both `set_flag aphelion_armed`
    /// (`assets/worlds/before_the_fire.toml:275`) against live stores go `0→1`
    /// (emits `FlagSet`) then `1→1` (emits nothing) = one event, so the
    /// downstream `on_flag_set` trigger fires exactly once. Against a snapshot
    /// both read `before = 0`, emit `FlagSet` twice, and it double-fires.
    pub base_flags: &'a FlagStore,
    /// Loaded sub-world layers, keyed by world TOML path. Each layer's `flags`
    /// carries the same live-store requirement as `base_flags`.
    pub layers: &'a HashMap<String, LayerView>,
    /// The base world's anchor table. Empty when no world config is loaded.
    pub base_anchors: &'a HashMap<String, [f32; 3]>,
    /// The live faction registry. `None` when the registry resource is absent,
    /// which makes the two faction arms warn and skip.
    pub factions: Option<&'a FactionRegistry>,
    /// Source of fresh UUIDs for `SpawnEntity`. Injected because
    /// `Uuid::new_v4()` is non-deterministic and would make the returned
    /// `DispatchResult` unassertable.
    pub uuid_source: &'a dyn Fn() -> String,
    /// Source of `SpawnEntity` templates. Injected (like `uuid_source`) so
    /// this module never touches the filesystem or the WASM config cache: the
    /// applier passes `entity_loader::WasmTemplateLoader` — config cache
    /// first, filesystem fallback on native — and tests pass a hand-written
    /// fake. Diagnostics are this module's job: `TemplateLoader` deliberately
    /// collapses "missing" and "malformed" into `None`, and the `SpawnEntity`
    /// arm turns that into a warning naming the entity and template path.
    pub template_loader: &'a dyn TemplateLoader,
}

// ── Result ────────────────────────────────────────────────────────────────

/// A flag mutation to apply to a resolved `FlagStore`.
///
/// Mirrors `FlagStore`'s mutators one-for-one.
#[derive(Clone, Debug, PartialEq)]
pub enum FlagMutation {
    /// `set_flag` — counter := 1.
    Set,
    /// `clear_flag` — counter := 0.
    Clear,
    /// `increment_flag` — counter := counter.saturating_add(by).
    Increment(i64),
    /// `set_flag_value` — counter := value.
    SetValue(i64),
}

/// A single side effect for the Bevy applier to perform.
///
/// Entity targets are UUID strings; faction targets are already-resolved
/// `Uuid`s. Nothing here names a Bevy `Entity`.
#[derive(Clone, Debug, PartialEq)]
pub enum ActionCmd {
    /// Add an objective with `targets` already resolved (explicit targets, or
    /// the trigger entity as fallback).
    AddObjective {
        id: String,
        text: String,
        mandatory: bool,
        targets: Vec<String>,
        directive: AiDirective,
        utility: UtilityConfig,
        source: ObjectiveSource,
        /// Sub-world layer that authored the trigger adding this objective, or
        /// `None` for base-world triggers (issue #751). The applier records
        /// layer-owned objective ids so `UnloadWorld` removes them.
        origin_layer: Option<String>,
    },
    /// Mark an objective complete. A no-op for unknown / non-Active ids.
    CompleteObjective { id: String },
    /// Re-arm the trigger(s) with the given authored id (issue #751).
    ResetTrigger { id: String },
    /// Mark an objective failed. A no-op for unknown / non-Active ids.
    FailObjective { id: String },
    /// Add or update a float modifier on the entity with `uuid`.
    ApplyModifier {
        uuid: String,
        tag: String,
        slot: ModifierSlot,
        bonus: f32,
    },
    /// Remove a float modifier from the entity with `uuid`.
    RemoveModifier {
        uuid: String,
        tag: String,
        slot: ModifierSlot,
    },
    /// Add a boolean flag modifier to the entity with `uuid`.
    ApplyFlag {
        uuid: String,
        tag: String,
        kind: FlagKind,
    },
    /// Remove a boolean flag modifier from the entity with `uuid`.
    RemoveFlag {
        uuid: String,
        tag: String,
        kind: FlagKind,
    },
    /// Add or update an integer modifier on the entity with `uuid`.
    ApplyIntModifier {
        uuid: String,
        tag: String,
        slot: IntModifierSlot,
        bonus: i32,
    },
    /// Remove an integer modifier from the entity with `uuid`.
    RemoveIntModifier {
        uuid: String,
        tag: String,
        slot: IntModifierSlot,
    },
    /// Write the game-over reason resource — reason string plus the declared
    /// [`Outcome`](crate::balance::Outcome) (#843, `None` for an undeclared
    /// scripted end).
    ///
    /// Always emitted *before* `SetNextState` — `OnEnter(GamePhase::GameOver)`
    /// reads the reason, so the ordering is load-bearing.
    SetGameOverReason {
        reason: String,
        outcome: Option<crate::balance::Outcome>,
    },
    /// Queue a game-phase transition.
    SetNextState { phase: GamePhase },
    /// Additively load a sub-world. `loader_path` is the layer that issued the
    /// action, recorded so `parent:` from the new layer resolves up to it.
    LoadWorld {
        path: String,
        loader_path: Option<String>,
    },
    /// Unload a previously loaded sub-world.
    UnloadWorld { path: String },
    /// Apply `mutation` to `name` in `target_layer`'s store (`None` = base
    /// world). `name` is already stripped of `parent:` prefixes and
    /// `target_layer` is the walk's resolved destination.
    MutateFlag {
        target_layer: Option<String>,
        name: String,
        mutation: FlagMutation,
    },
    /// Spawn an entity. `config` is the template already resolved via
    /// `DispatchContext::template_loader`, with the trigger's `name` patched
    /// in — the applier loads nothing. `position` is resolved (anchor lookups
    /// already done); `uuid` came from `DispatchContext::uuid_source`. When
    /// `layer_path` is `Some`, the applier records the spawned entity on that
    /// layer so `UnloadWorld` despawns it.
    ///
    /// A template that fails to resolve never reaches here: the dispatch arm
    /// returns a warning-only result instead — see `dispatch_spawn_entity`.
    ///
    /// `config` is boxed because `EntityConfig` dwarfs every other variant
    /// (clippy: `large_enum_variant`) and commands travel in `Vec<ActionCmd>`.
    SpawnEntity {
        config: Box<EntityConfig>,
        name: String,
        uuid: String,
        position: [f32; 3],
        rotation: Option<[f32; 3]>,
        scale: Option<[f32; 3]>,
        layer_path: Option<String>,
        /// Optional inline TOML overrides already applied to `config` by the
        /// dispatch function; preserved here for auditing / test assertions.
        overrides: Option<toml::Value>,
    },
    /// Destroy the entity with `uuid` and run the destruction cascade.
    DestroyEntity { uuid: String },
    /// Add `enemy_uuid` to `faction_uuid`'s enemies.
    ///
    /// Deliberately does *not* re-validate AI targets: adding a hostility
    /// cannot invalidate an existing engagement, and the next `enemy_in_range`
    /// tick picks the new relationship up organically.
    AddFactionEnemy {
        faction_uuid: Uuid,
        enemy_uuid: Uuid,
    },
    /// Remove `enemy_uuid` from `faction_uuid`'s enemies. **Only if the removal
    /// actually changed the registry** (`remove_enemy` returned true),
    /// re-validate every AI controller's target so an in-progress engagement
    /// does not stick on a now-friendly entity. Removing an absent hostility
    /// changes nothing, so it must not trigger the revalidation sweep.
    RemoveFactionEnemy {
        faction_uuid: Uuid,
        enemy_uuid: Uuid,
    },
}

/// What one `TriggerAction` decided to do.
///
/// Every field is additive and may be empty — an action that hit a failure path
/// returns only `warnings`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DispatchResult {
    /// Side effects for the applier, in the order they must be applied.
    pub commands: Vec<ActionCmd>,
    /// World events the action produced. **Destination-agnostic**: the caller
    /// routes them (into the current tick's chaining pass, or onto the next
    /// tick's queue).
    pub new_events: Vec<WorldEvent>,
    /// `(entity_name, uuid)` pairs to register in the runtime's name map.
    ///
    /// Unconditional, including from `SpawnEntity`: a spawn whose template
    /// fails to resolve returns a warning-only result with no inserts (issue
    /// #715 moved that contingency gate into `dispatch_spawn_entity`), so
    /// whatever arrives here the applier applies as-is.
    pub name_to_uuid_inserts: Vec<(String, String)>,
    /// `(group, entity_name)` pairs to register for `OnAllDestroyed` tracking.
    ///
    /// Unconditional, like `name_to_uuid_inserts`.
    pub entity_group_inserts: Vec<(String, String)>,
    /// Messages the applier should log at warn level.
    pub warnings: Vec<String>,
}

// ── Dispatch ──────────────────────────────────────────────────────────────

/// Decide what a single `TriggerAction` should do.
///
/// Pure: reads `context`, mutates nothing, returns a `DispatchResult` the Bevy
/// applier turns into real side effects. Covers every `TriggerAction` variant.
pub fn dispatch_action(action: &TriggerAction, context: &DispatchContext) -> DispatchResult {
    let mut out = DispatchResult::default();

    match action {
        // Mission-state actions — objectives, faction hostility, and the
        // game-over transition — are handled by `dispatch_state_action`
        // (issue #711). This table remains the single entry point over every
        // variant; it just routes these six to their own function.
        TriggerAction::AddObjective { .. }
        | TriggerAction::CompleteObjective { .. }
        | TriggerAction::FailObjective { .. }
        | TriggerAction::GameOver { .. }
        | TriggerAction::AddFactionEnemy { .. }
        | TriggerAction::RemoveFactionEnemy { .. } => {
            return dispatch_state_action(action, context);
        }

        TriggerAction::SetAiState {
            entity,
            state,
            target: _,
        } => {
            // No-op in doctrine-based AI (issue #572). FSM state slots are
            // gone; NPC behaviour is now driven by the scored doctrine pool.
            out.warnings.push(format!(
                "SetAiState('{entity}' → '{state}') ignored — doctrine-based AI"
            ));
        }

        // Entity modifier/flag actions — float modifiers, bool flags, and
        // integer modifiers on a named entity — are handled by
        // `dispatch_entity_modifier_action` (issue #712). Same routing shape
        // as the mission-state arm above.
        TriggerAction::ApplyModifier { .. }
        | TriggerAction::RemoveModifier { .. }
        | TriggerAction::ApplyFlag { .. }
        | TriggerAction::RemoveFlag { .. }
        | TriggerAction::ApplyIntModifier { .. }
        | TriggerAction::RemoveIntModifier { .. } => {
            return dispatch_entity_modifier_action(action, context);
        }

        TriggerAction::LoadWorld { path } => {
            out.commands.push(ActionCmd::LoadWorld {
                path: path.clone(),
                loader_path: context.origin_layer.clone(),
            });
        }

        TriggerAction::UnloadWorld { path } => {
            out.commands
                .push(ActionCmd::UnloadWorld { path: path.clone() });
        }

        TriggerAction::ResetTrigger { id } => {
            out.commands
                .push(ActionCmd::ResetTrigger { id: id.clone() });
        }

        // World-flag actions — the four mutations of a named flag in a
        // layer-resolved store — are handled by `dispatch_world_flag_action`
        // (issue #713). Same routing shape as the mission-state arm above.
        TriggerAction::SetWorldFlag { .. }
        | TriggerAction::ClearWorldFlag { .. }
        | TriggerAction::IncrementWorldFlag { .. }
        | TriggerAction::SetWorldFlagValue { .. } => {
            return dispatch_world_flag_action(action, context);
        }

        // Entity spawning — position resolution, template loading via the
        // injected `TemplateLoader`, uuid assignment, and the name/group
        // registrations — is handled by `dispatch_spawn_entity` (issue #715).
        // Same routing shape as the mission-state arm above.
        TriggerAction::SpawnEntity { .. } => {
            return dispatch_spawn_entity(action, context);
        }

        // Entity destruction — resolving the target's name to a UUID, then
        // emitting the destroy command plus the `Destroyed` event — is
        // handled by `dispatch_destroy_entity` (issue #714). Same routing
        // shape as the mission-state arm above.
        TriggerAction::DestroyEntity { .. } => {
            return dispatch_destroy_entity(action, context);
        }
    }

    out
}

/// Decide what one mission-state `TriggerAction` should do.
///
/// Owns the six actions that touch mission state — the three objective actions
/// (`AddObjective`, `CompleteObjective`, `FailObjective`), the two faction-
/// hostility actions (`AddFactionEnemy`, `RemoveFactionEnemy`), and the
/// `GameOver` transition. Split out of `dispatch_action` (issue #711), which
/// routes exactly these six variants here.
///
/// Pure, like `dispatch_action`: reads `context`, mutates nothing, returns a
/// `DispatchResult`. Called only for the six variants above; any other variant
/// is a routing bug in `dispatch_action` and trips the `unreachable!`.
fn dispatch_state_action(action: &TriggerAction, context: &DispatchContext) -> DispatchResult {
    let mut out = DispatchResult::default();

    match action {
        TriggerAction::AddObjective {
            id,
            text,
            mandatory,
            targets,
            directive,
            utility,
            source,
        } => {
            // Explicit targets win; otherwise fall back to the trigger
            // condition's entity (legacy behaviour).
            let resolved: Vec<String> = if targets.is_empty() {
                context.entity_name.clone().into_iter().collect()
            } else {
                targets.clone()
            };
            out.commands.push(ActionCmd::AddObjective {
                id: id.clone(),
                text: text.clone(),
                mandatory: *mandatory,
                targets: resolved,
                directive: directive.clone(),
                utility: utility.clone(),
                source: source.clone(),
                origin_layer: context.origin_layer.clone(),
            });
        }

        TriggerAction::CompleteObjective { id } => {
            out.commands
                .push(ActionCmd::CompleteObjective { id: id.clone() });
        }

        TriggerAction::FailObjective { id } => {
            out.commands
                .push(ActionCmd::FailObjective { id: id.clone() });
        }

        TriggerAction::GameOver { message, outcome } => {
            // `None` becomes `Some("")`, not `None` — preserved verbatim.
            let reason = message.clone().unwrap_or_default();
            // Reason first: `OnEnter(GamePhase::GameOver)` reads it.
            out.commands.push(ActionCmd::SetGameOverReason {
                reason,
                outcome: *outcome,
            });
            out.commands.push(ActionCmd::SetNextState {
                phase: GamePhase::GameOver,
            });
        }

        TriggerAction::AddFactionEnemy { faction, enemy } => {
            let Some(registry) = context.factions else {
                out.warnings.push(
                    "AddFactionEnemy skipped: FactionRegistryResource not present".to_string(),
                );
                return out;
            };
            let Some(faction_uuid) = registry.uuid_by_name(faction) else {
                out.warnings
                    .push(format!("AddFactionEnemy: unknown faction name '{faction}'"));
                return out;
            };
            let Some(enemy_uuid) = registry.uuid_by_name(enemy) else {
                out.warnings.push(format!(
                    "AddFactionEnemy: unknown enemy faction name '{enemy}'"
                ));
                return out;
            };
            out.commands.push(ActionCmd::AddFactionEnemy {
                faction_uuid,
                enemy_uuid,
            });
        }

        TriggerAction::RemoveFactionEnemy { faction, enemy } => {
            let Some(registry) = context.factions else {
                out.warnings.push(
                    "RemoveFactionEnemy skipped: FactionRegistryResource not present".to_string(),
                );
                return out;
            };
            let Some(faction_uuid) = registry.uuid_by_name(faction) else {
                out.warnings.push(format!(
                    "RemoveFactionEnemy: unknown faction name '{faction}'"
                ));
                return out;
            };
            let Some(enemy_uuid) = registry.uuid_by_name(enemy) else {
                out.warnings.push(format!(
                    "RemoveFactionEnemy: unknown enemy faction name '{enemy}'"
                ));
                return out;
            };
            out.commands.push(ActionCmd::RemoveFactionEnemy {
                faction_uuid,
                enemy_uuid,
            });
        }

        other => {
            unreachable!("dispatch_state_action called with non-state action: {other:?}")
        }
    }

    out
}

/// Decide what one entity-modifier `TriggerAction` should do.
///
/// Owns the six actions that stamp a modifier or flag onto a named entity —
/// the float-modifier pair (`ApplyModifier`, `RemoveModifier`), the bool-flag
/// pair (`ApplyFlag`, `RemoveFlag`), and the integer-modifier pair
/// (`ApplyIntModifier`, `RemoveIntModifier`). Split out of `dispatch_action`
/// (issue #712), which routes exactly these six variants here.
///
/// All six share one shape: resolve entity *name* → *UUID* via
/// `context.name_to_uuid` (warn + no-op on an unknown name), then emit the
/// matching `ActionCmd` keyed by UUID. The UUID → `Entity` → `ShipModifiers`
/// step stays in the applier — see "Purity boundaries" at the top of this
/// file.
///
/// Pure, like `dispatch_action`: reads `context`, mutates nothing, returns a
/// `DispatchResult`. Called only for the six variants above; any other variant
/// is a routing bug in `dispatch_action` and trips the `unreachable!`.
fn dispatch_entity_modifier_action(
    action: &TriggerAction,
    context: &DispatchContext,
) -> DispatchResult {
    let mut out = DispatchResult::default();

    match action {
        TriggerAction::ApplyModifier {
            entity,
            tag,
            slot,
            bonus,
        } => {
            let Some(uuid) = context.name_to_uuid.get(entity) else {
                out.warnings
                    .push(format!("ApplyModifier: unknown entity name '{entity}'"));
                return out;
            };
            out.commands.push(ActionCmd::ApplyModifier {
                uuid: uuid.clone(),
                tag: tag.clone(),
                slot: slot.clone(),
                bonus: *bonus,
            });
        }

        TriggerAction::RemoveModifier { entity, tag, slot } => {
            let Some(uuid) = context.name_to_uuid.get(entity) else {
                out.warnings
                    .push(format!("RemoveModifier: unknown entity name '{entity}'"));
                return out;
            };
            out.commands.push(ActionCmd::RemoveModifier {
                uuid: uuid.clone(),
                tag: tag.clone(),
                slot: slot.clone(),
            });
        }

        TriggerAction::ApplyFlag { entity, tag, kind } => {
            let Some(uuid) = context.name_to_uuid.get(entity) else {
                out.warnings
                    .push(format!("ApplyFlag: unknown entity name '{entity}'"));
                return out;
            };
            out.commands.push(ActionCmd::ApplyFlag {
                uuid: uuid.clone(),
                tag: tag.clone(),
                kind: kind.clone(),
            });
        }

        TriggerAction::RemoveFlag { entity, tag, kind } => {
            let Some(uuid) = context.name_to_uuid.get(entity) else {
                out.warnings
                    .push(format!("RemoveFlag: unknown entity name '{entity}'"));
                return out;
            };
            out.commands.push(ActionCmd::RemoveFlag {
                uuid: uuid.clone(),
                tag: tag.clone(),
                kind: kind.clone(),
            });
        }

        TriggerAction::ApplyIntModifier {
            entity,
            tag,
            slot,
            bonus,
        } => {
            let Some(uuid) = context.name_to_uuid.get(entity) else {
                out.warnings
                    .push(format!("ApplyIntModifier: unknown entity name '{entity}'"));
                return out;
            };
            out.commands.push(ActionCmd::ApplyIntModifier {
                uuid: uuid.clone(),
                tag: tag.clone(),
                slot: slot.clone(),
                bonus: *bonus,
            });
        }

        TriggerAction::RemoveIntModifier { entity, tag, slot } => {
            let Some(uuid) = context.name_to_uuid.get(entity) else {
                out.warnings
                    .push(format!("RemoveIntModifier: unknown entity name '{entity}'"));
                return out;
            };
            out.commands.push(ActionCmd::RemoveIntModifier {
                uuid: uuid.clone(),
                tag: tag.clone(),
                slot: slot.clone(),
            });
        }

        other => {
            unreachable!(
                "dispatch_entity_modifier_action called with non-modifier action: {other:?}"
            )
        }
    }

    out
}

/// Decide what one world-flag `TriggerAction` should do.
///
/// Owns the four actions that mutate a named world flag — `SetWorldFlag`,
/// `ClearWorldFlag`, `IncrementWorldFlag`, and `SetWorldFlagValue`. Split out
/// of `dispatch_action` (issue #713), which routes exactly these four
/// variants here.
///
/// All four share one body, `dispatch_flag_mutation` (below): resolve the
/// target layer across the loader chain (`resolve_flag_target`, honouring
/// `parent:` prefixes), preview the mutation against the resolved *live*
/// store (`preview_mutation` — see `DispatchContext::base_flags` for why it
/// must be live), emit the `ActionCmd::MutateFlag`, and push a `FlagSet` /
/// `FlagCleared` event when the boolean view flips (`push_flag_transition`).
/// The write itself stays in the applier — see "Purity boundaries" at the top
/// of this file.
///
/// Pure, like `dispatch_action`: reads `context`, mutates nothing, returns a
/// `DispatchResult`. Called only for the four variants above; any other
/// variant is a routing bug in `dispatch_action` and trips the `unreachable!`.
fn dispatch_world_flag_action(action: &TriggerAction, context: &DispatchContext) -> DispatchResult {
    let mut out = DispatchResult::default();

    match action {
        TriggerAction::SetWorldFlag { name } => {
            dispatch_flag_mutation(context, name, FlagMutation::Set, &mut out);
        }

        TriggerAction::ClearWorldFlag { name } => {
            dispatch_flag_mutation(context, name, FlagMutation::Clear, &mut out);
        }

        TriggerAction::IncrementWorldFlag { name, by } => {
            dispatch_flag_mutation(context, name, FlagMutation::Increment(*by), &mut out);
        }

        TriggerAction::SetWorldFlagValue { name, value } => {
            dispatch_flag_mutation(context, name, FlagMutation::SetValue(*value), &mut out);
        }

        other => {
            unreachable!("dispatch_world_flag_action called with non-world-flag action: {other:?}")
        }
    }

    out
}

// ── Flag-mutation helpers ─────────────────────────────────────────────────
//
// Used exclusively by `dispatch_world_flag_action` above (via
// `dispatch_flag_mutation`), so they live next to it.

/// Push a `FlagSet` / `FlagCleared` event when the boolean view (`counter != 0`)
/// of a flag flips.
///
/// `target_layer` is the *resolved* layer of the mutation (after `parent:`
/// walking) — embedded in the event so layer-scoped `on_flag_set` /
/// `on_flag_cleared` triggers only react to transitions in their own layer.
fn push_flag_transition(
    events: &mut Vec<WorldEvent>,
    name: &str,
    target_layer: &Option<String>,
    before: i64,
    after: i64,
) {
    let was_set = before != 0;
    let is_set = after != 0;
    if was_set == is_set {
        return;
    }
    if is_set {
        events.push(WorldEvent::FlagSet {
            name: name.to_string(),
            origin_layer: target_layer.clone(),
        });
    } else {
        events.push(WorldEvent::FlagCleared {
            name: name.to_string(),
            origin_layer: target_layer.clone(),
        });
    }
}

/// Resolve which layer a flag mutation targets, honouring `parent:` prefixes.
///
/// Each leading `parent:` walks one step up the loader chain from
/// `ctx.origin_layer`. Returns `(target_layer, stripped_name)` on success, or
/// the warning text on failure. There are two distinct failure modes and one
/// deliberate silent case:
///
/// * The walk overruns the base world → `Err` (no mutation, no event). This
///   keeps two scenarios from polluting each other's flag namespace.
/// * The resolved target layer is absent from the layer map → `Err`.
/// * A layer is missing from the map *mid-walk* → silently treated as the base
///   world, and the walk continues.
fn resolve_flag_target(
    ctx: &DispatchContext,
    name: &str,
) -> Result<(Option<String>, String), String> {
    let mut depth = 0usize;
    let mut rest = name;
    while let Some(stripped) = rest.strip_prefix("parent:") {
        depth += 1;
        rest = stripped;
    }
    let stripped = rest.to_string();

    let origin_layer = &ctx.origin_layer;
    let mut cur = origin_layer.clone();
    for _ in 0..depth {
        match cur {
            None => {
                return Err(format!(
                    "'{name}' from origin {origin_layer:?} walks past base world — ignoring"
                ));
            }
            Some(ref path) => {
                // A missing layer entry resolves to `None` — i.e. treated as
                // the base world — and the walk continues from there.
                cur = ctx.layers.get(path).and_then(|l| l.loader_path.clone());
            }
        }
    }
    let target_layer = cur;

    if let Some(path) = &target_layer {
        if !ctx.layers.contains_key(path) {
            return Err(format!(
                "target layer '{path}' missing from WorldLayerMap — ignoring '{name}'"
            ));
        }
    }
    Ok((target_layer, stripped))
}

/// Compute what `mutation` *would* do to `name` in `store`, without mutating.
///
/// Mirrors `FlagStore::set_flag` / `clear_flag` / `increment_flag` /
/// `set_flag_value` exactly, including `increment`'s saturating add.
fn preview_mutation(store: &FlagStore, name: &str, mutation: &FlagMutation) -> (i64, i64) {
    let before = store.counter(name);
    let after = match mutation {
        FlagMutation::Set => 1,
        FlagMutation::Clear => 0,
        FlagMutation::Increment(by) => before.saturating_add(*by),
        FlagMutation::SetValue(v) => *v,
    };
    (before, after)
}

/// Read-only lookup of the store a resolved `target_layer` points at.
fn store_for<'a>(ctx: &'a DispatchContext, target_layer: &Option<String>) -> &'a FlagStore {
    match target_layer {
        None => ctx.base_flags,
        // `resolve_flag_target` already proved the entry exists.
        Some(path) => ctx
            .layers
            .get(path)
            .map(|l| &l.flags)
            .unwrap_or(ctx.base_flags),
    }
}

/// Shared body of `dispatch_world_flag_action`'s four flag-mutation arms.
fn dispatch_flag_mutation(
    ctx: &DispatchContext,
    name: &str,
    mutation: FlagMutation,
    out: &mut DispatchResult,
) {
    let (target_layer, stripped) = match resolve_flag_target(ctx, name) {
        Ok(v) => v,
        Err(warning) => {
            out.warnings.push(warning);
            return;
        }
    };
    let (before, after) = preview_mutation(store_for(ctx, &target_layer), &stripped, &mutation);
    out.commands.push(ActionCmd::MutateFlag {
        target_layer: target_layer.clone(),
        name: stripped.clone(),
        mutation,
    });
    push_flag_transition(&mut out.new_events, &stripped, &target_layer, before, after);
}

/// Decide what a `DestroyEntity` `TriggerAction` should do.
///
/// Owns the single entity-destruction action. Split out of `dispatch_action`
/// (issue #714), which routes exactly this variant here.
///
/// Resolves entity *name* → *UUID* via `context.name_to_uuid` (warn + no-op
/// on an unknown name), then emits `ActionCmd::DestroyEntity` keyed by UUID
/// and pushes `WorldEvent::Destroyed` into `new_events` so chained
/// `on_destroyed` triggers observe the kill. The despawn + destruction
/// cascade — and the `AiEntityDestroyed` queueing — stay in the applier; see
/// "Purity boundaries" at the top of this file.
///
/// Pure, like `dispatch_action`: reads `context`, mutates nothing, returns a
/// `DispatchResult`. Called only for the variant above; any other variant is
/// a routing bug in `dispatch_action` and trips the `unreachable!`.
fn dispatch_destroy_entity(action: &TriggerAction, context: &DispatchContext) -> DispatchResult {
    let mut out = DispatchResult::default();

    match action {
        TriggerAction::DestroyEntity { entity } => {
            let Some(uuid) = context.name_to_uuid.get(entity) else {
                out.warnings
                    .push(format!("DestroyEntity: unknown entity name '{entity}'"));
                return out;
            };
            // The event lets chained `on_destroyed` triggers observe the kill;
            // the command runs the despawn + destruction cascade.
            out.new_events
                .push(WorldEvent::Destroyed { uuid: uuid.clone() });
            out.commands
                .push(ActionCmd::DestroyEntity { uuid: uuid.clone() });
        }

        other => {
            unreachable!("dispatch_destroy_entity called with non-destroy action: {other:?}")
        }
    }

    out
}

/// Decide what a `SpawnEntity` `TriggerAction` should do.
///
/// Owns the single entity-spawn action. Split out of `dispatch_action`
/// (issue #715), which routes exactly this variant here. Unlike the four
/// earlier extractions this one also *moved* work into the pure layer:
/// template loading, previously the applier's job, now goes through
/// `DispatchContext::template_loader`.
///
/// Steps, in order (template before uuid — matching the pre-#710 inline
/// semantics in `world::server`):
///
/// 1. Resolve the spawn position: an inline `position` wins; otherwise
///    `anchor` is looked up in the origin layer's anchor table (base-world
///    triggers look only in the base table, layer triggers only in their own
///    layer's — no fallback between them); otherwise warn + no-op.
/// 2. Load the template. `None` — missing or malformed; the trait collapses
///    both — warns and returns with **no command and no inserts**. This is
///    the contingency gate that used to live in the applier (`spawn_failed`):
///    registering the name/groups anyway would leave a phantom name → uuid
///    entry, and a later `DestroyEntity` would emit `WorldEvent::Destroyed`
///    for an entity that never existed, firing `on_destroyed` triggers for a
///    ghost.
/// 3. Draw the uuid from `context.uuid_source` — only after a successful
///    template load, so a failed spawn does not consume a uuid.
/// 4. Patch the trigger's `name` into the config, emit the command, and
///    register name → uuid plus group memberships (unconditional within this
///    function because step 2 already gated the failure path).
///
/// Pure, like `dispatch_action`: reads `context`, mutates nothing, returns a
/// `DispatchResult`. Called only for the variant above; any other variant is
/// a routing bug in `dispatch_action` and trips the `unreachable!`.
fn dispatch_spawn_entity(action: &TriggerAction, context: &DispatchContext) -> DispatchResult {
    let mut out = DispatchResult::default();

    match action {
        TriggerAction::SpawnEntity {
            template_path,
            name,
            anchor,
            position,
            rotation,
            scale,
            groups,
            overrides,
        } => {
            // 1. Resolve spawn position. `anchor` looks up in the origin
            // layer's anchors (or the base world's anchors for a `None`
            // origin). `position` is used directly.
            let resolved_position: [f32; 3] = if let Some(pos) = position {
                *pos
            } else if let Some(anchor_name) = anchor {
                let lookup = match &context.origin_layer {
                    Some(layer_path) => context
                        .layers
                        .get(layer_path)
                        .and_then(|l| l.anchors.get(anchor_name).copied()),
                    None => context.base_anchors.get(anchor_name).copied(),
                };
                match lookup {
                    Some(p) => p,
                    None => {
                        out.warnings.push(format!(
                            "SpawnEntity '{name}' anchor '{anchor_name}' not found"
                        ));
                        return out;
                    }
                }
            } else {
                out.warnings.push(format!(
                    "SpawnEntity '{name}' has neither anchor nor position"
                ));
                return out;
            };

            // 2. Load the template — the contingency gate (see doc comment).
            let Some(mut config) = context.template_loader.load_template(template_path) else {
                out.warnings.push(format!(
                    "SpawnEntity '{name}' template '{template_path}' not found"
                ));
                return out;
            };

            // 2b. Apply overrides if present.
            if let Some(overrides_val) = overrides {
                // Serialise **losslessly** (issue #838): `to_toml_value` re-emits
                // the `[[station]]`/`[[system]]`/`[power_groups]`/`[[shield_arc]]`
                // blocks that a plain `toml::to_string(&config)` drops (they live
                // in `#[serde(skip)]` fields). Without this the merged template
                // carried no ship systems, so an override-spawned hull had
                // nothing under AI control — no radar to lock with, no weapons to
                // fire — and sat inert however hostile its faction and doctrine.
                let Ok(template_value) = config.to_toml_value() else {
                    return out;
                };
                // The merge is fallible for exactly one reason (issue #911): an
                // override carrying the `_remove` tombstone. That marker is a
                // FRAGMENT-composition feature, and the merge rejects it here
                // rather than letting it through — see the note below. It is
                // reported on the same channel as a failed reparse, because the
                // consequence is the same (the template spawns unchanged) and
                // because the one thing that must not happen to a subtractive
                // marker is silence.
                let merged = crate::entity_override::merge_entity_config_toml(
                    &template_value,
                    overrides_val,
                );
                // Do not silently swallow a failed merge (issue #838). If the
                // merged config no longer parses, the *entire* override —
                // faction, behaviour, everything — would otherwise be discarded
                // and the raw template spawned, which is how a world-spawned
                // "hostile" ended up neither hostile nor armed. Keep the
                // template as the fallback (a partial spawn is better than none)
                // but surface the reason so the scenario author sees it rather
                // than debugging silent inertness.
                //
                // One rejection is reachable from an override that looks
                // perfectly well-formed on its own. Doctrine entries merge
                // per-field by id (`merge_keyed_array`), so an override that
                // flips an existing entry's `directive_kind` keeps the
                // template's directive fields beside its own: a Patrol entry
                // overridden to `directive_kind = "Reach"` arrives carrying both
                // `directive_anchors` and `directive_anchor`, and
                // `validate_doctrine_directives` rejects the pair. Clearing the
                // stale field in the same override entry — `directive_anchors =
                // []` — is the way through: `behaviour.doctrine.directive_anchors`
                // is absent from BOTH identity tables (issue #911), so it still
                // replaces wholesale at this layer and at the compose layer
                // alike. Because this path warns rather than failing, the hull
                // otherwise flies the doctrine the author meant to replace, so
                // the warning below is the only signal there is.
                //
                // This call is `merge_entity_config_toml`, i.e.
                // `MergePolicy::InstanceOverride` — unchanged by #911. An
                // override here replaces `tags`, `[[system]]`, `[[station]]`,
                // `[[shield_arc]]` and every weapon bank WHOLESALE, and does
                // NOT honour the `_remove` tombstone. A tombstone written here
                // is rejected BY THE MERGE, which is why the merge is fallible:
                // it cannot be left to `from_toml` below, because the one array
                // that reconciles at this layer is `behaviour.doctrine`, and
                // `DoctrineObjective` is not `deny_unknown_fields` — the marker
                // would deep-merge into the matching template entry, be ignored
                // by serde, and the doctrine would be silently unchanged.
                // Element-wise array extension is a FRAGMENT-composition
                // feature (`MergePolicy::ComposeFragments`), because widening it
                // here would change what every shipped world already means.
                let outcome = merged.and_then(|merged| {
                    let merged_str = toml::to_string(&merged).unwrap_or_default();
                    crate::entity_config::EntityConfig::from_toml(&merged_str)
                        .map_err(|e| e.to_string())
                });
                match outcome {
                    Ok(merged_config) => config = merged_config,
                    Err(e) => out.warnings.push(format!(
                        "SpawnEntity '{name}' overrides did not apply (kept template): {e}"
                    )),
                }
            }

            // 3. Only a spawn that will actually happen consumes a uuid.
            let uuid = (context.uuid_source)();

            // 4. Patch the entity name with the trigger's `name` field so the
            // spawned ECS entity gets EntityName("wave_1") (not the template's
            // display name like "Harrow Destroyer"). This mirrors what
            // `spawn_world_entities` does for static `[[entity]]` entries and
            // is required for `resolve_objective_target` to match Destroy
            // directives by the scenario name.
            if !name.is_empty() {
                config.name = Some(name.clone());
            }

            out.commands.push(ActionCmd::SpawnEntity {
                config: Box::new(config),
                name: name.clone(),
                uuid: uuid.clone(),
                position: resolved_position,
                rotation: *rotation,
                scale: *scale,
                layer_path: context.origin_layer.clone(),
                overrides: overrides.clone(),
            });

            // Register name → uuid for subsequent triggers, and the entity in
            // its groups for `OnAllDestroyed` tracking. Unconditional here:
            // step 2 already returned on the one failure mode that used to
            // make these contingent.
            out.name_to_uuid_inserts.push((name.clone(), uuid));

            for g in groups {
                out.entity_group_inserts.push((g.clone(), name.clone()));
            }
        }

        other => {
            unreachable!("dispatch_spawn_entity called with non-spawn action: {other:?}")
        }
    }

    out
}

// ── Unit Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::faction::FactionConfig;

    /// Deterministic stand-in for `entity_loader::assign_uuid()`.
    const STUB_UUID: &str = "stub-uuid-0001";

    fn stub_uuid() -> String {
        STUB_UUID.to_string()
    }

    /// Template path the `spawn()` helper names.
    const DESTROYER_TEMPLATE: &str = "assets/entities/destroyer.toml";

    /// Hand-written `TemplateLoader` fake serving `EntityConfig`s out of a
    /// map. The dispatch tests need their own local fake (test modules don't
    /// share the one in `entities::loader::tests`). The `Default` — an empty
    /// map, i.e. every template missing — is what non-spawn tests carry.
    #[derive(Default)]
    struct FakeTemplateLoader {
        templates: HashMap<String, EntityConfig>,
    }

    impl TemplateLoader for FakeTemplateLoader {
        fn load_template(&self, path: &str) -> Option<EntityConfig> {
            self.templates.get(path).cloned()
        }
    }

    /// A minimal template config with a display name, so tests can observe
    /// the trigger `name` overwriting it (or not, for an empty trigger name).
    fn destroyer_template() -> EntityConfig {
        EntityConfig {
            name: Some("Harrow Destroyer".to_string()),
            tags: vec!["npc".to_string()],
            ..Default::default()
        }
    }

    /// `destroyer_template()` as `dispatch_spawn_entity` patches and boxes
    /// it: the trigger's `name` wins over the template's display name.
    fn patched_destroyer_template(name: &str) -> Box<EntityConfig> {
        Box::new(EntityConfig {
            name: Some(name.to_string()),
            ..destroyer_template()
        })
    }

    /// Owned backing store for a `DispatchContext`. Tests mutate the public
    /// fields then call `ctx()` to borrow a context out of it.
    #[derive(Default)]
    struct Fixture {
        origin_layer: Option<String>,
        entity_name: Option<String>,
        name_to_uuid: HashMap<String, String>,
        base_flags: FlagStore,
        layers: HashMap<String, LayerView>,
        base_anchors: HashMap<String, [f32; 3]>,
        factions: Option<FactionRegistry>,
        loader: FakeTemplateLoader,
    }

    impl Fixture {
        fn new() -> Self {
            Self::default()
        }

        fn ctx(&self) -> DispatchContext<'_> {
            DispatchContext {
                origin_layer: self.origin_layer.clone(),
                entity_name: self.entity_name.clone(),
                name_to_uuid: &self.name_to_uuid,
                base_flags: &self.base_flags,
                layers: &self.layers,
                base_anchors: &self.base_anchors,
                factions: self.factions.as_ref(),
                uuid_source: &stub_uuid,
                template_loader: &self.loader,
            }
        }

        fn with_entity(mut self, name: &str, uuid: &str) -> Self {
            self.name_to_uuid.insert(name.to_string(), uuid.to_string());
            self
        }

        /// Pre-load the destroyer template that `spawn()` names, so the
        /// spawn arm's template load succeeds.
        fn with_destroyer(mut self) -> Self {
            self.loader
                .templates
                .insert(DESTROYER_TEMPLATE.to_string(), destroyer_template());
            self
        }
    }

    fn layer(loader_path: Option<&str>) -> LayerView {
        LayerView {
            flags: FlagStore::new(),
            loader_path: loader_path.map(str::to_string),
            anchors: HashMap::new(),
        }
    }

    /// A registry with two named factions plus their UUIDs.
    fn two_factions() -> (FactionRegistry, Uuid, Uuid) {
        let harrow = Uuid::from_u128(1);
        let federation = Uuid::from_u128(2);
        let mut registry = FactionRegistry::new();
        registry.insert(FactionConfig {
            uuid: harrow,
            name: "Harrow".to_string(),
            enemies: vec![],
        });
        registry.insert(FactionConfig {
            uuid: federation,
            name: "Federation".to_string(),
            enemies: vec![],
        });
        (registry, harrow, federation)
    }

    fn add_objective(targets: Vec<&str>) -> TriggerAction {
        TriggerAction::AddObjective {
            id: "obj1".to_string(),
            text: "Destroy the convoy".to_string(),
            mandatory: true,
            targets: targets.into_iter().map(str::to_string).collect(),
            directive: AiDirective::default(),
            utility: UtilityConfig::default(),
            source: ObjectiveSource::default(),
        }
    }

    fn spawn(anchor: Option<&str>, position: Option<[f32; 3]>, groups: Vec<&str>) -> TriggerAction {
        TriggerAction::SpawnEntity {
            template_path: DESTROYER_TEMPLATE.to_string(),
            name: "wave_1".to_string(),
            anchor: anchor.map(str::to_string),
            position,
            rotation: None,
            scale: None,
            groups: groups.into_iter().map(str::to_string).collect(),
            overrides: None,
        }
    }

    // ── AddObjective ──────────────────────────────────────────────────────

    #[test]
    fn add_objective_uses_explicit_targets() {
        let mut fx = Fixture::new();
        fx.entity_name = Some("trigger_ship".to_string());
        let out = dispatch_action(&add_objective(vec!["alpha", "beta"]), &fx.ctx());

        assert_eq!(
            out.commands,
            vec![ActionCmd::AddObjective {
                id: "obj1".to_string(),
                text: "Destroy the convoy".to_string(),
                mandatory: true,
                targets: vec!["alpha".to_string(), "beta".to_string()],
                directive: AiDirective::default(),
                utility: UtilityConfig::default(),
                source: ObjectiveSource::default(),
                origin_layer: None,
            }]
        );
        assert!(out.warnings.is_empty());
        assert!(out.new_events.is_empty());
    }

    #[test]
    fn add_objective_empty_targets_falls_back_to_trigger_entity() {
        let mut fx = Fixture::new();
        fx.entity_name = Some("trigger_ship".to_string());
        let out = dispatch_action(&add_objective(vec![]), &fx.ctx());

        let ActionCmd::AddObjective { targets, .. } = &out.commands[0] else {
            panic!("expected AddObjective");
        };
        assert_eq!(targets, &vec!["trigger_ship".to_string()]);
    }

    #[test]
    fn add_objective_empty_targets_and_no_entity_name_resolves_empty() {
        let fx = Fixture::new();
        let out = dispatch_action(&add_objective(vec![]), &fx.ctx());

        let ActionCmd::AddObjective { targets, .. } = &out.commands[0] else {
            panic!("expected AddObjective");
        };
        assert!(targets.is_empty());
    }

    // ── CompleteObjective / FailObjective ─────────────────────────────────

    #[test]
    fn complete_objective_emits_command() {
        let fx = Fixture::new();
        let action = TriggerAction::CompleteObjective {
            id: "obj1".to_string(),
        };
        let out = dispatch_action(&action, &fx.ctx());

        assert_eq!(
            out.commands,
            vec![ActionCmd::CompleteObjective {
                id: "obj1".to_string()
            }]
        );
        assert!(out.warnings.is_empty());
    }

    #[test]
    fn fail_objective_emits_command() {
        let fx = Fixture::new();
        let action = TriggerAction::FailObjective {
            id: "obj1".to_string(),
        };
        let out = dispatch_action(&action, &fx.ctx());

        assert_eq!(
            out.commands,
            vec![ActionCmd::FailObjective {
                id: "obj1".to_string()
            }]
        );
    }

    // ── SetAiState ────────────────────────────────────────────────────────

    #[test]
    fn set_ai_state_is_a_noop_that_warns() {
        let fx = Fixture::new();
        let action = TriggerAction::SetAiState {
            entity: "raider".to_string(),
            state: "Attack".to_string(),
            target: Some("player".to_string()),
        };
        let out = dispatch_action(&action, &fx.ctx());

        assert!(out.commands.is_empty());
        assert!(out.new_events.is_empty());
        assert_eq!(out.warnings.len(), 1);
        assert!(out.warnings[0].contains("doctrine-based AI"));
    }

    // ── Modifier arms ─────────────────────────────────────────────────────

    #[test]
    fn apply_modifier_resolves_name_to_uuid() {
        let fx = Fixture::new().with_entity("raider", "uuid-raider");
        let action = TriggerAction::ApplyModifier {
            entity: "raider".to_string(),
            tag: "buff".to_string(),
            slot: ModifierSlot::MaxSpeed,
            bonus: 2.5,
        };
        let out = dispatch_action(&action, &fx.ctx());

        assert_eq!(
            out.commands,
            vec![ActionCmd::ApplyModifier {
                uuid: "uuid-raider".to_string(),
                tag: "buff".to_string(),
                slot: ModifierSlot::MaxSpeed,
                bonus: 2.5,
            }]
        );
        assert!(out.warnings.is_empty());
    }

    #[test]
    fn apply_modifier_unknown_entity_warns_and_emits_nothing() {
        let fx = Fixture::new();
        let action = TriggerAction::ApplyModifier {
            entity: "ghost".to_string(),
            tag: "buff".to_string(),
            slot: ModifierSlot::MaxSpeed,
            bonus: 2.5,
        };
        let out = dispatch_action(&action, &fx.ctx());

        assert!(out.commands.is_empty());
        assert_eq!(
            out.warnings,
            vec!["ApplyModifier: unknown entity name 'ghost'".to_string()]
        );
    }

    #[test]
    fn remove_modifier_resolves_name_to_uuid() {
        let fx = Fixture::new().with_entity("raider", "uuid-raider");
        let action = TriggerAction::RemoveModifier {
            entity: "raider".to_string(),
            tag: "buff".to_string(),
            slot: ModifierSlot::MaxSpeed,
        };
        let out = dispatch_action(&action, &fx.ctx());

        assert_eq!(
            out.commands,
            vec![ActionCmd::RemoveModifier {
                uuid: "uuid-raider".to_string(),
                tag: "buff".to_string(),
                slot: ModifierSlot::MaxSpeed,
            }]
        );
    }

    #[test]
    fn remove_modifier_unknown_entity_warns_and_emits_nothing() {
        let fx = Fixture::new();
        let action = TriggerAction::RemoveModifier {
            entity: "ghost".to_string(),
            tag: "buff".to_string(),
            slot: ModifierSlot::MaxSpeed,
        };
        let out = dispatch_action(&action, &fx.ctx());

        assert!(out.commands.is_empty());
        assert_eq!(
            out.warnings,
            vec!["RemoveModifier: unknown entity name 'ghost'".to_string()]
        );
    }

    #[test]
    fn apply_flag_resolves_name_to_uuid() {
        let fx = Fixture::new().with_entity("raider", "uuid-raider");
        let action = TriggerAction::ApplyFlag {
            entity: "raider".to_string(),
            tag: "cloak".to_string(),
            kind: FlagKind::CommsJammed,
        };
        let out = dispatch_action(&action, &fx.ctx());

        assert_eq!(
            out.commands,
            vec![ActionCmd::ApplyFlag {
                uuid: "uuid-raider".to_string(),
                tag: "cloak".to_string(),
                kind: FlagKind::CommsJammed,
            }]
        );
    }

    #[test]
    fn apply_flag_unknown_entity_warns_and_emits_nothing() {
        let fx = Fixture::new();
        let action = TriggerAction::ApplyFlag {
            entity: "ghost".to_string(),
            tag: "cloak".to_string(),
            kind: FlagKind::CommsJammed,
        };
        let out = dispatch_action(&action, &fx.ctx());

        assert!(out.commands.is_empty());
        assert_eq!(
            out.warnings,
            vec!["ApplyFlag: unknown entity name 'ghost'".to_string()]
        );
    }

    #[test]
    fn remove_flag_resolves_name_to_uuid() {
        let fx = Fixture::new().with_entity("raider", "uuid-raider");
        let action = TriggerAction::RemoveFlag {
            entity: "raider".to_string(),
            tag: "cloak".to_string(),
            kind: FlagKind::CommsJammed,
        };
        let out = dispatch_action(&action, &fx.ctx());

        assert_eq!(
            out.commands,
            vec![ActionCmd::RemoveFlag {
                uuid: "uuid-raider".to_string(),
                tag: "cloak".to_string(),
                kind: FlagKind::CommsJammed,
            }]
        );
    }

    #[test]
    fn remove_flag_unknown_entity_warns_and_emits_nothing() {
        let fx = Fixture::new();
        let action = TriggerAction::RemoveFlag {
            entity: "ghost".to_string(),
            tag: "cloak".to_string(),
            kind: FlagKind::CommsJammed,
        };
        let out = dispatch_action(&action, &fx.ctx());

        assert!(out.commands.is_empty());
        assert_eq!(
            out.warnings,
            vec!["RemoveFlag: unknown entity name 'ghost'".to_string()]
        );
    }

    #[test]
    fn apply_int_modifier_resolves_name_to_uuid() {
        let fx = Fixture::new().with_entity("raider", "uuid-raider");
        let action = TriggerAction::ApplyIntModifier {
            entity: "raider".to_string(),
            tag: "crew".to_string(),
            slot: IntModifierSlot::RepairTeams,
            bonus: 3,
        };
        let out = dispatch_action(&action, &fx.ctx());

        assert_eq!(
            out.commands,
            vec![ActionCmd::ApplyIntModifier {
                uuid: "uuid-raider".to_string(),
                tag: "crew".to_string(),
                slot: IntModifierSlot::RepairTeams,
                bonus: 3,
            }]
        );
    }

    #[test]
    fn apply_int_modifier_unknown_entity_warns_and_emits_nothing() {
        let fx = Fixture::new();
        let action = TriggerAction::ApplyIntModifier {
            entity: "ghost".to_string(),
            tag: "crew".to_string(),
            slot: IntModifierSlot::RepairTeams,
            bonus: 3,
        };
        let out = dispatch_action(&action, &fx.ctx());

        assert!(out.commands.is_empty());
        assert_eq!(
            out.warnings,
            vec!["ApplyIntModifier: unknown entity name 'ghost'".to_string()]
        );
    }

    #[test]
    fn remove_int_modifier_resolves_name_to_uuid() {
        let fx = Fixture::new().with_entity("raider", "uuid-raider");
        let action = TriggerAction::RemoveIntModifier {
            entity: "raider".to_string(),
            tag: "crew".to_string(),
            slot: IntModifierSlot::RepairTeams,
        };
        let out = dispatch_action(&action, &fx.ctx());

        assert_eq!(
            out.commands,
            vec![ActionCmd::RemoveIntModifier {
                uuid: "uuid-raider".to_string(),
                tag: "crew".to_string(),
                slot: IntModifierSlot::RepairTeams,
            }]
        );
    }

    #[test]
    fn remove_int_modifier_unknown_entity_warns_and_emits_nothing() {
        let fx = Fixture::new();
        let action = TriggerAction::RemoveIntModifier {
            entity: "ghost".to_string(),
            tag: "crew".to_string(),
            slot: IntModifierSlot::RepairTeams,
        };
        let out = dispatch_action(&action, &fx.ctx());

        assert!(out.commands.is_empty());
        assert_eq!(
            out.warnings,
            vec!["RemoveIntModifier: unknown entity name 'ghost'".to_string()]
        );
    }

    // ── GameOver ──────────────────────────────────────────────────────────

    #[test]
    fn game_over_sets_reason_before_state() {
        let fx = Fixture::new();
        let action = TriggerAction::GameOver {
            message: Some("The ship was lost".to_string()),
            outcome: None,
        };
        let out = dispatch_action(&action, &fx.ctx());

        // Ordering is load-bearing: OnEnter(GameOver) reads the reason.
        assert_eq!(
            out.commands,
            vec![
                ActionCmd::SetGameOverReason {
                    reason: "The ship was lost".to_string(),
                    outcome: None,
                },
                ActionCmd::SetNextState {
                    phase: GamePhase::GameOver
                },
            ]
        );
    }

    #[test]
    fn game_over_without_message_yields_empty_reason_not_none() {
        let fx = Fixture::new();
        let action = TriggerAction::GameOver {
            message: None,
            outcome: None,
        };
        let out = dispatch_action(&action, &fx.ctx());

        assert_eq!(
            out.commands[0],
            ActionCmd::SetGameOverReason {
                reason: String::new(),
                outcome: None,
            }
        );
    }

    // ── LoadWorld / UnloadWorld ───────────────────────────────────────────

    #[test]
    fn load_world_records_origin_layer_as_loader_path() {
        let mut fx = Fixture::new();
        fx.origin_layer = Some("worlds/sub.toml".to_string());
        let action = TriggerAction::LoadWorld {
            path: "worlds/next.toml".to_string(),
        };
        let out = dispatch_action(&action, &fx.ctx());

        assert_eq!(
            out.commands,
            vec![ActionCmd::LoadWorld {
                path: "worlds/next.toml".to_string(),
                loader_path: Some("worlds/sub.toml".to_string()),
            }]
        );
    }

    #[test]
    fn load_world_from_base_world_has_no_loader_path() {
        let fx = Fixture::new();
        let action = TriggerAction::LoadWorld {
            path: "worlds/next.toml".to_string(),
        };
        let out = dispatch_action(&action, &fx.ctx());

        assert_eq!(
            out.commands,
            vec![ActionCmd::LoadWorld {
                path: "worlds/next.toml".to_string(),
                loader_path: None,
            }]
        );
    }

    #[test]
    fn unload_world_emits_command() {
        let fx = Fixture::new();
        let action = TriggerAction::UnloadWorld {
            path: "worlds/sub.toml".to_string(),
        };
        let out = dispatch_action(&action, &fx.ctx());

        assert_eq!(
            out.commands,
            vec![ActionCmd::UnloadWorld {
                path: "worlds/sub.toml".to_string()
            }]
        );
    }

    // ── SetWorldFlag ──────────────────────────────────────────────────────

    #[test]
    fn set_world_flag_emits_mutation_and_flag_set_event() {
        let fx = Fixture::new();
        let action = TriggerAction::SetWorldFlag {
            name: "alarm".to_string(),
        };
        let out = dispatch_action(&action, &fx.ctx());

        assert_eq!(
            out.commands,
            vec![ActionCmd::MutateFlag {
                target_layer: None,
                name: "alarm".to_string(),
                mutation: FlagMutation::Set,
            }]
        );
        assert_eq!(
            out.new_events,
            vec![WorldEvent::FlagSet {
                name: "alarm".to_string(),
                origin_layer: None,
            }]
        );
    }

    /// Pins flag idempotence, which depends on `base_flags` being the *live*
    /// store: a second `set_flag` of an armed flag must still command the
    /// mutation but emit no transition, so a downstream `on_flag_set` trigger
    /// fires exactly once no matter how many triggers set it
    /// (`assets/worlds/before_the_fire.toml:275`). Passing a per-pass snapshot
    /// instead would make both setters read `before = 0` and double-fire.
    #[test]
    fn set_world_flag_on_already_set_flag_emits_no_transition_event() {
        let mut fx = Fixture::new();
        fx.base_flags.set_flag_value("alarm", 1);
        let action = TriggerAction::SetWorldFlag {
            name: "alarm".to_string(),
        };
        let out = dispatch_action(&action, &fx.ctx());

        assert_eq!(
            out.commands,
            vec![ActionCmd::MutateFlag {
                target_layer: None,
                name: "alarm".to_string(),
                mutation: FlagMutation::Set,
            }]
        );
        assert!(out.new_events.is_empty());
    }

    // ── ClearWorldFlag ────────────────────────────────────────────────────

    #[test]
    fn clear_world_flag_emits_mutation_and_flag_cleared_event() {
        let mut fx = Fixture::new();
        fx.base_flags.set_flag_value("alarm", 1);
        let action = TriggerAction::ClearWorldFlag {
            name: "alarm".to_string(),
        };
        let out = dispatch_action(&action, &fx.ctx());

        assert_eq!(
            out.commands,
            vec![ActionCmd::MutateFlag {
                target_layer: None,
                name: "alarm".to_string(),
                mutation: FlagMutation::Clear,
            }]
        );
        assert_eq!(
            out.new_events,
            vec![WorldEvent::FlagCleared {
                name: "alarm".to_string(),
                origin_layer: None,
            }]
        );
    }

    #[test]
    fn clear_world_flag_already_clear_mutates_without_an_event() {
        let fx = Fixture::new();
        let action = TriggerAction::ClearWorldFlag {
            name: "alarm".to_string(),
        };
        let out = dispatch_action(&action, &fx.ctx());

        assert_eq!(out.commands.len(), 1);
        assert!(out.new_events.is_empty());
    }

    // ── IncrementWorldFlag ────────────────────────────────────────────────

    #[test]
    fn increment_world_flag_zero_to_nonzero_emits_flag_set() {
        let fx = Fixture::new();
        let action = TriggerAction::IncrementWorldFlag {
            name: "kills".to_string(),
            by: 1,
        };
        let out = dispatch_action(&action, &fx.ctx());

        assert_eq!(
            out.commands,
            vec![ActionCmd::MutateFlag {
                target_layer: None,
                name: "kills".to_string(),
                mutation: FlagMutation::Increment(1),
            }]
        );
        assert_eq!(
            out.new_events,
            vec![WorldEvent::FlagSet {
                name: "kills".to_string(),
                origin_layer: None,
            }]
        );
    }

    #[test]
    fn increment_world_flag_nonzero_to_nonzero_emits_no_event() {
        let mut fx = Fixture::new();
        fx.base_flags.set_flag_value("kills", 3);
        let action = TriggerAction::IncrementWorldFlag {
            name: "kills".to_string(),
            by: 2,
        };
        let out = dispatch_action(&action, &fx.ctx());

        assert_eq!(out.commands.len(), 1);
        assert!(out.new_events.is_empty());
    }

    #[test]
    fn increment_world_flag_to_zero_emits_flag_cleared() {
        let mut fx = Fixture::new();
        fx.base_flags.set_flag_value("kills", 2);
        let action = TriggerAction::IncrementWorldFlag {
            name: "kills".to_string(),
            by: -2,
        };
        let out = dispatch_action(&action, &fx.ctx());

        assert_eq!(
            out.new_events,
            vec![WorldEvent::FlagCleared {
                name: "kills".to_string(),
                origin_layer: None,
            }]
        );
    }

    #[test]
    fn increment_world_flag_overflow_does_not_panic() {
        let mut fx = Fixture::new();
        fx.base_flags.set_flag_value("kills", i64::MAX);
        let action = TriggerAction::IncrementWorldFlag {
            name: "kills".to_string(),
            by: 5,
        };
        let out = dispatch_action(&action, &fx.ctx());

        // Only the no-panic property is testable here: `ActionCmd::MutateFlag`
        // carries the mutation, not the resulting value, so saturating-vs-
        // wrapping is unobservable through `dispatch_action` — both stay
        // non-zero, so both emit one command and no event. `saturating_add`
        // fidelity is `FlagStore`'s contract and is tested at `flags.rs`.
        assert_eq!(out.commands.len(), 1);
        assert!(out.new_events.is_empty());
    }

    // ── SetWorldFlagValue ─────────────────────────────────────────────────

    #[test]
    fn set_world_flag_value_zero_emits_flag_cleared() {
        let mut fx = Fixture::new();
        fx.base_flags.set_flag_value("alarm", 7);
        let action = TriggerAction::SetWorldFlagValue {
            name: "alarm".to_string(),
            value: 0,
        };
        let out = dispatch_action(&action, &fx.ctx());

        assert_eq!(
            out.commands,
            vec![ActionCmd::MutateFlag {
                target_layer: None,
                name: "alarm".to_string(),
                mutation: FlagMutation::SetValue(0),
            }]
        );
        assert_eq!(
            out.new_events,
            vec![WorldEvent::FlagCleared {
                name: "alarm".to_string(),
                origin_layer: None,
            }]
        );
    }

    #[test]
    fn set_world_flag_value_nonzero_to_nonzero_emits_no_event() {
        let mut fx = Fixture::new();
        fx.base_flags.set_flag_value("alarm", 7);
        let action = TriggerAction::SetWorldFlagValue {
            name: "alarm".to_string(),
            value: 9,
        };
        let out = dispatch_action(&action, &fx.ctx());

        assert_eq!(out.commands.len(), 1);
        assert!(out.new_events.is_empty());
    }

    // ── Flag layer walking ────────────────────────────────────────────────

    #[test]
    fn flag_without_prefix_targets_the_origin_layer() {
        let mut fx = Fixture::new();
        fx.origin_layer = Some("sub.toml".to_string());
        fx.layers.insert("sub.toml".to_string(), layer(None));
        let action = TriggerAction::SetWorldFlag {
            name: "alarm".to_string(),
        };
        let out = dispatch_action(&action, &fx.ctx());

        assert_eq!(
            out.commands,
            vec![ActionCmd::MutateFlag {
                target_layer: Some("sub.toml".to_string()),
                name: "alarm".to_string(),
                mutation: FlagMutation::Set,
            }]
        );
        assert_eq!(
            out.new_events,
            vec![WorldEvent::FlagSet {
                name: "alarm".to_string(),
                origin_layer: Some("sub.toml".to_string()),
            }]
        );
    }

    #[test]
    fn flag_parent_prefix_walks_one_layer_up_to_base() {
        let mut fx = Fixture::new();
        fx.origin_layer = Some("sub.toml".to_string());
        // `loader_path: None` = loaded by the base world.
        fx.layers.insert("sub.toml".to_string(), layer(None));
        let action = TriggerAction::SetWorldFlag {
            name: "parent:alarm".to_string(),
        };
        let out = dispatch_action(&action, &fx.ctx());

        assert_eq!(
            out.commands,
            vec![ActionCmd::MutateFlag {
                target_layer: None,
                name: "alarm".to_string(),
                mutation: FlagMutation::Set,
            }]
        );
        assert_eq!(
            out.new_events,
            vec![WorldEvent::FlagSet {
                name: "alarm".to_string(),
                origin_layer: None,
            }]
        );
    }

    #[test]
    fn flag_parent_prefix_reads_the_target_layers_store_not_the_origins() {
        let mut fx = Fixture::new();
        fx.origin_layer = Some("inner.toml".to_string());
        // inner was loaded by outer; outer was loaded by the base world.
        fx.layers
            .insert("inner.toml".to_string(), layer(Some("outer.toml")));
        let mut outer = layer(None);
        // Already set in the *parent* store: no transition should be emitted.
        outer.flags.set_flag_value("alarm", 1);
        fx.layers.insert("outer.toml".to_string(), outer);

        let action = TriggerAction::SetWorldFlag {
            name: "parent:alarm".to_string(),
        };
        let out = dispatch_action(&action, &fx.ctx());

        assert_eq!(
            out.commands,
            vec![ActionCmd::MutateFlag {
                target_layer: Some("outer.toml".to_string()),
                name: "alarm".to_string(),
                mutation: FlagMutation::Set,
            }]
        );
        assert!(out.new_events.is_empty());
    }

    #[test]
    fn flag_double_parent_prefix_walks_two_layers_up() {
        let mut fx = Fixture::new();
        fx.origin_layer = Some("inner.toml".to_string());
        fx.layers
            .insert("inner.toml".to_string(), layer(Some("outer.toml")));
        fx.layers.insert("outer.toml".to_string(), layer(None));

        let action = TriggerAction::SetWorldFlag {
            name: "parent:parent:alarm".to_string(),
        };
        let out = dispatch_action(&action, &fx.ctx());

        assert_eq!(
            out.commands,
            vec![ActionCmd::MutateFlag {
                target_layer: None,
                name: "alarm".to_string(),
                mutation: FlagMutation::Set,
            }]
        );
    }

    #[test]
    fn flag_walk_past_base_world_warns_and_emits_nothing() {
        // Origin is already the base world, so any `parent:` overruns.
        let fx = Fixture::new();
        let action = TriggerAction::SetWorldFlag {
            name: "parent:alarm".to_string(),
        };
        let out = dispatch_action(&action, &fx.ctx());

        assert!(out.commands.is_empty());
        assert!(out.new_events.is_empty());
        assert_eq!(out.warnings.len(), 1);
        assert!(out.warnings[0].contains("walks past base world"));
    }

    #[test]
    fn flag_walk_past_base_world_from_a_layer_warns_and_emits_nothing() {
        let mut fx = Fixture::new();
        fx.origin_layer = Some("sub.toml".to_string());
        fx.layers.insert("sub.toml".to_string(), layer(None));
        // sub → base → overrun.
        let action = TriggerAction::SetWorldFlag {
            name: "parent:parent:alarm".to_string(),
        };
        let out = dispatch_action(&action, &fx.ctx());

        assert!(out.commands.is_empty());
        assert!(out.new_events.is_empty());
        assert_eq!(out.warnings.len(), 1);
        assert!(out.warnings[0].contains("walks past base world"));
    }

    #[test]
    fn flag_target_layer_missing_from_map_warns_and_emits_nothing() {
        let mut fx = Fixture::new();
        // The trigger's own layer is not in the map, and there is no `parent:`
        // to walk, so the resolved target is a layer we cannot find.
        fx.origin_layer = Some("ghost.toml".to_string());
        let action = TriggerAction::SetWorldFlag {
            name: "alarm".to_string(),
        };
        let out = dispatch_action(&action, &fx.ctx());

        assert!(out.commands.is_empty());
        assert!(out.new_events.is_empty());
        assert_eq!(out.warnings.len(), 1);
        assert!(out.warnings[0].contains("missing from WorldLayerMap"));
    }

    #[test]
    fn flag_layer_missing_mid_walk_is_silent_and_treated_as_base() {
        let mut fx = Fixture::new();
        // `ghost.toml` is absent from the map: the walk silently resolves its
        // loader_path to `None` (base) and carries on. This is deliberate —
        // only the *final* lookup warns.
        fx.origin_layer = Some("ghost.toml".to_string());
        let action = TriggerAction::SetWorldFlag {
            name: "parent:alarm".to_string(),
        };
        let out = dispatch_action(&action, &fx.ctx());

        assert!(out.warnings.is_empty());
        assert_eq!(
            out.commands,
            vec![ActionCmd::MutateFlag {
                target_layer: None,
                name: "alarm".to_string(),
                mutation: FlagMutation::Set,
            }]
        );
        assert_eq!(
            out.new_events,
            vec![WorldEvent::FlagSet {
                name: "alarm".to_string(),
                origin_layer: None,
            }]
        );
    }

    // ── SpawnEntity ───────────────────────────────────────────────────────

    #[test]
    fn spawn_entity_with_explicit_position() {
        let fx = Fixture::new().with_destroyer();
        let out = dispatch_action(&spawn(None, Some([1.0, 2.0, 3.0]), vec![]), &fx.ctx());

        assert_eq!(
            out.commands,
            vec![ActionCmd::SpawnEntity {
                config: patched_destroyer_template("wave_1"),
                name: "wave_1".to_string(),
                uuid: STUB_UUID.to_string(),
                position: [1.0, 2.0, 3.0],
                rotation: None,
                scale: None,
                layer_path: None,
                overrides: None,
            }]
        );
        assert_eq!(
            out.name_to_uuid_inserts,
            vec![("wave_1".to_string(), STUB_UUID.to_string())]
        );
        assert!(out.entity_group_inserts.is_empty());
        assert!(out.warnings.is_empty());
    }

    #[test]
    fn spawn_entity_resolves_anchor_from_base_world() {
        let mut fx = Fixture::new().with_destroyer();
        fx.base_anchors
            .insert("staging".to_string(), [10.0, 0.0, -5.0]);
        let out = dispatch_action(&spawn(Some("staging"), None, vec![]), &fx.ctx());

        let ActionCmd::SpawnEntity { position, .. } = &out.commands[0] else {
            panic!("expected SpawnEntity");
        };
        assert_eq!(position, &[10.0, 0.0, -5.0]);
    }

    #[test]
    fn spawn_entity_resolves_anchor_from_the_origin_layer() {
        let mut fx = Fixture::new().with_destroyer();
        fx.origin_layer = Some("sub.toml".to_string());
        let mut sub = layer(None);
        sub.anchors.insert("staging".to_string(), [4.0, 5.0, 6.0]);
        fx.layers.insert("sub.toml".to_string(), sub);
        // A same-named base anchor must NOT win for a layer-authored trigger.
        fx.base_anchors
            .insert("staging".to_string(), [99.0, 99.0, 99.0]);

        let out = dispatch_action(&spawn(Some("staging"), None, vec![]), &fx.ctx());

        assert_eq!(
            out.commands,
            vec![ActionCmd::SpawnEntity {
                config: patched_destroyer_template("wave_1"),
                name: "wave_1".to_string(),
                uuid: STUB_UUID.to_string(),
                position: [4.0, 5.0, 6.0],
                rotation: None,
                scale: None,
                layer_path: Some("sub.toml".to_string()),
                overrides: None,
            }]
        );
    }

    #[test]
    fn spawn_entity_position_wins_over_anchor() {
        let mut fx = Fixture::new().with_destroyer();
        fx.base_anchors
            .insert("staging".to_string(), [10.0, 0.0, -5.0]);
        let out = dispatch_action(
            &spawn(Some("staging"), Some([1.0, 1.0, 1.0]), vec![]),
            &fx.ctx(),
        );

        let ActionCmd::SpawnEntity { position, .. } = &out.commands[0] else {
            panic!("expected SpawnEntity");
        };
        assert_eq!(position, &[1.0, 1.0, 1.0]);
    }

    #[test]
    fn spawn_entity_unknown_anchor_warns_and_emits_nothing() {
        let fx = Fixture::new();
        let out = dispatch_action(&spawn(Some("nowhere"), None, vec![]), &fx.ctx());

        assert!(out.commands.is_empty());
        assert!(out.name_to_uuid_inserts.is_empty());
        assert_eq!(
            out.warnings,
            vec!["SpawnEntity 'wave_1' anchor 'nowhere' not found".to_string()]
        );
    }

    #[test]
    fn spawn_entity_without_anchor_or_position_warns_and_emits_nothing() {
        let fx = Fixture::new();
        let out = dispatch_action(&spawn(None, None, vec![]), &fx.ctx());

        assert!(out.commands.is_empty());
        assert!(out.name_to_uuid_inserts.is_empty());
        assert_eq!(
            out.warnings,
            vec!["SpawnEntity 'wave_1' has neither anchor nor position".to_string()]
        );
    }

    /// The contingency gate (issue #715): a spawn whose template fails to
    /// resolve must produce NO command and NO name/group inserts — only a
    /// warning. Before #715 this gate was the applier's `spawn_failed` local,
    /// exercisable only through a full Bevy app.
    #[test]
    fn spawn_entity_template_not_found_warns_and_emits_nothing() {
        // No `.with_destroyer()`: the loader has no templates at all.
        let fx = Fixture::new();
        let out = dispatch_action(&spawn(None, Some([0.0, 0.0, 0.0]), vec!["wave"]), &fx.ctx());

        assert!(out.commands.is_empty());
        assert!(out.name_to_uuid_inserts.is_empty());
        assert!(out.entity_group_inserts.is_empty());
        assert_eq!(
            out.warnings,
            vec![
                "SpawnEntity 'wave_1' template 'assets/entities/destroyer.toml' not found"
                    .to_string()
            ]
        );
    }

    #[test]
    fn spawn_entity_registers_every_group() {
        let fx = Fixture::new().with_destroyer();
        let out = dispatch_action(
            &spawn(None, Some([0.0, 0.0, 0.0]), vec!["wave", "hostiles"]),
            &fx.ctx(),
        );

        assert_eq!(
            out.entity_group_inserts,
            vec![
                ("wave".to_string(), "wave_1".to_string()),
                ("hostiles".to_string(), "wave_1".to_string()),
            ]
        );
    }

    #[test]
    fn spawn_entity_carries_rotation_and_scale_through() {
        let fx = Fixture::new().with_destroyer();
        let action = TriggerAction::SpawnEntity {
            template_path: DESTROYER_TEMPLATE.to_string(),
            name: "wave_1".to_string(),
            anchor: None,
            position: Some([0.0, 0.0, 0.0]),
            rotation: Some([0.0, 1.57, 0.0]),
            scale: Some([2.0, 2.0, 2.0]),
            groups: vec![],
            overrides: None,
        };
        let out = dispatch_action(&action, &fx.ctx());

        let ActionCmd::SpawnEntity {
            rotation, scale, ..
        } = &out.commands[0]
        else {
            panic!("expected SpawnEntity");
        };
        assert_eq!(rotation, &Some([0.0, 1.57, 0.0]));
        assert_eq!(scale, &Some([2.0, 2.0, 2.0]));
    }

    #[test]
    fn spawn_entity_uuid_comes_from_the_injected_source() {
        let fx = Fixture::new().with_destroyer();
        let counter = std::cell::Cell::new(0u32);
        let source = || {
            counter.set(counter.get() + 1);
            format!("uuid-{}", counter.get())
        };
        let ctx = DispatchContext {
            uuid_source: &source,
            ..fx.ctx()
        };

        let first = dispatch_action(&spawn(None, Some([0.0, 0.0, 0.0]), vec![]), &ctx);
        let second = dispatch_action(&spawn(None, Some([0.0, 0.0, 0.0]), vec![]), &ctx);

        assert_eq!(
            first.name_to_uuid_inserts,
            vec![("wave_1".to_string(), "uuid-1".to_string())]
        );
        assert_eq!(
            second.name_to_uuid_inserts,
            vec![("wave_1".to_string(), "uuid-2".to_string())]
        );
    }

    // ── DestroyEntity ─────────────────────────────────────────────────────

    #[test]
    fn destroy_entity_emits_command_and_destroyed_event() {
        let fx = Fixture::new().with_entity("wave_1", "uuid-wave-1");
        let action = TriggerAction::DestroyEntity {
            entity: "wave_1".to_string(),
        };
        let out = dispatch_action(&action, &fx.ctx());

        assert_eq!(
            out.commands,
            vec![ActionCmd::DestroyEntity {
                uuid: "uuid-wave-1".to_string()
            }]
        );
        // The event lets chained `on_destroyed` triggers fire.
        assert_eq!(
            out.new_events,
            vec![WorldEvent::Destroyed {
                uuid: "uuid-wave-1".to_string()
            }]
        );
        assert!(out.warnings.is_empty());
    }

    #[test]
    fn destroy_entity_unknown_name_warns_and_emits_nothing() {
        let fx = Fixture::new();
        let action = TriggerAction::DestroyEntity {
            entity: "ghost".to_string(),
        };
        let out = dispatch_action(&action, &fx.ctx());

        assert!(out.commands.is_empty());
        assert!(out.new_events.is_empty());
        assert_eq!(
            out.warnings,
            vec!["DestroyEntity: unknown entity name 'ghost'".to_string()]
        );
    }

    // ── AddFactionEnemy ───────────────────────────────────────────────────

    #[test]
    fn add_faction_enemy_resolves_both_names_to_uuids() {
        let (registry, harrow, federation) = two_factions();
        let mut fx = Fixture::new();
        fx.factions = Some(registry);
        let action = TriggerAction::AddFactionEnemy {
            faction: "Harrow".to_string(),
            enemy: "Federation".to_string(),
        };
        let out = dispatch_action(&action, &fx.ctx());

        assert_eq!(
            out.commands,
            vec![ActionCmd::AddFactionEnemy {
                faction_uuid: harrow,
                enemy_uuid: federation,
            }]
        );
        assert!(out.warnings.is_empty());
    }

    #[test]
    fn add_faction_enemy_without_registry_warns_and_emits_nothing() {
        let fx = Fixture::new();
        let action = TriggerAction::AddFactionEnemy {
            faction: "Harrow".to_string(),
            enemy: "Federation".to_string(),
        };
        let out = dispatch_action(&action, &fx.ctx());

        assert!(out.commands.is_empty());
        assert_eq!(
            out.warnings,
            vec!["AddFactionEnemy skipped: FactionRegistryResource not present".to_string()]
        );
    }

    #[test]
    fn add_faction_enemy_unknown_faction_warns_and_emits_nothing() {
        let (registry, _, _) = two_factions();
        let mut fx = Fixture::new();
        fx.factions = Some(registry);
        let action = TriggerAction::AddFactionEnemy {
            faction: "Nobody".to_string(),
            enemy: "Federation".to_string(),
        };
        let out = dispatch_action(&action, &fx.ctx());

        assert!(out.commands.is_empty());
        assert_eq!(
            out.warnings,
            vec!["AddFactionEnemy: unknown faction name 'Nobody'".to_string()]
        );
    }

    #[test]
    fn add_faction_enemy_unknown_enemy_warns_and_emits_nothing() {
        let (registry, _, _) = two_factions();
        let mut fx = Fixture::new();
        fx.factions = Some(registry);
        let action = TriggerAction::AddFactionEnemy {
            faction: "Harrow".to_string(),
            enemy: "Nobody".to_string(),
        };
        let out = dispatch_action(&action, &fx.ctx());

        assert!(out.commands.is_empty());
        assert_eq!(
            out.warnings,
            vec!["AddFactionEnemy: unknown enemy faction name 'Nobody'".to_string()]
        );
    }

    #[test]
    fn faction_name_lookup_is_case_sensitive() {
        let (registry, _, _) = two_factions();
        let mut fx = Fixture::new();
        fx.factions = Some(registry);
        let action = TriggerAction::AddFactionEnemy {
            faction: "harrow".to_string(),
            enemy: "Federation".to_string(),
        };
        let out = dispatch_action(&action, &fx.ctx());

        assert!(out.commands.is_empty());
        assert_eq!(out.warnings.len(), 1);
    }

    // ── RemoveFactionEnemy ────────────────────────────────────────────────

    #[test]
    fn remove_faction_enemy_resolves_both_names_to_uuids() {
        let (registry, harrow, federation) = two_factions();
        let mut fx = Fixture::new();
        fx.factions = Some(registry);
        let action = TriggerAction::RemoveFactionEnemy {
            faction: "Harrow".to_string(),
            enemy: "Federation".to_string(),
        };
        let out = dispatch_action(&action, &fx.ctx());

        assert_eq!(
            out.commands,
            vec![ActionCmd::RemoveFactionEnemy {
                faction_uuid: harrow,
                enemy_uuid: federation,
            }]
        );
        assert!(out.warnings.is_empty());
    }

    #[test]
    fn remove_faction_enemy_without_registry_warns_and_emits_nothing() {
        let fx = Fixture::new();
        let action = TriggerAction::RemoveFactionEnemy {
            faction: "Harrow".to_string(),
            enemy: "Federation".to_string(),
        };
        let out = dispatch_action(&action, &fx.ctx());

        assert!(out.commands.is_empty());
        assert_eq!(
            out.warnings,
            vec!["RemoveFactionEnemy skipped: FactionRegistryResource not present".to_string()]
        );
    }

    #[test]
    fn remove_faction_enemy_unknown_faction_warns_and_emits_nothing() {
        let (registry, _, _) = two_factions();
        let mut fx = Fixture::new();
        fx.factions = Some(registry);
        let action = TriggerAction::RemoveFactionEnemy {
            faction: "Nobody".to_string(),
            enemy: "Federation".to_string(),
        };
        let out = dispatch_action(&action, &fx.ctx());

        assert!(out.commands.is_empty());
        assert_eq!(
            out.warnings,
            vec!["RemoveFactionEnemy: unknown faction name 'Nobody'".to_string()]
        );
    }

    #[test]
    fn remove_faction_enemy_unknown_enemy_warns_and_emits_nothing() {
        let (registry, _, _) = two_factions();
        let mut fx = Fixture::new();
        fx.factions = Some(registry);
        let action = TriggerAction::RemoveFactionEnemy {
            faction: "Harrow".to_string(),
            enemy: "Nobody".to_string(),
        };
        let out = dispatch_action(&action, &fx.ctx());

        assert!(out.commands.is_empty());
        assert_eq!(
            out.warnings,
            vec!["RemoveFactionEnemy: unknown enemy faction name 'Nobody'".to_string()]
        );
    }

    // ── dispatch_state_action (direct) ────────────────────────────────────
    //
    // The tests above drive the six state arms through `dispatch_action`,
    // proving the routing arm delegates. These call `dispatch_state_action`
    // directly, proving the extracted function is what produces the result and
    // that the two entry points agree (issue #711).

    #[test]
    fn state_add_objective_falls_back_to_trigger_entity_directly() {
        let mut fx = Fixture::new();
        fx.entity_name = Some("trigger_ship".to_string());
        let out = dispatch_state_action(&add_objective(vec![]), &fx.ctx());

        assert_eq!(
            out.commands,
            vec![ActionCmd::AddObjective {
                id: "obj1".to_string(),
                text: "Destroy the convoy".to_string(),
                mandatory: true,
                targets: vec!["trigger_ship".to_string()],
                directive: AiDirective::default(),
                utility: UtilityConfig::default(),
                source: ObjectiveSource::default(),
                origin_layer: None,
            }]
        );
        assert!(out.warnings.is_empty());
        assert!(out.new_events.is_empty());
    }

    #[test]
    fn state_add_objective_matches_dispatch_action_routing() {
        let mut fx = Fixture::new();
        fx.entity_name = Some("trigger_ship".to_string());
        let action = add_objective(vec!["alpha", "beta"]);

        // Both entry points must produce byte-identical results.
        assert_eq!(
            dispatch_action(&action, &fx.ctx()),
            dispatch_state_action(&action, &fx.ctx())
        );
    }

    #[test]
    fn state_complete_objective_emits_command_directly() {
        let fx = Fixture::new();
        let action = TriggerAction::CompleteObjective {
            id: "obj1".to_string(),
        };
        let out = dispatch_state_action(&action, &fx.ctx());

        assert_eq!(
            out.commands,
            vec![ActionCmd::CompleteObjective {
                id: "obj1".to_string()
            }]
        );
        assert!(out.warnings.is_empty());
    }

    #[test]
    fn state_fail_objective_emits_command_directly() {
        let fx = Fixture::new();
        let action = TriggerAction::FailObjective {
            id: "obj1".to_string(),
        };
        let out = dispatch_state_action(&action, &fx.ctx());

        assert_eq!(
            out.commands,
            vec![ActionCmd::FailObjective {
                id: "obj1".to_string()
            }]
        );
    }

    #[test]
    fn state_game_over_sets_reason_before_state_directly() {
        let fx = Fixture::new();
        let action = TriggerAction::GameOver {
            message: Some("The ship was lost".to_string()),
            outcome: None,
        };
        let out = dispatch_state_action(&action, &fx.ctx());

        // Ordering is load-bearing: OnEnter(GameOver) reads the reason first.
        assert_eq!(
            out.commands,
            vec![
                ActionCmd::SetGameOverReason {
                    reason: "The ship was lost".to_string(),
                    outcome: None,
                },
                ActionCmd::SetNextState {
                    phase: GamePhase::GameOver
                },
            ]
        );
    }

    #[test]
    fn state_game_over_without_message_yields_empty_reason_not_none_directly() {
        let fx = Fixture::new();
        let action = TriggerAction::GameOver {
            message: None,
            outcome: None,
        };
        let out = dispatch_state_action(&action, &fx.ctx());

        assert_eq!(
            out.commands[0],
            ActionCmd::SetGameOverReason {
                reason: String::new(),
                outcome: None,
            }
        );
    }

    #[test]
    fn state_add_faction_enemy_emits_command_directly() {
        let (registry, harrow, federation) = two_factions();
        let mut fx = Fixture::new();
        fx.factions = Some(registry);
        let action = TriggerAction::AddFactionEnemy {
            faction: "Harrow".to_string(),
            enemy: "Federation".to_string(),
        };
        let out = dispatch_state_action(&action, &fx.ctx());

        assert_eq!(
            out.commands,
            vec![ActionCmd::AddFactionEnemy {
                faction_uuid: harrow,
                enemy_uuid: federation,
            }]
        );
        assert!(out.warnings.is_empty());
    }

    #[test]
    fn state_add_faction_enemy_without_registry_warns_directly() {
        let fx = Fixture::new();
        let action = TriggerAction::AddFactionEnemy {
            faction: "Harrow".to_string(),
            enemy: "Federation".to_string(),
        };
        let out = dispatch_state_action(&action, &fx.ctx());

        assert!(out.commands.is_empty());
        assert_eq!(
            out.warnings,
            vec!["AddFactionEnemy skipped: FactionRegistryResource not present".to_string()]
        );
    }

    #[test]
    fn state_remove_faction_enemy_emits_command_directly() {
        let (registry, harrow, federation) = two_factions();
        let mut fx = Fixture::new();
        fx.factions = Some(registry);
        let action = TriggerAction::RemoveFactionEnemy {
            faction: "Harrow".to_string(),
            enemy: "Federation".to_string(),
        };
        let out = dispatch_state_action(&action, &fx.ctx());

        assert_eq!(
            out.commands,
            vec![ActionCmd::RemoveFactionEnemy {
                faction_uuid: harrow,
                enemy_uuid: federation,
            }]
        );
        assert!(out.warnings.is_empty());
    }

    #[test]
    #[should_panic(expected = "dispatch_state_action called with non-state action")]
    fn state_action_on_non_state_variant_panics() {
        // The guard exists so a routing bug in `dispatch_action` fails loudly
        // rather than silently returning an empty result.
        let fx = Fixture::new();
        let action = TriggerAction::UnloadWorld {
            path: "worlds/sub.toml".to_string(),
        };
        let _ = dispatch_state_action(&action, &fx.ctx());
    }

    // ── dispatch_entity_modifier_action (direct) ──────────────────────────
    //
    // The tests above drive the six modifier/flag arms through
    // `dispatch_action`, proving the routing arm delegates. These call
    // `dispatch_entity_modifier_action` directly, proving the extracted
    // function is what produces the result and that the two entry points
    // agree (issue #712).

    #[test]
    fn modifier_apply_modifier_resolves_name_to_uuid_directly() {
        let fx = Fixture::new().with_entity("raider", "uuid-raider");
        let action = TriggerAction::ApplyModifier {
            entity: "raider".to_string(),
            tag: "buff".to_string(),
            slot: ModifierSlot::MaxSpeed,
            bonus: 2.5,
        };
        let out = dispatch_entity_modifier_action(&action, &fx.ctx());

        assert_eq!(
            out,
            DispatchResult {
                commands: vec![ActionCmd::ApplyModifier {
                    uuid: "uuid-raider".to_string(),
                    tag: "buff".to_string(),
                    slot: ModifierSlot::MaxSpeed,
                    bonus: 2.5,
                }],
                ..Default::default()
            }
        );
    }

    #[test]
    fn modifier_apply_modifier_unknown_entity_warns_directly() {
        let fx = Fixture::new();
        let action = TriggerAction::ApplyModifier {
            entity: "ghost".to_string(),
            tag: "buff".to_string(),
            slot: ModifierSlot::MaxSpeed,
            bonus: 2.5,
        };
        let out = dispatch_entity_modifier_action(&action, &fx.ctx());

        assert_eq!(
            out,
            DispatchResult {
                warnings: vec!["ApplyModifier: unknown entity name 'ghost'".to_string()],
                ..Default::default()
            }
        );
    }

    #[test]
    fn modifier_apply_modifier_matches_dispatch_action_routing() {
        let fx = Fixture::new().with_entity("raider", "uuid-raider");
        let action = TriggerAction::ApplyModifier {
            entity: "raider".to_string(),
            tag: "buff".to_string(),
            slot: ModifierSlot::MaxSpeed,
            bonus: 2.5,
        };

        // Both entry points must produce byte-identical results.
        assert_eq!(
            dispatch_action(&action, &fx.ctx()),
            dispatch_entity_modifier_action(&action, &fx.ctx())
        );
    }

    #[test]
    fn modifier_remove_modifier_resolves_name_to_uuid_directly() {
        let fx = Fixture::new().with_entity("raider", "uuid-raider");
        let action = TriggerAction::RemoveModifier {
            entity: "raider".to_string(),
            tag: "buff".to_string(),
            slot: ModifierSlot::MaxSpeed,
        };
        let out = dispatch_entity_modifier_action(&action, &fx.ctx());

        assert_eq!(
            out,
            DispatchResult {
                commands: vec![ActionCmd::RemoveModifier {
                    uuid: "uuid-raider".to_string(),
                    tag: "buff".to_string(),
                    slot: ModifierSlot::MaxSpeed,
                }],
                ..Default::default()
            }
        );
    }

    #[test]
    fn modifier_remove_modifier_unknown_entity_warns_directly() {
        let fx = Fixture::new();
        let action = TriggerAction::RemoveModifier {
            entity: "ghost".to_string(),
            tag: "buff".to_string(),
            slot: ModifierSlot::MaxSpeed,
        };
        let out = dispatch_entity_modifier_action(&action, &fx.ctx());

        assert_eq!(
            out,
            DispatchResult {
                warnings: vec!["RemoveModifier: unknown entity name 'ghost'".to_string()],
                ..Default::default()
            }
        );
    }

    #[test]
    fn modifier_apply_flag_resolves_name_to_uuid_directly() {
        let fx = Fixture::new().with_entity("raider", "uuid-raider");
        let action = TriggerAction::ApplyFlag {
            entity: "raider".to_string(),
            tag: "cloak".to_string(),
            kind: FlagKind::CommsJammed,
        };
        let out = dispatch_entity_modifier_action(&action, &fx.ctx());

        assert_eq!(
            out,
            DispatchResult {
                commands: vec![ActionCmd::ApplyFlag {
                    uuid: "uuid-raider".to_string(),
                    tag: "cloak".to_string(),
                    kind: FlagKind::CommsJammed,
                }],
                ..Default::default()
            }
        );
    }

    #[test]
    fn modifier_apply_flag_unknown_entity_warns_directly() {
        let fx = Fixture::new();
        let action = TriggerAction::ApplyFlag {
            entity: "ghost".to_string(),
            tag: "cloak".to_string(),
            kind: FlagKind::CommsJammed,
        };
        let out = dispatch_entity_modifier_action(&action, &fx.ctx());

        assert_eq!(
            out,
            DispatchResult {
                warnings: vec!["ApplyFlag: unknown entity name 'ghost'".to_string()],
                ..Default::default()
            }
        );
    }

    #[test]
    fn modifier_remove_flag_resolves_name_to_uuid_directly() {
        let fx = Fixture::new().with_entity("raider", "uuid-raider");
        let action = TriggerAction::RemoveFlag {
            entity: "raider".to_string(),
            tag: "cloak".to_string(),
            kind: FlagKind::CommsJammed,
        };
        let out = dispatch_entity_modifier_action(&action, &fx.ctx());

        assert_eq!(
            out,
            DispatchResult {
                commands: vec![ActionCmd::RemoveFlag {
                    uuid: "uuid-raider".to_string(),
                    tag: "cloak".to_string(),
                    kind: FlagKind::CommsJammed,
                }],
                ..Default::default()
            }
        );
    }

    #[test]
    fn modifier_remove_flag_unknown_entity_warns_directly() {
        let fx = Fixture::new();
        let action = TriggerAction::RemoveFlag {
            entity: "ghost".to_string(),
            tag: "cloak".to_string(),
            kind: FlagKind::CommsJammed,
        };
        let out = dispatch_entity_modifier_action(&action, &fx.ctx());

        assert_eq!(
            out,
            DispatchResult {
                warnings: vec!["RemoveFlag: unknown entity name 'ghost'".to_string()],
                ..Default::default()
            }
        );
    }

    #[test]
    fn modifier_apply_int_modifier_resolves_name_to_uuid_directly() {
        let fx = Fixture::new().with_entity("raider", "uuid-raider");
        let action = TriggerAction::ApplyIntModifier {
            entity: "raider".to_string(),
            tag: "crew".to_string(),
            slot: IntModifierSlot::RepairTeams,
            bonus: 3,
        };
        let out = dispatch_entity_modifier_action(&action, &fx.ctx());

        assert_eq!(
            out,
            DispatchResult {
                commands: vec![ActionCmd::ApplyIntModifier {
                    uuid: "uuid-raider".to_string(),
                    tag: "crew".to_string(),
                    slot: IntModifierSlot::RepairTeams,
                    bonus: 3,
                }],
                ..Default::default()
            }
        );
    }

    #[test]
    fn modifier_apply_int_modifier_unknown_entity_warns_directly() {
        let fx = Fixture::new();
        let action = TriggerAction::ApplyIntModifier {
            entity: "ghost".to_string(),
            tag: "crew".to_string(),
            slot: IntModifierSlot::RepairTeams,
            bonus: 3,
        };
        let out = dispatch_entity_modifier_action(&action, &fx.ctx());

        assert_eq!(
            out,
            DispatchResult {
                warnings: vec!["ApplyIntModifier: unknown entity name 'ghost'".to_string()],
                ..Default::default()
            }
        );
    }

    #[test]
    fn modifier_remove_int_modifier_resolves_name_to_uuid_directly() {
        let fx = Fixture::new().with_entity("raider", "uuid-raider");
        let action = TriggerAction::RemoveIntModifier {
            entity: "raider".to_string(),
            tag: "crew".to_string(),
            slot: IntModifierSlot::RepairTeams,
        };
        let out = dispatch_entity_modifier_action(&action, &fx.ctx());

        assert_eq!(
            out,
            DispatchResult {
                commands: vec![ActionCmd::RemoveIntModifier {
                    uuid: "uuid-raider".to_string(),
                    tag: "crew".to_string(),
                    slot: IntModifierSlot::RepairTeams,
                }],
                ..Default::default()
            }
        );
    }

    #[test]
    fn modifier_remove_int_modifier_unknown_entity_warns_directly() {
        let fx = Fixture::new();
        let action = TriggerAction::RemoveIntModifier {
            entity: "ghost".to_string(),
            tag: "crew".to_string(),
            slot: IntModifierSlot::RepairTeams,
        };
        let out = dispatch_entity_modifier_action(&action, &fx.ctx());

        assert_eq!(
            out,
            DispatchResult {
                warnings: vec!["RemoveIntModifier: unknown entity name 'ghost'".to_string()],
                ..Default::default()
            }
        );
    }

    #[test]
    #[should_panic(expected = "dispatch_entity_modifier_action called with non-modifier action")]
    fn entity_modifier_action_on_non_modifier_variant_panics() {
        // The guard exists so a routing bug in `dispatch_action` fails loudly
        // rather than silently returning an empty result.
        let fx = Fixture::new();
        let action = TriggerAction::UnloadWorld {
            path: "worlds/sub.toml".to_string(),
        };
        let _ = dispatch_entity_modifier_action(&action, &fx.ctx());
    }

    // ── dispatch_world_flag_action (direct) ───────────────────────────────
    //
    // The tests above drive the four world-flag arms through
    // `dispatch_action`, proving the routing arm delegates. These call
    // `dispatch_world_flag_action` directly, proving the extracted function
    // is what produces the result and that the two entry points agree
    // (issue #713).

    #[test]
    fn world_flag_set_emits_mutation_and_flag_set_event_directly() {
        let fx = Fixture::new();
        let action = TriggerAction::SetWorldFlag {
            name: "alarm".to_string(),
        };
        let out = dispatch_world_flag_action(&action, &fx.ctx());

        assert_eq!(
            out,
            DispatchResult {
                commands: vec![ActionCmd::MutateFlag {
                    target_layer: None,
                    name: "alarm".to_string(),
                    mutation: FlagMutation::Set,
                }],
                new_events: vec![WorldEvent::FlagSet {
                    name: "alarm".to_string(),
                    origin_layer: None,
                }],
                ..Default::default()
            }
        );
    }

    #[test]
    fn world_flag_clear_emits_mutation_and_flag_cleared_event_directly() {
        let mut fx = Fixture::new();
        fx.base_flags.set_flag_value("alarm", 1);
        let action = TriggerAction::ClearWorldFlag {
            name: "alarm".to_string(),
        };
        let out = dispatch_world_flag_action(&action, &fx.ctx());

        assert_eq!(
            out,
            DispatchResult {
                commands: vec![ActionCmd::MutateFlag {
                    target_layer: None,
                    name: "alarm".to_string(),
                    mutation: FlagMutation::Clear,
                }],
                new_events: vec![WorldEvent::FlagCleared {
                    name: "alarm".to_string(),
                    origin_layer: None,
                }],
                ..Default::default()
            }
        );
    }

    #[test]
    fn world_flag_increment_zero_to_nonzero_emits_flag_set_directly() {
        let fx = Fixture::new();
        let action = TriggerAction::IncrementWorldFlag {
            name: "kills".to_string(),
            by: 2,
        };
        let out = dispatch_world_flag_action(&action, &fx.ctx());

        assert_eq!(
            out,
            DispatchResult {
                commands: vec![ActionCmd::MutateFlag {
                    target_layer: None,
                    name: "kills".to_string(),
                    mutation: FlagMutation::Increment(2),
                }],
                new_events: vec![WorldEvent::FlagSet {
                    name: "kills".to_string(),
                    origin_layer: None,
                }],
                ..Default::default()
            }
        );
    }

    #[test]
    fn world_flag_set_value_to_zero_emits_flag_cleared_directly() {
        let mut fx = Fixture::new();
        fx.base_flags.set_flag_value("alarm", 7);
        let action = TriggerAction::SetWorldFlagValue {
            name: "alarm".to_string(),
            value: 0,
        };
        let out = dispatch_world_flag_action(&action, &fx.ctx());

        assert_eq!(
            out,
            DispatchResult {
                commands: vec![ActionCmd::MutateFlag {
                    target_layer: None,
                    name: "alarm".to_string(),
                    mutation: FlagMutation::SetValue(0),
                }],
                new_events: vec![WorldEvent::FlagCleared {
                    name: "alarm".to_string(),
                    origin_layer: None,
                }],
                ..Default::default()
            }
        );
    }

    /// Layer-chain happy path: a `parent:` prefix from a nested layer resolves
    /// to its loader layer, and both the command and the event carry the
    /// stripped name plus the *resolved* target layer.
    #[test]
    fn world_flag_parent_prefix_resolves_to_the_loader_layer_directly() {
        let mut fx = Fixture::new();
        fx.origin_layer = Some("inner.toml".to_string());
        // inner was loaded by outer; outer was loaded by the base world.
        fx.layers
            .insert("inner.toml".to_string(), layer(Some("outer.toml")));
        fx.layers.insert("outer.toml".to_string(), layer(None));
        let action = TriggerAction::SetWorldFlag {
            name: "parent:alarm".to_string(),
        };
        let out = dispatch_world_flag_action(&action, &fx.ctx());

        assert_eq!(
            out,
            DispatchResult {
                commands: vec![ActionCmd::MutateFlag {
                    target_layer: Some("outer.toml".to_string()),
                    name: "alarm".to_string(),
                    mutation: FlagMutation::Set,
                }],
                new_events: vec![WorldEvent::FlagSet {
                    name: "alarm".to_string(),
                    origin_layer: Some("outer.toml".to_string()),
                }],
                ..Default::default()
            }
        );
    }

    #[test]
    fn world_flag_walk_past_base_world_warns_and_emits_nothing_directly() {
        // Origin is already the base world, so any `parent:` overruns.
        let fx = Fixture::new();
        let action = TriggerAction::SetWorldFlag {
            name: "parent:alarm".to_string(),
        };
        let out = dispatch_world_flag_action(&action, &fx.ctx());

        assert_eq!(
            out,
            DispatchResult {
                warnings: vec![
                    "'parent:alarm' from origin None walks past base world — ignoring".to_string()
                ],
                ..Default::default()
            }
        );
    }

    #[test]
    fn world_flag_target_layer_missing_warns_and_emits_nothing_directly() {
        let mut fx = Fixture::new();
        // The trigger's own layer is not in the map, and there is no `parent:`
        // to walk, so the resolved target is a layer we cannot find.
        fx.origin_layer = Some("ghost.toml".to_string());
        let action = TriggerAction::SetWorldFlag {
            name: "alarm".to_string(),
        };
        let out = dispatch_world_flag_action(&action, &fx.ctx());

        assert_eq!(
            out,
            DispatchResult {
                warnings: vec![
                    "target layer 'ghost.toml' missing from WorldLayerMap — ignoring 'alarm'"
                        .to_string()
                ],
                ..Default::default()
            }
        );
    }

    #[test]
    fn world_flag_layer_missing_mid_walk_is_silent_and_treated_as_base_directly() {
        let mut fx = Fixture::new();
        // `ghost.toml` is absent from the map: the walk silently resolves its
        // loader_path to `None` (base) and carries on. This is deliberate —
        // only the *final* lookup warns.
        fx.origin_layer = Some("ghost.toml".to_string());
        let action = TriggerAction::SetWorldFlag {
            name: "parent:alarm".to_string(),
        };
        let out = dispatch_world_flag_action(&action, &fx.ctx());

        assert_eq!(
            out,
            DispatchResult {
                commands: vec![ActionCmd::MutateFlag {
                    target_layer: None,
                    name: "alarm".to_string(),
                    mutation: FlagMutation::Set,
                }],
                new_events: vec![WorldEvent::FlagSet {
                    name: "alarm".to_string(),
                    origin_layer: None,
                }],
                ..Default::default()
            }
        );
    }

    /// Transition-edge case: setting an already-set flag still commands the
    /// mutation but emits no event, because the boolean view did not flip.
    /// The routed twin (`set_world_flag_on_already_set_flag_emits_no_transition_event`,
    /// issue #708) pins why this depends on `base_flags` being the live store.
    #[test]
    fn world_flag_set_on_already_set_flag_emits_no_event_directly() {
        let mut fx = Fixture::new();
        fx.base_flags.set_flag_value("alarm", 1);
        let action = TriggerAction::SetWorldFlag {
            name: "alarm".to_string(),
        };
        let out = dispatch_world_flag_action(&action, &fx.ctx());

        assert_eq!(
            out,
            DispatchResult {
                commands: vec![ActionCmd::MutateFlag {
                    target_layer: None,
                    name: "alarm".to_string(),
                    mutation: FlagMutation::Set,
                }],
                ..Default::default()
            }
        );
    }

    #[test]
    fn world_flag_set_matches_dispatch_action_routing() {
        let mut fx = Fixture::new();
        fx.origin_layer = Some("sub.toml".to_string());
        fx.layers.insert("sub.toml".to_string(), layer(None));
        let action = TriggerAction::SetWorldFlag {
            name: "parent:alarm".to_string(),
        };

        // Both entry points must produce byte-identical results.
        assert_eq!(
            dispatch_action(&action, &fx.ctx()),
            dispatch_world_flag_action(&action, &fx.ctx())
        );
    }

    #[test]
    #[should_panic(expected = "dispatch_world_flag_action called with non-world-flag action")]
    fn world_flag_action_on_non_world_flag_variant_panics() {
        // The guard exists so a routing bug in `dispatch_action` fails loudly
        // rather than silently returning an empty result.
        let fx = Fixture::new();
        let action = TriggerAction::UnloadWorld {
            path: "worlds/sub.toml".to_string(),
        };
        let _ = dispatch_world_flag_action(&action, &fx.ctx());
    }

    // ── dispatch_destroy_entity (direct) ──────────────────────────────────
    //
    // The tests above drive the DestroyEntity arm through `dispatch_action`
    // (`destroy_entity_emits_command_and_destroyed_event`,
    // `destroy_entity_unknown_name_warns_and_emits_nothing`), proving the
    // routing arm delegates. These call `dispatch_destroy_entity` directly,
    // proving the extracted function is what produces the result and that
    // the two entry points agree (issue #714).

    #[test]
    fn destroy_known_entity_emits_command_and_destroyed_event_directly() {
        let fx = Fixture::new().with_entity("wave_1", "uuid-wave-1");
        let action = TriggerAction::DestroyEntity {
            entity: "wave_1".to_string(),
        };
        let out = dispatch_destroy_entity(&action, &fx.ctx());

        assert_eq!(
            out,
            DispatchResult {
                commands: vec![ActionCmd::DestroyEntity {
                    uuid: "uuid-wave-1".to_string()
                }],
                new_events: vec![WorldEvent::Destroyed {
                    uuid: "uuid-wave-1".to_string()
                }],
                ..Default::default()
            }
        );
    }

    #[test]
    fn destroy_unknown_entity_warns_and_emits_nothing_directly() {
        let fx = Fixture::new();
        let action = TriggerAction::DestroyEntity {
            entity: "ghost".to_string(),
        };
        let out = dispatch_destroy_entity(&action, &fx.ctx());

        assert_eq!(
            out,
            DispatchResult {
                warnings: vec!["DestroyEntity: unknown entity name 'ghost'".to_string()],
                ..Default::default()
            }
        );
    }

    #[test]
    fn destroy_entity_matches_dispatch_action_routing() {
        let fx = Fixture::new().with_entity("wave_1", "uuid-wave-1");
        let action = TriggerAction::DestroyEntity {
            entity: "wave_1".to_string(),
        };

        // Both entry points must produce byte-identical results.
        assert_eq!(
            dispatch_action(&action, &fx.ctx()),
            dispatch_destroy_entity(&action, &fx.ctx())
        );
    }

    #[test]
    #[should_panic(expected = "dispatch_destroy_entity called with non-destroy action")]
    fn destroy_entity_on_non_destroy_variant_panics() {
        // The guard exists so a routing bug in `dispatch_action` fails loudly
        // rather than silently returning an empty result.
        let fx = Fixture::new();
        let action = TriggerAction::UnloadWorld {
            path: "worlds/sub.toml".to_string(),
        };
        let _ = dispatch_destroy_entity(&action, &fx.ctx());
    }

    // ── dispatch_spawn_entity (direct) ────────────────────────────────────
    //
    // The tests above drive the SpawnEntity arm through `dispatch_action`,
    // proving the routing arm delegates. These call `dispatch_spawn_entity`
    // directly, proving the extracted function is what produces the result
    // and that the two entry points agree (issue #715). This is also where
    // the behaviour that MOVED in #715 — template loading behind
    // `DispatchContext::template_loader`, and the failed-spawn contingency
    // gate — is pinned.

    #[test]
    fn spawn_template_loads_with_patched_name_and_inserts_directly() {
        let fx = Fixture::new().with_destroyer();
        let out = dispatch_spawn_entity(
            &spawn(None, Some([1.0, 2.0, 3.0]), vec!["wave", "hostiles"]),
            &fx.ctx(),
        );

        assert_eq!(
            out,
            DispatchResult {
                commands: vec![ActionCmd::SpawnEntity {
                    config: patched_destroyer_template("wave_1"),
                    name: "wave_1".to_string(),
                    uuid: STUB_UUID.to_string(),
                    position: [1.0, 2.0, 3.0],
                    rotation: None,
                    scale: None,
                    layer_path: None,
                    overrides: None,
                }],
                name_to_uuid_inserts: vec![("wave_1".to_string(), STUB_UUID.to_string())],
                entity_group_inserts: vec![
                    ("wave".to_string(), "wave_1".to_string()),
                    ("hostiles".to_string(), "wave_1".to_string()),
                ],
                ..Default::default()
            }
        );
    }

    /// The shipped test for the contingency gate (issue #715): a template
    /// that fails to resolve produces a warning-only result — no command, no
    /// name → uuid insert, no group insert. Before #715 this gate lived in
    /// the applier (`spawn_failed`), where a #710 review flagged it had only
    /// throwaway coverage.
    #[test]
    fn spawn_template_not_found_warns_and_emits_nothing_directly() {
        // No `.with_destroyer()`: the loader has no templates at all.
        let fx = Fixture::new();
        let out =
            dispatch_spawn_entity(&spawn(None, Some([1.0, 2.0, 3.0]), vec!["wave"]), &fx.ctx());

        assert_eq!(
            out,
            DispatchResult {
                warnings: vec![
                    "SpawnEntity 'wave_1' template 'assets/entities/destroyer.toml' not found"
                        .to_string()
                ],
                ..Default::default()
            }
        );
    }

    /// **A `_remove` tombstone in a `spawn_entity` override WARNS** (issue
    /// #911), and the spawn still happens on the unmodified template.
    ///
    /// This is the other instance-layer entry point — `entity_loader::
    /// apply_overrides` is the first — and it is the one that cannot fail the
    /// load, so the warning is the only signal an author gets. It must exist:
    /// a tombstone is subtractive, the author asked for something to be GONE,
    /// and before #911's fix it was accepted in silence (`DoctrineObjective`
    /// is not `deny_unknown_fields`, so the marker vanished into serde and the
    /// doctrine survived). Nothing exercised this path's override arm at all
    /// before this test.
    #[test]
    fn spawn_entity_override_carrying_a_tombstone_warns_and_keeps_the_template() {
        let fx = Fixture::new().with_destroyer();
        let action = TriggerAction::SpawnEntity {
            template_path: DESTROYER_TEMPLATE.to_string(),
            name: "wave_1".to_string(),
            anchor: None,
            position: Some([0.0, 0.0, 0.0]),
            rotation: None,
            scale: None,
            groups: vec![],
            overrides: Some(
                toml::from_str(
                    "[[behaviour.doctrine]]\nid = \"destroy-hostiles\"\n_remove = true\n",
                )
                .unwrap(),
            ),
        };
        let out = dispatch_spawn_entity(&action, &fx.ctx());

        assert_eq!(
            out.warnings.len(),
            1,
            "the tombstone must be reported, got {:?}",
            out.warnings
        );
        assert!(
            out.warnings[0].contains(crate::entity_override::REMOVE_KEY),
            "the warning must name the marker so the author can find it, got {:?}",
            out.warnings[0]
        );
        // The spawn still happens, on the template as authored — a partial
        // spawn is better than none, exactly as for a failed reparse.
        assert_eq!(out.commands.len(), 1, "the template still spawns");
    }

    /// The control: an override WITHOUT a tombstone still applies here. Pinned
    /// alongside the test above so "warns" cannot be achieved by rejecting
    /// every override.
    #[test]
    fn spawn_entity_override_without_a_tombstone_still_applies() {
        let fx = Fixture::new().with_destroyer();
        let action = TriggerAction::SpawnEntity {
            template_path: DESTROYER_TEMPLATE.to_string(),
            name: "wave_1".to_string(),
            anchor: None,
            position: Some([0.0, 0.0, 0.0]),
            rotation: None,
            scale: None,
            groups: vec![],
            overrides: Some(toml::from_str(r#"tags = ["npc", "enemy"]"#).unwrap()),
        };
        let out = dispatch_spawn_entity(&action, &fx.ctx());

        assert!(out.warnings.is_empty(), "got {:?}", out.warnings);
        let ActionCmd::SpawnEntity { config, .. } = &out.commands[0] else {
            panic!("expected a SpawnEntity command, got {:?}", out.commands[0])
        };
        assert_eq!(
            config.tags,
            vec!["npc".to_string(), "enemy".to_string()],
            "an instance override REPLACES tags — the pre-#911 rule, unchanged"
        );
    }

    /// A failed spawn must not consume a uuid: the source is drawn only
    /// after the template resolves (template before uuid — the pre-#710
    /// inline ordering), so failures leave the uuid sequence untouched.
    #[test]
    fn spawn_failed_template_load_does_not_consume_a_uuid_directly() {
        let fx = Fixture::new().with_destroyer();
        let counter = std::cell::Cell::new(0u32);
        let source = || {
            counter.set(counter.get() + 1);
            format!("uuid-{}", counter.get())
        };
        let ctx = DispatchContext {
            uuid_source: &source,
            ..fx.ctx()
        };

        // A template the loader does not know: warns, draws nothing.
        let missing = TriggerAction::SpawnEntity {
            template_path: "assets/entities/missing.toml".to_string(),
            name: "wave_1".to_string(),
            anchor: None,
            position: Some([0.0, 0.0, 0.0]),
            rotation: None,
            scale: None,
            groups: vec![],
            overrides: None,
        };
        let out = dispatch_spawn_entity(&missing, &ctx);
        assert_eq!(out.warnings.len(), 1);
        assert_eq!(counter.get(), 0, "a failed spawn must not consume a uuid");

        // The next successful spawn draws the FIRST uuid, not the second.
        let out = dispatch_spawn_entity(&spawn(None, Some([0.0, 0.0, 0.0]), vec![]), &ctx);
        assert_eq!(
            out.name_to_uuid_inserts,
            vec![("wave_1".to_string(), "uuid-1".to_string())]
        );
    }

    #[test]
    fn spawn_anchor_resolves_from_the_origin_layer_directly() {
        let mut fx = Fixture::new().with_destroyer();
        fx.origin_layer = Some("sub.toml".to_string());
        let mut sub = layer(None);
        sub.anchors.insert("staging".to_string(), [4.0, 5.0, 6.0]);
        fx.layers.insert("sub.toml".to_string(), sub);
        // A same-named base anchor must NOT win for a layer-authored trigger.
        fx.base_anchors
            .insert("staging".to_string(), [99.0, 99.0, 99.0]);

        let out = dispatch_spawn_entity(&spawn(Some("staging"), None, vec![]), &fx.ctx());

        let ActionCmd::SpawnEntity {
            position,
            layer_path,
            ..
        } = &out.commands[0]
        else {
            panic!("expected SpawnEntity");
        };
        assert_eq!(position, &[4.0, 5.0, 6.0]);
        // Origin-layer tracking: the command records the authoring layer so
        // the applier attaches the spawn to it for cascade unload.
        assert_eq!(layer_path, &Some("sub.toml".to_string()));
    }

    /// Layer-originated triggers look ONLY in their own layer's anchor
    /// table: a same-named base-world anchor must not rescue a layer trigger
    /// whose own layer lacks the anchor.
    #[test]
    fn spawn_origin_layer_anchor_missing_warns_despite_base_anchor_directly() {
        let mut fx = Fixture::new().with_destroyer();
        fx.origin_layer = Some("sub.toml".to_string());
        fx.layers.insert("sub.toml".to_string(), layer(None)); // no anchors
        fx.base_anchors
            .insert("staging".to_string(), [99.0, 99.0, 99.0]);

        let out = dispatch_spawn_entity(&spawn(Some("staging"), None, vec![]), &fx.ctx());

        assert_eq!(
            out,
            DispatchResult {
                warnings: vec!["SpawnEntity 'wave_1' anchor 'staging' not found".to_string()],
                ..Default::default()
            }
        );
    }

    #[test]
    fn spawn_anchor_resolves_from_the_base_world_directly() {
        let mut fx = Fixture::new().with_destroyer();
        fx.base_anchors
            .insert("staging".to_string(), [10.0, 0.0, -5.0]);

        let out = dispatch_spawn_entity(&spawn(Some("staging"), None, vec![]), &fx.ctx());

        let ActionCmd::SpawnEntity {
            position,
            layer_path,
            ..
        } = &out.commands[0]
        else {
            panic!("expected SpawnEntity");
        };
        assert_eq!(position, &[10.0, 0.0, -5.0]);
        // Base-world origin: no layer to attach the spawn to.
        assert_eq!(layer_path, &None);
    }

    #[test]
    fn spawn_rotation_and_scale_pass_through_directly() {
        let fx = Fixture::new().with_destroyer();
        let action = TriggerAction::SpawnEntity {
            template_path: DESTROYER_TEMPLATE.to_string(),
            name: "wave_1".to_string(),
            anchor: None,
            position: Some([0.0, 0.0, 0.0]),
            rotation: Some([0.0, 1.57, 0.0]),
            scale: Some([2.0, 2.0, 2.0]),
            groups: vec![],
            overrides: None,
        };
        let out = dispatch_spawn_entity(&action, &fx.ctx());

        let ActionCmd::SpawnEntity {
            rotation, scale, ..
        } = &out.commands[0]
        else {
            panic!("expected SpawnEntity");
        };
        assert_eq!(rotation, &Some([0.0, 1.57, 0.0]));
        assert_eq!(scale, &Some([2.0, 2.0, 2.0]));
    }

    #[test]
    fn spawn_registers_every_group_directly() {
        let fx = Fixture::new().with_destroyer();
        let out = dispatch_spawn_entity(
            &spawn(None, Some([0.0, 0.0, 0.0]), vec!["wave", "hostiles"]),
            &fx.ctx(),
        );

        assert_eq!(
            out.entity_group_inserts,
            vec![
                ("wave".to_string(), "wave_1".to_string()),
                ("hostiles".to_string(), "wave_1".to_string()),
            ]
        );
    }

    /// An empty trigger `name` leaves the template's display name in place —
    /// the patch is conditional on `!name.is_empty()`.
    #[test]
    fn spawn_empty_name_keeps_the_template_display_name_directly() {
        let fx = Fixture::new().with_destroyer();
        let action = TriggerAction::SpawnEntity {
            template_path: DESTROYER_TEMPLATE.to_string(),
            name: String::new(),
            anchor: None,
            position: Some([0.0, 0.0, 0.0]),
            rotation: None,
            scale: None,
            groups: vec![],
            overrides: None,
        };
        let out = dispatch_spawn_entity(&action, &fx.ctx());

        let ActionCmd::SpawnEntity { config, .. } = &out.commands[0] else {
            panic!("expected SpawnEntity");
        };
        assert_eq!(config.name, Some("Harrow Destroyer".to_string()));
    }

    #[test]
    fn spawn_entity_matches_dispatch_action_routing() {
        let mut fx = Fixture::new().with_destroyer();
        fx.origin_layer = Some("sub.toml".to_string());
        let mut sub = layer(None);
        sub.anchors.insert("staging".to_string(), [4.0, 5.0, 6.0]);
        fx.layers.insert("sub.toml".to_string(), sub);
        let action = spawn(Some("staging"), None, vec!["wave"]);

        // Both entry points must produce byte-identical results.
        assert_eq!(
            dispatch_action(&action, &fx.ctx()),
            dispatch_spawn_entity(&action, &fx.ctx())
        );
    }

    #[test]
    #[should_panic(expected = "dispatch_spawn_entity called with non-spawn action")]
    fn spawn_entity_on_non_spawn_variant_panics() {
        // The guard exists so a routing bug in `dispatch_action` fails loudly
        // rather than silently returning an empty result.
        let fx = Fixture::new();
        let action = TriggerAction::UnloadWorld {
            path: "worlds/sub.toml".to_string(),
        };
        let _ = dispatch_spawn_entity(&action, &fx.ctx());
    }
}
