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
//   `DispatchResult::warnings` (or, for the failures severe enough to warrant
//   it, `override_failures` — issue #1048 for a dropped override, issue #1046
//   for an unresolvable template) instead of calling the Bevy log macros. The
//   applier logs each at its field's level (warn / error respectively); tests
//   assert on them.
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

use crate::ai::faction::FactionRegistry;
use crate::core::messages::FlagKind;
use crate::core::messages::{AiDirective, GamePhase, ModifierSlot, ObjectiveSource};
use crate::entities::config::EntityConfig;
use crate::entities::loader::TemplateLoader;
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
        /// Runtime values interpolated into `text`'s `{placeholder}` tokens by
        /// the client. See `messages::TEXT_PARAMS_SUFFIX`.
        text_params: std::collections::BTreeMap<String, String>,
        mandatory: bool,
        targets: Vec<String>,
        directive: AiDirective,
        utility: UtilityConfig,
        source: ObjectiveSource,
        /// An objective-specific Command stance contributed to a named target
        /// Station while this objective is active (issue #1110). `None` for
        /// objectives that contribute no stance.
        command_stance: Option<(
            crate::core::messages::StationId,
            crate::ship::config::StationStanceConfig,
        )>,
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
    /// Move the named entity's infrastructure condition by `delta` points —
    /// negative degrades, positive repairs (issue #1025).
    ///
    /// `entity` is the world's authored entity NAME, not a UUID: the applier
    /// resolves it against `WorldContentRuntime::name_to_uuid` and *queues* the
    /// delta rather than applying it on the spot, so every condition move lands
    /// in the one system that owns operational-flag edges. A timed field-repair
    /// operation applies one small slice of this per tick.
    AdjustInfrastructureCondition { entity: String, delta: f32 },
    /// Move one of the named entity's published `[[infrastructure.capacity]]`
    /// levels by `delta` units — negative spends, positive returns
    /// (issue #1042).
    ///
    /// The capacity sibling of [`Self::AdjustInfrastructureCondition`], and it
    /// resolves and queues on identical terms: `entity` is the authored NAME,
    /// the applier looks it up in `WorldContentRuntime::name_to_uuid`, and
    /// `tick_infrastructure_condition` — the one system that re-publishes a
    /// structure's numbers onto the counters a script predicate reads — does the
    /// arithmetic. A `transfer` completing queues the same
    /// [`CapacityAdjustment`](crate::infrastructure::CapacityAdjustment); this
    /// is a scenario's door to the same queue.
    ///
    /// A DELTA rather than a set, matching every other move in this vocabulary
    /// (condition points, a transfer's cargo) and for the same reason: the sign
    /// convention lives at the call site, where the author can see what they
    /// meant. A scenario publishing a *computed* number writes
    /// `want - <the live counter>` and lands on the value it worked out.
    AdjustInfrastructureCapacity {
        entity: String,
        capacity: String,
        delta: i64,
    },
    /// Call out, settle, or re-price one side of a labour dispute
    /// (issue #1035).
    ///
    /// `id` is the world's authored `[[workforce]]` id, not an entity name and
    /// not a UUID: a workforce is a *party*, exactly as a commitment's
    /// `made_to` is, and the people who run a skyway are not any one hull. So
    /// there is nothing for the applier to resolve — it applies the mutation to
    /// [`WorkforceRegister`](crate::world::workforce::WorkforceRegister)
    /// directly, and a mutation naming a side this world never declared is a
    /// logged no-op rather than a load error, for the reason the register's
    /// own lookup returns "at work" for an unknown id.
    ///
    /// The mirror flag is **not** written here. The host fn that emits this
    /// pushes an ordinary [`ActionCmd::MutateFlag`] beside it, so the flag a
    /// script reads back gets its `FlagSet`/`FlagCleared` transition from the
    /// one path that emits them and an `on_flag_cleared` trigger chains off a
    /// settlement without this command knowing triggers exist.
    SetWorkforceState {
        id: String,
        mutation: crate::world::workforce::WorkforceMutation,
    },
    /// Order the named ship to hold or release its fire (issue #1041).
    ///
    /// `entity` is the world's authored entity NAME, resolved by the applier
    /// against `WorldContentRuntime::name_to_uuid` for the reason every other
    /// name-carrying command here is, and queued rather than applied on the
    /// spot because the applier holds that map and no entity query at all.
    ///
    /// The mirror flag is **not** written here, and this is where the shape
    /// parts company with [`Self::SetWorkforceState`]: a weapons hold has a
    /// second author — the ship's own captain, human or AI, through the
    /// admitted `SetWeaponsHold` command — so the flag is mirrored off the
    /// authoritative component every tick by the one system that owns the
    /// mirror, and a scenario's order gets the same `FlagSet`/`FlagCleared`
    /// transition a captain's press does. A flag written here would have been
    /// written for the scenario's orders and silently absent for the crew's.
    SetWeaponsHold { entity: String, held: bool },
    /// Order the named civilian to hold, divert or dock (issue #1028).
    ///
    /// `entity` is the world's authored entity NAME, not a UUID, and the applier
    /// resolves it against `WorldContentRuntime::name_to_uuid` before *queueing*
    /// the order — it is not applied on the spot, so a scripted order goes
    /// through the same compliance state machine, the same acknowledgement
    /// delay and the same authored disposition a crew's order does. A scenario
    /// cannot remote-control traffic that a crew has to negotiate with.
    OrderCivilian {
        entity: String,
        order: crate::civilian::CivilianOrder,
    },
    /// Write one finding onto a subject's dossier (issue #1031).
    ///
    /// `subject` is the world's authored entity NAME, resolved by the applier
    /// against `WorldContentRuntime::name_to_uuid` for the reason every other
    /// name-carrying command here is — that map is the applier's, not the script
    /// boundary's — and a name no entity answers to is a warned no-op there.
    ///
    /// `gathered_at_tick` is stamped at the SCRIPT surface rather than filled in
    /// by the applier: it is the tick the handler ran on, which is what "when
    /// the crew learned it" means, and the applier drains on whatever tick it
    /// drains on.
    RecordDossierEvidence {
        subject: String,
        text: String,
        provenance: crate::dossier::evidence::EvidenceProvenance,
        gathered_at_tick: u64,
    },
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
    /// [`Outcome`](crate::core::balance::Outcome) (#843, `None` for an undeclared
    /// scripted end).
    ///
    /// Always emitted *before* `SetNextState` — `OnEnter(GamePhase::GameOver)`
    /// reads the reason, so the ordering is load-bearing.
    SetGameOverReason {
        reason: String,
        outcome: Option<crate::core::balance::Outcome>,
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
        /// The template path `config` was resolved from, carried alongside the
        /// resolved config rather than instead of it (issue #863).
        ///
        /// The applier stamps it onto the spawned entity as part of its
        /// [`crate::world::spawn_origin::SpawnOrigin`], which is what lets a
        /// resume rebuild a mid-run spawn no fresh boot re-derives. `config` is
        /// still the thing that gets spawned — nothing re-loads the template
        /// here — so the two cannot disagree about *this* spawn; the path is
        /// what a *later* rebuild resolves.
        template_path: String,
        /// Optional inline TOML overrides already applied to `config` by the
        /// dispatch function; preserved here for auditing / test assertions —
        /// and, since issue #863, so the spawn's origin record can carry the
        /// same document a rebuild has to merge again.
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
/// returns only its diagnostic, on `warnings` or on `override_failures`
/// depending which the failure warrants.
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
    /// Messages the applier should log at ERROR level (issue #1048) — louder
    /// than `warnings`, and deliberately a separate field rather than a
    /// severity tag inside one shared `Vec<String>`, so the applier's log
    /// level per message is structural rather than inferred from text.
    ///
    /// The first producer was `dispatch_spawn_entity`'s override-apply
    /// step: a `spawn_entity` override that was present but could not be
    /// applied at all (the merged document failed to deserialize back into
    /// `EntityConfig`) drops the WHOLE override map and spawns the bare
    /// template — a silent behavioural gap ("hostile" spawns unarmed,
    /// "elite" spawns undamaged) that ordinary warn-level noise is easy to
    /// miss in review. A rejected `_remove` tombstone (issue #911) shares
    /// this channel too: it is reported "on the same channel as a failed
    /// reparse" by original design (see `dispatch_spawn_entity`), and that
    /// design is unchanged here — only the shared channel's volume moved.
    ///
    /// Issue #1046 added the second: a `spawn_entity` whose TEMPLATE does not
    /// resolve. The gap is larger than a dropped override — the entity does not
    /// arrive at all — and it is the runtime half of that issue's contract.
    /// `world::validate` now catches a literal `template_path` at load, so a
    /// miss reaching here is either a computed path (legal, and unreachable by
    /// any load-time scan) or a host whose loader could not be authoritative
    /// about absence at validate time. Both are exactly the cases that must not
    /// pass quietly.
    pub override_failures: Vec<String>,
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
            text_params,
            mandatory,
            targets,
            directive,
            utility,
            source,
            command_stance,
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
                text_params: text_params.clone(),
                mandatory: *mandatory,
                targets: resolved,
                directive: directive.clone(),
                utility: utility.clone(),
                source: source.clone(),
                command_stance: command_stance.clone(),
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
pub(crate) fn push_flag_transition(
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
pub(crate) fn preview_mutation(
    store: &FlagStore,
    name: &str,
    mutation: &FlagMutation,
) -> (i64, i64) {
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
            //
            // Reported on `override_failures` (ERROR) rather than `warnings`
            // since issue #1046, and for that field's own reason: the spawn is
            // refused and the world is silently short of an entity, which is
            // the same class of behavioural gap a dropped override is.
            //
            // WHICH spawns can still reach here is worth being exact about,
            // because it differs by host. `world::validate` now walks scripted
            // `spawn_entity` calls, so a LITERAL `template_path` that does not
            // resolve is caught at load — and on native, where
            // `SpawnTemplateLoader` is authoritative about absence, that is a
            // blocked activation rather than a message here. What is left is
            // the genuinely undecidable: a COMPUTED path (`duel.toml`'s
            // `spawn_slot(ctx, name, template, …)`, whose hull comes from
            // `--side-a`/`--side-b`), which no load-time scan can resolve and
            // which stays legal; and, on a host whose loader cannot be
            // authoritative about absence (wasm before the #984 preloader has
            // served the path), a literal one the gate declined to judge. Both
            // are exactly the cases that must not pass quietly.
            let Some(mut config) = context.template_loader.load_template(template_path) else {
                out.override_failures.push(format!(
                    "SpawnEntity '{name}' template '{template_path}' not found"
                ));
                return out;
            };

            // 2a. The content-ledger residual, said out loud once (issue #1047).
            //
            // `eager_record_world_entities` now walks every template a world's
            // scripts name with a LITERAL path, so those are in the frozen set and
            // an edit to one refuses a save exactly as an edit to a
            // declaratively-listed hull does. What cannot be walked is a COMPUTED
            // path — `duel.toml`'s `spawn_slot(ctx, name, template, …)`, whose hull
            // arrives from `--side-a`/`--side-b` — and reaching this line with an
            // uncovered path is the only moment anything in the system learns such
            // a template exists at all.
            //
            // It reports rather than records: folding it in late would make the
            // content digest depend on how far a session got, so a save taken after
            // this spawn would carry a digest a freshly-booted resume could not
            // reproduce, and the resume would refuse a perfectly valid save. See
            // `content_ledger::note_uncovered_spawn` for that argument in full.
            if crate::content_ledger::note_uncovered_spawn(template_path) {
                out.warnings.push(format!(
                    "SpawnEntity '{name}' template '{template_path}' is not in the frozen \
                     content set, so a save will not refuse to load if this file changes \
                     (issue #1047); a computed template_path cannot be walked at load — \
                     naming it literally is what binds it"
                ));
            }

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
                // reported on the same channel as a failed reparse (now
                // `override_failures`, issue #1048 — see that field's doc for
                // why the whole channel got louder), because the consequence is
                // the same (the template spawns unchanged) and because the one
                // thing that must not happen to a subtractive marker is silence.
                let merged = crate::entities::entity_override::merge_entity_config_toml(
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
                // Issue #1048 raised the volume on that surfacing: reaching
                // this arm at all now includes the common authoring slip of a
                // scripted int-target override leaf missing its `int(…)`
                // marker (`dynamic_to_toml` still defaults a bare int to a
                // toml float), which `EntityConfig::from_toml` rejects here
                // exactly like any other shape mismatch. `override_failures`
                // is logged at ERROR rather than warn (see its doc), because
                // a whole-override drop is a bigger behavioural gap than the
                // rest of this function's warn-level, single-field failures.
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
                // alike. Because this path is non-fatal rather than refusing
                // the spawn outright, the hull otherwise flies the doctrine the
                // author meant to replace, so the `override_failures` entry
                // below is the only signal there is.
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
                    crate::entities::config::EntityConfig::from_toml(&merged_str)
                        .map_err(|e| e.to_string())
                });
                match outcome {
                    Ok(merged_config) => config = merged_config,
                    Err(e) => out.override_failures.push(format!(
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
                template_path: template_path.clone(),
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
#[path = "dispatch_tests.rs"]
mod tests;
