use bevy::prelude::*;
use std::collections::{HashMap, HashSet};

use crate::comms_inbox::CommsInbox;
// Issue #608: the hail / respond / clear / show-on-screen / channel-2 comms
// conversation handlers moved to the comms console module, along the
// comms-inbox seam. Re-imported here (not re-exported) so `WorldPlugin`'s
// system registration and the test app builders below can still reference
// them by their original short names.
use crate::console::comms::server::{
    current_sender_in_range, handle_clear_comms, handle_comms_channel2, handle_hail,
    handle_respond_to_message, handle_show_on_screen,
};
use crate::lobby::{Sessions, Target, WorldResource};
use crate::messages::{CommsContact, CommsMessage, GamePhase, ServerMessage, StationId, ViewMode};
use crate::objectives::ObjectiveManager;
use crate::simulation::SimOutbox;
use crate::world::content::{
    comms_template_states_from_world, evaluate_comms_templates, trigger_states_from_world,
    ActiveDialogue, CommsTemplateState, PendingFollowUp, TriggerAction, TriggerCondition,
    TriggerState, WorldEvent,
};

// -- Resources --------------------------------------------------------------

/// Server-side runtime state for the currently active world content.
///
/// Populated at `Startup` from the unified `WorldConfig` resource (which is
/// inserted by `insert_world_config_resource` when the JS bridge has called
/// `wasm_load_world`). When no world is loaded all vecs/maps are empty and
/// comms systems are no-ops.
#[derive(Resource, Default)]
pub struct WorldContentRuntime {
    /// Mutable per-trigger runtime state (fired flag).
    pub trigger_states: Vec<TriggerState>,
    /// Mutable per-template runtime state (fired flag).
    pub comms_template_states: Vec<CommsTemplateState>,
    /// Active in-flight dialogues keyed by CommsMessage id.
    pub active_dialogues: HashMap<String, ActiveDialogue>,
    /// Named-entity ? UUID mapping (populated from `WorldConfig.name_to_uuid`).
    pub name_to_uuid: HashMap<String, String>,
    /// Hailable contacts derived from world comms templates.
    pub contacts: Vec<CommsContact>,
    /// Set to `true` whenever contacts or other world-level data changes so
    /// `broadcast_comms_state` knows to push a fresh snapshot even if the
    /// inbox itself hasn't changed.
    pub needs_broadcast: bool,
    /// Paths of world TOML files already merged into this runtime, used to
    /// de-duplicate additive world loads (no-op if path already active).
    pub loaded_scenario_paths: HashSet<String>,
    /// Per-entity-UUID snapshot of comms-range flags. Populated by
    /// `update_comms_range_flags` each tick from ship + entity transforms +
    /// `CommsRange` components. UUIDs absent from the map default to true at
    /// stamp time *only when `range_active == false`* (backward compat for
    /// pure-handler tests and lobby phase). When `range_active == true`,
    /// missing UUIDs are treated as `sender_in_range = false`.
    pub range_flags: HashMap<String, bool>,
    /// `true` once `update_comms_range_flags` has located a player `Ship`
    /// and is maintaining `range_flags`. While `false`, range gating is
    /// fully bypassed (preserves lobby + pure-handler tests).
    pub range_active: bool,
    /// World flag / counter store consumed by predicate-gated triggers
    /// (`when = "..."`) and mutated by `set_flag` / `clear_flag` /
    /// `increment_flag` / `set_flag_value` trigger actions. Mutations are
    /// observed inside `handle_ai_events`, which emits `FlagSet` /
    /// `FlagCleared` `WorldEvent`s on transitions and re-evaluates the
    /// trigger table in the same tick so chained `on_flag_set` /
    /// `on_flag_cleared` triggers fire as part of the same Bevy frame.
    pub flags: crate::world::flags::FlagStore,
    /// Queue of synthesised `WorldEvent`s to be drained by `handle_ai_events`
    /// on the next Update tick. Used by `init_world_runtime` (base-world
    /// Startup) and `apply_world_layer_changes` (sub-world Load) to inject
    /// `WorldEvent::WorldLoaded` into the trigger evaluation pipeline
    /// without duplicating the dispatch logic that lives inside
    /// `handle_ai_events`.
    pub pending_world_events: Vec<WorldEvent>,
    /// Comms follow-ups awaiting their trigger condition before injection.
    /// Response follow-ups carry a `placeholder_id` so the inbox shows a
    /// `...` row while the trigger is pending; chained roots stay silent.
    pub pending_follow_ups: Vec<PendingFollowUp>,
    /// `Time::elapsed_secs()` snapshot taken when the base world was loaded
    /// (set by `init_world_runtime`). `on_timer` triggers fire when
    /// `time.elapsed_secs() - world_loaded_at_secs >= after_secs`.
    /// `None` while no world is loaded (lobby, fallback bootstrap), in
    /// which case `handle_ai_events` skips emitting `TimerElapsed` events.
    /// (#475)
    pub world_loaded_at_secs: Option<f32>,
}

/// Bevy resource wrapping the server-side comms inbox.
///
/// Wrapping `CommsInbox` in a newtype lets us insert it as a Bevy `Resource`
/// without adding Bevy dependency to the pure `comms_inbox` module.
#[derive(Resource, Default)]
pub struct CommsInboxRes(pub CommsInbox);

/// Bevy resource wrapping the server-side objective manager.
#[derive(Resource, Default)]
pub struct ObjectiveManagerRes(pub ObjectiveManager);

/// Queue of world TOML paths to load additively into the live `WorldContentRuntime`.
///
/// The `apply_pending_scenario_loads` system drains it each frame, parses the
/// TOML, and merges the new triggers/comms into the runtime. (Currently no
/// trigger action enqueues into this; retained as the merge-side plumbing.)
#[derive(Resource, Default)]
pub struct PendingScenarioLoad(pub Vec<String>);

/// Serialisable runtime snapshot for one additively-loaded sub-world.
///
/// Keyed by world TOML path in `WorldLayerMap`. Holds the trigger and comms
/// states that were derived from the sub-world's `WorldConfig` at load time,
/// enabling them to be cleanly removed when `UnloadWorld` fires.
/// Also tracks ECS entity handles spawned from the sub-world's `[[entity]]`
/// blocks so they can be despawned when `UnloadWorld` fires.
#[derive(Clone, Debug, Default)]
pub struct WorldRuntime {
    pub trigger_states: Vec<TriggerState>,
    pub comms_template_states: Vec<CommsTemplateState>,
    /// ECS entity handles spawned when this layer was loaded.
    pub spawned_entities: Vec<Entity>,
    /// Anchor table from the layer's `WorldConfig`. Used by `spawn_entity`
    /// trigger actions (issue #417) to resolve `anchor = "..."` action
    /// fields when this layer authored the trigger.
    pub anchors: HashMap<String, [f32; 3]>,
    /// Per-layer world flag store (PRD #397 fix 1). Mutations from this
    /// layer's triggers default to this store; `parent:` prefixes on
    /// flag-mutation actions walk up via `loader_path`.
    pub flags: crate::world::flags::FlagStore,
    /// Path of the layer whose trigger called `LoadWorld(path)` to bring
    /// this layer in. `None` = loaded at startup (base world's
    /// `extra_worlds`) Ã¢â‚¬â€ the loader is the base world itself, so
    /// `parent:` from this layer walks straight to the base
    /// `WorldContentRuntime.flags` store.
    pub loader_path: Option<String>,
}

/// Map of `path Ã¢â€ â€™ WorldRuntime` for sub-worlds loaded via `LoadWorld` / `extra_worlds`.
///
/// Each entry is keyed by the world TOML path so `UnloadWorld` can remove it by
/// the same path. Stored as a Bevy `Resource`; an empty map is the initial state.
#[derive(Resource, Default)]
pub struct WorldLayerMap(pub HashMap<String, WorldRuntime>);

/// Queue of `LoadWorld` / `UnloadWorld` actions to execute on the next frame.
///
/// `handle_ai_events` pushes path-keyed commands here; `apply_world_layer_changes`
/// drains it and mutates `WorldLayerMap` + `WorldContentRuntime` accordingly.
#[derive(Resource, Default)]
pub struct PendingWorldLayerChanges(pub Vec<WorldLayerChange>);

/// A single pending world-layer command.
#[derive(Clone, Debug)]
pub enum WorldLayerChange {
    /// Load a sub-world. `loader_path` is the layer whose trigger called
    /// `LoadWorld(path)` to enqueue this Ã¢â‚¬â€ `None` for startup-time loads
    /// (base world's `extra_worlds`). Recorded on the new
    /// `WorldRuntime.loader_path` so `parent:` walks from the loaded
    /// layer reach the right outer flag store (PRD #397 fix 1).
    Load {
        path: String,
        loader_path: Option<String>,
    },
    Unload(String),
}

/// The comms message currently being displayed on the viewscreen.
///
/// Set when a Comms officer sends `ShowOnScreen { message_id }`.
/// Cleared automatically when:
/// - The message is responded to.
/// - The message becomes orphaned or the sender goes out of range.
/// - The captain overrides the view mode away from `ViewMode::Comms`.
#[derive(Resource, Default)]
pub struct OnScreenMessage(pub Option<CommsMessage>);

/// Channel-2 (immediate sim-level) delivery of scenario content into the Comms system.
/// Fired by the world engine instead of mutating `CommsInboxRes` directly; consumed by
/// `handle_comms_channel2` in the Broadcast set.
#[derive(Message, Clone, Debug)]
pub struct CommsChannel2Event {
    pub message: CommsMessage,
}

pub struct WorldPlugin;

impl Plugin for WorldPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<WorldContentRuntime>()
            .init_resource::<CommsInboxRes>()
            .init_resource::<ObjectiveManagerRes>()
            .init_resource::<PendingScenarioLoad>()
            .init_resource::<WorldLayerMap>()
            .init_resource::<PendingWorldLayerChanges>()
            .init_resource::<OnScreenMessage>()
            .add_message::<CommsChannel2Event>()
            .add_systems(
                Startup,
                (
                    insert_world_config_resource,
                    spawn_world_entities,
                    init_world_runtime,
                    load_extra_worlds,
                )
                    .chain(),
            )
            .add_systems(
                OnEnter(crate::messages::GamePhase::InProgress),
                mark_comms_dirty_on_game_start,
            )
            .add_systems(
                Update,
                (
                    handle_hail.in_set(crate::sim_sets::SimSet::Input),
                    handle_respond_to_message.in_set(crate::sim_sets::SimSet::Input),
                    handle_clear_comms.in_set(crate::sim_sets::SimSet::Input),
                    handle_show_on_screen.in_set(crate::sim_sets::SimSet::Input),
                    handle_comms_channel2.in_set(crate::sim_sets::SimSet::Broadcast),
                    auto_clear_on_screen_message.in_set(crate::sim_sets::SimSet::Broadcast),
                    update_comms_range_flags.in_set(crate::sim_sets::SimSet::Broadcast),
                    broadcast_comms_state.in_set(crate::sim_sets::SimSet::Broadcast),
                    broadcast_objective_summary.in_set(crate::sim_sets::SimSet::Broadcast),
                )
                    .chain(),
            )
            .add_systems(
                Update,
                handle_ai_events.in_set(crate::sim_sets::SimSet::Physics),
            )
            .add_systems(
                Update,
                tick_pending_follow_ups
                    .in_set(crate::sim_sets::SimSet::Physics)
                    .before(handle_ai_events),
            )
            .add_systems(
                Update,
                apply_pending_scenario_loads.in_set(crate::sim_sets::SimSet::Physics),
            )
            .add_systems(
                Update,
                apply_world_layer_changes.in_set(crate::sim_sets::SimSet::Physics),
            )
            .add_observer(handle_region_entered_event)
            .add_observer(handle_region_exited_event);
    }
}

/// Observer: bridge `RegionEntered` (player ship boundary crossing into a
/// region) into a queued `WorldEvent::EnteredRegion` so `handle_ai_events`
/// can fan it out to matching triggers on the next tick.
///
/// Looks up the region entity's UUID via `RegionMembership.region_uuids`
/// (populated each tick by `update_region_membership`, and persisted after
/// the entity despawns). Drops the event silently if no UUID is cached
/// (e.g. a region entity spawned without an `EntityUuid` component Ã¢â‚¬â€ not
/// expected in production paths but possible in narrow unit tests).
///
/// Single-fire-per-transition is provided by the region containment
/// system itself: `update_region_membership` uses set differences between
/// the previous and current "inside" sets, so it only triggers the
/// observer event once per boundary crossing. Staying inside on the next
/// tick produces no further `RegionEntered` events.
///
/// After PRD #597 PR 9, `update_region_membership` tracks region membership
/// for every ship (player + NPCs). World-scenario triggers, however, remain
/// player-driven: only crossings by the `LocalShip` are bridged into
/// `pending_world_events`.
fn handle_region_entered_event(
    trigger: On<crate::regions::server::RegionEntered>,
    membership: Option<Res<crate::regions::server::RegionMembership>>,
    runtime: Option<ResMut<WorldContentRuntime>>,
    local_ship_q: Query<(), With<crate::simulation::LocalShip>>,
) {
    let (Some(membership), Some(mut runtime)) = (membership, runtime) else {
        return;
    };
    let ev = trigger.event();
    // World triggers fire only on player-ship boundary crossings; NPC ships
    // (also tracked in RegionMembership after PRD #597 PR 9) are silently
    // dropped here — they still receive region effects via the other
    // observers/systems.
    if local_ship_q.get(ev.subject).is_err() {
        return;
    }
    let Some(uuid) = membership.region_uuids.get(&ev.region_entity).cloned() else {
        return;
    };
    runtime
        .pending_world_events
        .push(WorldEvent::EnteredRegion { uuid });
}

/// Observer: mirror of `handle_region_entered_event` for region exits.
/// Fires both on boundary-crossing exits and on implicit exits when the
/// region entity is despawned while the ship is inside. Filters on
/// `LocalShip` for the same reason: world-scenario triggers are player-driven.
fn handle_region_exited_event(
    trigger: On<crate::regions::server::RegionExited>,
    membership: Option<Res<crate::regions::server::RegionMembership>>,
    runtime: Option<ResMut<WorldContentRuntime>>,
    local_ship_q: Query<(), With<crate::simulation::LocalShip>>,
) {
    let (Some(membership), Some(mut runtime)) = (membership, runtime) else {
        return;
    };
    let ev = trigger.event();
    if local_ship_q.get(ev.subject).is_err() {
        return;
    }
    let Some(uuid) = membership.region_uuids.get(&ev.region_entity).cloned() else {
        return;
    };
    runtime
        .pending_world_events
        .push(WorldEvent::ExitedRegion { uuid });
}

/// Startup system: copy the unified `WorldConfig` from the WASM-side
/// thread-local cache into a Bevy `Resource` so downstream systems
/// (`spawn_world_entities`, `ai::server::tick_ai_controllers`) can read it
/// via `Res<WorldConfig>`.
///
/// On native (no WASM bridge) `get_world_config()` returns `None` and this
/// system is a no-op; downstream systems that iterate world entities
/// simply see an empty world (native unit tests only — production always
/// loads a world TOML through the WASM bridge).
pub(crate) fn insert_world_config_resource(mut commands: Commands) {
    if let Some(world_config) = crate::config_cache::get_world_config() {
        commands.insert_resource(world_config);
    }
}

/// Startup system: spawn `[[entity]]` instances owned by the unified
/// `WorldConfig` pipeline.
///
/// The unified pipeline owns both asteroid-field templates AND any
/// `[[entity]]` carrying a `name` field. The complementary `setup_world`
/// in `server_app.rs` handles anonymous non-asteroid immediate entries
/// (stars, planets); the shared `is_owned_by_unified_pipeline` helper
/// guarantees no entry is spawned twice.
///
/// For named entries the UUID is read from `WorldConfig.name_to_uuid`
/// (populated by an earlier assign-uuid pass in this same system), so the
/// spawned `EntityUuid` component matches the UUID that trigger / comms
/// lookups resolve to. For asteroid-field entries a fresh UUID is allocated.
fn spawn_world_entities(
    mut commands: Commands,
    world_config: Option<ResMut<crate::world::config::WorldConfig>>,
    mut runtime: Option<ResMut<WorldContentRuntime>>,
) {
    let Some(mut world_config) = world_config else {
        return; // No unified WorldConfig (native tests, hardcoded fallback).
    };

    // First pass (PRD #339 slice 2): assign UUIDs to every named [[entity]]
    // entry and register them in `WorldConfig.name_to_uuid` (and mirror
    // into `WorldContentRuntime.name_to_uuid` if present so trigger / comms
    // lookup paths see the same names). This pass runs independently of
    // template resolution so it works even when the config cache is empty
    // (e.g. in unit tests).
    let new_names = crate::world::config::assign_named_entity_uuids(
        &world_config.entities,
        crate::entity_loader::assign_uuid,
    );
    for (name, uuid) in &new_names {
        world_config.name_to_uuid.insert(name.clone(), uuid.clone());
    }
    if let Some(runtime) = runtime.as_mut() {
        for (name, uuid) in &new_names {
            runtime.name_to_uuid.insert(name.clone(), uuid.clone());
        }
    }

    let config_cache = crate::config_cache::get_config_cache();
    let world_snapshot = world_config.clone();
    let _spawned = spawn_immediate_entities_internal(&mut commands, &world_snapshot, &config_cache);
}

/// Spawn the unified-pipeline-owned immediate `[[entity]]` instances.
///
/// Returns the list of spawned `Entity` handles in spawn order
/// (asteroid fields first, then named non-asteroid entries). Callers must
/// flush commands (e.g. via `app.update()`) before querying components.
///
/// Extracted from `spawn_world_entities` so the spawn logic is testable
/// on native: tests pass a fixture `ConfigCache` (plain `HashMap`) directly
/// instead of relying on the WASM-only `CONFIG_CACHE` thread-local.
pub fn spawn_immediate_entities_internal(
    commands: &mut Commands,
    world_config: &crate::world::config::WorldConfig,
    config_cache: &crate::config_cache::ConfigCache,
) -> Vec<Entity> {
    let (fields, named, _anon) =
        crate::world::config::partition_immediate_entities_three_way(world_config, |path| {
            config_cache
                .get(path)
                .and_then(|c| c.asteroid_field.as_ref())
                .is_some()
        });

    // Pre-resolve named-entity positions so `relative_to` references can be
    // looked up during spawn (PRD #337).
    let named_positions = crate::world::config::build_named_entity_positions(world_config);

    let mut spawned = Vec::with_capacity(fields.len() + named.len());

    // Asteroid-field entries get a fresh UUID (they have no name to anchor to).
    for entity_inst in fields {
        let mut config = match crate::entity_loader::resolve_entity(entity_inst, config_cache) {
            Ok(c) => c,
            Err(e) => {
                bevy::log::error!(
                    "spawn_world_entities: failed to resolve asteroid field '{}': {}",
                    entity_inst.template_path,
                    e
                );
                continue;
            }
        };
        // Resolve optional `anchor` reference into a concrete world-space offset
        // applied to the streaming spawner. Missing anchor Ã¢â€ â€™ warn + fall back
        // to world origin so a typo never silently relocates the field.
        if let Some(field) = config.asteroid_field.as_mut() {
            if let Some(anchor_name) = field.anchor.as_ref() {
                match world_config.anchors.get(anchor_name) {
                    Some(pos) => field.anchor_offset = *pos,
                    None => {
                        bevy::log::warn!(
                            "spawn_world_entities: asteroid field '{}' references unknown anchor '{}' Ã¢â‚¬â€ falling back to world origin",
                            entity_inst.template_path, anchor_name
                        );
                        field.anchor_offset = [0.0, 0.0, 0.0];
                    }
                }
            }
        }
        let uuid = crate::entity_loader::assign_uuid();
        let pos = match resolve_position(entity_inst, &world_config.anchors, &named_positions) {
            Ok(p) => p,
            Err(e) => {
                bevy::log::error!("spawn_world_entities: {e}");
                continue;
            }
        };
        let entity = crate::entity_spawner::spawn_entity(
            commands,
            &config,
            pos,
            uuid,
            entity_inst.id.clone(),
        );
        spawned.push(entity);
    }

    // Named non-asteroid entries MUST use the UUID already registered in
    // `world_config.name_to_uuid` so triggers / comms resolve to a real
    // entity. A missing registration is a programmer error Ã¢â‚¬â€ log and skip
    // rather than allocate a fresh UUID (which would silently desync).
    for entity_inst in named {
        let name = entity_inst
            .name
            .as_ref()
            .expect("partition guarantees Some");
        let uuid = match world_config.name_to_uuid.get(name) {
            Some(u) => u.clone(),
            None => {
                bevy::log::error!(
                    "spawn_world_entities: named entity '{}' has no UUID in WorldConfig.name_to_uuid Ã¢â‚¬â€ skipping",
                    name
                );
                continue;
            }
        };
        let config = match crate::entity_loader::resolve_entity(entity_inst, config_cache) {
            Ok(c) => c,
            Err(e) => {
                bevy::log::error!(
                    "spawn_world_entities: failed to resolve named entity '{}' ({}): {}",
                    name,
                    entity_inst.template_path,
                    e
                );
                continue;
            }
        };
        let pos = match resolve_position(entity_inst, &world_config.anchors, &named_positions) {
            Ok(p) => p,
            Err(e) => {
                bevy::log::error!("spawn_world_entities: named entity '{name}': {e}");
                continue;
            }
        };
        let mut config = config;
        if entity_inst.name.is_some() {
            config.name = entity_inst.name.clone();
        }
        let entity = crate::entity_spawner::spawn_entity(
            commands,
            &config,
            pos,
            uuid,
            entity_inst.id.clone(),
        );
        spawned.push(entity);
    }

    spawned
}

/// Resolve an `[[entity]]` instance's spawn position via the pure
/// `world::config::resolve_entity_position` helper, then widen to a Bevy `Vec3`.
///
/// Centralises position resolution for the unified pipeline so anchor-named
/// entries (PRD #337 slice 3) share the same code path as inline-position
/// entries.
fn resolve_position(
    entity_inst: &crate::world::config::WorldEntity,
    anchors: &HashMap<String, [f32; 3]>,
    entities_by_name: &HashMap<String, [f32; 3]>,
) -> Result<Vec3, String> {
    let pos =
        crate::world::config::resolve_entity_position_with(entity_inst, anchors, entities_by_name)?;
    Ok(Vec3::new(pos[0], pos[1], pos[2]))
}

// -- Startup systems ---------------------------------------------------------

/// Startup system: initialise `WorldContentRuntime`, `CommsInboxRes`, and
/// `WorldResource` from the loaded `WorldConfig` (if any).
///
/// This is the post-PRD-#341 sole runtime-init entry point: the legacy
/// scenario / map split is gone. When no `WorldConfig`
/// resource is present (native unit tests) this is a
/// no-op and downstream comms / trigger systems remain quiet.
fn init_world_runtime(
    world_config: Option<Res<crate::world::config::WorldConfig>>,
    mut runtime: ResMut<WorldContentRuntime>,
    mut inbox: ResMut<CommsInboxRes>,
    mut world_resource: ResMut<WorldResource>,
    time: Option<Res<bevy::time::Time>>,
) {
    let Some(world_config) = world_config else {
        return;
    };

    // (#475) Stamp the load-time anchor for `on_timer` triggers. All
    // `after_secs` values are measured relative to this; `handle_ai_events`
    // emits `WorldEvent::TimerElapsed { elapsed_secs }` each tick using
    // `time.elapsed_secs() - world_loaded_at_secs`. `Time` is wrapped in
    // `Option` so older test apps that don't install `TimePlugin` continue
    // to work (they just never see `TimerElapsed` events â€” same as today).
    if let Some(t) = time {
        runtime.world_loaded_at_secs = Some(t.elapsed_secs());
    }

    // Populate scenario metadata so the lobby title/description render correctly.
    world_resource.0.scenario_title = world_config.global.title.clone().unwrap_or_default();
    world_resource.0.scenario_description =
        world_config.global.description.clone().unwrap_or_default();

    // `spawn_world_entities` ran earlier in the Startup chain and already
    // populated `runtime.name_to_uuid` for named [[entity]] instances. Fold
    // in any additional names from `WorldConfig.name_to_uuid` WITHOUT
    // overwriting: existing entries win (single source of truth from the
    // spawn pass).
    for (name, uuid) in &world_config.name_to_uuid {
        runtime
            .name_to_uuid
            .entry(name.clone())
            .or_insert_with(|| uuid.clone());
    }

    // Derive trigger/comms runtime states straight from the parsed world.
    runtime.trigger_states = trigger_states_from_world(&world_config);
    runtime.comms_template_states = comms_template_states_from_world(&world_config);

    // Build the contact list from comms templates using the merged
    // `runtime.name_to_uuid` so unified-pipeline UUIDs are picked up.
    let mut contacts: Vec<CommsContact> = Vec::new();
    for tmpl in &world_config.comms {
        let uuid = match runtime.name_to_uuid.get(&tmpl.from) {
            Some(u) => u.clone(),
            None => continue,
        };
        if !contacts.iter().any(|c: &CommsContact| c.uuid == uuid) {
            contacts.push(CommsContact {
                uuid,
                name: tmpl.from.clone(),
                in_range: true,
                is_urgent: false,
            });
        }
    }
    runtime.contacts = contacts;
    runtime.needs_broadcast = true;

    // Issue #415: emit a WorldLoaded event so `on_world_loaded` triggers
    // declared in the base world fire on the first Update tick. Pushed onto
    // the pending queue (rather than evaluated here) so the dispatch logic
    // inside `handle_ai_events` is the single owner of trigger action
    // execution.
    runtime.pending_world_events.push(WorldEvent::WorldLoaded);

    // Mark inbox dirty so the first InProgress broadcast fires even though
    // no messages have arrived yet.
    inbox.0.mark_dirty();
}

/// Startup system: queue all `extra_worlds` paths from the loaded `WorldConfig`
/// as `LoadWorld` commands so they are merged into the runtime on the first frame.
///
/// Runs after `init_world_runtime` in the Startup chain. Each path is pushed
/// into `PendingWorldLayerChanges` rather than applied directly so the same
/// `apply_world_layer_changes` path handles both startup and trigger-fired loads.
fn load_extra_worlds(
    world_config: Option<Res<crate::world::config::WorldConfig>>,
    mut pending: ResMut<PendingWorldLayerChanges>,
) {
    let Some(world_config) = world_config else {
        return;
    };
    for path in &world_config.extra_worlds {
        pending.0.push(WorldLayerChange::Load {
            path: path.clone(),
            loader_path: None,
        });
    }
}

/// Re-mark the comms runtime dirty when the game enters InProgress.
///
/// `init_world_runtime` marks the runtime dirty during Startup so the first
/// `broadcast_comms_state` fires. However, if no player holds the Comms console
/// during Lobby, that broadcast clears the dirty flag without sending anything.
/// This system ensures the flag is restored when InProgress begins, so the Comms
/// console holder receives the initial contact list on the first InProgress tick.
fn mark_comms_dirty_on_game_start(
    mut runtime: ResMut<WorldContentRuntime>,
    mut inbox: ResMut<CommsInboxRes>,
) {
    if runtime.contacts.is_empty() && runtime.comms_template_states.is_empty() {
        return;
    }
    runtime.needs_broadcast = true;
    inbox.0.mark_dirty();
}

// -- Update systems ----------------------------------------------------------

/// Tick pending comms follow-ups: advance queue-relative timers, evaluate
/// trigger conditions against current world state plus this tick's pending
/// events, and inject any follow-ups whose conditions are now met.
///
/// Ordering: scheduled `.before(handle_ai_events)` so this system observes
/// `pending_world_events` BEFORE `handle_ai_events` drains them. This lets
/// follow-ups react to events on the same tick they fire.
///
/// "Fire immediately if already true" semantics applies to state-based
/// triggers: `OnEnteredRegion` fires if the ship is currently inside the
/// region; `OnFlagSet` fires if the flag is currently set; `OnDestroyed`
/// fires if the named entity is no longer in the ECS; `OnWorldLoaded`
/// always fires. Event-only triggers (`OnAttacked`, `OnHailed`) require
/// the matching event to be observed in `pending_world_events`.
pub(crate) fn tick_pending_follow_ups(
    time: Res<bevy::time::Time>,
    mut runtime: ResMut<WorldContentRuntime>,
    mut inbox: ResMut<CommsInboxRes>,
    mut channel2_writer: MessageWriter<CommsChannel2Event>,
    region_membership: Option<Res<crate::regions::server::RegionMembership>>,
    ship_query: Query<Entity, With<crate::simulation::LocalShip>>,
    entity_uuid_q: Query<&EntityUuid>,
) {
    if runtime.pending_follow_ups.is_empty() {
        return;
    }

    let dt = time.delta_secs();

    // Snapshot events + flags + name lookup before we touch the queue, so
    // every pending follow-up evaluated this tick sees the same world.
    let events_snapshot: Vec<WorldEvent> = runtime.pending_world_events.clone();
    let name_to_uuid_snapshot = runtime.name_to_uuid.clone();
    let flags_snapshot = runtime.flags.clone();

    // Build the set of region UUIDs the player ship is currently inside.
    let inside_region_uuids: HashSet<String> = if let (Some(membership), Some(ship_entity)) =
        (region_membership.as_ref(), ship_query.iter().next())
    {
        membership
            .inside
            .get(&ship_entity)
            .map(|set| {
                set.iter()
                    .filter_map(|e| membership.region_uuids.get(e).cloned())
                    .collect()
            })
            .unwrap_or_default()
    } else {
        HashSet::new()
    };

    // Build the set of all live entity UUIDs (for OnDestroyed checks).
    let live_uuids: HashSet<String> = entity_uuid_q.iter().map(|u| u.0.clone()).collect();

    let mut ready: Vec<PendingFollowUp> = Vec::new();
    let mut keep: Vec<PendingFollowUp> = Vec::with_capacity(runtime.pending_follow_ups.len());

    for mut pfu in runtime.pending_follow_ups.drain(..) {
        pfu.elapsed_secs += dt;
        let fires = follow_up_trigger_holds(
            pfu.node.trigger.as_ref(),
            pfu.elapsed_secs,
            &events_snapshot,
            &name_to_uuid_snapshot,
            &flags_snapshot,
            &inside_region_uuids,
            &live_uuids,
        );
        if fires {
            ready.push(pfu);
        } else {
            keep.push(pfu);
        }
    }
    runtime.pending_follow_ups = keep;

    for pfu in ready {
        if let Some(placeholder_id) = &pfu.placeholder_id {
            inbox.0.remove(placeholder_id);
        }

        // Inject the real message.
        let new_msg_id = uuid::Uuid::new_v4().to_string();
        let responses: Vec<String> = pfu.node.responses.iter().map(|r| r.text.clone()).collect();
        let new_msg = CommsMessage {
            id: new_msg_id.clone(),
            sender_uuid: pfu.sender_uuid.clone(),
            sender_name: pfu.sender_name.clone(),
            subject: pfu.node.body.chars().take(40).collect(),
            body: pfu.node.body.clone(),
            responses,
            selected_response: None,
            is_read: false,
            is_orphaned: false,
            sender_in_range: current_sender_in_range(&runtime, &pfu.sender_uuid),
            thread_id: pfu.thread_id.clone(),
            is_urgent: pfu.urgent,
        };
        channel2_writer.write(CommsChannel2Event { message: new_msg });
        runtime.active_dialogues.insert(
            new_msg_id,
            ActiveDialogue {
                current_node: pfu.node.clone(),
                thread_id: pfu.thread_id.clone(),
            },
        );
    }
}

/// Pure evaluator: returns true when a follow-up trigger condition is met
/// for the given snapshot of world state and observed events.
///
/// State-based conditions check current world state and fire immediately
/// when "already true" â€” `OnEnteredRegion` fires while the ship is inside
/// the region, `OnFlagSet` fires while the flag holds a non-zero counter,
/// `OnDestroyed` fires once the named entity's UUID is absent from the
/// live ECS set, `OnWorldLoaded` always fires (the world is, by
/// construction, loaded once a follow-up is queued).
///
/// Event-based conditions (`OnAttacked`, `OnHailed`) require a matching
/// `WorldEvent` in `events`. `OnTimer` is queue-relative: it compares
/// `elapsed_secs` against the configured `after_secs`.
///
/// A `None` trigger means "fire immediately" â€” the follow-up arrives on
/// the next tick after being queued.
pub(crate) fn follow_up_trigger_holds(
    trigger: Option<&TriggerCondition>,
    elapsed_secs: f32,
    events: &[WorldEvent],
    name_to_uuid: &HashMap<String, String>,
    flags: &crate::world::flags::FlagStore,
    inside_region_uuids: &HashSet<String>,
    live_uuids: &HashSet<String>,
) -> bool {
    let Some(condition) = trigger else {
        return true;
    };
    match condition {
        TriggerCondition::OnTimer { after_secs } => elapsed_secs >= *after_secs,
        TriggerCondition::OnWorldLoaded => true,
        TriggerCondition::OnEnteredRegion { entity_name } => name_to_uuid
            .get(entity_name)
            .map(|u| inside_region_uuids.contains(u))
            .unwrap_or(false),
        TriggerCondition::OnExitedRegion { entity_name } => name_to_uuid
            .get(entity_name)
            .map(|u| !inside_region_uuids.contains(u))
            .unwrap_or(false),
        TriggerCondition::OnFlagSet { name } => {
            // Follow-ups don't currently participate in sub-world layer
            // chains; strip any `parent:` prefix to keep the predicate
            // resolving against the base store. (Matches the comms-template
            // evaluator, which passes a base-only chain.)
            let key = strip_parent_prefix(name);
            flags.flag(key)
        }
        TriggerCondition::OnFlagCleared { name } => {
            let key = strip_parent_prefix(name);
            !flags.flag(key)
        }
        TriggerCondition::OnDestroyed { entity_name } => {
            // "Already destroyed" â€” the entity was registered in
            // `name_to_uuid` but its UUID is no longer in the live ECS set.
            // Also fires on a fresh `Destroyed` event observed this tick.
            name_to_uuid
                .get(entity_name)
                .map(|u| {
                    !live_uuids.contains(u)
                        || events
                            .iter()
                            .any(|e| matches!(e, WorldEvent::Destroyed { uuid } if uuid == u))
                })
                .unwrap_or(false)
        }
        TriggerCondition::OnAllDestroyed { entity_names } => entity_names.iter().all(|name| {
            name_to_uuid
                .get(name)
                .map(|u| !live_uuids.contains(u))
                .unwrap_or(false)
        }),
        TriggerCondition::OnAttacked { entity_name } => name_to_uuid
            .get(entity_name)
            .map(|u| {
                events
                    .iter()
                    .any(|e| matches!(e, WorldEvent::Attacked { uuid, .. } if uuid == u))
            })
            .unwrap_or(false),
        TriggerCondition::OnHailed { entity_name } => name_to_uuid
            .get(entity_name)
            .map(|u| {
                events
                    .iter()
                    .any(|e| matches!(e, WorldEvent::Hailed { target_uuid } if target_uuid == u))
            })
            .unwrap_or(false),
    }
}

fn strip_parent_prefix(name: &str) -> &str {
    let mut rest = name;
    while let Some(s) = rest.strip_prefix("parent:") {
        rest = s;
    }
    rest
}

/// Auto-clear `OnScreenMessage` when the displayed message is no longer valid.
///
/// Clears when:
/// - The message has been responded to (`selected_response` is `Some`).
/// - The message is orphaned (sender entity destroyed/despawned).
/// - The sender is out of comms range.
/// - The ship view mode is no longer `ViewMode::Comms` (captain overrode it).
fn auto_clear_on_screen_message(
    mut on_screen: ResMut<OnScreenMessage>,
    inbox: Res<CommsInboxRes>,
    view_mode_q: Query<&crate::ship_state::ShipViewMode, With<crate::simulation::LocalShip>>,
) {
    if on_screen.0.is_none() {
        return;
    }
    let current_view = view_mode_q
        .single()
        .map(|vm| vm.view_mode.clone())
        .unwrap_or(crate::messages::ViewMode::Camera(
            crate::messages::ViewDirection::Fore,
        ));
    // If the captain (or anyone) has switched away from Comms view, clear.
    if !matches!(current_view, ViewMode::Comms) {
        on_screen.0 = None;
        return;
    }
    // Check the live inbox record for the displayed message.
    let should_clear = if let Some(ref displayed) = on_screen.0 {
        match inbox
            .0
            .messages()
            .into_iter()
            .find(|m| m.id == displayed.id)
        {
            None => true, // message purged from inbox
            Some(live) => {
                live.selected_response.is_some()   // responded to
                || live.is_orphaned                // sender gone
                || !live.sender_in_range // out of range
            }
        }
    } else {
        false
    };
    if should_clear {
        on_screen.0 = None;
    }
}

/// Recompute per-entity comms-range flags from ship + entity transforms.
///
/// Runs before `broadcast_comms_state`. Finds the player ship (entity with
/// `Ship` marker + `Transform` + optional `CommsRange`) and computes
/// `crate::comms::in_range(distance, ship_range, entity_range)` for every
/// entity carrying `EntityUuid` + `Transform` + `CommsRange`. Updates the
/// `runtime.range_flags` map and stamps `runtime.contacts[i].in_range`. Sets
/// `runtime.needs_broadcast = true` if any flag flipped vs. the prior snapshot.
fn update_comms_range_flags(
    mut runtime: ResMut<WorldContentRuntime>,
    ship_q: Query<
        (&Transform, Option<&crate::comms::CommsRange>),
        With<crate::simulation::LocalShip>,
    >,
    entity_q: Query<(
        &crate::entities::spawner::EntityUuid,
        &Transform,
        &crate::comms::CommsRange,
    )>,
) {
    let Some((ship_tf, ship_range_opt)) = ship_q.iter().next() else {
        // No ship: either lobby/pure-handler tests (range tracking never
        // activated Ã¢â‚¬â€ preserve default-true semantics) or the ship was
        // destroyed mid-game. In the latter case, do NOT reset
        // `range_active` to false Ã¢â‚¬â€ that would silently re-enable all
        // comms (a back-door past the Hail/Respond gates). Instead, force
        // every tracked flag to false so the gates stay closed.
        if runtime.range_active {
            let mut any_changed = false;
            for v in runtime.range_flags.values_mut() {
                if *v {
                    *v = false;
                    any_changed = true;
                }
            }
            let before = runtime.contacts.len();
            for c in runtime.contacts.iter_mut() {
                if c.in_range {
                    c.in_range = false;
                    any_changed = true;
                }
            }
            let _ = before;
            if any_changed {
                runtime.needs_broadcast = true;
            }
        }
        return;
    };
    let ship_range = ship_range_opt.map(|r| r.0).unwrap_or(0.0);
    let ship_pos = ship_tf.translation;

    let mut any_changed = !runtime.range_active;
    runtime.range_active = true;

    // Build the live set of comms-range-bearing UUIDs and refresh flags.
    let mut live: HashSet<String> = HashSet::new();
    for (uuid, tf, range) in entity_q.iter() {
        let dist = ship_pos.distance(tf.translation);
        let in_range = crate::comms::in_range(dist, ship_range, range.0);
        let prior = runtime.range_flags.insert(uuid.0.clone(), in_range);
        if prior != Some(in_range) {
            any_changed = true;
        }
        live.insert(uuid.0.clone());
    }

    // Remove stale flags for despawned entities.
    let stale: Vec<String> = runtime
        .range_flags
        .keys()
        .filter(|k| !live.contains(*k))
        .cloned()
        .collect();
    if !stale.is_empty() {
        any_changed = true;
        for k in stale {
            runtime.range_flags.remove(&k);
        }
    }

    // Prune contacts whose entity has no [comms] block (no CommsRange).
    let before = runtime.contacts.len();
    let live_ref = &live;
    runtime.contacts.retain(|c| live_ref.contains(&c.uuid));
    if runtime.contacts.len() != before {
        any_changed = true;
    }

    // Stamp the surviving contacts in place from the flag map.
    let WorldContentRuntime {
        range_flags,
        contacts,
        ..
    } = &mut *runtime;
    for c in contacts.iter_mut() {
        if let Some(flag) = range_flags.get(&c.uuid).copied() {
            c.in_range = flag;
        }
    }

    if any_changed {
        runtime.needs_broadcast = true;
    }
}

/// Broadcast `CommsState` to the Comms console holder when the inbox is dirty
/// or `WorldContentRuntime::needs_broadcast` is set.
fn broadcast_comms_state(
    sessions: Res<Sessions>,
    ship_query: Query<(), With<crate::simulation::LocalShip>>,
    mut runtime: ResMut<WorldContentRuntime>,
    mut inbox: ResMut<CommsInboxRes>,
    objectives: Res<ObjectiveManagerRes>,
    mut outbox: ResMut<SimOutbox>,
) {
    let dirty = inbox.0.is_dirty() || runtime.needs_broadcast || objectives.0.is_dirty();
    if !dirty {
        return;
    }

    let Some(()) = ship_query.iter().next() else {
        return;
    };
    let Some(comms_token) = sessions.0.holder_for_station(&StationId("comms".into())) else {
        inbox.0.mark_clean();
        runtime.needs_broadcast = false;
        return;
    };

    let mut messages = inbox.0.messages();
    for m in messages.iter_mut() {
        if let Some(flag) = runtime.range_flags.get(&m.sender_uuid).copied() {
            m.sender_in_range = flag;
        } else if runtime.range_active {
            // Synthetic senders (non-UUID ids like "_self", "Starcorp Command")
            // are always readable â€” they have no physical entity to range-check.
            if uuid::Uuid::parse_str(&m.sender_uuid).is_ok() {
                m.sender_in_range = false;
            }
            // else: leave sender_in_range = true for synthetic senders
        }
    }
    let objectives_snap = objectives.0.sorted_snapshots();
    let mut contacts = runtime.contacts.clone();
    // Auto-derive is_urgent: a contact is urgent when it has at least one
    // unread urgent message in the current inbox.
    for contact in contacts.iter_mut() {
        contact.is_urgent = messages
            .iter()
            .any(|m| m.sender_uuid == contact.uuid && m.is_urgent && !m.is_read);
    }

    outbox.0.push((
        Target::Token(comms_token.to_string()),
        ServerMessage::CommsState {
            messages,
            objectives: objectives_snap,
            contacts,
        },
    ));

    inbox.0.mark_clean();
    runtime.needs_broadcast = false;
}

/// Broadcast `ObjectiveSummary` when objectives change.
fn broadcast_objective_summary(
    mut objectives: ResMut<ObjectiveManagerRes>,
    mut outbox: ResMut<SimOutbox>,
) {
    if !objectives.0.is_dirty() {
        return;
    }

    let objectives_snap = objectives.0.sorted_snapshots();

    outbox.0.push((
        Target::All,
        ServerMessage::ObjectiveSummary {
            objectives: objectives_snap,
        },
    ));

    objectives.0.mark_clean();
}

// -- AI-event trigger system -------------------------------------------------

/// Read `AiEntityAttacked` and `AiEntityDestroyed` messages, translate them
/// into `WorldEvent`s, evaluate the scenario trigger table, and execute the
/// resulting actions (including `SetAiState`, `ApplyModifier`, `RemoveModifier`,
/// `ApplyFlag`, and `RemoveFlag`).
fn handle_ai_events(
    mut runtime: ResMut<WorldContentRuntime>,
    mut objectives: ResMut<ObjectiveManagerRes>,
    mut channel2_writer: MessageWriter<CommsChannel2Event>,
    mut commands: Commands,
    mut attacked_reader: MessageReader<crate::ai_plugin::AiEntityAttacked>,
    mut destroyed_reader: MessageReader<crate::ai_plugin::AiEntityDestroyed>,
    mut ai_query: Query<
        (
            &EntityUuid,
            Option<&mut crate::ai_plugin::ShipAiMemory>,
            Option<&crate::entities::spawner::FactionComponent>,
        ),
        With<AiControllerComponent>,
    >,
    mut ship_modifiers: ShipModifiersParams,
    mut next_state: Option<ResMut<NextState<GamePhase>>>,
    mut game_over_reason: Option<ResMut<crate::simulation::GameOverReason>>,
    mut pending_layers: Option<ResMut<PendingWorldLayerChanges>>,
    mut layer_map: Option<ResMut<WorldLayerMap>>,
    base_world_config: Option<Res<crate::world::config::WorldConfig>>,
    entity_uuid_query: Query<(Entity, &EntityUuid)>,
    mut faction_dispatch: FactionDispatchParams,
    time: Option<Res<bevy::time::Time>>,
) {
    let mut world_events: Vec<WorldEvent> = Vec::new();
    for ev in attacked_reader.read() {
        world_events.push(WorldEvent::Attacked {
            uuid: ev.entity_uuid.clone(),
            attacker_uuid: ev.attacker_uuid.to_string(),
        });
    }
    for ev in destroyed_reader.read() {
        world_events.push(WorldEvent::Destroyed {
            uuid: ev.entity_uuid.clone(),
        });
    }
    // Drain any externally-queued world events (e.g. WorldLoaded pushed by
    // init_world_runtime or apply_world_layer_changes). This lets those
    // emission sites participate in the existing evaluate+dispatch+chain
    // loop below without duplicating the action dispatch table.
    if !runtime.pending_world_events.is_empty() {
        let drained: Vec<WorldEvent> = runtime.pending_world_events.drain(..).collect();
        world_events.extend(drained);
    }
    // (#475) Emit a `TimerElapsed` event each tick once the world has
    // loaded. `on_timer` triggers fire when `elapsed_secs >= after_secs`,
    // measured from `world_loaded_at_secs` (so `after_secs = 0` fires on
    // the first post-load tick, and `after_secs = 300` fires 300s into
    // the scenario regardless of how long the lobby was up beforehand).
    // Single-shot semantics on `TriggerState.fired` prevent re-firing.
    // `Time` is optional so test apps without `TimePlugin` continue to
    // work (they just never see `TimerElapsed`).
    if let (Some(t), Some(loaded_at)) = (time.as_ref(), runtime.world_loaded_at_secs) {
        let elapsed_secs = (t.elapsed_secs() - loaded_at).max(0.0);
        world_events.push(WorldEvent::TimerElapsed { elapsed_secs });
    }
    if world_events.is_empty() {
        return;
    }

    let name_to_uuid = runtime.name_to_uuid.clone();

    // Build UUID → ECS Entity map once per tick so the six per-entity
    // modifier/flag arms below can resolve their `entity` target in O(1)
    // instead of scanning `entity_uuid_query` each time. Used by
    // `ApplyModifier` / `RemoveModifier` / `ApplyFlag` / `RemoveFlag` /
    // `ApplyIntModifier` / `RemoveIntModifier` to write to the target
    // entity's per-entity `ShipModifiers` Component.
    let uuid_to_entity: std::collections::HashMap<String, Entity> = entity_uuid_query
        .iter()
        .map(|(ent, uuid_comp)| (uuid_comp.0.clone(), ent))
        .collect();

    // Auto-fire comms templates that match the world events (e.g. on_attacked distress calls).
    // These are injected without any player hailing Ã¢â‚¬â€ they are broadcast messages.
    let fired_comms = evaluate_comms_templates(
        &mut runtime.comms_template_states,
        &world_events,
        &name_to_uuid,
    );
    for fc in fired_comms {
        let thread_id = fc
            .thread_id
            .clone()
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        // `_self` is the reserved synthetic internal-sender name; render it as
        // "Internal Report" in the comms UI so the crew sees a ship-generated
        // intelligence summary rather than a literal "_self" sender label.
        let channel_name = if fc.from == "_self" {
            "Internal Report".to_string()
        } else {
            fc.from.clone()
        };
        let sender_name = fc.node.speaker.clone().unwrap_or(channel_name);
        let sender_uuid = name_to_uuid
            .get(&fc.from)
            .cloned()
            .unwrap_or_else(|| fc.from.clone());

        // Root templates inject immediately when their template-level
        // `trigger` fires. Per-node triggers are reserved for follow-ups.
        let msg_id = uuid::Uuid::new_v4().to_string();
        let responses: Vec<String> = fc.node.responses.iter().map(|r| r.text.clone()).collect();
        let msg = crate::messages::CommsMessage {
            id: msg_id.clone(),
            sender_uuid: sender_uuid.clone(),
            sender_name: sender_name.clone(),
            subject: fc.node.body.chars().take(40).collect(),
            body: fc.node.body.clone(),
            responses,
            selected_response: None,
            is_read: false,
            is_orphaned: false,
            sender_in_range: current_sender_in_range(&runtime, &sender_uuid),
            thread_id: thread_id.clone(),
            is_urgent: fc.urgent,
        };
        channel2_writer.write(CommsChannel2Event { message: msg });
        runtime.active_dialogues.insert(
            msg_id,
            ActiveDialogue {
                current_node: fc.node.clone(),
                thread_id: thread_id.clone(),
            },
        );

        // Schedule the chained root follow_up, if any. See the
        // matching block in `handle_hail` for the rationale.
        if let Some(ref fu) = fc.root_follow_up {
            let fu_sender_name = fu.speaker.clone().unwrap_or(sender_name.clone());
            runtime.pending_follow_ups.push(PendingFollowUp {
                node: fu.clone(),
                sender_uuid: sender_uuid.clone(),
                sender_name: fu_sender_name,
                thread_id: thread_id.clone(),
                elapsed_secs: 0.0,
                placeholder_id: None,
                urgent: fc.urgent,
            });
        }
    }

    // Loop to support within-tick chaining: a trigger that fires a
    // `set_flag` action emits a `FlagSet` event which a downstream
    // `on_flag_set` trigger can react to in the same Bevy frame. Bounded
    // for safety against pathological feedback loops.
    //
    // PRD #397 fix 1: each trigger is evaluated with its OWN flag chain
    // and layer chain, computed from its `origin_layer` by walking
    // `loader_path` pointers up via `WorldLayerMap` until reaching the
    // base world (whose store is `runtime.flags`). The chains are
    // snapshotted once per pass so trigger ordering within a pass is
    // deterministic (later triggers in the same pass see the same
    // flag values as earlier ones; their mutations land in `next_events`
    // and are observed on the next pass).
    let mut current_events = world_events.clone();
    let mut pass = 0;
    let max_passes = 16;
    loop {
        pass += 1;
        // Snapshot per-layer flag stores and loader pointers once per pass.
        let base_flags_snapshot = runtime.flags.clone();
        let layer_flags_snapshot: HashMap<String, crate::world::flags::FlagStore> = layer_map
            .as_ref()
            .map(|lm| {
                lm.0.iter()
                    .map(|(p, wr)| (p.clone(), wr.flags.clone()))
                    .collect()
            })
            .unwrap_or_default();
        let layer_loaders_snapshot: HashMap<String, Option<String>> = layer_map
            .as_ref()
            .map(|lm| {
                lm.0.iter()
                    .map(|(p, wr)| (p.clone(), wr.loader_path.clone()))
                    .collect()
            })
            .unwrap_or_default();

        // Per-trigger evaluation: build chain from origin_layer up.
        let mut fired: Vec<crate::world::content::FiredTrigger> = Vec::new();
        // We have to clone the origin_layer slice up front to avoid
        // holding a borrow on `runtime.trigger_states` across the chain
        // build (chain references the snapshot, not the live store).
        let trigger_origins: Vec<Option<String>> = runtime
            .trigger_states
            .iter()
            .map(|s| s.origin_layer.clone())
            .collect();
        for (idx, origin) in trigger_origins.iter().enumerate() {
            // Build the flag-store and layer-path chains for this trigger.
            let mut flag_chain_owned: Vec<&crate::world::flags::FlagStore> = Vec::new();
            let mut layer_chain: Vec<Option<String>> = Vec::new();
            let mut cur = origin.clone();
            loop {
                layer_chain.push(cur.clone());
                match &cur {
                    Some(p) => {
                        if let Some(fs) = layer_flags_snapshot.get(p) {
                            flag_chain_owned.push(fs);
                        } else {
                            // Layer missing from snapshot â€” treat as empty.
                            // (Shouldn't happen in normal flow.)
                            flag_chain_owned.push(&base_flags_snapshot);
                            break;
                        }
                        cur = layer_loaders_snapshot.get(p).cloned().flatten();
                    }
                    None => {
                        flag_chain_owned.push(&base_flags_snapshot);
                        break;
                    }
                }
            }
            // The base entry was already pushed above when cur went to None.
            // If we exited via the layer-missing branch we also pushed base.
            let result = crate::world::content::evaluate_single_trigger(
                &mut runtime.trigger_states[idx],
                &current_events,
                &name_to_uuid,
                &flag_chain_owned,
                &layer_chain,
            );
            if let Some(ft) = result {
                fired.push(ft);
            }
        }

        if fired.is_empty() {
            break;
        }

        let mut next_events: Vec<WorldEvent> = Vec::new();
        for ft in fired {
            for action in &ft.actions {
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
                        // Explicit targets win; otherwise fall back to the
                        // trigger condition's entity (legacy behaviour).
                        let resolved = if targets.is_empty() {
                            ft.entity_name.clone().into_iter().collect()
                        } else {
                            targets.clone()
                        };
                        objectives.0.add_full(
                            id.clone(),
                            text.clone(),
                            *mandatory,
                            resolved,
                            directive.clone(),
                            utility.clone(),
                            source.clone(),
                        );
                    }
                    TriggerAction::CompleteObjective { id } => {
                        objectives.0.complete(id);
                    }
                    TriggerAction::FailObjective { id } => {
                        objectives.0.fail(id);
                    }
                    TriggerAction::SetAiState {
                        entity,
                        state,
                        target: _,
                    } => {
                        // No-op in doctrine-based AI (issue #572). FSM state slots are
                        // gone; NPC behaviour is now driven by the scored doctrine pool.
                        bevy::log::warn!(
                            "handle_ai_events: SetAiState(‘{entity}’ \u{2192} ‘{state}’) ignored \u{2014} doctrine-based AI"
                        );
                    }
                    TriggerAction::ApplyModifier {
                        entity,
                        tag,
                        slot,
                        bonus,
                    } => {
                        let Some(uuid) = name_to_uuid.get(entity) else {
                            bevy::log::warn!(
                                "handle_ai_events: ApplyModifier: unknown entity name '{entity}'"
                            );
                            continue;
                        };
                        let Some(target) = uuid_to_entity.get(uuid).copied() else {
                            bevy::log::warn!(
                                "handle_ai_events: ApplyModifier: no ECS entity with UUID '{uuid}' for name '{entity}'"
                            );
                            continue;
                        };
                        let Ok(mut mods) = ship_modifiers.components.get_mut(target) else {
                            bevy::log::warn!(
                                "handle_ai_events: ApplyModifier: entity '{entity}' has no ShipModifiers component"
                            );
                            continue;
                        };
                        mods.add_or_update(crate::modifiers::Modifier {
                            source: crate::messages::ModifierSource::World {
                                id: "world".to_string(),
                                tag: tag.clone(),
                            },
                            slot: slot.clone(),
                            bonus: *bonus,
                        });
                    }
                    TriggerAction::RemoveModifier { entity, tag, slot } => {
                        let Some(uuid) = name_to_uuid.get(entity) else {
                            bevy::log::warn!(
                                "handle_ai_events: RemoveModifier: unknown entity name '{entity}'"
                            );
                            continue;
                        };
                        let Some(target) = uuid_to_entity.get(uuid).copied() else {
                            bevy::log::warn!(
                                "handle_ai_events: RemoveModifier: no ECS entity with UUID '{uuid}' for name '{entity}'"
                            );
                            continue;
                        };
                        let Ok(mut mods) = ship_modifiers.components.get_mut(target) else {
                            bevy::log::warn!(
                                "handle_ai_events: RemoveModifier: entity '{entity}' has no ShipModifiers component"
                            );
                            continue;
                        };
                        mods.remove(
                            &crate::messages::ModifierSource::World {
                                id: "world".to_string(),
                                tag: tag.clone(),
                            },
                            slot,
                        );
                    }
                    TriggerAction::ApplyFlag { entity, tag, kind } => {
                        let Some(uuid) = name_to_uuid.get(entity) else {
                            bevy::log::warn!(
                                "handle_ai_events: ApplyFlag: unknown entity name '{entity}'"
                            );
                            continue;
                        };
                        let Some(target) = uuid_to_entity.get(uuid).copied() else {
                            bevy::log::warn!(
                                "handle_ai_events: ApplyFlag: no ECS entity with UUID '{uuid}' for name '{entity}'"
                            );
                            continue;
                        };
                        let Ok(mut mods) = ship_modifiers.components.get_mut(target) else {
                            bevy::log::warn!(
                                "handle_ai_events: ApplyFlag: entity '{entity}' has no ShipModifiers component"
                            );
                            continue;
                        };
                        mods.add_flag(
                            crate::messages::ModifierSource::World {
                                id: "world".to_string(),
                                tag: tag.clone(),
                            },
                            kind.clone(),
                        );
                    }
                    TriggerAction::RemoveFlag { entity, tag, kind } => {
                        let Some(uuid) = name_to_uuid.get(entity) else {
                            bevy::log::warn!(
                                "handle_ai_events: RemoveFlag: unknown entity name '{entity}'"
                            );
                            continue;
                        };
                        let Some(target) = uuid_to_entity.get(uuid).copied() else {
                            bevy::log::warn!(
                                "handle_ai_events: RemoveFlag: no ECS entity with UUID '{uuid}' for name '{entity}'"
                            );
                            continue;
                        };
                        let Ok(mut mods) = ship_modifiers.components.get_mut(target) else {
                            bevy::log::warn!(
                                "handle_ai_events: RemoveFlag: entity '{entity}' has no ShipModifiers component"
                            );
                            continue;
                        };
                        mods.remove_flag(
                            crate::messages::ModifierSource::World {
                                id: "world".to_string(),
                                tag: tag.clone(),
                            },
                            kind.clone(),
                        );
                    }
                    TriggerAction::ApplyIntModifier {
                        entity,
                        tag,
                        slot,
                        bonus,
                    } => {
                        let Some(uuid) = name_to_uuid.get(entity) else {
                            bevy::log::warn!(
                                "handle_ai_events: ApplyIntModifier: unknown entity name '{entity}'"
                            );
                            continue;
                        };
                        let Some(target) = uuid_to_entity.get(uuid).copied() else {
                            bevy::log::warn!(
                                "handle_ai_events: ApplyIntModifier: no ECS entity with UUID '{uuid}' for name '{entity}'"
                            );
                            continue;
                        };
                        let Ok(mut mods) = ship_modifiers.components.get_mut(target) else {
                            bevy::log::warn!(
                                "handle_ai_events: ApplyIntModifier: entity '{entity}' has no ShipModifiers component"
                            );
                            continue;
                        };
                        mods.add_or_update_int(crate::modifiers::IntModifier {
                            source: crate::messages::ModifierSource::World {
                                id: "world".to_string(),
                                tag: tag.clone(),
                            },
                            slot: slot.clone(),
                            bonus: *bonus,
                        });
                    }
                    TriggerAction::RemoveIntModifier { entity, tag, slot } => {
                        let Some(uuid) = name_to_uuid.get(entity) else {
                            bevy::log::warn!(
                                "handle_ai_events: RemoveIntModifier: unknown entity name '{entity}'"
                            );
                            continue;
                        };
                        let Some(target) = uuid_to_entity.get(uuid).copied() else {
                            bevy::log::warn!(
                                "handle_ai_events: RemoveIntModifier: no ECS entity with UUID '{uuid}' for name '{entity}'"
                            );
                            continue;
                        };
                        let Ok(mut mods) = ship_modifiers.components.get_mut(target) else {
                            bevy::log::warn!(
                                "handle_ai_events: RemoveIntModifier: entity '{entity}' has no ShipModifiers component"
                            );
                            continue;
                        };
                        mods.remove_int(
                            &crate::messages::ModifierSource::World {
                                id: "world".to_string(),
                                tag: tag.clone(),
                            },
                            slot,
                        );
                    }
                    TriggerAction::GameOver { message } => {
                        let reason = message.clone().unwrap_or_default();
                        if let Some(ref mut gr) = game_over_reason {
                            gr.0 = Some(reason);
                        }
                        if let Some(ref mut ns) = next_state {
                            ns.set(GamePhase::GameOver);
                        }
                    }
                    TriggerAction::LoadWorld { path } => {
                        if let Some(ref mut lc) = pending_layers {
                            // PRD #397 fix 1: record the layer that issued
                            // this LoadWorld so `parent:` from the new
                            // sub-world resolves up to it.
                            lc.0.push(WorldLayerChange::Load {
                                path: path.clone(),
                                loader_path: ft.origin_layer.clone(),
                            });
                        }
                    }
                    TriggerAction::UnloadWorld { path } => {
                        if let Some(ref mut lc) = pending_layers {
                            lc.0.push(WorldLayerChange::Unload(path.clone()));
                        }
                    }
                    TriggerAction::SetWorldFlag { name } => {
                        if let Some((target_layer, stripped, before, after)) = mutate_world_flag(
                            &mut runtime.flags,
                            layer_map.as_deref_mut().map(|lm| &mut lm.0),
                            &ft.origin_layer,
                            name,
                            FlagMutation::Set,
                        ) {
                            emit_flag_transition(
                                &mut next_events,
                                &stripped,
                                &target_layer,
                                before,
                                after,
                            );
                        }
                    }
                    TriggerAction::ClearWorldFlag { name } => {
                        if let Some((target_layer, stripped, before, after)) = mutate_world_flag(
                            &mut runtime.flags,
                            layer_map.as_deref_mut().map(|lm| &mut lm.0),
                            &ft.origin_layer,
                            name,
                            FlagMutation::Clear,
                        ) {
                            emit_flag_transition(
                                &mut next_events,
                                &stripped,
                                &target_layer,
                                before,
                                after,
                            );
                        }
                    }
                    TriggerAction::IncrementWorldFlag { name, by } => {
                        if let Some((target_layer, stripped, before, after)) = mutate_world_flag(
                            &mut runtime.flags,
                            layer_map.as_deref_mut().map(|lm| &mut lm.0),
                            &ft.origin_layer,
                            name,
                            FlagMutation::Increment(*by),
                        ) {
                            emit_flag_transition(
                                &mut next_events,
                                &stripped,
                                &target_layer,
                                before,
                                after,
                            );
                        }
                    }
                    TriggerAction::SetWorldFlagValue { name, value } => {
                        if let Some((target_layer, stripped, before, after)) = mutate_world_flag(
                            &mut runtime.flags,
                            layer_map.as_deref_mut().map(|lm| &mut lm.0),
                            &ft.origin_layer,
                            name,
                            FlagMutation::SetValue(*value),
                        ) {
                            emit_flag_transition(
                                &mut next_events,
                                &stripped,
                                &target_layer,
                                before,
                                after,
                            );
                        }
                    }
                    TriggerAction::SpawnEntity {
                        template_path,
                        name,
                        anchor,
                        position,
                        rotation,
                        scale,
                    } => {
                        // Resolve spawn position. `anchor` looks up in the
                        // origin layer's anchors (or the base world's anchors
                        // for `None` origin). `position` is used directly.
                        let pos_arr: [f32; 3] = if let Some(pos) = position {
                            *pos
                        } else if let Some(anchor_name) = anchor {
                            let lookup = match &ft.origin_layer {
                                Some(layer_path) => layer_map
                                    .as_ref()
                                    .and_then(|lm| lm.0.get(layer_path))
                                    .map(|wr| wr.anchors.get(anchor_name).copied())
                                    .unwrap_or(None),
                                None => base_world_config
                                    .as_ref()
                                    .and_then(|wc| wc.anchors.get(anchor_name).copied()),
                            };
                            match lookup {
                                Some(p) => p,
                                None => {
                                    bevy::log::warn!(
                                        "handle_ai_events: SpawnEntity '{name}' anchor '{anchor_name}' not found"
                                    );
                                    continue;
                                }
                            }
                        } else {
                            bevy::log::warn!(
                                "handle_ai_events: SpawnEntity '{name}' has neither anchor nor position"
                            );
                            continue;
                        };

                        // Resolve template via config cache.
                        let config_cache = crate::config_cache::get_config_cache();
                        let template_inst = crate::world::config::WorldEntity {
                            template_path: template_path.clone(),
                            ..Default::default()
                        };
                        let entity_config = match crate::entity_loader::resolve_entity(
                            &template_inst,
                            &config_cache,
                        ) {
                            Ok(c) => c,
                            Err(e) => {
                                // Native fallback: try reading from disk.
                                #[cfg(not(target_arch = "wasm32"))]
                                {
                                    match std::fs::read_to_string(template_path) {
                                        Ok(toml_str) => {
                                            match crate::entity_config::EntityConfig::from_toml(
                                                &toml_str,
                                            ) {
                                                Ok(c) => c,
                                                Err(err) => {
                                                    bevy::log::warn!(
                                                        "handle_ai_events: SpawnEntity '{name}' template '{template_path}' parse error: {err:?}"
                                                    );
                                                    continue;
                                                }
                                            }
                                        }
                                        Err(_) => {
                                            bevy::log::warn!(
                                                "handle_ai_events: SpawnEntity '{name}' template '{template_path}' not in cache nor on disk: {e}"
                                            );
                                            continue;
                                        }
                                    }
                                }
                                #[cfg(target_arch = "wasm32")]
                                {
                                    bevy::log::warn!(
                                        "handle_ai_events: SpawnEntity '{name}' template '{template_path}' not in cache: {e}"
                                    );
                                    continue;
                                }
                            }
                        };

                        let uuid = crate::entity_loader::assign_uuid();
                        let pos_vec = Vec3::new(pos_arr[0], pos_arr[1], pos_arr[2]);
                        // Patch the entity name with the trigger's `name` field so that the
                        // spawned ECS entity gets EntityName("wave_1") (not the template's
                        // display name like "Harrow Destroyer"). This mirrors what
                        // `spawn_world_entities` does for static `[[entity]]` entries and is
                        // required for `resolve_objective_target` to match Destroy directives
                        // by the scenario name (e.g. `target = "wave_1"`).
                        let mut entity_config = entity_config;
                        if !name.is_empty() {
                            entity_config.name = Some(name.clone());
                        }
                        let spawned = crate::entity_spawner::spawn_entity(
                            &mut commands,
                            &entity_config,
                            pos_vec,
                            uuid.clone(),
                            None,
                        );

                        // Apply optional rotation (XYZ Euler radians) and
                        // scale (per-axis), mirroring `TransformConfig`
                        // semantics from the static `[[entity]]` schema.
                        // `spawn_entity` only set translation; we overwrite
                        // the Transform with translation + rotation + scale
                        // when either is supplied.
                        if rotation.is_some() || scale.is_some() {
                            let [rx, ry, rz] = rotation.unwrap_or([0.0, 0.0, 0.0]);
                            let quat = Quat::from_euler(EulerRot::XYZ, rx, ry, rz);
                            let [sx, sy, sz] = scale.unwrap_or([1.0, 1.0, 1.0]);
                            let scale_vec = Vec3::new(sx, sy, sz);
                            commands.entity(spawned).insert(Transform {
                                translation: pos_vec,
                                rotation: quat,
                                scale: scale_vec,
                            });
                        }

                        // Register name Ã¢â€ â€™ uuid for subsequent triggers.
                        runtime.name_to_uuid.insert(name.clone(), uuid);

                        // Attach to the parent layer's spawned_entities so
                        // `UnloadWorld` despawns the entity (base-world
                        // origin: entity just persists for the session).
                        if let (Some(layer_path), Some(ref mut lm)) =
                            (&ft.origin_layer, &mut layer_map)
                        {
                            if let Some(layer) = lm.0.get_mut(layer_path) {
                                layer.spawned_entities.push(spawned);
                            }
                        }
                    }
                    TriggerAction::DestroyEntity { entity } => {
                        let uuid = match runtime.name_to_uuid.get(entity) {
                            Some(u) => u.clone(),
                            None => {
                                bevy::log::warn!(
                                    "handle_ai_events: DestroyEntity: unknown entity name '{entity}'"
                                );
                                continue;
                            }
                        };
                        // Find the Bevy entity with that UUID.
                        let mut target_entity: Option<Entity> = None;
                        for (ent, uuid_comp) in entity_uuid_query.iter() {
                            if uuid_comp.0 == uuid {
                                target_entity = Some(ent);
                                break;
                            }
                        }
                        // Push into our local pipeline so chained
                        // on_destroyed triggers in the same frame fire.
                        // We deliberately do NOT use
                        // MessageWriter<AiEntityDestroyed> directly here
                        // because this system already holds the matching
                        // reader, which would trigger Bevy's B0002 access
                        // check. Instead, defer the message write via a
                        // command so it runs after this system exits and
                        // external consumers (telemetry, save/load,
                        // achievements) observe script-killed entities the
                        // same as combat-killed ones.
                        next_events.push(WorldEvent::Destroyed { uuid: uuid.clone() });
                        let msg_uuid = uuid.clone();
                        commands.queue(move |world: &mut World| {
                            if let Some(mut msgs) = world
                                .get_resource_mut::<Messages<crate::ai_plugin::AiEntityDestroyed>>()
                            {
                                msgs.write(crate::ai_plugin::AiEntityDestroyed {
                                    entity_uuid: msg_uuid,
                                });
                            }
                        });
                        // Despawn the underlying entity if we found it.
                        if let Some(ent) = target_entity {
                            commands.entity(ent).try_despawn();
                        }
                    }
                    TriggerAction::AddFactionEnemy { faction, enemy } => {
                        let Some(registry) = faction_dispatch.registry.as_deref_mut() else {
                            bevy::log::warn!(
                                "handle_ai_events: AddFactionEnemy skipped: FactionRegistryResource not present"
                            );
                            continue;
                        };
                        let faction_uuid = match registry.0.uuid_by_name(faction) {
                            Some(u) => u,
                            None => {
                                bevy::log::warn!(
                                    "handle_ai_events: AddFactionEnemy: unknown faction name '{faction}'"
                                );
                                continue;
                            }
                        };
                        let enemy_uuid = match registry.0.uuid_by_name(enemy) {
                            Some(u) => u,
                            None => {
                                bevy::log::warn!(
                                    "handle_ai_events: AddFactionEnemy: unknown enemy faction name '{enemy}'"
                                );
                                continue;
                            }
                        };
                        // Idempotent: returns false if `enemy_uuid` is
                        // already listed. Either way no target re-validation
                        // is needed because adding a new hostility cannot
                        // invalidate an existing engagement â€” the next
                        // `enemy_in_range` tick organically picks up the
                        // new relationship.
                        registry.0.add_enemy(faction_uuid, enemy_uuid);
                    }
                    TriggerAction::RemoveFactionEnemy { faction, enemy } => {
                        let Some(registry) = faction_dispatch.registry.as_deref_mut() else {
                            bevy::log::warn!(
                                "handle_ai_events: RemoveFactionEnemy skipped: FactionRegistryResource not present"
                            );
                            continue;
                        };
                        let faction_uuid = match registry.0.uuid_by_name(faction) {
                            Some(u) => u,
                            None => {
                                bevy::log::warn!(
                                    "handle_ai_events: RemoveFactionEnemy: unknown faction name '{faction}'"
                                );
                                continue;
                            }
                        };
                        let enemy_uuid = match registry.0.uuid_by_name(enemy) {
                            Some(u) => u,
                            None => {
                                bevy::log::warn!(
                                    "handle_ai_events: RemoveFactionEnemy: unknown enemy faction name '{enemy}'"
                                );
                                continue;
                            }
                        };
                        let removed = registry.0.remove_enemy(faction_uuid, enemy_uuid);
                        if removed {
                            // Snapshot every AI controller's own faction
                            // BEFORE we take the &mut on the query for
                            // re-validation. `iter()` on a `&mut Query`
                            // yields immutable refs so no borrow conflict
                            // with the subsequent `iter_mut()`.
                            let ai_factions: Vec<(uuid::Uuid, uuid::Uuid)> = ai_query
                                .iter()
                                .filter_map(|(uid, _, fc)| {
                                    let self_uuid = uuid::Uuid::parse_str(&uid.0).ok()?;
                                    fc.map(|fc| (self_uuid, fc.0))
                                })
                                .collect();
                            let uuid_to_faction = build_uuid_to_faction(
                                &faction_dispatch.non_ai_factions,
                                &ai_factions,
                            );
                            revalidate_ai_targets_after_faction_change(
                                &mut ai_query,
                                &registry.0,
                                &uuid_to_faction,
                            );
                        }
                    }
                }
            }
        }

        if next_events.is_empty() {
            break;
        }
        if pass >= max_passes {
            bevy::log::warn!(
                "handle_ai_events: trigger chain exceeded {max_passes} passes; \
                 stopping to prevent infinite loop"
            );
            break;
        }
        current_events = next_events;
    }
}

/// Build a `UUID â†’ faction UUID` map from every entity that carries a
/// `FactionComponent`. Used by `revalidate_ai_targets_after_faction_change`
/// to resolve a controller's `blackboard.target` UUID back to a faction so
/// the new `is_enemy` relationship can be evaluated.
///
/// The two queries cover disjoint sets of entities: `non_ai_factions`
/// holds factioned entities without an `AiControllerComponent` (player
/// ship, stations, factioned beacons) and the AI controllers themselves
/// (which may also carry a faction) are gathered from `ai_factions`.
pub(crate) fn build_uuid_to_faction(
    non_ai_factions: &Query<
        (&EntityUuid, &crate::entities::spawner::FactionComponent),
        Without<AiControllerComponent>,
    >,
    ai_factions: &[(uuid::Uuid, uuid::Uuid)],
) -> std::collections::HashMap<uuid::Uuid, uuid::Uuid> {
    let mut map = std::collections::HashMap::new();
    for (uid, fc) in non_ai_factions.iter() {
        if let Ok(uuid) = uuid::Uuid::parse_str(&uid.0) {
            map.insert(uuid, fc.0);
        }
    }
    for (self_uuid, faction_uuid) in ai_factions {
        map.insert(*self_uuid, *faction_uuid);
    }
    map
}

/// Bundle of system params used by the two trigger-dispatch sites for
/// the `add_faction_enemy` / `remove_faction_enemy` actions. Grouping
/// these keeps both `handle_ai_events` and `handle_respond_to_message`
/// under Bevy's per-system parameter cap (16).
///
/// `registry` is `Option<ResMut<_>>` so test apps that don't insert
/// `FactionRegistryResource` (most of `world::server::tests`) still load
/// the systems without a "resource does not exist" panic. Production
/// Bundles the optional world-layer mutation resources used by
/// `handle_respond_to_message` and `handle_ai_events` into a single
/// `SystemParam` so both functions stay within Bevy's 16-parameter limit.
#[derive(bevy::ecs::system::SystemParam)]
pub struct WorldLayerParams<'w> {
    pub pending_layers: Option<ResMut<'w, PendingWorldLayerChanges>>,
    pub layer_map: Option<ResMut<'w, WorldLayerMap>>,
    pub base_world_config: Option<Res<'w, crate::world::config::WorldConfig>>,
}

/// Bundle of per-entity `ShipModifiers` writers used by
/// `handle_respond_to_message` and `handle_ai_events` to route
/// `TriggerAction::{Apply,Remove}{Modifier,Flag,IntModifier}` actions to
/// the named target entity's Component (not the legacy global Resource).
///
/// Grouping the mutable query into a `SystemParam` keeps both handlers
/// under Bevy's 16-parameter limit. Every ship entity (player + NPC) is
/// spawned with a `ShipModifiers` Component (`src/entities/spawner.rs`
/// and `spawn_game_start_entities`), so `.get_mut(entity)` is the correct
/// primary write target after the name is resolved through
/// `WorldContentRuntime.name_to_uuid` → UUID → ECS `Entity`.
#[derive(bevy::ecs::system::SystemParam)]
pub struct ShipModifiersParams<'w, 's> {
    pub components: Query<'w, 's, &'static mut crate::modifiers::ShipModifiers>,
}

/// builds always insert the registry via `init_world_runtime`, so the
/// `None` branch is a test-only safety net that logs and skips the
/// action.
#[derive(bevy::ecs::system::SystemParam)]
pub struct FactionDispatchParams<'w, 's> {
    pub registry: Option<ResMut<'w, crate::config_cache::FactionRegistryResource>>,
    pub non_ai_factions: Query<
        'w,
        's,
        (
            &'static EntityUuid,
            &'static crate::entities::spawner::FactionComponent,
        ),
        Without<AiControllerComponent>,
    >,
}

/// After a faction relationship is mutated, walk every AI controller and
/// clear `blackboard.target` if the controller's `target` faction is no
/// longer hostile to the controller's own faction.
///
/// Required because `enemy_in_range` only seeds `blackboard.target` â€”
/// once set, the controller's current state (`Pursuing`, `Attacking`,
/// `Fleeing`) keeps engaging the target via the blackboard UUID without
/// re-checking the faction relationship. A scenario that demotes a
/// faction from hostile to neutral via `remove_faction_enemy` would
/// otherwise leave existing engagements stuck on a now-friendly target.
///
/// Controllers with no target, no faction, or a target that has no
/// faction (factionless entities like the starbase or an asteroid) are
/// left untouched.
pub(crate) fn revalidate_ai_targets_after_faction_change(
    ai_query: &mut Query<
        (
            &EntityUuid,
            Option<&mut crate::ai_plugin::ShipAiMemory>,
            Option<&crate::entities::spawner::FactionComponent>,
        ),
        With<AiControllerComponent>,
    >,
    registry: &crate::faction::FactionRegistry,
    uuid_to_faction: &std::collections::HashMap<uuid::Uuid, uuid::Uuid>,
) {
    for (_uid, ai_mem_opt, self_faction_comp) in ai_query.iter_mut() {
        let Some(mut ai_mem) = ai_mem_opt else {
            continue;
        };
        let Some(target_uuid) = ai_mem.0.target else {
            continue;
        };
        let self_faction = self_faction_comp.map(|fc| fc.0);
        let target_faction = uuid_to_faction.get(&target_uuid).copied();
        if !crate::faction::is_enemy(self_faction, target_faction, registry) {
            ai_mem.0.target = None;
        }
    }
}

/// Compare `before`/`after` flag values and push a `FlagSet` or `FlagCleared`
/// event into `events` when the boolean view (`counter != 0`) flips.
///
/// `origin_layer` is the resolved target layer of the mutation (after
/// `parent:` walking) â€” embedded in the emitted event so layer-scoped
/// `on_flag_set` / `on_flag_cleared` triggers only react to transitions
/// in their own layer (PRD #397 fix 1).
pub(crate) fn emit_flag_transition(
    events: &mut Vec<WorldEvent>,
    name: &str,
    origin_layer: &Option<String>,
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
            origin_layer: origin_layer.clone(),
        });
    } else {
        events.push(WorldEvent::FlagCleared {
            name: name.to_string(),
            origin_layer: origin_layer.clone(),
        });
    }
}

/// Specifies the kind of mutation to apply via `mutate_world_flag`.
pub(crate) enum FlagMutation {
    Set,
    Clear,
    Increment(i64),
    SetValue(i64),
}

/// Apply a flag mutation to the correct per-layer `FlagStore`, honouring
/// `parent:` prefixes on `name` (PRD #397 fix 1).
///
/// `origin_layer` is the trigger's authoring layer (`None` = base world).
/// Each `parent:` prefix walks one step up the loader chain (via the
/// layer's `loader_path`). Walking past the base world is a no-op + warn
/// to avoid two scenarios polluting each other's flag namespace.
///
/// On success returns `(resolved_target_layer, stripped_name, before, after)`.
/// Returns `None` when the walk overruns the loader chain.
pub(crate) fn mutate_world_flag(
    base_flags: &mut crate::world::flags::FlagStore,
    layer_map: Option<&mut HashMap<String, WorldRuntime>>,
    origin_layer: &Option<String>,
    name: &str,
    mutation: FlagMutation,
) -> Option<(Option<String>, String, i64, i64)> {
    // Walk `parent:` prefixes to determine the target layer.
    let mut depth = 0usize;
    let mut rest = name;
    while let Some(s) = rest.strip_prefix("parent:") {
        depth += 1;
        rest = s;
    }
    let stripped = rest.to_string();

    // Determine the target layer by walking `depth` steps from `origin_layer`.
    // `None` at any step means "base world".
    let mut cur = origin_layer.clone();
    for _ in 0..depth {
        match cur {
            None => {
                bevy::log::warn!(
                    "mutate_world_flag: '{name}' from origin {origin_layer:?} walks past base world â€” ignoring"
                );
                return None;
            }
            Some(ref path) => {
                cur = layer_map
                    .as_deref()
                    .and_then(|lm| lm.get(path))
                    .and_then(|wr| wr.loader_path.clone());
                // If the layer entry is missing entirely treat as base.
            }
        }
    }
    let target_layer = cur;

    // Look up the target store and apply the mutation.
    let store: &mut crate::world::flags::FlagStore = match &target_layer {
        None => base_flags,
        Some(path) => match layer_map.and_then(|lm| lm.get_mut(path)) {
            Some(wr) => &mut wr.flags,
            None => {
                bevy::log::warn!(
                    "mutate_world_flag: target layer '{path}' missing from WorldLayerMap â€” ignoring '{name}'"
                );
                return None;
            }
        },
    };
    let (before, after) = match mutation {
        FlagMutation::Set => store.set_flag(&stripped),
        FlagMutation::Clear => store.clear_flag(&stripped),
        FlagMutation::Increment(by) => store.increment_flag(&stripped, by),
        FlagMutation::SetValue(v) => store.set_flag_value(&stripped, v),
    };
    Some((target_layer, stripped, before, after))
}

use crate::ai_plugin::AiControllerComponent;
#[cfg(test)]
use crate::entity_spawner::BehaviourSection;
use crate::entity_spawner::EntityUuid;

// Ã¢â€â‚¬Ã¢â€â‚¬ Pending scenario load system Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬

/// Bevy system: drain `PendingScenarioLoad` and merge each world TOML into the
/// live `WorldContentRuntime` (trigger states + comms templates + contacts).
///
/// On WASM the TOML string is not available at runtime (JS pre-fetches only the
/// initial world), so we push paths into the WASM-side pending-world queue and
/// the implementation returns early until the JS bridge delivers the TOML via
/// `wasm_push_world_toml`. On native targets `std::fs::read_to_string` is used.
fn apply_pending_scenario_loads(
    mut pending: ResMut<PendingScenarioLoad>,
    mut runtime: ResMut<WorldContentRuntime>,
) {
    if pending.0.is_empty() {
        return;
    }

    let paths: Vec<String> = pending.0.drain(..).collect();

    for path in paths {
        // De-duplicate: skip paths already merged.
        if runtime.loaded_scenario_paths.contains(&path) {
            continue;
        }

        let toml_str_opt = load_scenario_toml(&path);
        match toml_str_opt {
            None => {
                // WASM: TOML not yet available; re-queue for the next frame.
                pending.0.push(path);
            }
            Some(toml_str) => {
                match crate::world::config::parse_world(&toml_str) {
                    Err(e) => {
                        bevy::log::error!(
                            "apply_pending_scenario_loads: failed to parse {}: {}",
                            path,
                            e
                        );
                        runtime.loaded_scenario_paths.insert(path);
                    }
                    Ok(scenario_config) => {
                        // Merge trigger states (don't overwrite existing ones).
                        let new_triggers = trigger_states_from_world(&scenario_config);
                        runtime.trigger_states.extend(new_triggers);

                        // Merge comms template states.
                        let new_comms = comms_template_states_from_world(&scenario_config);
                        runtime.comms_template_states.extend(new_comms);

                        // Merge contacts (skip duplicates by uuid).
                        for tmpl in &scenario_config.comms {
                            let uuid = match runtime.name_to_uuid.get(&tmpl.from) {
                                Some(u) => u.clone(),
                                None => continue,
                            };
                            if !runtime
                                .contacts
                                .iter()
                                .any(|c: &crate::messages::CommsContact| c.uuid == uuid)
                            {
                                runtime.contacts.push(crate::messages::CommsContact {
                                    uuid,
                                    name: tmpl.from.clone(),
                                    in_range: true,
                                    is_urgent: false,
                                });
                            }
                        }

                        runtime.needs_broadcast = true;
                        runtime.loaded_scenario_paths.insert(path);
                    }
                }
            }
        }
    }
}

// Ã¢â€â‚¬Ã¢â€â‚¬ World layer system (LoadWorld / UnloadWorld) Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬

/// Build a `ConfigCache` suitable for spawning entities from a world layer.
///
/// On WASM the global config cache (pre-loaded by the JS bridge) is returned
/// unchanged.  On native the global cache is always empty (no WASM pre-load
/// step), so we fall back to reading each template file from disk so that
/// `spawn_immediate_entities_internal` can resolve them.
fn build_layer_config_cache(
    _world_config: &crate::world::config::WorldConfig,
) -> crate::config_cache::ConfigCache {
    #[cfg(not(target_arch = "wasm32"))]
    {
        use crate::entity_config::EntityConfig;
        let mut cache = crate::config_cache::get_config_cache();
        for entity in &_world_config.entities {
            if cache.contains_key(&entity.template_path) {
                continue;
            }
            match std::fs::read_to_string(&entity.template_path) {
                Ok(toml_str) => {
                    if let Ok(cfg) = EntityConfig::from_toml(&toml_str) {
                        cache.insert(entity.template_path.clone(), cfg);
                    } else {
                        bevy::log::warn!(
                            "build_layer_config_cache: failed to parse '{}' Ã¢â‚¬â€ entity will be skipped",
                            entity.template_path
                        );
                    }
                }
                Err(_) => {
                    // Template not on disk (e.g. test fixture); skip silently.
                    // spawn_immediate_entities_internal logs and continues for missing templates.
                }
            }
        }
        cache
    }

    #[cfg(target_arch = "wasm32")]
    {
        crate::config_cache::get_config_cache()
    }
}

/// Bevy system: drain `PendingWorldLayerChanges` and apply each `LoadWorld` or
/// `UnloadWorld` command to `WorldLayerMap` and `WorldContentRuntime`.
///
/// `LoadWorld` parses the TOML, merges triggers/comms into the live runtime, and
/// stores a `WorldRuntime` snapshot keyed by path so `UnloadWorld` can reverse it.
///
/// `UnloadWorld` removes the stored snapshot and retains only triggers/comms
/// states that do not belong to the unloaded world (matched by pointer equality
/// of the underlying `Trigger`/`CommsTemplate` clone identity Ã¢â‚¬â€ we use indices
/// tracked in the snapshot length at load time).
fn apply_world_layer_changes(
    mut commands: Commands,
    mut pending: ResMut<PendingWorldLayerChanges>,
    mut layer_map: ResMut<WorldLayerMap>,
    mut runtime: ResMut<WorldContentRuntime>,
) {
    if pending.0.is_empty() {
        return;
    }

    let changes: Vec<WorldLayerChange> = pending.0.drain(..).collect();

    for change in changes {
        match change {
            WorldLayerChange::Load { path, loader_path } => {
                if layer_map.0.contains_key(&path) {
                    // Already loaded â€” de-duplicate, no-op.
                    continue;
                }
                let toml_str_opt = load_scenario_toml(&path);
                match toml_str_opt {
                    None => {
                        // WASM: re-queue until the fetch completes.
                        pending.0.push(WorldLayerChange::Load { path, loader_path });
                    }
                    Some(toml_str) => {
                        match crate::world::config::parse_world(&toml_str) {
                            Err(e) => {
                                bevy::log::error!(
                                    "apply_world_layer_changes: failed to parse {path}: {e}"
                                );
                                // Insert an empty entry so we don't retry a broken file.
                                layer_map.0.insert(path, WorldRuntime::default());
                            }
                            Ok(mut scenario_config) => {
                                let mut trigger_states =
                                    trigger_states_from_world(&scenario_config);
                                // Tag every trigger state from this layer with
                                // its origin path so `spawn_entity` actions can
                                // attach the new entity to the right
                                // `WorldLayerMap` entry (issue #417).
                                for ts in trigger_states.iter_mut() {
                                    ts.origin_layer = Some(path.clone());
                                }
                                let comms_template_states =
                                    comms_template_states_from_world(&scenario_config);

                                // Merge into live runtime.
                                runtime.trigger_states.extend(trigger_states.clone());
                                runtime
                                    .comms_template_states
                                    .extend(comms_template_states.clone());

                                // Assign UUIDs to named entities in this layer's config
                                // and register them in the live runtime's name_to_uuid map.
                                let new_names = crate::world::config::assign_named_entity_uuids(
                                    &scenario_config.entities,
                                    crate::entity_loader::assign_uuid,
                                );
                                for (name, uuid) in &new_names {
                                    scenario_config
                                        .name_to_uuid
                                        .insert(name.clone(), uuid.clone());
                                    runtime.name_to_uuid.insert(name.clone(), uuid.clone());
                                }

                                // Spawn the layer's [[entity]] blocks into the ECS.
                                // On native the global config cache is always empty (no WASM
                                // pre-load step), so we build a local cache by reading each
                                // referenced template from disk.  WASM uses the pre-loaded
                                // global cache as normal.
                                let config_cache = build_layer_config_cache(&scenario_config);
                                let spawned_entities = spawn_immediate_entities_internal(
                                    &mut commands,
                                    &scenario_config,
                                    &config_cache,
                                );

                                // Merge contacts (skip duplicates by uuid).
                                for tmpl in &scenario_config.comms {
                                    let uuid = match runtime.name_to_uuid.get(&tmpl.from) {
                                        Some(u) => u.clone(),
                                        None => continue,
                                    };
                                    if !runtime
                                        .contacts
                                        .iter()
                                        .any(|c: &crate::messages::CommsContact| c.uuid == uuid)
                                    {
                                        runtime.contacts.push(crate::messages::CommsContact {
                                            uuid,
                                            name: tmpl.from.clone(),
                                            in_range: true,
                                            is_urgent: false,
                                        });
                                    }
                                }

                                runtime.needs_broadcast = true;

                                // Issue #415: emit a WorldLoaded event so
                                // `on_world_loaded` triggers declared inside
                                // this sub-world (and merged into the live
                                // runtime above) fire on the next Update
                                // tick via `handle_ai_events`.
                                runtime.pending_world_events.push(WorldEvent::WorldLoaded);

                                layer_map.0.insert(
                                    path,
                                    WorldRuntime {
                                        trigger_states,
                                        comms_template_states,
                                        spawned_entities,
                                        anchors: scenario_config.anchors.clone(),
                                        flags: crate::world::flags::FlagStore::new(),
                                        loader_path,
                                    },
                                );
                            }
                        }
                    }
                }
            }
            WorldLayerChange::Unload(path) => {
                let Some(layer) = layer_map.0.remove(&path) else {
                    continue; // Not loaded Ã¢â‚¬â€ no-op.
                };

                // Despawn ECS entities that were spawned when this layer loaded.
                // Use try_despawn: entities may have already died (e.g. hull = 0) before the layer unloads.
                for entity in &layer.spawned_entities {
                    commands.entity(*entity).try_despawn();
                }

                // Remove trigger states belonging to this layer.
                // We identify them by the condition+actions equality of the stored snapshot.
                let removed_triggers: std::collections::HashSet<usize> = layer
                    .trigger_states
                    .iter()
                    .filter_map(|ls| {
                        runtime
                            .trigger_states
                            .iter()
                            .position(|rs| rs.trigger == ls.trigger)
                    })
                    .collect();
                let mut ti = 0usize;
                runtime.trigger_states.retain(|_| {
                    let keep = !removed_triggers.contains(&ti);
                    ti += 1;
                    keep
                });

                // Remove comms template states belonging to this layer.
                let removed_comms: std::collections::HashSet<usize> = layer
                    .comms_template_states
                    .iter()
                    .filter_map(|ls| {
                        runtime
                            .comms_template_states
                            .iter()
                            .position(|rs| rs.template == ls.template)
                    })
                    .collect();
                let mut ci = 0usize;
                runtime.comms_template_states.retain(|_| {
                    let keep = !removed_comms.contains(&ci);
                    ci += 1;
                    keep
                });

                runtime.needs_broadcast = true;
            }
        }
    }
}

/// Load a world TOML string for the given path.
///
/// - **Native**: uses `std::fs::read_to_string` (for tests and dev builds).
/// - **WASM**: checks the pending world TOML queue populated by JS via
///   `wasm_push_world_toml`; returns `None` if the fetch is not yet complete.
fn load_scenario_toml(path: &str) -> Option<String> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::fs::read_to_string(path).ok()
    }
    #[cfg(target_arch = "wasm32")]
    {
        crate::config_cache::pop_pending_world_toml(path).or_else(|| {
            // Fire a JS fetch request if we haven't already.
            crate::config_cache::request_world_fetch(path.to_string());
            None
        })
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::ai_plugin::{AiEntityAttacked, AiEntityDestroyed};
    use crate::lobby::{InboundMessage, LobbyPlugin, OutboundMessage, WorldResource};
    use crate::messages::*;
    use crate::world::content::{
        CommsDialogueNode, CommsResponse, CommsTemplateState, TriggerCondition,
    };

    // -- Test app -------------------------------------------------------------

    #[derive(Resource, Default)]
    pub(crate) struct Outbox(pub(crate) Vec<OutboundMessage>);

    fn collect(mut reader: MessageReader<OutboundMessage>, mut box_: ResMut<Outbox>) {
        for m in reader.read() {
            box_.0.push(m.clone());
        }
    }

    pub(crate) fn comms_test_app() -> App {
        let mut app = App::new();
        app.add_plugins(LobbyPlugin)
            .add_plugins(bevy::time::TimePlugin)
            .add_plugins(crate::server_app::AdmissionPlugin)
            .init_resource::<WorldContentRuntime>()
            .init_resource::<CommsInboxRes>()
            .init_resource::<ObjectiveManagerRes>()
            .init_resource::<SimOutbox>()
            .init_resource::<Outbox>()
            .add_message::<CommsChannel2Event>()
            .add_systems(
                Update,
                (
                    handle_hail,
                    handle_respond_to_message,
                    handle_clear_comms,
                    tick_pending_follow_ups,
                    handle_comms_channel2,
                    update_comms_range_flags,
                    broadcast_comms_state,
                    broadcast_objective_summary,
                )
                    .chain()
                    .after(crate::server_app::AdmissionSet),
            )
            .add_systems(PostUpdate, collect);
        app.world_mut().spawn((
            crate::simulation::Ship,
            crate::simulation::LocalShip,
            crate::ship_plugin::ShipConfigComponent::default(),
            crate::ship_plugin::ShipSystemControlSources::default(),
            crate::ship_plugin::ActiveStationRatings::default(),
            crate::ship_plugin::CoordinationQueue::default(),
            crate::messages::AdmittedCommands::default(),
        ));
        app
    }

    pub(crate) fn push_msg(app: &mut App, token: &str, msg: ClientMessage) {
        app.world_mut()
            .resource_mut::<Messages<InboundMessage>>()
            .write(InboundMessage {
                token: token.into(),
                msg,
            });
    }

    pub(crate) fn tick(app: &mut App) -> Vec<OutboundMessage> {
        app.update();
        let sim_entries = std::mem::take(&mut app.world_mut().resource_mut::<SimOutbox>().0);
        let mut msgs = app.world().resource::<Outbox>().0.clone();
        for (target, msg) in sim_entries {
            msgs.push(OutboundMessage {
                target,
                msg,
                delivery: crate::messages::DeliveryClass::Reliable,
            });
        }
        app.world_mut().resource_mut::<Outbox>().0.clear();
        msgs
    }

    /// Set up a game in InProgress phase with a comms player and captain.
    pub(crate) fn setup_game_with_comms(app: &mut App, station_uuid: &str) {
        // Register captain
        push_msg(
            app,
            "captain",
            ClientMessage::Identify {
                token: "captain".into(),
                name: "Alice".into(),
            },
        );
        tick(app);
        push_msg(
            app,
            "captain",
            ClientMessage::SelectStation {
                station: "Captain".into(),
            },
        );
        tick(app);
        // Register comms
        push_msg(
            app,
            "comms",
            ClientMessage::Identify {
                token: "comms".into(),
                name: "Uhura".into(),
            },
        );
        tick(app);
        push_msg(
            app,
            "comms",
            ClientMessage::SelectStation {
                station: "Comms".into(),
            },
        );
        tick(app);
        // Start game
        push_msg(app, "captain", ClientMessage::SetReady { ready: true });
        push_msg(app, "comms", ClientMessage::SetReady { ready: true });
        tick(app);

        // Manually install a comms template into the runtime so tests are
        // independent of TOML loading.
        let runtime = &mut app.world_mut().resource_mut::<WorldContentRuntime>();
        runtime
            .name_to_uuid
            .insert("starbase_alpha".into(), station_uuid.into());
        runtime.contacts.push(CommsContact {
            uuid: station_uuid.into(),
            name: "Starbase Alpha".into(),
            in_range: true,
            is_urgent: false,
        });
        runtime.comms_template_states.push(CommsTemplateState {
            template: crate::world::content::CommsTemplate {
                from: "starbase_alpha".into(),
                trigger: TriggerCondition::OnHailed {
                    entity_name: "starbase_alpha".into(),
                },
                node: CommsDialogueNode {
                    body: "USS Phoenix, please identify yourself.".into(),
                    responses: vec![CommsResponse {
                        text: "We are on a survey mission.".into(),
                        actions: vec![TriggerAction::AddObjective {
                            id: "obj-survey".into(),
                            text: "Complete the survey".into(),
                            mandatory: true,
                            targets: vec![],
                            directive: crate::messages::AiDirective::None,
                            utility: crate::objectives::UtilityConfig::default(),
                            source: crate::messages::ObjectiveSource::default(),
                        }],
                        follow_up: None,
                    }],
                    speaker: None,
                    trigger: None,
                },
                thread_id: None,
                urgent: false,
                root_follow_up: None,
            },
            fired: false,
        });
        runtime.needs_broadcast = true;
    }

    #[test]
    fn root_comms_template_with_on_timer_trigger_waits_silently() {
        use crate::world::content::{CommsDialogueNode, CommsTemplate, CommsTemplateState};

        let mut app = ai_trigger_test_app();

        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime.name_to_uuid.insert(
                "Research Outpost".to_string(),
                "research-outpost-uuid".to_string(),
            );
            runtime.comms_template_states = vec![CommsTemplateState {
                template: CommsTemplate {
                    from: "Research Outpost".to_string(),
                    trigger: TriggerCondition::OnTimer { after_secs: 3.0 },
                    node: CommsDialogueNode {
                        body: "Ardent, this is Dr. Myst.".to_string(),
                        responses: vec![],
                        speaker: Some("Dr. Myst".to_string()),
                        trigger: None,
                    },
                    thread_id: Some("research-scholar".to_string()),
                    urgent: true,
                    root_follow_up: None,
                },
                fired: false,
            }];
            // Simulate that the world has been alive for less than the
            // template's `after_secs` â€” no TimerElapsed event yet.
            runtime
                .pending_world_events
                .push(WorldEvent::TimerElapsed { elapsed_secs: 1.0 });
        }

        app.update();

        {
            let messages = app.world().resource::<CommsInboxRes>().0.messages();
            assert!(
                messages.is_empty(),
                "on_timer root comms must stay silent until the timer fires"
            );
            let runtime = app.world().resource::<WorldContentRuntime>();
            assert!(
                runtime.pending_follow_ups.is_empty(),
                "root templates do not queue onto pending_follow_ups"
            );
        }

        // Push a TimerElapsed event past the threshold; template fires now.
        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime
                .pending_world_events
                .push(WorldEvent::TimerElapsed { elapsed_secs: 3.5 });
        }
        app.update();

        let messages = app.world().resource::<CommsInboxRes>().0.messages();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].sender_name, "Dr. Myst");
        assert_eq!(messages[0].body, "Ardent, this is Dr. Myst.");
        assert_eq!(messages[0].thread_id, "research-scholar");
        assert!(messages[0].is_urgent);
    }

    /// `handle_ai_events` (auto-fire path: `on_world_loaded`,
    /// `on_attacked`, `on_destroyed`, `on_flag_set`) also schedules the
    /// chained `root_follow_up`. Verified by emitting `WorldLoaded` on a
    /// template with a chained node.
    #[test]
    fn root_follow_up_fires_for_auto_triggered_template() {
        use crate::world::content::{CommsDialogueNode, CommsTemplate, CommsTemplateState};

        let mut app = ai_trigger_test_app();

        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime.name_to_uuid.insert(
                "Research Outpost".to_string(),
                "research-outpost-uuid".to_string(),
            );
            runtime.comms_template_states = vec![CommsTemplateState {
                template: CommsTemplate {
                    from: "Research Outpost".to_string(),
                    trigger: TriggerCondition::OnWorldLoaded,
                    node: CommsDialogueNode {
                        body: "Stand by.".to_string(),
                        responses: vec![],
                        speaker: None,
                        trigger: None,
                    },
                    thread_id: Some("research-scholar".to_string()),
                    urgent: false,
                    root_follow_up: Some(CommsDialogueNode {
                        body: "Captain. Dr. Myst speaking.".to_string(),
                        responses: vec![],
                        speaker: Some("Dr. Myst".to_string()),
                        trigger: Some(TriggerCondition::OnTimer { after_secs: 2.0 }),
                    }),
                },
                fired: false,
            }];
            runtime.pending_world_events.push(WorldEvent::WorldLoaded);
        }

        app.update();

        // Root injected; chained follow-up queued.
        {
            let messages = app.world().resource::<CommsInboxRes>().0.messages();
            assert_eq!(messages.len(), 1);
            assert_eq!(messages[0].body, "Stand by.");
            let runtime = app.world().resource::<WorldContentRuntime>();
            assert_eq!(runtime.pending_follow_ups.len(), 1);
        }

        // Trip the queue-relative timer and tick.
        app.world_mut()
            .resource_mut::<WorldContentRuntime>()
            .pending_follow_ups[0]
            .elapsed_secs = 5.0;
        app.update();

        let messages = app.world().resource::<CommsInboxRes>().0.messages();
        assert_eq!(messages.len(), 2);
        let chained = &messages[1];
        assert_eq!(chained.sender_name, "Dr. Myst");
        assert_eq!(chained.body, "Captain. Dr. Myst speaking.");
        assert_eq!(chained.thread_id, "research-scholar");
    }

    // -- AI-event trigger tests -----------------------------------------------

    /// Build a minimal test app that includes just what handle_ai_events needs.
    fn ai_trigger_test_app() -> App {
        let mut app = App::new();
        app.add_plugins(LobbyPlugin)
            .add_plugins(bevy::time::TimePlugin)
            .add_plugins(crate::ai_plugin::AiPlugin)
            .insert_resource(crate::config_cache::FactionRegistryResource(
                crate::config_cache::get_faction_registry(),
            ))
            .init_resource::<WorldContentRuntime>()
            .init_resource::<CommsInboxRes>()
            .init_resource::<ObjectiveManagerRes>()
            .init_resource::<SimOutbox>()
            .add_message::<CommsChannel2Event>()
            .add_systems(
                Update,
                (
                    tick_pending_follow_ups,
                    handle_ai_events,
                    handle_comms_channel2,
                )
                    .chain(),
            );
        // Set phase to InProgress
        app.world_mut()
            .insert_resource(State::new(GamePhase::InProgress));
        app
    }

    #[test]
    fn on_entity_destroyed_trigger_fires_add_objective_action() {
        let mut app = ai_trigger_test_app();

        let npc_uuid = "dead-npc-uuid-001";
        let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
        runtime
            .name_to_uuid
            .insert("station_alpha".to_string(), npc_uuid.to_string());
        runtime.trigger_states = vec![TriggerState {
            trigger: crate::world::content::Trigger {
                condition: TriggerCondition::OnDestroyed {
                    entity_name: "station_alpha".to_string(),
                },
                actions: vec![TriggerAction::AddObjective {
                    id: "obj-001".to_string(),
                    text: "Station destroyed".to_string(),
                    mandatory: false,
                    targets: vec![],
                    directive: crate::messages::AiDirective::None,
                    utility: crate::objectives::UtilityConfig::default(),
                    source: crate::messages::ObjectiveSource::default(),
                }],
                when: None,
            },
            fired: false,
            origin_layer: None,
            seen_destroyed: HashSet::new(),
        }];

        // Emit the AiEntityDestroyed message.
        app.world_mut()
            .resource_mut::<Messages<AiEntityDestroyed>>()
            .write(AiEntityDestroyed {
                entity_uuid: npc_uuid.to_string(),
            });

        app.update();

        let objectives = &app.world().resource::<ObjectiveManagerRes>().0;
        assert!(
            objectives
                .sorted_snapshots()
                .iter()
                .any(|o| o.id == "obj-001"),
            "AddObjective action must have fired"
        );
    }

    // -- AI-event ApplyModifier / per-entity target regression tests -------
    //
    // The following six tests exercise `handle_ai_events` dispatch of the
    // per-entity trigger actions (`ApplyModifier`, `RemoveModifier`,
    // `ApplyFlag`, `RemoveFlag`, `ApplyIntModifier`, `RemoveIntModifier`)
    // and prove that the action lands on the target entity's per-entity
    // `ShipModifiers` Component — not the legacy global Resource — and
    // that non-target entities (e.g. the player ship) remain unaffected.
    // These are the regression tests for the audit-report bug where world
    // triggers silently misrouted every named-entity write to whichever
    // ship happened to own the global Resource.

    /// Spawns two entities (an NPC and a "player" ship) with distinct
    /// UUIDs and per-entity `ShipModifiers` components. Registers name
    /// mappings for both. Returns `(npc_entity, player_entity)`.
    fn spawn_two_modifier_targets(app: &mut App) -> (Entity, Entity) {
        let npc = app
            .world_mut()
            .spawn((
                EntityUuid("npc-target-uuid".to_string()),
                crate::modifiers::ShipModifiers::new(),
            ))
            .id();
        let player = app
            .world_mut()
            .spawn((
                EntityUuid("player-target-uuid".to_string()),
                crate::modifiers::ShipModifiers::new(),
            ))
            .id();
        let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
        runtime
            .name_to_uuid
            .insert("raider_alpha".into(), "npc-target-uuid".into());
        runtime
            .name_to_uuid
            .insert("player_ship".into(), "player-target-uuid".into());
        (npc, player)
    }

    /// Installs a single `OnDestroyed { raider_alpha }` trigger whose
    /// action list is `actions`, emits `AiEntityDestroyed { npc-target-uuid }`,
    /// and ticks once so `handle_ai_events` dispatches the actions.
    fn fire_ai_event_trigger(app: &mut App, actions: Vec<TriggerAction>) {
        let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
        runtime.trigger_states = vec![TriggerState {
            trigger: crate::world::content::Trigger {
                condition: TriggerCondition::OnDestroyed {
                    entity_name: "raider_alpha".to_string(),
                },
                actions,
                when: None,
            },
            fired: false,
            origin_layer: None,
            seen_destroyed: HashSet::new(),
        }];
        app.world_mut()
            .resource_mut::<Messages<AiEntityDestroyed>>()
            .write(AiEntityDestroyed {
                entity_uuid: "npc-target-uuid".to_string(),
            });
        app.update();
    }

    #[test]
    fn ai_events_apply_modifier_lands_on_target_entity_not_player() {
        let mut app = ai_trigger_test_app();
        let (npc, player) = spawn_two_modifier_targets(&mut app);
        fire_ai_event_trigger(
            &mut app,
            vec![TriggerAction::ApplyModifier {
                entity: "raider_alpha".into(),
                tag: "boost".into(),
                slot: crate::messages::ModifierSlot::MaxSpeed,
                bonus: 1.5,
            }],
        );
        let npc_mods = app
            .world()
            .entity(npc)
            .get::<crate::modifiers::ShipModifiers>()
            .expect("NPC entity must carry ShipModifiers");
        let player_mods = app
            .world()
            .entity(player)
            .get::<crate::modifiers::ShipModifiers>()
            .expect("player entity must carry ShipModifiers");
        assert!(
            npc_mods.get(&crate::messages::ModifierSlot::MaxSpeed) > 1.0,
            "ApplyModifier must land on the target NPC's per-entity component; got {}",
            npc_mods.get(&crate::messages::ModifierSlot::MaxSpeed)
        );
        assert!(
            (player_mods.get(&crate::messages::ModifierSlot::MaxSpeed) - 1.0).abs() < 1e-3,
            "player entity must be unaffected by an NPC-targeted ApplyModifier; got {}",
            player_mods.get(&crate::messages::ModifierSlot::MaxSpeed)
        );
    }

    #[test]
    fn ai_events_remove_modifier_undoes_only_the_target_entity() {
        let mut app = ai_trigger_test_app();
        let (npc, _player) = spawn_two_modifier_targets(&mut app);
        fire_ai_event_trigger(
            &mut app,
            vec![
                TriggerAction::ApplyModifier {
                    entity: "raider_alpha".into(),
                    tag: "boost".into(),
                    slot: crate::messages::ModifierSlot::MaxSpeed,
                    bonus: 2.0,
                },
                TriggerAction::RemoveModifier {
                    entity: "raider_alpha".into(),
                    tag: "boost".into(),
                    slot: crate::messages::ModifierSlot::MaxSpeed,
                },
            ],
        );
        let npc_mods = app
            .world()
            .entity(npc)
            .get::<crate::modifiers::ShipModifiers>()
            .expect("NPC entity must carry ShipModifiers");
        let value = npc_mods.get(&crate::messages::ModifierSlot::MaxSpeed);
        assert!(
            (value - 1.0).abs() < 1e-3,
            "RemoveModifier must reverse the previously-applied bonus on the NPC's component; expected 1.0, got {value}"
        );
    }

    #[test]
    fn ai_events_apply_flag_lands_on_target_entity_not_player() {
        let mut app = ai_trigger_test_app();
        let (npc, player) = spawn_two_modifier_targets(&mut app);
        fire_ai_event_trigger(
            &mut app,
            vec![TriggerAction::ApplyFlag {
                entity: "raider_alpha".into(),
                tag: "jammer".into(),
                kind: crate::flag_kind::FlagKind::CommsJammed,
            }],
        );
        let npc_mods = app
            .world()
            .entity(npc)
            .get::<crate::modifiers::ShipModifiers>()
            .unwrap();
        let player_mods = app
            .world()
            .entity(player)
            .get::<crate::modifiers::ShipModifiers>()
            .unwrap();
        assert!(
            npc_mods.has_flag(&crate::flag_kind::FlagKind::CommsJammed),
            "ApplyFlag must register on the target NPC's per-entity component"
        );
        assert!(
            !player_mods.has_flag(&crate::flag_kind::FlagKind::CommsJammed),
            "player entity must be unaffected by an NPC-targeted ApplyFlag"
        );
    }

    #[test]
    fn ai_events_remove_flag_undoes_only_the_target_entity() {
        let mut app = ai_trigger_test_app();
        let (npc, _player) = spawn_two_modifier_targets(&mut app);
        fire_ai_event_trigger(
            &mut app,
            vec![
                TriggerAction::ApplyFlag {
                    entity: "raider_alpha".into(),
                    tag: "jammer".into(),
                    kind: crate::flag_kind::FlagKind::CommsJammed,
                },
                TriggerAction::RemoveFlag {
                    entity: "raider_alpha".into(),
                    tag: "jammer".into(),
                    kind: crate::flag_kind::FlagKind::CommsJammed,
                },
            ],
        );
        let npc_mods = app
            .world()
            .entity(npc)
            .get::<crate::modifiers::ShipModifiers>()
            .unwrap();
        assert!(
            !npc_mods.has_flag(&crate::flag_kind::FlagKind::CommsJammed),
            "RemoveFlag must un-register the flag on the NPC's per-entity component"
        );
    }

    #[test]
    fn ai_events_apply_int_modifier_lands_on_target_entity_not_player() {
        let mut app = ai_trigger_test_app();
        let (npc, player) = spawn_two_modifier_targets(&mut app);
        fire_ai_event_trigger(
            &mut app,
            vec![TriggerAction::ApplyIntModifier {
                entity: "raider_alpha".into(),
                tag: "extra_team".into(),
                slot: crate::modifiers::IntModifierSlot::RepairTeams,
                bonus: 2,
            }],
        );
        let npc_mods = app
            .world()
            .entity(npc)
            .get::<crate::modifiers::ShipModifiers>()
            .unwrap();
        let player_mods = app
            .world()
            .entity(player)
            .get::<crate::modifiers::ShipModifiers>()
            .unwrap();
        assert_eq!(
            npc_mods.get_int(&crate::modifiers::IntModifierSlot::RepairTeams),
            2,
            "ApplyIntModifier must land on the target NPC's per-entity component"
        );
        assert_eq!(
            player_mods.get_int(&crate::modifiers::IntModifierSlot::RepairTeams),
            0,
            "player entity must be unaffected by an NPC-targeted ApplyIntModifier"
        );
    }

    #[test]
    fn ai_events_remove_int_modifier_undoes_only_the_target_entity() {
        let mut app = ai_trigger_test_app();
        let (npc, _player) = spawn_two_modifier_targets(&mut app);
        fire_ai_event_trigger(
            &mut app,
            vec![
                TriggerAction::ApplyIntModifier {
                    entity: "raider_alpha".into(),
                    tag: "extra_team".into(),
                    slot: crate::modifiers::IntModifierSlot::RepairTeams,
                    bonus: 3,
                },
                TriggerAction::RemoveIntModifier {
                    entity: "raider_alpha".into(),
                    tag: "extra_team".into(),
                    slot: crate::modifiers::IntModifierSlot::RepairTeams,
                },
            ],
        );
        let npc_mods = app
            .world()
            .entity(npc)
            .get::<crate::modifiers::ShipModifiers>()
            .unwrap();
        assert_eq!(
            npc_mods.get_int(&crate::modifiers::IntModifierSlot::RepairTeams),
            0,
            "RemoveIntModifier must reverse the int modifier on the NPC's per-entity component"
        );
    }

    #[test]
    fn ai_events_apply_modifier_unknown_entity_name_is_ignored() {
        let mut app = ai_trigger_test_app();
        let (npc, player) = spawn_two_modifier_targets(&mut app);
        fire_ai_event_trigger(
            &mut app,
            vec![TriggerAction::ApplyModifier {
                entity: "does_not_exist".into(),
                tag: "boost".into(),
                slot: crate::messages::ModifierSlot::MaxSpeed,
                bonus: 5.0,
            }],
        );
        let npc_mods = app
            .world()
            .entity(npc)
            .get::<crate::modifiers::ShipModifiers>()
            .unwrap();
        let player_mods = app
            .world()
            .entity(player)
            .get::<crate::modifiers::ShipModifiers>()
            .unwrap();
        assert!(
            (npc_mods.get(&crate::messages::ModifierSlot::MaxSpeed) - 1.0).abs() < 1e-3,
            "unknown entity name must not touch any entity's per-entity component (NPC)"
        );
        assert!(
            (player_mods.get(&crate::messages::ModifierSlot::MaxSpeed) - 1.0).abs() < 1e-3,
            "unknown entity name must not touch any entity's per-entity component (player)"
        );
    }

    #[test]
    fn ai_events_apply_modifier_registered_name_without_ecs_entity_is_ignored() {
        let mut app = ai_trigger_test_app();
        let (npc, _player) = spawn_two_modifier_targets(&mut app);
        // Register a phantom name → UUID mapping with no matching ECS entity.
        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime
                .name_to_uuid
                .insert("phantom".into(), "phantom-uuid".into());
        }
        fire_ai_event_trigger(
            &mut app,
            vec![TriggerAction::ApplyModifier {
                entity: "phantom".into(),
                tag: "boost".into(),
                slot: crate::messages::ModifierSlot::MaxSpeed,
                bonus: 5.0,
            }],
        );
        // No entity should have received the modifier.
        let npc_mods = app
            .world()
            .entity(npc)
            .get::<crate::modifiers::ShipModifiers>()
            .unwrap();
        assert!(
            (npc_mods.get(&crate::messages::ModifierSlot::MaxSpeed) - 1.0).abs() < 1e-3,
            "registered name with no ECS entity must not silently misroute the modifier"
        );
    }

    #[test]
    fn ai_events_apply_modifier_target_entity_without_component_is_ignored() {
        // Guards the third defensive branch: name resolves + UUID resolves +
        // Entity exists but has no `ShipModifiers` Component → warn+continue.
        let mut app = ai_trigger_test_app();
        let (npc, _player) = spawn_two_modifier_targets(&mut app);
        // Spawn an entity with a UUID but WITHOUT a ShipModifiers component.
        app.world_mut()
            .spawn(EntityUuid("componentless-uuid".to_string()));
        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime
                .name_to_uuid
                .insert("componentless".into(), "componentless-uuid".into());
        }
        fire_ai_event_trigger(
            &mut app,
            vec![TriggerAction::ApplyModifier {
                entity: "componentless".into(),
                tag: "boost".into(),
                slot: crate::messages::ModifierSlot::MaxSpeed,
                bonus: 5.0,
            }],
        );
        // The NPC should NOT have been affected — no silent misroute.
        let npc_mods = app
            .world()
            .entity(npc)
            .get::<crate::modifiers::ShipModifiers>()
            .unwrap();
        assert!(
            (npc_mods.get(&crate::messages::ModifierSlot::MaxSpeed) - 1.0).abs() < 1e-3,
            "entity without ShipModifiers component must not misroute to other ships"
        );
    }

    #[test]
    fn on_all_destroyed_trigger_fires_after_all_named_entities_die_across_ticks() {
        // End-to-end Bevy runtime check: an `OnAllDestroyed` trigger over two
        // named entities must accumulate `seen_destroyed` across separate
        // `app.update()` calls and fire its action only when the last entity
        // dies. Mirrors `on_entity_destroyed_trigger_fires_add_objective_action`
        // but uses two separate destruction ticks. (#470)
        let mut app = ai_trigger_test_app();

        let uuid_a = "wave-a-uuid";
        let uuid_b = "wave-b-uuid";
        let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
        runtime
            .name_to_uuid
            .insert("wave_a".to_string(), uuid_a.to_string());
        runtime
            .name_to_uuid
            .insert("wave_b".to_string(), uuid_b.to_string());
        runtime.trigger_states = vec![TriggerState {
            trigger: crate::world::content::Trigger {
                condition: TriggerCondition::OnAllDestroyed {
                    entity_names: vec!["wave_a".to_string(), "wave_b".to_string()],
                },
                actions: vec![TriggerAction::AddObjective {
                    id: "obj-victory".to_string(),
                    text: "All waves cleared".to_string(),
                    mandatory: false,
                    targets: vec![],
                    directive: crate::messages::AiDirective::None,
                    utility: crate::objectives::UtilityConfig::default(),
                    source: crate::messages::ObjectiveSource::default(),
                }],
                when: None,
            },
            fired: false,
            origin_layer: None,
            seen_destroyed: HashSet::new(),
        }];

        // Tick 1: only wave_a dies. Trigger must NOT fire yet.
        app.world_mut()
            .resource_mut::<Messages<AiEntityDestroyed>>()
            .write(AiEntityDestroyed {
                entity_uuid: uuid_a.to_string(),
            });
        app.update();

        let objectives = &app.world().resource::<ObjectiveManagerRes>().0;
        assert!(
            !objectives
                .sorted_snapshots()
                .iter()
                .any(|o| o.id == "obj-victory"),
            "AddObjective must not fire after only first wave dies"
        );

        // Tick 2: wave_b dies. Trigger must NOW fire.
        app.world_mut()
            .resource_mut::<Messages<AiEntityDestroyed>>()
            .write(AiEntityDestroyed {
                entity_uuid: uuid_b.to_string(),
            });
        app.update();

        let objectives = &app.world().resource::<ObjectiveManagerRes>().0;
        assert!(
            objectives
                .sorted_snapshots()
                .iter()
                .any(|o| o.id == "obj-victory"),
            "AddObjective must fire after the last named entity dies"
        );
    }

    // -- Flag-system integration tests (issue #412) ---------------------------

    #[test]
    fn when_predicate_suppresses_action_dispatch_when_false() {
        // on_destroyed with when="flag(green_light)" must not fire its actions
        // while the flag is unset, but the trigger MUST remain live so it can
        // fire on a subsequent matching event once the flag is set.
        let mut app = ai_trigger_test_app();
        let npc_uuid = "uuid-destroyed-target";
        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime
                .name_to_uuid
                .insert("target".into(), npc_uuid.into());
            runtime.trigger_states = vec![TriggerState {
                trigger: crate::world::content::Trigger {
                    condition: TriggerCondition::OnDestroyed {
                        entity_name: "target".into(),
                    },
                    actions: vec![TriggerAction::AddObjective {
                        id: "obj-gated".into(),
                        text: "Should only fire after flag is set".into(),
                        mandatory: false,
                        targets: vec![],
                        directive: crate::messages::AiDirective::None,
                        utility: crate::objectives::UtilityConfig::default(),
                        source: crate::messages::ObjectiveSource::default(),
                    }],
                    when: Some(crate::world::flags::parse_predicate("flag(green_light)").unwrap()),
                },
                fired: false,
                origin_layer: None,
                seen_destroyed: HashSet::new(),
            }];
        }
        // First firing: flag unset Ã¢â€ â€™ no objective.
        app.world_mut()
            .resource_mut::<Messages<AiEntityDestroyed>>()
            .write(AiEntityDestroyed {
                entity_uuid: npc_uuid.into(),
            });
        app.update();
        {
            let runtime = app.world().resource::<WorldContentRuntime>();
            assert!(!runtime.trigger_states[0].fired, "trigger must remain live");
            let objs = &app.world().resource::<ObjectiveManagerRes>().0;
            assert!(
                !objs.sorted_snapshots().iter().any(|o| o.id == "obj-gated"),
                "gated action must NOT have fired"
            );
        }
        // Set the flag and re-fire.
        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime.flags.set_flag("green_light");
        }
        app.world_mut()
            .resource_mut::<Messages<AiEntityDestroyed>>()
            .write(AiEntityDestroyed {
                entity_uuid: npc_uuid.into(),
            });
        app.update();
        let objs = &app.world().resource::<ObjectiveManagerRes>().0;
        assert!(
            objs.sorted_snapshots().iter().any(|o| o.id == "obj-gated"),
            "gated action must fire once the flag is set"
        );
    }

    #[test]
    fn set_flag_action_fires_on_flag_set_trigger_within_same_tick() {
        // Trigger A: on_destroyed Ã¢â€ â€™ set_flag a
        // Trigger B: on_flag_set { name="a" } Ã¢â€ â€™ add_objective B
        // A and B must both fire in a single tick.
        let mut app = ai_trigger_test_app();
        let npc_uuid = "uuid-chain-source";
        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime
                .name_to_uuid
                .insert("source".into(), npc_uuid.into());
            runtime.trigger_states = vec![
                TriggerState {
                    trigger: crate::world::content::Trigger {
                        condition: TriggerCondition::OnDestroyed {
                            entity_name: "source".into(),
                        },
                        actions: vec![TriggerAction::SetWorldFlag { name: "a".into() }],
                        when: None,
                    },
                    fired: false,
                    origin_layer: None,
                    seen_destroyed: HashSet::new(),
                },
                TriggerState {
                    trigger: crate::world::content::Trigger {
                        condition: TriggerCondition::OnFlagSet { name: "a".into() },
                        actions: vec![TriggerAction::AddObjective {
                            id: "obj-chain".into(),
                            text: "Reacted to flag set".into(),
                            mandatory: false,
                            targets: vec![],
                            directive: crate::messages::AiDirective::None,
                            utility: crate::objectives::UtilityConfig::default(),
                            source: crate::messages::ObjectiveSource::default(),
                        }],
                        when: None,
                    },
                    fired: false,
                    origin_layer: None,
                    seen_destroyed: HashSet::new(),
                },
            ];
        }
        app.world_mut()
            .resource_mut::<Messages<AiEntityDestroyed>>()
            .write(AiEntityDestroyed {
                entity_uuid: npc_uuid.into(),
            });
        app.update();

        let runtime = app.world().resource::<WorldContentRuntime>();
        assert!(
            runtime.flags.flag("a"),
            "set_flag action must have mutated the store"
        );
        assert!(
            runtime.trigger_states[1].fired,
            "on_flag_set trigger must have fired"
        );
        let objs = &app.world().resource::<ObjectiveManagerRes>().0;
        assert!(
            objs.sorted_snapshots().iter().any(|o| o.id == "obj-chain"),
            "chained AddObjective must have fired in the same tick"
        );
    }

    #[test]
    fn no_op_reset_of_already_set_flag_does_not_emit_transition() {
        // Flag "a" starts set. A trigger fires set_flag a (no-op, value stays 1).
        // An on_flag_set trigger for "a" must NOT fire (transitions only).
        let mut app = ai_trigger_test_app();
        let npc_uuid = "uuid-noop-source";
        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime.flags.set_flag("a"); // pre-set
            runtime
                .name_to_uuid
                .insert("source".into(), npc_uuid.into());
            runtime.trigger_states = vec![
                TriggerState {
                    trigger: crate::world::content::Trigger {
                        condition: TriggerCondition::OnDestroyed {
                            entity_name: "source".into(),
                        },
                        actions: vec![TriggerAction::SetWorldFlag { name: "a".into() }],
                        when: None,
                    },
                    fired: false,
                    origin_layer: None,
                    seen_destroyed: HashSet::new(),
                },
                TriggerState {
                    trigger: crate::world::content::Trigger {
                        condition: TriggerCondition::OnFlagSet { name: "a".into() },
                        actions: vec![TriggerAction::AddObjective {
                            id: "obj-no-op".into(),
                            text: "Should not fire on no-op re-set".into(),
                            mandatory: false,
                            targets: vec![],
                            directive: crate::messages::AiDirective::None,
                            utility: crate::objectives::UtilityConfig::default(),
                            source: crate::messages::ObjectiveSource::default(),
                        }],
                        when: None,
                    },
                    fired: false,
                    origin_layer: None,
                    seen_destroyed: HashSet::new(),
                },
            ];
        }
        app.world_mut()
            .resource_mut::<Messages<AiEntityDestroyed>>()
            .write(AiEntityDestroyed {
                entity_uuid: npc_uuid.into(),
            });
        app.update();

        let runtime = app.world().resource::<WorldContentRuntime>();
        assert!(
            !runtime.trigger_states[1].fired,
            "on_flag_set must not fire when the flag was already set (no transition)"
        );
        let objs = &app.world().resource::<ObjectiveManagerRes>().0;
        assert!(
            !objs.sorted_snapshots().iter().any(|o| o.id == "obj-no-op"),
            "no objective expected from a no-op flag re-set"
        );
    }

    #[test]
    fn clear_flag_action_fires_on_flag_cleared_trigger() {
        let mut app = ai_trigger_test_app();
        let npc_uuid = "uuid-clear-source";
        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime.flags.set_flag("shields_up"); // pre-set so we transition trueÃ¢â€ â€™false
            runtime
                .name_to_uuid
                .insert("source".into(), npc_uuid.into());
            runtime.trigger_states = vec![
                TriggerState {
                    trigger: crate::world::content::Trigger {
                        condition: TriggerCondition::OnDestroyed {
                            entity_name: "source".into(),
                        },
                        actions: vec![TriggerAction::ClearWorldFlag {
                            name: "shields_up".into(),
                        }],
                        when: None,
                    },
                    fired: false,
                    origin_layer: None,
                    seen_destroyed: HashSet::new(),
                },
                TriggerState {
                    trigger: crate::world::content::Trigger {
                        condition: TriggerCondition::OnFlagCleared {
                            name: "shields_up".into(),
                        },
                        actions: vec![TriggerAction::AddObjective {
                            id: "obj-shields-down".into(),
                            text: "Shields are down".into(),
                            mandatory: true,
                            targets: vec![],
                            directive: crate::messages::AiDirective::None,
                            utility: crate::objectives::UtilityConfig::default(),
                            source: crate::messages::ObjectiveSource::default(),
                        }],
                        when: None,
                    },
                    fired: false,
                    origin_layer: None,
                    seen_destroyed: HashSet::new(),
                },
            ];
        }
        app.world_mut()
            .resource_mut::<Messages<AiEntityDestroyed>>()
            .write(AiEntityDestroyed {
                entity_uuid: npc_uuid.into(),
            });
        app.update();

        let runtime = app.world().resource::<WorldContentRuntime>();
        assert!(!runtime.flags.flag("shields_up"));
        let objs = &app.world().resource::<ObjectiveManagerRes>().0;
        assert!(
            objs.sorted_snapshots()
                .iter()
                .any(|o| o.id == "obj-shields-down"),
            "on_flag_cleared trigger must fire on trueÃ¢â€ â€™false transition"
        );
    }

    // -- PRD #397 fix 1: parent: walker in Bevy dispatch ----------------------

    /// A trigger in a sub-world layer can gate its `when` predicate on a
    /// flag in the loader (base) world via the `parent:` prefix.
    #[test]
    fn parent_prefix_in_when_predicate_reads_loader_layer_flag() {
        let mut app = ai_trigger_test_app();
        app.init_resource::<WorldLayerMap>();
        app.init_resource::<PendingWorldLayerChanges>();

        let layer_path = "child.toml".to_string();
        // Pre-register a sub-world layer whose loader is the base world.
        {
            let mut lm = app.world_mut().resource_mut::<WorldLayerMap>();
            lm.0.insert(
                layer_path.clone(),
                WorldRuntime {
                    loader_path: None,
                    ..Default::default()
                },
            );
        }
        let npc_uuid = "uuid-parent-when-source";
        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            // Base layer: flag `armed` is set.
            runtime.flags.set_flag("armed");
            runtime
                .name_to_uuid
                .insert("source".into(), npc_uuid.into());
            // Sub-world trigger: on_destroyed with when=flag(parent:armed).
            runtime.trigger_states = vec![TriggerState {
                trigger: crate::world::content::Trigger {
                    condition: TriggerCondition::OnDestroyed {
                        entity_name: "source".into(),
                    },
                    actions: vec![TriggerAction::AddObjective {
                        id: "obj-parent-when".into(),
                        text: "parent flag was set".into(),
                        mandatory: false,
                        targets: vec![],
                        directive: crate::messages::AiDirective::None,
                        utility: crate::objectives::UtilityConfig::default(),
                        source: crate::messages::ObjectiveSource::default(),
                    }],
                    when: Some(crate::world::flags::parse_predicate("flag(parent:armed)").unwrap()),
                },
                fired: false,
                origin_layer: Some(layer_path.clone()),
                seen_destroyed: HashSet::new(),
            }];
        }
        app.world_mut()
            .resource_mut::<Messages<AiEntityDestroyed>>()
            .write(AiEntityDestroyed {
                entity_uuid: npc_uuid.into(),
            });
        app.update();

        let objs = &app.world().resource::<ObjectiveManagerRes>().0;
        assert!(
            objs.sorted_snapshots()
                .iter()
                .any(|o| o.id == "obj-parent-when"),
            "sub-world trigger gated on parent:armed must fire when base flag is set"
        );
    }

    /// Per-layer flag scoping: a flag set inside a sub-world must NOT
    /// fire a base-world trigger that watches the same name.
    #[test]
    fn same_named_flag_in_sub_world_does_not_fire_base_world_on_flag_set() {
        let mut app = ai_trigger_test_app();
        app.init_resource::<WorldLayerMap>();
        app.init_resource::<PendingWorldLayerChanges>();

        let layer_path = "child.toml".to_string();
        {
            let mut lm = app.world_mut().resource_mut::<WorldLayerMap>();
            lm.0.insert(
                layer_path.clone(),
                WorldRuntime {
                    loader_path: None,
                    ..Default::default()
                },
            );
        }
        let npc_uuid = "uuid-scoped-source";
        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime
                .name_to_uuid
                .insert("source".into(), npc_uuid.into());
            runtime.trigger_states = vec![
                // Sub-world trigger: setting `armed` in the sub-world layer.
                TriggerState {
                    trigger: crate::world::content::Trigger {
                        condition: TriggerCondition::OnDestroyed {
                            entity_name: "source".into(),
                        },
                        actions: vec![TriggerAction::SetWorldFlag {
                            name: "armed".into(),
                        }],
                        when: None,
                    },
                    fired: false,
                    origin_layer: Some(layer_path.clone()),
                    seen_destroyed: HashSet::new(),
                },
                // Base-world watcher: on_flag_set armed.
                TriggerState {
                    trigger: crate::world::content::Trigger {
                        condition: TriggerCondition::OnFlagSet {
                            name: "armed".into(),
                        },
                        actions: vec![TriggerAction::AddObjective {
                            id: "obj-base-armed".into(),
                            text: "should NOT fire â€” different layer".into(),
                            mandatory: false,
                            targets: vec![],
                            directive: crate::messages::AiDirective::None,
                            utility: crate::objectives::UtilityConfig::default(),
                            source: crate::messages::ObjectiveSource::default(),
                        }],
                        when: None,
                    },
                    fired: false,
                    origin_layer: None,
                    seen_destroyed: HashSet::new(),
                },
            ];
        }
        app.world_mut()
            .resource_mut::<Messages<AiEntityDestroyed>>()
            .write(AiEntityDestroyed {
                entity_uuid: npc_uuid.into(),
            });
        app.update();

        // Sub-world layer's flag store got the mutation; base store did not.
        let lm = app.world().resource::<WorldLayerMap>();
        let layer_flags = &lm.0.get(&layer_path).expect("layer present").flags;
        assert!(
            layer_flags.flag("armed"),
            "mutation lands in sub-world layer"
        );
        let runtime = app.world().resource::<WorldContentRuntime>();
        assert!(!runtime.flags.flag("armed"), "base store must remain empty");
        let objs = &app.world().resource::<ObjectiveManagerRes>().0;
        assert!(
            !objs
                .sorted_snapshots()
                .iter()
                .any(|o| o.id == "obj-base-armed"),
            "base trigger must not cross-fire on sub-world flag"
        );
    }

    /// `parent:flag` mutation from the base world walks past root â†’ no-op +
    /// warn; the predicate read also resolves as unset.
    #[test]
    fn parent_walk_past_root_from_base_is_noop_for_mutation_and_reads_unset() {
        let mut app = ai_trigger_test_app();
        app.init_resource::<WorldLayerMap>();
        app.init_resource::<PendingWorldLayerChanges>();
        let npc_uuid = "uuid-past-root";
        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime
                .name_to_uuid
                .insert("source".into(), npc_uuid.into());
            runtime.trigger_states = vec![TriggerState {
                trigger: crate::world::content::Trigger {
                    condition: TriggerCondition::OnDestroyed {
                        entity_name: "source".into(),
                    },
                    // Base-world trigger (origin_layer=None) tries to mutate
                    // `parent:armed` â€” must be a no-op.
                    actions: vec![TriggerAction::SetWorldFlag {
                        name: "parent:armed".into(),
                    }],
                    when: None,
                },
                fired: false,
                origin_layer: None,
                seen_destroyed: HashSet::new(),
            }];
        }
        app.world_mut()
            .resource_mut::<Messages<AiEntityDestroyed>>()
            .write(AiEntityDestroyed {
                entity_uuid: npc_uuid.into(),
            });
        app.update();

        // Neither the base `armed` nor `parent:armed` should be set.
        let runtime = app.world().resource::<WorldContentRuntime>();
        assert!(
            !runtime.flags.flag("armed"),
            "past-root mutation must not write to base"
        );
        assert!(
            !runtime.flags.flag("parent:armed"),
            "past-root mutation must not write the literal prefixed name"
        );
    }

    #[test]
    fn on_entity_attacked_trigger_fires_add_objective_action() {
        let mut app = ai_trigger_test_app();

        let npc_uuid = "attacked-npc-uuid-002";
        let attacker_uuid = uuid::Uuid::parse_str("aaaaaaaa-0000-0000-0000-000000000001").unwrap();
        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime
                .name_to_uuid
                .insert("enemy_ship".to_string(), npc_uuid.to_string());
            runtime.trigger_states = vec![TriggerState {
                trigger: crate::world::content::Trigger {
                    condition: TriggerCondition::OnAttacked {
                        entity_name: "enemy_ship".to_string(),
                    },
                    actions: vec![TriggerAction::AddObjective {
                        id: "obj-002".to_string(),
                        text: "Enemy attacked".to_string(),
                        mandatory: false,
                        targets: vec![],
                        directive: crate::messages::AiDirective::None,
                        utility: crate::objectives::UtilityConfig::default(),
                        source: crate::messages::ObjectiveSource::default(),
                    }],
                    when: None,
                },
                fired: false,
                origin_layer: None,
                seen_destroyed: HashSet::new(),
            }];
        }

        app.world_mut()
            .resource_mut::<Messages<AiEntityAttacked>>()
            .write(AiEntityAttacked {
                entity_uuid: npc_uuid.to_string(),
                attacker_uuid,
            });

        app.update();

        let objectives = &app.world().resource::<ObjectiveManagerRes>().0;
        assert!(
            objectives
                .sorted_snapshots()
                .iter()
                .any(|o| o.id == "obj-002"),
            "AddObjective action from on_entity_attacked must have fired"
        );
    }

    #[test]
    fn set_ai_state_action_is_noop_in_doctrine_based_ai() {
        // Issue #572: SetAiState is kept in TriggerAction for TOML backward compat
        // but is now a no-op — doctrine-based AI has no FSM state slots. Verify
        // the system doesn't crash and the controller is unmodified.
        use crate::entity_config::BehaviourConfig;

        let mut app = ai_trigger_test_app();

        let npc_uuid = "npc-state-change-uuid-003";
        let attacker_uuid = uuid::Uuid::parse_str("bbbbbbbb-0000-0000-0000-000000000002").unwrap();

        let entity = app
            .world_mut()
            .spawn((
                Transform::from_xyz(0.0, 0.0, 0.0),
                EntityUuid(npc_uuid.to_string()),
                BehaviourSection(BehaviourConfig::default()),
            ))
            .id();
        app.update(); // attach controller

        // Set up trigger: on attacked → SetAiState (now a no-op).
        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime
                .name_to_uuid
                .insert("npc_alpha".to_string(), npc_uuid.to_string());
            runtime.trigger_states = vec![TriggerState {
                trigger: crate::world::content::Trigger {
                    condition: TriggerCondition::OnAttacked {
                        entity_name: "npc_alpha".to_string(),
                    },
                    actions: vec![TriggerAction::SetAiState {
                        entity: "npc_alpha".to_string(),
                        state: "chase".to_string(),
                        target: None,
                    }],
                    when: None,
                },
                fired: false,
                origin_layer: None,
                seen_destroyed: HashSet::new(),
            }];
        }

        // Fire the attacked event.
        app.world_mut()
            .resource_mut::<Messages<AiEntityAttacked>>()
            .write(AiEntityAttacked {
                entity_uuid: npc_uuid.to_string(),
                attacker_uuid,
            });

        // Must not panic — SetAiState is silently ignored.
        app.update();

        // Controller must still exist and have default memory (no FSM state).
        assert!(
            app.world().get::<AiControllerComponent>(entity).is_some(),
            "AiControllerComponent must survive a SetAiState no-op"
        );
    }

    // -- add_faction_enemy / remove_faction_enemy dispatch tests --------------

    /// Helper: fire a single trigger with the given action via
    /// `handle_ai_events`. Uses `on_world_loaded` so we only need a
    /// `WorldLoaded` event to fire it. Returns the post-update App so
    /// tests can inspect mutated resources.
    fn fire_world_loaded_action(actions: Vec<TriggerAction>) -> App {
        let mut app = ai_trigger_test_app();
        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime.trigger_states = vec![TriggerState {
                trigger: crate::world::content::Trigger {
                    condition: TriggerCondition::OnWorldLoaded,
                    actions,
                    when: None,
                },
                fired: false,
                origin_layer: None,
                seen_destroyed: HashSet::new(),
            }];
            runtime.pending_world_events.push(WorldEvent::WorldLoaded);
        }
        app.update();
        app
    }

    /// Convenience: faction UUIDs from the bundled TOML asset files
    /// (loaded by `get_faction_registry`). Centralises the literal UUIDs
    /// so the tests can refer to them by symbolic name.
    fn fed_faction_uuid() -> uuid::Uuid {
        uuid::Uuid::parse_str("aaaaaaaa-1111-4111-8111-aaaaaaaaaaaa").unwrap()
    }
    fn harrow_faction_uuid() -> uuid::Uuid {
        uuid::Uuid::parse_str("cccccccc-3333-4333-8333-cccccccccccc").unwrap()
    }

    #[test]
    fn add_faction_enemy_action_makes_factions_mutually_hostile() {
        // Pre-condition: Harrow and Federation default to neutral.
        let app = ai_trigger_test_app();
        {
            let reg = &app
                .world()
                .resource::<crate::config_cache::FactionRegistryResource>()
                .0;
            assert!(
                !crate::faction::is_enemy(
                    Some(fed_faction_uuid()),
                    Some(harrow_faction_uuid()),
                    reg
                ),
                "precondition: Federation must not consider Harrow hostile by default"
            );
            assert!(
                !crate::faction::is_enemy(
                    Some(harrow_faction_uuid()),
                    Some(fed_faction_uuid()),
                    reg
                ),
                "precondition: Harrow must not consider Federation hostile by default"
            );
        }

        // Fire a trigger that flips both directions hostile.
        let app = fire_world_loaded_action(vec![
            TriggerAction::AddFactionEnemy {
                faction: "Harrow".into(),
                enemy: "Federation".into(),
            },
            TriggerAction::AddFactionEnemy {
                faction: "Federation".into(),
                enemy: "Harrow".into(),
            },
        ]);

        let reg = &app
            .world()
            .resource::<crate::config_cache::FactionRegistryResource>()
            .0;
        assert!(
            crate::faction::is_enemy(Some(fed_faction_uuid()), Some(harrow_faction_uuid()), reg),
            "Federation must consider Harrow hostile after add_faction_enemy"
        );
        assert!(
            crate::faction::is_enemy(Some(harrow_faction_uuid()), Some(fed_faction_uuid()), reg),
            "Harrow must consider Federation hostile after add_faction_enemy"
        );
    }

    #[test]
    fn add_faction_enemy_action_with_unknown_faction_name_is_noop() {
        // The Federation registry stays unchanged when the named faction
        // is missing. Verifies the warn-and-skip dispatch path.
        let app = fire_world_loaded_action(vec![TriggerAction::AddFactionEnemy {
            faction: "Klingon".into(), // not a registered faction
            enemy: "Federation".into(),
        }]);

        let reg = &app
            .world()
            .resource::<crate::config_cache::FactionRegistryResource>()
            .0;
        // Federation's enemies list must still contain Pirate (its default)
        // and nothing else from the AddFactionEnemy dispatch.
        let fed = reg.get(&fed_faction_uuid()).expect("federation present");
        let pirate_uuid = uuid::Uuid::parse_str("bbbbbbbb-2222-4222-8222-bbbbbbbbbbbb").unwrap();
        assert_eq!(
            fed.enemies,
            vec![pirate_uuid],
            "unknown faction name must not mutate any other faction's enemy list"
        );
    }

    #[test]
    fn remove_faction_enemy_action_removes_relationship() {
        // First add the relationship, then verify remove undoes it.
        let app = fire_world_loaded_action(vec![
            TriggerAction::AddFactionEnemy {
                faction: "Harrow".into(),
                enemy: "Federation".into(),
            },
            TriggerAction::RemoveFactionEnemy {
                faction: "Harrow".into(),
                enemy: "Federation".into(),
            },
        ]);

        let reg = &app
            .world()
            .resource::<crate::config_cache::FactionRegistryResource>()
            .0;
        assert!(
            !crate::faction::is_enemy(Some(harrow_faction_uuid()), Some(fed_faction_uuid()), reg),
            "remove_faction_enemy must undo the prior add_faction_enemy"
        );
    }

    #[test]
    fn remove_faction_enemy_action_clears_blackboard_target_when_target_becomes_friendly() {
        // Scenario:
        //   1. Spawn a Harrow-factioned NPC that targets a Federation
        //      player ship (set blackboard.target manually to simulate a
        //      prior `enemy_in_range` engagement).
        //   2. Make Federation hostile to Harrow via add_faction_enemy
        //      so the precondition is mutually hostile.
        //   3. Fire remove_faction_enemy for Harrow â†’ Federation.
        //   4. Assert: the NPC's blackboard.target is now None (the
        //      revalidation kicked in because target_faction is no
        //      longer hostile to self_faction).
        use crate::entity_config::BehaviourConfig;

        let mut app = ai_trigger_test_app();

        // Prepare a Federation-factioned "player ship" entity.
        let player_uuid_str = "11111111-1111-1111-1111-111111111111";
        let player_uuid = uuid::Uuid::parse_str(player_uuid_str).unwrap();
        app.world_mut().spawn((
            Transform::from_xyz(0.0, 0.0, 0.0),
            EntityUuid(player_uuid_str.to_string()),
            crate::entities::spawner::FactionComponent(fed_faction_uuid()),
        ));

        // Prepare a Harrow-factioned NPC entity with an AI behaviour.
        let npc_uuid_str = "22222222-2222-2222-2222-222222222222";
        let npc_entity = app
            .world_mut()
            .spawn((
                Transform::from_xyz(10.0, 0.0, 0.0),
                EntityUuid(npc_uuid_str.to_string()),
                BehaviourSection(BehaviourConfig::default()),
                crate::entities::spawner::FactionComponent(harrow_faction_uuid()),
            ))
            .id();

        // First update: attach the AiControllerComponent marker + ShipAiMemory.
        app.update();

        // Manually seed the engagement: NPC's ShipAiMemory.target = player.
        {
            let mut mem = app
                .world_mut()
                .get_mut::<crate::ai_plugin::ShipAiMemory>(npc_entity)
                .expect("ShipAiMemory must be attached");
            mem.0.target = Some(player_uuid);
        }

        // Bring both sides into mutual hostility, then fire
        // remove_faction_enemy on Harrow's side. Two trigger states â‡’ the
        // first establishes the relationship that the second tears down.
        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime.trigger_states = vec![
                TriggerState {
                    trigger: crate::world::content::Trigger {
                        condition: TriggerCondition::OnWorldLoaded,
                        actions: vec![
                            TriggerAction::AddFactionEnemy {
                                faction: "Harrow".into(),
                                enemy: "Federation".into(),
                            },
                            TriggerAction::AddFactionEnemy {
                                faction: "Federation".into(),
                                enemy: "Harrow".into(),
                            },
                        ],
                        when: None,
                    },
                    fired: false,
                    origin_layer: None,
                    seen_destroyed: HashSet::new(),
                },
                TriggerState {
                    trigger: crate::world::content::Trigger {
                        condition: TriggerCondition::OnFlagSet {
                            name: "peace".into(),
                        },
                        actions: vec![TriggerAction::RemoveFactionEnemy {
                            faction: "Harrow".into(),
                            enemy: "Federation".into(),
                        }],
                        when: None,
                    },
                    fired: false,
                    origin_layer: None,
                    seen_destroyed: HashSet::new(),
                },
            ];
            runtime.pending_world_events.push(WorldEvent::WorldLoaded);
            runtime.pending_world_events.push(WorldEvent::FlagSet {
                name: "peace".into(),
                origin_layer: None,
            });
        }

        app.update();

        // The NPC's ShipAiMemory.target must be cleared because Harrow
        // no longer considers Federation hostile.
        let mem = app
            .world()
            .get::<crate::ai_plugin::ShipAiMemory>(npc_entity)
            .unwrap();
        assert_eq!(
            mem.0.target, None,
            "remove_faction_enemy must clear memory.target when target is no longer hostile"
        );
    }

    // -- on_attacked comms template auto-injection tests -----------------------

    /// When an entity is attacked, comms templates with `on_attacked` condition
    /// must fire automatically (no player hailing required) and inject a message
    /// into the CommsInbox.
    #[test]
    fn on_attacked_comms_template_auto_injects_into_inbox() {
        use crate::world::content::{
            CommsDialogueNode, CommsTemplate, CommsTemplateState, TriggerCondition,
        };

        let mut app = ai_trigger_test_app();

        let raider_uuid = "raider-uuid-auto-001";
        let attacker_uuid = uuid::Uuid::parse_str("cccccccc-0000-0000-0000-000000000001").unwrap();
        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime
                .name_to_uuid
                .insert("raider".to_string(), raider_uuid.to_string());
            runtime.comms_template_states = vec![CommsTemplateState {
                template: CommsTemplate {
                    from: "raider".to_string(),
                    trigger: TriggerCondition::OnAttacked {
                        entity_name: "raider".to_string(),
                    },
                    node: CommsDialogueNode {
                        body: "Mayday! We are under attack!".to_string(),
                        responses: vec![],
                        speaker: None,
                        trigger: None,
                    },
                    thread_id: None,
                    urgent: false,
                    root_follow_up: None,
                },
                fired: false,
            }];
        }

        app.world_mut()
            .resource_mut::<Messages<AiEntityAttacked>>()
            .write(AiEntityAttacked {
                entity_uuid: raider_uuid.to_string(),
                attacker_uuid,
            });

        app.update();

        let inbox = &app.world().resource::<CommsInboxRes>().0;
        let messages = inbox.messages();
        assert_eq!(
            messages.len(),
            1,
            "on_attacked comms template must auto-inject one message"
        );
        assert_eq!(messages[0].body, "Mayday! We are under attack!");
        assert_eq!(
            messages[0].responses.len(),
            0,
            "broadcast message should have no responses"
        );
    }

    /// A comms template with `on_attacked` must fire only once (single-shot).
    #[test]
    fn on_attacked_comms_template_fires_only_once() {
        use crate::world::content::{
            CommsDialogueNode, CommsTemplate, CommsTemplateState, TriggerCondition,
        };

        let mut app = ai_trigger_test_app();

        let raider_uuid = "raider-uuid-once-002";
        let attacker_uuid = uuid::Uuid::parse_str("cccccccc-0000-0000-0000-000000000002").unwrap();
        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime
                .name_to_uuid
                .insert("raider".to_string(), raider_uuid.to_string());
            runtime.comms_template_states = vec![CommsTemplateState {
                template: CommsTemplate {
                    from: "raider".to_string(),
                    trigger: TriggerCondition::OnAttacked {
                        entity_name: "raider".to_string(),
                    },
                    node: CommsDialogueNode {
                        body: "Distress signal transmitted.".to_string(),
                        responses: vec![],
                        speaker: None,
                        trigger: None,
                    },
                    thread_id: None,
                    urgent: false,
                    root_follow_up: None,
                },
                fired: false,
            }];
        }

        // First attack
        app.world_mut()
            .resource_mut::<Messages<AiEntityAttacked>>()
            .write(AiEntityAttacked {
                entity_uuid: raider_uuid.to_string(),
                attacker_uuid,
            });
        app.update();

        // Second attack
        app.world_mut()
            .resource_mut::<Messages<AiEntityAttacked>>()
            .write(AiEntityAttacked {
                entity_uuid: raider_uuid.to_string(),
                attacker_uuid,
            });
        app.update();

        let inbox = &app.world().resource::<CommsInboxRes>().0;
        assert_eq!(
            inbox.messages().len(),
            1,
            "on_attacked comms template must fire only once"
        );
    }

    // -- Unified [[entity]] name ? uuid pipeline (PRD #337/#339 slice 2) -------

    #[test]
    fn spawn_world_entities_populates_name_to_uuid_for_named_entity() {
        use crate::world::config::WorldConfig as UnifiedWorldConfig;
        use crate::world::config::WorldEntity;

        // Build a unified WorldConfig with one named entry (no template
        // resolution needed Ã¢â‚¬â€ the helper that mutates `name_to_uuid` runs
        // independently of the asteroid-field spawning path).
        let mut world_cfg = UnifiedWorldConfig::default();
        world_cfg.entities.push(WorldEntity {
            template_path: "assets/entities/station_outpost.toml".into(),
            name: Some("starbase_alpha".into()),
            transform: Some(crate::world::config::TransformConfig {
                position: Some([500.0, 0.0, 0.0]),
                ..Default::default()
            }),
            ..Default::default()
        });
        world_cfg.entities.push(WorldEntity {
            template_path: "assets/entities/star_sun.toml".into(),
            name: None,
            transform: Some(crate::world::config::TransformConfig {
                position: Some([0.0, 0.0, 0.0]),
                ..Default::default()
            }),
            ..Default::default()
        });

        let mut app = App::new();
        app.insert_resource(world_cfg);
        app.add_systems(Update, spawn_world_entities);
        app.update();

        let cfg = app.world().resource::<UnifiedWorldConfig>();
        assert_eq!(
            cfg.name_to_uuid.len(),
            1,
            "only named [[entity]] entries get a uuid"
        );
        let uuid = cfg
            .name_to_uuid
            .get("starbase_alpha")
            .expect("named entity must register");
        assert!(!uuid.is_empty(), "registered uuid must be non-empty");
    }

    #[test]
    fn spawn_world_entities_mirrors_names_into_world_content_runtime() {
        // PRD #337/#339 slice 2: trigger / comms lookup paths read from
        // `WorldContentRuntime.name_to_uuid`. The unified pipeline must
        // mirror its registrations into that map so the lookup path stays
        // a single source of truth during the transitional slices.
        use crate::world::config::WorldConfig as UnifiedWorldConfig;
        use crate::world::config::WorldEntity;

        let mut world_cfg = UnifiedWorldConfig::default();
        world_cfg.entities.push(WorldEntity {
            template_path: "assets/entities/station_outpost.toml".into(),
            name: Some("earth".into()),
            ..Default::default()
        });

        let mut app = App::new();
        app.insert_resource(world_cfg);
        app.init_resource::<WorldContentRuntime>();
        // Pre-populate runtime with a legacy entry to prove merge (not overwrite).
        app.world_mut()
            .resource_mut::<WorldContentRuntime>()
            .name_to_uuid
            .insert("legacy_spawn".into(), "legacy-uuid".into());

        app.add_systems(Update, spawn_world_entities);
        app.update();

        let runtime = app.world().resource::<WorldContentRuntime>();
        assert!(
            runtime.name_to_uuid.contains_key("earth"),
            "unified pipeline must mirror named entries into runtime"
        );
        assert!(
            runtime.name_to_uuid.contains_key("legacy_spawn"),
            "pre-existing legacy entries must survive the mirror"
        );
    }

    #[test]
    fn init_world_runtime_preserves_existing_name_to_uuid() {
        // PRD #341: `spawn_world_entities` runs before `init_world_runtime`
        // and writes names from the unified [[entity]] pipeline into
        // `WorldContentRuntime.name_to_uuid`. `init_world_runtime` (which
        // folds `WorldConfig.name_to_uuid` in) must NOT overwrite those Ã¢â‚¬â€
        // otherwise trigger and comms lookups for unified-pipeline names
        // would silently disappear.
        use crate::world::config::WorldConfig as UnifiedWorldConfig;

        let mut world_cfg = UnifiedWorldConfig::default();
        world_cfg
            .name_to_uuid
            .insert("starbase_alpha".into(), "world-config-uuid".into());
        world_cfg
            .name_to_uuid
            .insert("only_in_world".into(), "world-only-uuid".into());

        let mut app = App::new();
        app.init_resource::<WorldContentRuntime>();
        app.init_resource::<CommsInboxRes>();
        app.insert_resource(WorldResource(crate::messages::WorldData::default()));
        app.insert_resource(world_cfg);

        // Pre-populate the runtime with a value that must survive the merge.
        app.world_mut()
            .resource_mut::<WorldContentRuntime>()
            .name_to_uuid
            .insert("starbase_alpha".into(), "unified-pipeline-uuid".into());

        app.add_systems(Update, init_world_runtime);
        app.update();

        let runtime = app.world().resource::<WorldContentRuntime>();
        assert_eq!(
            runtime
                .name_to_uuid
                .get("starbase_alpha")
                .map(String::as_str),
            Some("unified-pipeline-uuid"),
            "init_world_runtime must preserve unified-pipeline registrations"
        );
        assert_eq!(
            runtime
                .name_to_uuid
                .get("only_in_world")
                .map(String::as_str),
            Some("world-only-uuid"),
            "names that exist only in WorldConfig.name_to_uuid still flow through"
        );
    }

    #[test]
    fn spawn_immediate_entities_spawns_named_non_asteroid_with_registered_uuid() {
        // PRD #339 slice 2 (rejection fix): named [[entity]] entries MUST be
        // spawned as real Bevy entities Ã¢â‚¬â€ otherwise triggers / comms resolve
        // to a UUID that has no Transform behind it. The spawned entity's
        // `EntityUuid` component must equal the UUID already registered in
        // `WorldConfig.name_to_uuid` for that name (single source of truth Ã¢â‚¬â€
        // no fresh UUID allocation inside the spawn loop).
        use crate::entity_config::EntityConfig;
        use crate::entity_spawner::EntityUuid;
        use crate::world::config::WorldConfig as UnifiedWorldConfig;
        use crate::world::config::WorldEntity;
        use std::collections::HashMap;

        let mut world_cfg = UnifiedWorldConfig::default();
        world_cfg.entities.push(WorldEntity {
            template_path: "fixture/station.toml".into(),
            name: Some("starbase_alpha".into()),
            transform: Some(crate::world::config::TransformConfig {
                position: Some([500.0, 0.0, 0.0]),
                ..Default::default()
            }),
            ..Default::default()
        });
        // An anonymous entry must NOT be spawned by the unified pipeline
        // (the complementary `setup_world` in `server_app.rs` owns it).
        world_cfg.entities.push(WorldEntity {
            template_path: "fixture/star.toml".into(),
            transform: Some(crate::world::config::TransformConfig {
                position: Some([0.0, 0.0, 0.0]),
                ..Default::default()
            }),
            ..Default::default()
        });

        // Pre-populate name_to_uuid as `spawn_world_entities`'s
        // assign-uuid pass would have.
        world_cfg
            .name_to_uuid
            .insert("starbase_alpha".into(), "stable-station-uuid".into());

        // Build a fixture ConfigCache with the templates referenced above.
        // Empty EntityConfig is sufficient Ã¢â‚¬â€ no asteroid_field section, so
        // `is_owned_by_unified_pipeline` routes by `name.is_some()`.
        let mut m: HashMap<String, EntityConfig> = HashMap::new();
        m.insert(
            "fixture/station.toml".into(),
            EntityConfig::from_toml("").unwrap(),
        );
        m.insert(
            "fixture/star.toml".into(),
            EntityConfig::from_toml("").unwrap(),
        );
        let cache = crate::config_cache::ConfigCache::from(m);

        let mut app = App::new();
        app.add_plugins(bevy::time::TimePlugin);
        app.insert_resource(world_cfg.clone());

        let spawned: Vec<Entity> = {
            let world_cfg = app.world().resource::<UnifiedWorldConfig>().clone();
            let mut commands = app.world_mut().commands();
            spawn_immediate_entities_internal(&mut commands, &world_cfg, &cache)
        };
        app.update();

        // Exactly one entity from the unified pipeline.
        assert_eq!(spawned.len(), 1, "only the named entry must be spawned");

        // Its EntityUuid must equal the registered UUID Ã¢â‚¬â€ not a fresh one.
        let uuid_component = app
            .world()
            .get::<EntityUuid>(spawned[0])
            .expect("spawned entity must carry EntityUuid");
        assert_eq!(
            uuid_component.0, "stable-station-uuid",
            "spawned entity's UUID must match the one in WorldConfig.name_to_uuid"
        );

        // And it must have a Transform at the configured position so
        // trigger / comms position queries work.
        let transform = app
            .world()
            .get::<Transform>(spawned[0])
            .expect("spawned entity must have a Transform");
        assert_eq!(transform.translation, Vec3::new(500.0, 0.0, 0.0));
    }

    // -- PRD #337 slice 3: NPCs through unified pipeline ------------------

    #[test]
    fn spawn_immediate_entities_resolves_anchor_position_for_named_entry() {
        // PRD #337 slice 3: a `[[entity]]` with `name = ...` AND
        // `anchor = "..."` (no inline `position`) must be spawned at the
        // anchor's coordinates. This is the migration path for the patrol
        // raider NPC moving off `[[spawn]]`.
        use crate::entity_config::EntityConfig;
        use crate::world::config::WorldConfig as UnifiedWorldConfig;
        use crate::world::config::WorldEntity;
        use std::collections::HashMap;

        let mut world_cfg = UnifiedWorldConfig::default();
        world_cfg
            .anchors
            .insert("patrol_alpha".into(), [300.0, 0.0, -300.0]);
        world_cfg.entities.push(WorldEntity {
            template_path: "fixture/raider.toml".into(),
            name: Some("raider_alpha".into()),
            transform: Some(crate::world::config::TransformConfig {
                anchor: Some("patrol_alpha".into()),
                ..Default::default()
            }),
            ..Default::default()
        });
        world_cfg
            .name_to_uuid
            .insert("raider_alpha".into(), "raider-uuid-001".into());

        let mut m: HashMap<String, EntityConfig> = HashMap::new();
        m.insert(
            "fixture/raider.toml".into(),
            EntityConfig::from_toml("").unwrap(),
        );
        let cache = crate::config_cache::ConfigCache::from(m);

        let mut app = App::new();
        app.add_plugins(bevy::time::TimePlugin);
        app.insert_resource(world_cfg.clone());

        let spawned: Vec<Entity> = {
            let mut commands = app.world_mut().commands();
            spawn_immediate_entities_internal(&mut commands, &world_cfg, &cache)
        };
        app.update();

        assert_eq!(spawned.len(), 1, "exactly one named entry must spawn");

        let transform = app
            .world()
            .get::<Transform>(spawned[0])
            .expect("spawned entity must have Transform");
        assert_eq!(
            transform.translation,
            Vec3::new(300.0, 0.0, -300.0),
            "named entity with anchor must be positioned at the anchor"
        );
    }

    #[test]
    fn spawn_immediate_entities_wires_behaviour_for_npc_with_anchor() {
        // PRD #337 slice 3: a named [[entity]] whose template carries a
        // [behaviour] block must end up with a BehaviourSection Ã¢â‚¬â€ the
        // AiPlugin's `attach_controllers_on_spawn` system reads that to
        // wire the AiController. This guarantees NPCs migrated from
        // [[spawn]] to [[entity]] still get AI on spawn.
        use crate::entity_config::EntityConfig;
        use crate::entity_spawner::BehaviourSection;
        use crate::world::config::WorldConfig as UnifiedWorldConfig;
        use crate::world::config::WorldEntity;
        use std::collections::HashMap;

        let raider_toml = r#"
tags = ["ship","npc","enemy"]

[behaviour]

[[behaviour.doctrine]]
id = "destroy-hostiles"
text = "Destroy hostiles"
directive_kind = "Destroy"
base_priority = 35.0
"#;
        let mut world_cfg = UnifiedWorldConfig::default();
        world_cfg
            .anchors
            .insert("patrol_alpha".into(), [300.0, 0.0, -300.0]);
        world_cfg.entities.push(WorldEntity {
            template_path: "fixture/raider.toml".into(),
            name: Some("raider_alpha".into()),
            transform: Some(crate::world::config::TransformConfig {
                anchor: Some("patrol_alpha".into()),
                ..Default::default()
            }),
            ..Default::default()
        });
        world_cfg
            .name_to_uuid
            .insert("raider_alpha".into(), "raider-uuid-002".into());

        let mut m: HashMap<String, EntityConfig> = HashMap::new();
        m.insert(
            "fixture/raider.toml".into(),
            EntityConfig::from_toml(raider_toml).unwrap(),
        );
        let cache = crate::config_cache::ConfigCache::from(m);

        let mut app = App::new();
        app.add_plugins(bevy::time::TimePlugin);
        app.insert_resource(world_cfg.clone());

        let spawned: Vec<Entity> = {
            let mut commands = app.world_mut().commands();
            spawn_immediate_entities_internal(&mut commands, &world_cfg, &cache)
        };
        app.update();

        assert_eq!(spawned.len(), 1);
        assert!(
            app.world().get::<BehaviourSection>(spawned[0]).is_some(),
            "NPC spawned through unified pipeline must carry BehaviourSection so AiPlugin can attach a controller"
        );
    }

    #[test]
    fn spawn_immediate_entities_resolves_anchor_offset_on_asteroid_field() {
        // PRD #397 fix 5: an asteroid_field template whose
        // `[asteroid_field] anchor = "name"` references a known world anchor
        // must have its `anchor_offset` populated with that anchor's world
        // position. The streaming spawner reads this offset to translate
        // the eligibility annulus and per-asteroid spawn positions.
        use crate::entity_config::{
            AsteroidFieldConfig, AsteroidFieldShape, EntityConfig, GridConfig,
        };
        use crate::entity_spawner::AsteroidFieldSection;
        use crate::world::config::WorldConfig as UnifiedWorldConfig;
        use crate::world::config::WorldEntity;
        use std::collections::HashMap;

        let mut world_cfg = UnifiedWorldConfig::default();
        world_cfg
            .anchors
            .insert("belt_origin".into(), [500.0, 0.0, -250.0]);
        world_cfg.entities.push(WorldEntity {
            template_path: "fixture/anchored_belt.toml".into(),
            ..Default::default()
        });

        // Build an EntityConfig in the cache with an asteroid_field that
        // references the anchor. Anchor offset starts at origin and must
        // be filled in by `spawn_immediate_entities_internal`.
        let mut field_template = EntityConfig::from_toml("").unwrap();
        field_template.asteroid_field = Some(AsteroidFieldConfig {
            inner_radius: 100.0,
            outer_radius: 200.0,
            density: 0.005,
            spawn_distance: 150.0,
            despawn_distance: 250.0,
            asteroid_type_paths: vec!["a.toml".into()],
            cosmetic_type_paths: vec![],
            tags: vec![],
            grid: Some(GridConfig {
                resolution: 15.0,
                fill_gameplay: 0.4,
                fill_cosmetic: 0.0,
                uniformity: 0.3,
                noise_freq: 0.02,
                noise_octaves: 1,
                density_noise_freq: 0.01,
                density_noise_octaves: 1,
                jitter: 0.0,
                cosmetic_y_offset: 0.0,
                gameplay_y_variance: 0.0,
                spawn_cells: 4,
                despawn_cells: 6,
            }),
            shield_pierce: 0.0,
            shape: Some(AsteroidFieldShape::Torus),
            anchor: Some("belt_origin".into()),
            anchor_offset: [0.0, 0.0, 0.0],
            random_rotation: None,
        });
        let mut m: HashMap<String, EntityConfig> = HashMap::new();
        m.insert("fixture/anchored_belt.toml".into(), field_template);
        let cache = crate::config_cache::ConfigCache::from(m);

        let mut app = App::new();
        app.add_plugins(bevy::time::TimePlugin);
        app.insert_resource(world_cfg.clone());

        let spawned: Vec<Entity> = {
            let mut commands = app.world_mut().commands();
            spawn_immediate_entities_internal(&mut commands, &world_cfg, &cache)
        };
        app.update();

        assert_eq!(
            spawned.len(),
            1,
            "exactly one asteroid_field entry must spawn"
        );
        let section = app
            .world()
            .get::<AsteroidFieldSection>(spawned[0])
            .expect("asteroid_field entry must carry an AsteroidFieldSection");
        assert_eq!(
            section.0.anchor_offset,
            [500.0, 0.0, -250.0],
            "spawn-time resolution must copy the anchor's world position into anchor_offset"
        );
        assert_eq!(
            section.0.anchor.as_deref(),
            Some("belt_origin"),
            "anchor name must be preserved alongside the resolved offset"
        );
    }

    #[test]
    fn spawn_immediate_entities_falls_back_to_origin_when_anchor_unknown() {
        // PRD #397 fix 5: an asteroid_field referencing an anchor name that
        // is NOT present in `WorldConfig.anchors` must fall back to the
        // world origin (`anchor_offset = [0,0,0]`) rather than silently
        // relocating somewhere unexpected or refusing to spawn. Implementation
        // also logs a `warn!` so the typo is visible in console output.
        use crate::entity_config::{
            AsteroidFieldConfig, AsteroidFieldShape, EntityConfig, GridConfig,
        };
        use crate::entity_spawner::AsteroidFieldSection;
        use crate::world::config::WorldConfig as UnifiedWorldConfig;
        use crate::world::config::WorldEntity;
        use std::collections::HashMap;

        let mut world_cfg = UnifiedWorldConfig::default();
        // Note: NO anchor named "typo_anchor" Ã¢â‚¬â€ only "real_anchor".
        world_cfg
            .anchors
            .insert("real_anchor".into(), [999.0, 0.0, 999.0]);
        world_cfg.entities.push(WorldEntity {
            template_path: "fixture/typo_belt.toml".into(),
            ..Default::default()
        });

        let mut field_template = EntityConfig::from_toml("").unwrap();
        field_template.asteroid_field = Some(AsteroidFieldConfig {
            inner_radius: 100.0,
            outer_radius: 200.0,
            density: 0.005,
            spawn_distance: 150.0,
            despawn_distance: 250.0,
            asteroid_type_paths: vec!["a.toml".into()],
            cosmetic_type_paths: vec![],
            tags: vec![],
            grid: Some(GridConfig {
                resolution: 15.0,
                fill_gameplay: 0.4,
                fill_cosmetic: 0.0,
                uniformity: 0.3,
                noise_freq: 0.02,
                noise_octaves: 1,
                density_noise_freq: 0.01,
                density_noise_octaves: 1,
                jitter: 0.0,
                cosmetic_y_offset: 0.0,
                gameplay_y_variance: 0.0,
                spawn_cells: 4,
                despawn_cells: 6,
            }),
            shield_pierce: 0.0,
            shape: Some(AsteroidFieldShape::Torus),
            anchor: Some("typo_anchor".into()),
            anchor_offset: [0.0, 0.0, 0.0],
            random_rotation: None,
        });
        let mut m: HashMap<String, EntityConfig> = HashMap::new();
        m.insert("fixture/typo_belt.toml".into(), field_template);
        let cache = crate::config_cache::ConfigCache::from(m);

        let mut app = App::new();
        app.add_plugins(bevy::time::TimePlugin);
        app.insert_resource(world_cfg.clone());

        let spawned: Vec<Entity> = {
            let mut commands = app.world_mut().commands();
            spawn_immediate_entities_internal(&mut commands, &world_cfg, &cache)
        };
        app.update();

        assert_eq!(
            spawned.len(),
            1,
            "unknown anchor must NOT block spawn Ã¢â‚¬â€ fallback to origin keeps the field alive"
        );
        let section = app
            .world()
            .get::<AsteroidFieldSection>(spawned[0])
            .expect("asteroid_field entry must still carry an AsteroidFieldSection");
        assert_eq!(
            section.0.anchor_offset,
            [0.0, 0.0, 0.0],
            "missing anchor must fall back to world origin, not the only other known anchor"
        );
    }

    // Ã¢â€â‚¬Ã¢â€â‚¬ extra_worlds + LoadWorld / UnloadWorld (issue #352) Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬

    /// Helper: build an `App` with `WorldLayerMap`, `WorldContentRuntime`, and
    /// the `apply_world_layer_changes` system wired in.  No LobbyPlugin needed.
    fn layer_test_app() -> App {
        let mut app = App::new();
        app.init_resource::<WorldLayerMap>()
            .init_resource::<WorldContentRuntime>()
            .init_resource::<PendingWorldLayerChanges>()
            .add_systems(Update, apply_world_layer_changes);
        app
    }

    /// `extra_worlds` on `WorldConfig` starts empty by default.
    #[test]
    fn world_config_extra_worlds_defaults_to_empty() {
        let cfg = crate::world::config::WorldConfig::default();
        assert!(cfg.extra_worlds.is_empty());
    }

    /// `load_extra_worlds` queues one `Load` command per `extra_worlds` entry.
    #[test]
    fn load_extra_worlds_startup_queues_pending_layer_changes() {
        let mut app = App::new();
        app.init_resource::<WorldLayerMap>()
            .init_resource::<WorldContentRuntime>()
            .init_resource::<PendingWorldLayerChanges>();

        let mut world_cfg = crate::world::config::WorldConfig::default();
        world_cfg
            .extra_worlds
            .push("assets/worlds/patrol.toml".into());
        world_cfg
            .extra_worlds
            .push("assets/worlds/side.toml".into());
        app.insert_resource(world_cfg);

        app.add_systems(Startup, load_extra_worlds);
        app.world_mut().run_schedule(Startup);

        let pending = app.world().resource::<PendingWorldLayerChanges>();
        assert_eq!(
            pending.0.len(),
            2,
            "one Load command per extra_worlds entry"
        );
        assert!(
            matches!(&pending.0[0], WorldLayerChange::Load { path: p, .. } if p == "assets/worlds/patrol.toml")
        );
        assert!(
            matches!(&pending.0[1], WorldLayerChange::Load { path: p, .. } if p == "assets/worlds/side.toml")
        );
    }

    /// `LoadWorld` action via trigger queues a `Load` command into `PendingWorldLayerChanges`.
    #[test]
    fn load_world_trigger_action_queues_pending_layer_change() {
        let mut app = ai_trigger_test_app();
        app.init_resource::<WorldLayerMap>()
            .init_resource::<PendingWorldLayerChanges>();

        let npc_uuid = "trigger-load-world-npc-001";
        let attacker_uuid = uuid::Uuid::parse_str("dddddddd-0000-0000-0000-000000000001").unwrap();
        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime
                .name_to_uuid
                .insert("raider".into(), npc_uuid.into());
            runtime.trigger_states = vec![TriggerState {
                trigger: crate::world::content::Trigger {
                    condition: TriggerCondition::OnAttacked {
                        entity_name: "raider".into(),
                    },
                    actions: vec![TriggerAction::LoadWorld {
                        path: "assets/worlds/patrol.toml".into(),
                    }],
                    when: None,
                },
                fired: false,
                origin_layer: None,
                seen_destroyed: HashSet::new(),
            }];
        }

        app.world_mut()
            .resource_mut::<Messages<AiEntityAttacked>>()
            .write(AiEntityAttacked {
                entity_uuid: npc_uuid.into(),
                attacker_uuid,
            });
        app.update();

        let pending = app.world().resource::<PendingWorldLayerChanges>();
        assert_eq!(pending.0.len(), 1, "one Load must be queued");
        assert!(
            matches!(&pending.0[0], WorldLayerChange::Load { path: p, .. } if p == "assets/worlds/patrol.toml")
        );
    }

    /// `UnloadWorld` action via trigger queues an `Unload` command.
    #[test]
    fn unload_world_trigger_action_queues_pending_layer_change() {
        let mut app = ai_trigger_test_app();
        app.init_resource::<WorldLayerMap>()
            .init_resource::<PendingWorldLayerChanges>();

        let npc_uuid = "trigger-unload-world-npc-002";
        let attacker_uuid = uuid::Uuid::parse_str("dddddddd-0000-0000-0000-000000000002").unwrap();
        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime
                .name_to_uuid
                .insert("raider".into(), npc_uuid.into());
            runtime.trigger_states = vec![TriggerState {
                trigger: crate::world::content::Trigger {
                    condition: TriggerCondition::OnAttacked {
                        entity_name: "raider".into(),
                    },
                    actions: vec![TriggerAction::UnloadWorld {
                        path: "assets/worlds/patrol.toml".into(),
                    }],
                    when: None,
                },
                fired: false,
                origin_layer: None,
                seen_destroyed: HashSet::new(),
            }];
        }

        app.world_mut()
            .resource_mut::<Messages<AiEntityAttacked>>()
            .write(AiEntityAttacked {
                entity_uuid: npc_uuid.into(),
                attacker_uuid,
            });
        app.update();

        let pending = app.world().resource::<PendingWorldLayerChanges>();
        assert_eq!(pending.0.len(), 1, "one Unload must be queued");
        assert!(
            matches!(&pending.0[0], WorldLayerChange::Unload(p) if p == "assets/worlds/patrol.toml")
        );
    }

    /// `apply_world_layer_changes` with `LoadWorld(patrol.toml)` reads the TOML
    /// on native, merges triggers into `WorldContentRuntime`, and registers the
    /// layer in `WorldLayerMap`.
    #[test]
    fn load_world_action_merges_triggers_into_runtime() {
        let mut app = layer_test_app();

        app.world_mut()
            .resource_mut::<PendingWorldLayerChanges>()
            .0
            .push(WorldLayerChange::Load {
                path: "assets/worlds/patrol.toml".into(),
                loader_path: None,
            });

        app.update();

        let layer_map = app.world().resource::<WorldLayerMap>();
        assert!(
            layer_map.0.contains_key("assets/worlds/patrol.toml"),
            "WorldLayerMap must contain the loaded path"
        );

        let runtime = app.world().resource::<WorldContentRuntime>();
        assert!(
            !runtime.trigger_states.is_empty(),
            "trigger states must be merged into runtime"
        );
    }

    /// A second `LoadWorld` for the same path is a no-op (de-duplicated).
    #[test]
    fn load_world_is_deduped_when_already_in_layer_map() {
        let mut app = layer_test_app();

        // Load once.
        app.world_mut()
            .resource_mut::<PendingWorldLayerChanges>()
            .0
            .push(WorldLayerChange::Load {
                path: "assets/worlds/patrol.toml".into(),
                loader_path: None,
            });
        app.update();

        let trigger_count_after_first = app
            .world()
            .resource::<WorldContentRuntime>()
            .trigger_states
            .len();

        // Load again Ã¢â‚¬â€ must not double-add.
        app.world_mut()
            .resource_mut::<PendingWorldLayerChanges>()
            .0
            .push(WorldLayerChange::Load {
                path: "assets/worlds/patrol.toml".into(),
                loader_path: None,
            });
        app.update();

        let trigger_count_after_second = app
            .world()
            .resource::<WorldContentRuntime>()
            .trigger_states
            .len();

        assert_eq!(
            trigger_count_after_first, trigger_count_after_second,
            "duplicate LoadWorld must not add duplicate trigger states"
        );
    }

    /// `UnloadWorld` removes the triggers that were added by the matching `LoadWorld`.
    #[test]
    fn unload_world_removes_triggers_added_by_load_world() {
        let mut app = layer_test_app();

        // Load patrol.toml.
        app.world_mut()
            .resource_mut::<PendingWorldLayerChanges>()
            .0
            .push(WorldLayerChange::Load {
                path: "assets/worlds/patrol.toml".into(),
                loader_path: None,
            });
        app.update();

        let trigger_count_loaded = app
            .world()
            .resource::<WorldContentRuntime>()
            .trigger_states
            .len();
        assert!(
            trigger_count_loaded > 0,
            "patrol.toml must add at least one trigger"
        );

        // Unload it.
        app.world_mut()
            .resource_mut::<PendingWorldLayerChanges>()
            .0
            .push(WorldLayerChange::Unload("assets/worlds/patrol.toml".into()));
        app.update();

        let trigger_count_unloaded = app
            .world()
            .resource::<WorldContentRuntime>()
            .trigger_states
            .len();

        assert_eq!(
            trigger_count_unloaded, 0,
            "UnloadWorld must remove all triggers that were added by the LoadWorld"
        );

        let layer_map = app.world().resource::<WorldLayerMap>();
        assert!(
            !layer_map.0.contains_key("assets/worlds/patrol.toml"),
            "WorldLayerMap must no longer contain the unloaded path"
        );
    }

    /// Two `LoadWorld` commands for the same path queued within a single tick
    /// produce exactly one load Ã¢â‚¬â€ no duplicate entities, no duplicate trigger
    /// states, no duplicate `WorldLayerMap` entry (issue #413).
    #[test]
    fn two_load_world_same_path_same_tick_is_single_load() {
        let (world_path, _template_path) = write_layer_entity_fixtures();

        let mut app = App::new();
        app.add_plugins(bevy::time::TimePlugin)
            .init_resource::<WorldLayerMap>()
            .init_resource::<WorldContentRuntime>()
            .init_resource::<PendingWorldLayerChanges>()
            .add_systems(Update, apply_world_layer_changes);

        // Two triggers in the same world both request the same load in one tick.
        {
            let mut pending = app.world_mut().resource_mut::<PendingWorldLayerChanges>();
            pending.0.push(WorldLayerChange::Load {
                path: world_path.clone(),
                loader_path: None,
            });
            pending.0.push(WorldLayerChange::Load {
                path: world_path.clone(),
                loader_path: None,
            });
        }
        // First update: apply_world_layer_changes drains both commands; the
        // second must be a no-op because the first already inserted into
        // WorldLayerMap.
        app.update();
        // Second update: Bevy flushes deferred spawns.
        app.update();

        let layer_map = app.world().resource::<WorldLayerMap>();
        let layer = layer_map
            .0
            .get(&world_path)
            .expect("WorldLayerMap must contain the loaded path");

        // Exactly one layer entry (a Vec-backed entity list would otherwise
        // hold double the entities).
        let entity_count = layer.spawned_entities.len();
        assert!(
            entity_count > 0,
            "precondition: world fixture must spawn at least one entity"
        );

        // Drop borrow before mutating pending again.
        let _ = layer_map;

        // Capture trigger/contact/name counts after the first-tick double-load.
        let runtime = app.world().resource::<WorldContentRuntime>();
        let triggers_after_double = runtime.trigger_states.len();
        let names_after_double = runtime.name_to_uuid.len();
        let contacts_after_double = runtime.contacts.len();
        let _ = runtime;

        // Now load the same path AGAIN on a separate tick: must also be a
        // no-op (existing behaviour) and keep the same counts.
        app.world_mut()
            .resource_mut::<PendingWorldLayerChanges>()
            .0
            .push(WorldLayerChange::Load {
                path: world_path.clone(),
                loader_path: None,
            });
        app.update();
        app.update();

        let layer_map = app.world().resource::<WorldLayerMap>();
        let layer = layer_map
            .0
            .get(&world_path)
            .expect("layer must still be present");
        assert_eq!(
            layer.spawned_entities.len(),
            entity_count,
            "second-tick duplicate LoadWorld must not double-spawn entities"
        );

        let runtime = app.world().resource::<WorldContentRuntime>();
        assert_eq!(
            runtime.trigger_states.len(),
            triggers_after_double,
            "duplicate LoadWorld must not add duplicate trigger states"
        );
        assert_eq!(
            runtime.name_to_uuid.len(),
            names_after_double,
            "duplicate LoadWorld must not re-register named entities"
        );
        assert_eq!(
            runtime.contacts.len(),
            contacts_after_double,
            "duplicate LoadWorld must not duplicate comms contacts"
        );
    }

    /// `UnloadWorld` for a path that was never loaded is a silent no-op.
    #[test]
    fn unload_world_unknown_path_is_noop() {
        let mut app = layer_test_app();

        app.world_mut()
            .resource_mut::<PendingWorldLayerChanges>()
            .0
            .push(WorldLayerChange::Unload(
                "assets/worlds/nonexistent.toml".into(),
            ));
        app.update(); // must not panic

        let runtime = app.world().resource::<WorldContentRuntime>();
        assert!(runtime.trigger_states.is_empty());
    }

    // Ã¢â€â‚¬Ã¢â€â‚¬ Entity spawn / despawn via LoadWorld / UnloadWorld (issue #352) Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬

    /// Write a minimal world TOML and a stub entity template to temp files,
    /// return `(world_path, template_path)` as `String`s.
    ///
    /// The world has one named `[[entity]]` so we get a predictable spawn count
    /// without relying on the shipped `patrol.toml` config-cache.  Uses an
    /// atomic counter for unique paths so parallel test runs don't collide.
    fn write_layer_entity_fixtures() -> (String, String) {
        use std::sync::atomic::{AtomicU32, Ordering};
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let tmp = std::env::temp_dir();
        let tag = COUNTER.fetch_add(1, Ordering::Relaxed);
        let template_path = tmp.join(format!("layer_test_npc_{tag}.toml"));
        let world_path = tmp.join(format!("layer_test_world_{tag}.toml"));

        let template_toml = r##"
tags = ["npc"]

[appearance]
colour = "#888888"
size_min = 1.0
size_max = 2.0
"##;
        std::fs::write(&template_path, template_toml).expect("failed to write stub template");

        let template_path_str = template_path.to_string_lossy().replace('\\', "/");

        let world_toml = format!(
            r#"
[global]
seed = 1

[[entity]]
template_path = "{template_path_str}"
name = "layer_npc"
position = [1.0, 0.0, 0.0]

[[trigger]]
condition = "on_destroyed"
entity = "layer_npc"

  [[trigger.action]]
  type = "add_objective"
  id   = "obj-layer-npc"
  text = "Destroyed."
  mandatory = false
"#,
        );

        std::fs::write(&world_path, &world_toml).expect("failed to write layer world TOML");

        (
            world_path.to_string_lossy().into_owned(),
            template_path.to_string_lossy().into_owned(),
        )
    }

    /// `LoadWorld` spawns the world's `[[entity]]` blocks into the ECS and
    /// records them in `WorldLayerMap`.
    #[test]
    fn load_world_spawns_entities_into_ecs() {
        let (world_path, _template_path) = write_layer_entity_fixtures();

        let mut app = App::new();
        app.add_plugins(bevy::time::TimePlugin)
            .init_resource::<WorldLayerMap>()
            .init_resource::<WorldContentRuntime>()
            .init_resource::<PendingWorldLayerChanges>()
            .add_systems(Update, apply_world_layer_changes);

        app.world_mut()
            .resource_mut::<PendingWorldLayerChanges>()
            .0
            .push(WorldLayerChange::Load {
                path: world_path.clone(),
                loader_path: None,
            });

        // First update: commands are queued by apply_world_layer_changes.
        app.update();
        // Second update: Bevy flushes deferred commands, entities become real.
        app.update();

        let layer_map = app.world().resource::<WorldLayerMap>();
        let layer = layer_map
            .0
            .get(&world_path)
            .expect("WorldLayerMap must contain the loaded path");

        assert!(
            !layer.spawned_entities.is_empty(),
            "LoadWorld must record spawned entity handles in WorldLayerMap"
        );

        // Every recorded entity must actually exist in the ECS.
        for &entity in &layer.spawned_entities {
            assert!(
                app.world().get_entity(entity).is_ok(),
                "entity {entity:?} recorded in WorldLayerMap must exist in ECS after LoadWorld"
            );
        }
    }

    /// `UnloadWorld` despawns the ECS entities that were spawned by the
    /// matching `LoadWorld`.
    #[test]
    fn unload_world_despawns_entities_from_ecs() {
        let (world_path, _template_path) = write_layer_entity_fixtures();

        let mut app = App::new();
        app.add_plugins(bevy::time::TimePlugin)
            .init_resource::<WorldLayerMap>()
            .init_resource::<WorldContentRuntime>()
            .init_resource::<PendingWorldLayerChanges>()
            .add_systems(Update, apply_world_layer_changes);

        // Load first.
        app.world_mut()
            .resource_mut::<PendingWorldLayerChanges>()
            .0
            .push(WorldLayerChange::Load {
                path: world_path.clone(),
                loader_path: None,
            });
        app.update();
        app.update();

        // Capture the spawned entity handles before unload.
        let spawned_before: Vec<Entity> = app
            .world()
            .resource::<WorldLayerMap>()
            .0
            .get(&world_path)
            .expect("must be loaded")
            .spawned_entities
            .clone();

        assert!(
            !spawned_before.is_empty(),
            "precondition: LoadWorld must have spawned at least one entity"
        );

        // Now unload.
        app.world_mut()
            .resource_mut::<PendingWorldLayerChanges>()
            .0
            .push(WorldLayerChange::Unload(world_path.clone()));
        app.update();
        app.update();

        // Each entity spawned by LoadWorld must now be gone.
        for entity in spawned_before {
            assert!(
                app.world().get_entity(entity).is_err(),
                "entity {entity:?} must be despawned after UnloadWorld"
            );
        }

        // WorldLayerMap entry must be removed.
        assert!(
            !app.world()
                .resource::<WorldLayerMap>()
                .0
                .contains_key(&world_path),
            "WorldLayerMap must not contain the path after UnloadWorld"
        );
    }

    // -- Slice 7: range-aware comms broadcast --------------------------------

    #[test]
    fn comms_state_marks_contact_out_of_range_when_ship_too_far() {
        use crate::comms::CommsRange;
        use crate::entities::spawner::EntityUuid;
        use crate::simulation::Ship;

        let station_uuid = "station-uuid-range-far";
        let mut app = comms_test_app();
        setup_game_with_comms(&mut app, station_uuid);

        // Spawn the ship close to the station so the initial hail succeeds,
        // then move the station far away to verify the flag flips.
        let ship_entity = app
            .world_mut()
            .spawn((
                Ship,
                crate::simulation::LocalShip,
                Transform::from_xyz(0.0, 0.0, 0.0),
                CommsRange(100.0),
            ))
            .id();
        let station_entity = app
            .world_mut()
            .spawn((
                EntityUuid(station_uuid.into()),
                Transform::from_xyz(50.0, 0.0, 0.0),
                CommsRange(100.0),
            ))
            .id();

        // Flush initial broadcast.
        let _ = tick(&mut app);

        // Hail in range so a message is injected.
        push_msg(
            &mut app,
            "comms",
            ClientMessage::ControlSystem {
                target: crate::system_registry::comms_system_id(),
                payload: crate::messages::SystemControlPayload::Hail {
                    target_uuid: station_uuid.into(),
                },
            },
        );
        let _ = tick(&mut app);

        // Now move the station far away (combined range 200, distance 1000).
        let _ = ship_entity;
        if let Ok(mut e) = app.world_mut().get_entity_mut(station_entity) {
            e.insert(Transform::from_xyz(1000.0, 0.0, 0.0));
        }
        let out = tick(&mut app);

        let (messages, contacts) = out
            .iter()
            .find_map(|m| {
                if let ServerMessage::CommsState {
                    messages, contacts, ..
                } = &m.msg
                {
                    Some((messages.clone(), contacts.clone()))
                } else {
                    None
                }
            })
            .expect("CommsState must be broadcast after range flip");

        let contact = contacts
            .iter()
            .find(|c| c.uuid == station_uuid)
            .expect("contact present");
        assert!(!contact.in_range, "contact should be out of range");
        assert_eq!(messages.len(), 1, "one hail message expected");
        assert!(
            !messages[0].sender_in_range,
            "sender_in_range must be false when station is far"
        );
    }

    #[test]
    fn comms_state_marks_contact_in_range_when_ship_close() {
        use crate::comms::CommsRange;
        use crate::entities::spawner::EntityUuid;
        use crate::simulation::Ship;

        let station_uuid = "station-uuid-range-near";
        let mut app = comms_test_app();
        setup_game_with_comms(&mut app, station_uuid);

        app.world_mut().spawn((
            Ship,
            crate::simulation::LocalShip,
            Transform::from_xyz(0.0, 0.0, 0.0),
            CommsRange(500.0),
        ));
        app.world_mut().spawn((
            EntityUuid(station_uuid.into()),
            Transform::from_xyz(100.0, 0.0, 0.0),
            CommsRange(500.0),
        ));

        let _ = tick(&mut app);
        push_msg(
            &mut app,
            "comms",
            ClientMessage::ControlSystem {
                target: crate::system_registry::comms_system_id(),
                payload: crate::messages::SystemControlPayload::Hail {
                    target_uuid: station_uuid.into(),
                },
            },
        );
        let out = tick(&mut app);

        let (messages, contacts) = out
            .iter()
            .find_map(|m| {
                if let ServerMessage::CommsState {
                    messages, contacts, ..
                } = &m.msg
                {
                    Some((messages.clone(), contacts.clone()))
                } else {
                    None
                }
            })
            .expect("CommsState must be broadcast");

        let contact = contacts
            .iter()
            .find(|c| c.uuid == station_uuid)
            .expect("contact present");
        assert!(contact.in_range, "contact should be in range");
        assert!(
            messages[0].sender_in_range,
            "sender_in_range true when station within range"
        );
    }

    // -- Review fixes: pruning, server enforcement, despawn handling ---------

    /// Contacts whose UUID has no matching `CommsRange`-bearing entity in
    /// the world (e.g. the world TOML names a `[[comms]]` template but the
    /// referenced entity doesn't declare a `[comms]` block) MUST be pruned
    /// before broadcast so they never appear as permanently in-range.
    #[test]
    fn contact_without_comms_range_entity_is_pruned_from_broadcast() {
        use crate::comms::CommsRange;
        use crate::simulation::Ship;

        let bogus_uuid = "no-such-entity";
        let mut app = comms_test_app();
        setup_game_with_comms(&mut app, bogus_uuid);

        // Spawn the ship so range tracking activates, but DO NOT spawn an
        // entity with `bogus_uuid` + CommsRange.
        app.world_mut().spawn((
            Ship,
            crate::simulation::LocalShip,
            Transform::from_xyz(0.0, 0.0, 0.0),
            CommsRange(500.0),
        ));

        let out = tick(&mut app);
        let contacts = out
            .iter()
            .find_map(|m| {
                if let ServerMessage::CommsState { contacts, .. } = &m.msg {
                    Some(contacts.clone())
                } else {
                    None
                }
            })
            .expect("CommsState must be broadcast");

        assert!(
            !contacts.iter().any(|c| c.uuid == bogus_uuid),
            "contact for entity without [comms] block must be pruned, got {contacts:?}"
        );
    }

    /// When a comms-bearing entity is despawned, its `range_flags` entry
    /// must be removed and any inbox message from that sender must be
    /// stamped `sender_in_range = false` on the next broadcast.
    #[test]
    fn entity_despawn_flips_sender_in_range_to_false() {
        use crate::comms::CommsRange;
        use crate::entities::spawner::EntityUuid;
        use crate::simulation::Ship;

        // Use a real UUID4 so the non-UUID synthetic-sender exception introduced
        // for `_self` / "Starcorp Command" does not suppress the range flip.
        let station_uuid = "a1b2c3d4-e5f6-4789-abcd-ef0123456789";
        let mut app = comms_test_app();
        setup_game_with_comms(&mut app, station_uuid);

        app.world_mut().spawn((
            Ship,
            crate::simulation::LocalShip,
            Transform::from_xyz(0.0, 0.0, 0.0),
            CommsRange(1000.0),
        ));
        let station_entity = app
            .world_mut()
            .spawn((
                EntityUuid(station_uuid.into()),
                Transform::from_xyz(50.0, 0.0, 0.0),
                CommsRange(1000.0),
            ))
            .id();
        let _ = tick(&mut app);

        // Hail to populate the inbox while in range.
        push_msg(
            &mut app,
            "comms",
            ClientMessage::ControlSystem {
                target: crate::system_registry::comms_system_id(),
                payload: crate::messages::SystemControlPayload::Hail {
                    target_uuid: station_uuid.into(),
                },
            },
        );
        let _ = tick(&mut app);

        // Now despawn the station entity.
        app.world_mut().despawn(station_entity);
        let out = tick(&mut app);

        let messages = out
            .iter()
            .find_map(|m| {
                if let ServerMessage::CommsState { messages, .. } = &m.msg {
                    Some(messages.clone())
                } else {
                    None
                }
            })
            .expect("a broadcast must fire after despawn (range flip)");

        assert!(
            messages.iter().all(|m| !m.sender_in_range),
            "after despawn, all messages from that sender must have sender_in_range=false: {messages:?}"
        );
    }

    /// Two entities at different distances each get their own flag; flipping
    /// only the closer one's range must not affect the farther one's flag.
    #[test]
    fn multiple_entities_have_independent_range_flags() {
        use crate::comms::CommsRange;
        use crate::entities::spawner::EntityUuid;
        use crate::simulation::Ship;

        let near_uuid = "near-1";
        let far_uuid = "far-1";
        let mut app = comms_test_app();
        setup_game_with_comms(&mut app, near_uuid);
        // Manually add a second contact.
        {
            let runtime = &mut app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime.contacts.push(CommsContact {
                uuid: far_uuid.into(),
                name: "Far".into(),
                in_range: true,
                is_urgent: false,
            });
        }

        app.world_mut().spawn((
            Ship,
            crate::simulation::LocalShip,
            Transform::from_xyz(0.0, 0.0, 0.0),
            CommsRange(500.0),
        ));
        app.world_mut().spawn((
            EntityUuid(near_uuid.into()),
            Transform::from_xyz(100.0, 0.0, 0.0),
            CommsRange(500.0),
        ));
        app.world_mut().spawn((
            EntityUuid(far_uuid.into()),
            Transform::from_xyz(5000.0, 0.0, 0.0),
            CommsRange(500.0),
        ));

        let out = tick(&mut app);
        let contacts = out
            .iter()
            .find_map(|m| {
                if let ServerMessage::CommsState { contacts, .. } = &m.msg {
                    Some(contacts.clone())
                } else {
                    None
                }
            })
            .expect("CommsState must be broadcast");

        let near = contacts
            .iter()
            .find(|c| c.uuid == near_uuid)
            .expect("near contact");
        let far = contacts
            .iter()
            .find(|c| c.uuid == far_uuid)
            .expect("far contact");
        assert!(near.in_range, "near contact must be in range");
        assert!(!far.in_range, "far contact must be out of range");
    }

    /// When a contact flips in_range, a CommsState broadcast must fire even
    /// if the inbox itself is clean.
    #[test]
    fn range_flip_triggers_fresh_broadcast() {
        use crate::comms::CommsRange;
        use crate::entities::spawner::EntityUuid;
        use crate::simulation::Ship;

        let station_uuid = "station-flip";
        let mut app = comms_test_app();
        setup_game_with_comms(&mut app, station_uuid);

        let ship_entity = app
            .world_mut()
            .spawn((
                Ship,
                crate::simulation::LocalShip,
                Transform::from_xyz(0.0, 0.0, 0.0),
                CommsRange(500.0),
            ))
            .id();
        app.world_mut().spawn((
            EntityUuid(station_uuid.into()),
            Transform::from_xyz(100.0, 0.0, 0.0),
            CommsRange(500.0),
        ));

        // Drain initial broadcasts.
        let _ = tick(&mut app);
        let _ = tick(&mut app);

        // Move ship far away Ã¢â‚¬â€ this must trigger a fresh broadcast even
        // though the inbox didn't change.
        if let Ok(mut e) = app.world_mut().get_entity_mut(ship_entity) {
            e.insert(Transform::from_xyz(5000.0, 0.0, 0.0));
        }
        let out = tick(&mut app);

        let has_broadcast = out
            .iter()
            .any(|m| matches!(&m.msg, ServerMessage::CommsState { .. }));
        assert!(
            has_broadcast,
            "range flip from inÃ¢â€ â€™out must trigger a fresh CommsState broadcast"
        );
    }

    /// If the player ship is despawned mid-game (hypothetical hull-zero edge
    /// case), the server must NOT silently re-enable comms by flipping
    /// `range_active` back to false. All tracked flags must be forced to
    /// false so the Hail / Respond gates stay closed.
    #[test]
    fn ship_despawn_mid_game_keeps_gates_closed() {
        use crate::comms::CommsRange;
        use crate::entities::spawner::EntityUuid;
        use crate::simulation::Ship;

        let station_uuid = "station-ship-despawn";
        let mut app = comms_test_app();
        setup_game_with_comms(&mut app, station_uuid);

        let ship_entity = app
            .world_mut()
            .spawn((
                Ship,
                crate::simulation::LocalShip,
                Transform::from_xyz(0.0, 0.0, 0.0),
                CommsRange(1000.0),
            ))
            .id();
        app.world_mut().spawn((
            EntityUuid(station_uuid.into()),
            Transform::from_xyz(100.0, 0.0, 0.0),
            CommsRange(1000.0),
        ));
        let _ = tick(&mut app);

        // Sanity: contact is in range.
        {
            let runtime = app.world().resource::<WorldContentRuntime>();
            assert!(runtime.range_active, "range_active must be true with ship");
            assert_eq!(runtime.range_flags.get(station_uuid).copied(), Some(true));
        }

        // Despawn the ship.
        app.world_mut().despawn(ship_entity);
        let _ = tick(&mut app);

        let runtime = app.world().resource::<WorldContentRuntime>();
        assert!(
            runtime.range_active,
            "range_active must REMAIN true after ship despawn (no back-door)"
        );
        assert_eq!(
            runtime.range_flags.get(station_uuid).copied(),
            Some(false),
            "tracked flag must be forced false on ship despawn"
        );
        assert!(
            runtime.contacts.iter().all(|c| !c.in_range),
            "all contacts must be out of range after ship despawn"
        );
    }

    // -- on_world_loaded (issue #415) ----------------------------------------

    /// `handle_ai_events` drains `pending_world_events` and dispatches their
    /// matching triggers' actions. Seeds a `WorldLoaded` event directly into
    /// the queue and asserts the `add_objective` action fires.
    #[test]
    fn pending_world_loaded_event_fires_on_world_loaded_trigger() {
        let mut app = ai_trigger_test_app();
        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime.trigger_states = vec![TriggerState {
                trigger: crate::world::content::Trigger {
                    condition: TriggerCondition::OnWorldLoaded,
                    actions: vec![TriggerAction::AddObjective {
                        id: "obj-loaded".into(),
                        text: "World loaded.".into(),
                        mandatory: false,
                        targets: vec![],
                        directive: crate::messages::AiDirective::None,
                        utility: crate::objectives::UtilityConfig::default(),
                        source: crate::messages::ObjectiveSource::default(),
                    }],
                    when: None,
                },
                fired: false,
                origin_layer: None,
                seen_destroyed: HashSet::new(),
            }];
            runtime.pending_world_events.push(WorldEvent::WorldLoaded);
        }

        app.update();

        let objectives = &app.world().resource::<ObjectiveManagerRes>().0;
        assert!(
            objectives
                .sorted_snapshots()
                .iter()
                .any(|o| o.id == "obj-loaded"),
            "on_world_loaded trigger must have fired its add_objective action"
        );
        // Queue must be drained.
        let runtime = app.world().resource::<WorldContentRuntime>();
        assert!(
            runtime.pending_world_events.is_empty(),
            "pending_world_events must be drained by handle_ai_events"
        );
        assert!(
            runtime.trigger_states[0].fired,
            "trigger must be marked fired"
        );
    }

    /// Base-world Startup: `init_world_runtime` must push a `WorldLoaded`
    /// event onto `pending_world_events` so any `on_world_loaded` triggers
    /// declared in the base world fire on the first Update tick.
    #[test]
    fn init_world_runtime_queues_world_loaded_event() {
        let mut app = App::new();
        app.add_plugins(bevy::asset::AssetPlugin::default())
            .init_asset::<Mesh>()
            .init_asset::<StandardMaterial>()
            .init_resource::<WorldResource>()
            .add_plugins(WorldPlugin);

        // Insert a WorldConfig so init_world_runtime takes its non-no-op path.
        let mut cfg = crate::world::config::WorldConfig::default();
        cfg.triggers.push(crate::world::content::Trigger {
            condition: TriggerCondition::OnWorldLoaded,
            actions: vec![TriggerAction::AddObjective {
                id: "obj-startup".into(),
                text: "Startup objective.".into(),
                mandatory: false,
                targets: vec![],
                directive: crate::messages::AiDirective::None,
                utility: crate::objectives::UtilityConfig::default(),
                source: crate::messages::ObjectiveSource::default(),
            }],
            when: None,
        });
        app.insert_resource(cfg);

        app.world_mut().run_schedule(Startup);

        let runtime = app.world().resource::<WorldContentRuntime>();
        assert!(
            runtime
                .pending_world_events
                .iter()
                .any(|e| matches!(e, WorldEvent::WorldLoaded)),
            "init_world_runtime must queue a WorldLoaded event during Startup"
        );
        assert_eq!(
            runtime.trigger_states.len(),
            1,
            "trigger states must be populated"
        );
    }

    /// Sub-world `LoadWorld` must push a `WorldLoaded` event so any
    /// `on_world_loaded` triggers declared in the loaded sub-world fire
    /// after the load merges.
    #[test]
    fn apply_world_layer_changes_queues_world_loaded_event_on_load() {
        let world_path = write_on_world_loaded_layer_fixture();

        let mut app = App::new();
        app.add_plugins(bevy::time::TimePlugin)
            .init_resource::<WorldLayerMap>()
            .init_resource::<WorldContentRuntime>()
            .init_resource::<PendingWorldLayerChanges>()
            .add_systems(Update, apply_world_layer_changes);

        app.world_mut()
            .resource_mut::<PendingWorldLayerChanges>()
            .0
            .push(WorldLayerChange::Load {
                path: world_path.clone(),
                loader_path: None,
            });
        app.update();

        let runtime = app.world().resource::<WorldContentRuntime>();
        assert!(
            runtime
                .pending_world_events
                .iter()
                .any(|e| matches!(e, WorldEvent::WorldLoaded)),
            "apply_world_layer_changes Load branch must queue a WorldLoaded event"
        );
    }

    /// End-to-end: load a sub-world with an `on_world_loaded` trigger,
    /// unload it, then re-load it. The trigger must fire on both load
    /// cycles (proves `fired` is reset because the trigger state is
    /// recreated fresh on re-load).
    #[test]
    fn on_world_loaded_fires_again_after_unload_and_reload() {
        let world_path = write_on_world_loaded_layer_fixture();

        let mut app = ai_trigger_test_app();
        app.init_resource::<WorldLayerMap>()
            .init_resource::<PendingWorldLayerChanges>()
            .add_systems(Update, apply_world_layer_changes);

        // -- First load cycle --
        app.world_mut()
            .resource_mut::<PendingWorldLayerChanges>()
            .0
            .push(WorldLayerChange::Load {
                path: world_path.clone(),
                loader_path: None,
            });
        app.update(); // applies load + queues WorldLoaded
        app.update(); // handle_ai_events drains pending event + fires trigger

        {
            let objectives = &app.world().resource::<ObjectiveManagerRes>().0;
            assert!(
                objectives
                    .sorted_snapshots()
                    .iter()
                    .any(|o| o.id == "obj-on-load"),
                "on_world_loaded trigger must fire on first load"
            );
        }

        // Complete the objective so we can detect the second add as a
        // distinct event (ObjectiveManager dedupes by id; re-adding the
        // same id leaves the existing objective in place which is fine Ã¢â‚¬â€
        // we instead assert the trigger's `fired` flag flips back to true
        // after re-load).
        // -- Unload --
        app.world_mut()
            .resource_mut::<PendingWorldLayerChanges>()
            .0
            .push(WorldLayerChange::Unload(world_path.clone()));
        app.update();
        app.update();

        {
            let runtime = app.world().resource::<WorldContentRuntime>();
            assert!(
                !runtime
                    .trigger_states
                    .iter()
                    .any(|s| matches!(s.trigger.condition, TriggerCondition::OnWorldLoaded)),
                "Unload must remove the on_world_loaded trigger state"
            );
        }

        // -- Second load cycle --
        app.world_mut()
            .resource_mut::<PendingWorldLayerChanges>()
            .0
            .push(WorldLayerChange::Load {
                path: world_path.clone(),
                loader_path: None,
            });
        app.update(); // applies load + queues WorldLoaded
        app.update(); // drain + dispatch

        let runtime = app.world().resource::<WorldContentRuntime>();
        let reloaded_trigger = runtime
            .trigger_states
            .iter()
            .find(|s| matches!(s.trigger.condition, TriggerCondition::OnWorldLoaded))
            .expect("on_world_loaded trigger must be re-registered on re-load");
        assert!(
            reloaded_trigger.fired,
            "on_world_loaded trigger must fire again on re-load \
             (proves fired flag is reset via fresh TriggerState on Load)"
        );
    }

    /// Writes a tiny world TOML containing exactly one `on_world_loaded`
    /// trigger to a temp file. Returns the path. Each call uses a unique
    /// path so parallel test runs do not collide.
    fn write_on_world_loaded_layer_fixture() -> String {
        use std::sync::atomic::{AtomicU32, Ordering};
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let tmp = std::env::temp_dir();
        let tag = COUNTER.fetch_add(1, Ordering::Relaxed);
        let world_path = tmp.join(format!("on_world_loaded_{tag}.toml"));
        let toml = r#"
[global]
seed = 1

[[trigger]]
condition = "on_world_loaded"

  [[trigger.action]]
  type = "add_objective"
  id   = "obj-on-load"
  text = "Loaded."
  mandatory = false
"#;
        std::fs::write(&world_path, toml).expect("failed to write fixture world TOML");
        world_path.to_string_lossy().into_owned()
    }

    // -- Region enter/exit triggers (issue #416) -----------------------------

    use crate::entity_config::EntityConfig;
    use crate::entity_spawner::{spawn_entity, EntityUuid};
    use crate::region_shape::RegionShape;
    use crate::regions::server::RegionPlugin;

    /// Build a minimal app that wires `RegionPlugin` + the issue-#416
    /// observers + `handle_ai_events` into the same world. Skips the
    /// heavyweight `WorldPlugin`/`AiPlugin`/`LobbyPlugin` bootstrap so the
    /// test focuses on the region-event Ã¢â€ â€™ trigger-fire path.
    fn region_trigger_test_app() -> App {
        let mut app = App::new();
        app.add_plugins(bevy::time::TimePlugin)
            .add_plugins(RegionPlugin)
            .init_resource::<WorldContentRuntime>()
            .init_resource::<CommsInboxRes>()
            .init_resource::<ObjectiveManagerRes>()
            .init_resource::<SimOutbox>()
            .add_message::<crate::ai::server::AiEntityAttacked>()
            .add_message::<crate::ai::server::AiEntityDestroyed>()
            .add_message::<CommsChannel2Event>()
            .add_systems(Update, (handle_ai_events, handle_comms_channel2).chain())
            .add_observer(handle_region_entered_event)
            .add_observer(handle_region_exited_event);
        // Spawn the player ship (with a Transform so RegionPlugin's
        // membership query succeeds).
        app.world_mut().spawn((
            crate::simulation::Ship,
            crate::simulation::LocalShip,
            Transform::default(),
            crate::ship_state::ShipPhysics::default(),
            crate::ship_plugin::ShipConfigComponent::default(),
            crate::ship_plugin::ShipSystemControlSources::default(),
            crate::modifiers::ShipModifiers::new(),
        ));
        app
    }

    fn spawn_region_with_uuid(app: &mut App, x: f32, z: f32, radius: f32, uuid: &str) -> Entity {
        let config = EntityConfig {
            name: None,
            star: None,
            class: None,
            hull_id: None,
            power_rating: None,
            css: None,
            light: Vec::new(),
            ship_config: None,
            shield_arcs: Vec::new(),
            tags: vec!["region".to_string()],
            shape: Some(RegionShape::Sphere { radius }),
            effects: None,
            hull: None,
            collider: None,
            appearance: None,
            helm_console: None,
            weapons_console: None,
            engineering_console: None,
            captain_console: None,
            power: None,
            sensors_console: None,
            navigation_console: None,
            shields_console: None,
            torpedoes: None,
            repair: None,
            comms: None,
            asteroid_field: None,
            faction: None,
            behaviour: None,
            radar_appearance: None,
            mesh: None,
            target: None,
        };
        let mut commands = app.world_mut().commands();
        spawn_entity(
            &mut commands,
            &config,
            Vec3::new(x, 0.0, z),
            uuid.to_string(),
            None,
        )
    }

    fn set_ship_pos(app: &mut App, x: f32, z: f32) {
        let mut q = app
            .world_mut()
            .query_filtered::<&mut crate::ship_state::ShipPhysics, With<crate::simulation::LocalShip>>();
        if let Ok(mut p) = q.single_mut(app.world_mut()) {
            p.x = x;
            p.z = z;
        } else {
            // Fallback: the world test_app may not have a LocalShip entity yet.
            // No-op; world tests that call set_ship_pos must ensure a LocalShip entity exists.
        }
    }

    fn install_region_trigger(
        app: &mut App,
        name: &str,
        uuid: &str,
        condition: TriggerCondition,
        obj_id: &str,
    ) {
        let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
        runtime.name_to_uuid.insert(name.into(), uuid.into());
        runtime.trigger_states.push(TriggerState {
            trigger: crate::world::content::Trigger {
                condition,
                actions: vec![TriggerAction::AddObjective {
                    id: obj_id.into(),
                    text: "region trigger objective".into(),
                    mandatory: false,
                    targets: vec![],
                    directive: crate::messages::AiDirective::None,
                    utility: crate::objectives::UtilityConfig::default(),
                    source: crate::messages::ObjectiveSource::default(),
                }],
                when: None,
            },
            fired: false,
            origin_layer: None,
            seen_destroyed: HashSet::new(),
        });
    }

    fn objective_present(app: &App, id: &str) -> bool {
        app.world()
            .resource::<ObjectiveManagerRes>()
            .0
            .sorted_snapshots()
            .iter()
            .any(|o| o.id == id)
    }

    #[test]
    fn ship_entering_region_fires_on_entered_region_trigger_exactly_once() {
        let mut app = region_trigger_test_app();
        let uuid = "uuid-nebula";
        spawn_region_with_uuid(&mut app, 100.0, 0.0, 50.0, uuid);
        install_region_trigger(
            &mut app,
            "nebula",
            uuid,
            TriggerCondition::OnEnteredRegion {
                entity_name: "nebula".into(),
            },
            "obj-entered",
        );

        // Tick 1: ship outside (at origin), no enter Ã¢â€ â€™ no fire.
        app.update();
        assert!(
            !objective_present(&app, "obj-entered"),
            "trigger must not fire while outside"
        );

        // Move ship inside. The membership system runs in Physics and
        // queues a WorldEvent via the observer; `handle_ai_events` (also
        // in Physics) drains the queue on the NEXT tick Ã¢â‚¬â€ matching the
        // documented `WorldLoaded` two-tick pattern.
        set_ship_pos(&mut app, 110.0, 0.0);
        app.update(); // queues EnteredRegion
        app.update(); // handle_ai_events drains + fires
        assert!(
            objective_present(&app, "obj-entered"),
            "trigger must fire on entry"
        );

        // Confirm single-shot: trigger is marked fired, queue is drained.
        let runtime = app.world().resource::<WorldContentRuntime>();
        assert!(
            runtime.trigger_states[0].fired,
            "trigger state must be marked fired after entry"
        );
        assert!(
            runtime.pending_world_events.is_empty(),
            "pending_world_events must be drained"
        );

        // Stay inside on subsequent ticks Ã¢â‚¬â€ membership system must not
        // re-emit `RegionEntered`, so no new events queue up.
        app.update();
        app.update();
        let runtime = app.world().resource::<WorldContentRuntime>();
        assert!(
            runtime.pending_world_events.is_empty(),
            "staying inside must not enqueue further EnteredRegion events"
        );
    }

    #[test]
    fn ship_exiting_region_fires_on_exited_region_trigger() {
        let mut app = region_trigger_test_app();
        let uuid = "uuid-nebula";
        spawn_region_with_uuid(&mut app, 0.0, 0.0, 50.0, uuid);
        install_region_trigger(
            &mut app,
            "nebula",
            uuid,
            TriggerCondition::OnExitedRegion {
                entity_name: "nebula".into(),
            },
            "obj-exited",
        );

        // Move inside first so we enter cleanly.
        set_ship_pos(&mut app, 10.0, 0.0);
        app.update();
        app.update();
        assert!(
            !objective_present(&app, "obj-exited"),
            "exit trigger must not fire on entry"
        );

        // Now move outside Ã¢â€ â€™ RegionExited Ã¢â€ â€™ queued Ã¢â€ â€™ drained next tick.
        set_ship_pos(&mut app, 200.0, 0.0);
        app.update();
        app.update();
        assert!(
            objective_present(&app, "obj-exited"),
            "exit trigger must fire when ship moves outside the region"
        );
    }

    #[test]
    fn region_despawn_while_ship_inside_fires_on_exited_region_trigger() {
        let mut app = region_trigger_test_app();
        let uuid = "uuid-fragile";
        let region_entity = spawn_region_with_uuid(&mut app, 0.0, 0.0, 50.0, uuid);
        install_region_trigger(
            &mut app,
            "fragile",
            uuid,
            TriggerCondition::OnExitedRegion {
                entity_name: "fragile".into(),
            },
            "obj-imploded",
        );

        // Enter the region.
        set_ship_pos(&mut app, 10.0, 0.0);
        app.update();
        app.update();
        assert!(!objective_present(&app, "obj-imploded"));

        // Despawn the region while ship is inside Ã¢â‚¬â€ membership system
        // emits an implicit RegionExited.
        app.world_mut().despawn(region_entity);
        app.update(); // queues ExitedRegion
        app.update(); // drains + fires

        assert!(
            objective_present(&app, "obj-imploded"),
            "exit trigger must fire when the region is despawned while ship is inside"
        );
    }

    #[test]
    fn overlapping_regions_fire_independent_enter_triggers() {
        let mut app = region_trigger_test_app();
        let uuid_a = "uuid-region-a";
        let uuid_b = "uuid-region-b";
        // Both regions cover the origin.
        spawn_region_with_uuid(&mut app, 0.0, 0.0, 80.0, uuid_a);
        spawn_region_with_uuid(&mut app, 20.0, 0.0, 80.0, uuid_b);
        install_region_trigger(
            &mut app,
            "region_a",
            uuid_a,
            TriggerCondition::OnEnteredRegion {
                entity_name: "region_a".into(),
            },
            "obj-a",
        );
        install_region_trigger(
            &mut app,
            "region_b",
            uuid_b,
            TriggerCondition::OnEnteredRegion {
                entity_name: "region_b".into(),
            },
            "obj-b",
        );

        // Ship at origin is inside both regions. First tick queues both
        // events, second tick drains + fires both triggers.
        set_ship_pos(&mut app, 0.0, 0.0);
        app.update();
        app.update();

        assert!(
            objective_present(&app, "obj-a"),
            "region A enter trigger must fire"
        );
        assert!(
            objective_present(&app, "obj-b"),
            "region B enter trigger must fire"
        );
    }

    #[test]
    fn npc_entering_region_does_not_fire_trigger() {
        // Region membership is tracked per-ship (PRD #597 PR 9), so an NPC ship
        // does now enter `RegionMembership.inside` when it crosses a region
        // boundary. World-scenario triggers, however, remain player-driven:
        // `handle_region_entered_event` filters on `LocalShip`, so an NPC
        // crossing does not fire an `OnEnteredRegion` trigger.
        //
        // This test uses a bare entity (no `Ship` marker) to keep the setup
        // narrow. See `npc_ship_in_damage_zone_takes_hull_damage` in
        // `src/regions/server.rs` for the equivalent test with a full NPC ship.
        let mut app = region_trigger_test_app();
        let uuid = "uuid-quarantine";
        // Region at (100, 0); player ship stays at origin (outside).
        spawn_region_with_uuid(&mut app, 100.0, 0.0, 50.0, uuid);
        install_region_trigger(
            &mut app,
            "quarantine",
            uuid,
            TriggerCondition::OnEnteredRegion {
                entity_name: "quarantine".into(),
            },
            "obj-ship-quarantined",
        );

        // Spawn an "NPC" entity inside the region by placing a generic
        // entity (no Ship marker) at (110, 0). The membership system
        // ignores it because the only Ship is the player ship at origin.
        let npc_config = EntityConfig {
            name: None,
            star: None,
            class: None,
            hull_id: None,
            power_rating: None,
            css: None,
            light: Vec::new(),
            ship_config: None,
            shield_arcs: Vec::new(),
            tags: vec!["npc".into()],
            shape: None,
            effects: None,
            hull: None,
            collider: None,
            appearance: None,
            helm_console: None,
            weapons_console: None,
            engineering_console: None,
            captain_console: None,
            power: None,
            sensors_console: None,
            navigation_console: None,
            shields_console: None,
            torpedoes: None,
            repair: None,
            comms: None,
            asteroid_field: None,
            faction: None,
            behaviour: None,
            radar_appearance: None,
            mesh: None,
            target: None,
        };
        {
            let mut commands = app.world_mut().commands();
            let _npc = spawn_entity(
                &mut commands,
                &npc_config,
                Vec3::new(110.0, 0.0, 0.0),
                "uuid-npc".into(),
                None,
            );
        }

        // Tick a few times; player ship stays at origin (outside).
        app.update();
        app.update();

        assert!(
            !objective_present(&app, "obj-ship-quarantined"),
            "NPC entering the region must not fire the player-ship trigger"
        );
        let runtime = app.world().resource::<WorldContentRuntime>();
        assert!(
            !runtime.trigger_states[0].fired,
            "trigger must remain unfired when only an NPC is inside"
        );
    }

    #[test]
    fn on_entered_region_trigger_with_when_filter_obeys_predicate() {
        let mut app = region_trigger_test_app();
        let uuid = "uuid-zone";
        spawn_region_with_uuid(&mut app, 0.0, 0.0, 50.0, uuid);

        // Install a trigger gated by `flag(armed)`.
        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime.name_to_uuid.insert("zone".into(), uuid.into());
            runtime.trigger_states.push(TriggerState {
                trigger: crate::world::content::Trigger {
                    condition: TriggerCondition::OnEnteredRegion {
                        entity_name: "zone".into(),
                    },
                    actions: vec![TriggerAction::AddObjective {
                        id: "obj-armed-entry".into(),
                        text: "Armed entry.".into(),
                        mandatory: false,
                        targets: vec![],
                        directive: crate::messages::AiDirective::None,
                        utility: crate::objectives::UtilityConfig::default(),
                        source: crate::messages::ObjectiveSource::default(),
                    }],
                    when: Some(crate::world::flags::parse_predicate("flag(armed)").unwrap()),
                },
                fired: false,
                origin_layer: None,
                seen_destroyed: HashSet::new(),
            });
        }

        // First entry: flag unset Ã¢â€ â€™ predicate false Ã¢â€ â€™ no objective.
        set_ship_pos(&mut app, 10.0, 0.0);
        app.update();
        assert!(
            !objective_present(&app, "obj-armed-entry"),
            "gated trigger must not fire while flag is unset"
        );
        {
            let runtime = app.world().resource::<WorldContentRuntime>();
            assert!(
                !runtime.trigger_states[0].fired,
                "predicate-false firings must NOT consume the trigger"
            );
        }

        // Set the flag, leave the region, re-enter Ã¢â‚¬â€ trigger should fire now.
        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime.flags.set_flag("armed");
        }
        set_ship_pos(&mut app, 200.0, 0.0); // exit
        app.update();
        set_ship_pos(&mut app, 10.0, 0.0); // re-enter
        app.update();

        assert!(
            objective_present(&app, "obj-armed-entry"),
            "gated trigger must fire once the flag is set and ship re-enters"
        );
    }

    // -- Issue #417: spawn_entity / destroy_entity trigger actions ---------

    /// Writes a minimal NPC template to a temp file and returns its path.
    pub(crate) fn write_spawn_template_fixture() -> String {
        use std::sync::atomic::{AtomicU32, Ordering};
        static C: AtomicU32 = AtomicU32::new(0);
        let tag = C.fetch_add(1, Ordering::Relaxed);
        let p = std::env::temp_dir().join(format!("spawn_action_template_{tag}.toml"));
        std::fs::write(
            &p,
            r##"
tags = ["npc"]

[appearance]
colour = "#ff8800"
size_min = 1.0
size_max = 2.0
"##,
        )
        .expect("write template");
        p.to_string_lossy().into_owned()
    }

    /// SpawnEntity action with an explicit `position` spawns a new entity into
    /// the ECS, registers it in `name_to_uuid`, and a follow-up DestroyEntity
    /// removes it again.
    #[test]
    fn spawn_entity_action_with_position_spawns_and_registers_uuid() {
        use crate::entities::spawner::EntityUuid;

        let template_path = write_spawn_template_fixture();
        let mut app = ai_trigger_test_app();

        // Trigger fires on attack of a pre-registered marker.
        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime
                .name_to_uuid
                .insert("marker".to_string(), "marker-uuid".to_string());
            runtime.trigger_states = vec![TriggerState {
                trigger: crate::world::content::Trigger {
                    condition: TriggerCondition::OnAttacked {
                        entity_name: "marker".to_string(),
                    },
                    actions: vec![TriggerAction::SpawnEntity {
                        template_path: template_path.clone(),
                        name: "spawned_one".to_string(),
                        anchor: None,
                        position: Some([7.0, 0.0, 3.0]),
                        rotation: None,
                        scale: None,
                    }],
                    when: None,
                },
                fired: false,
                origin_layer: None,
                seen_destroyed: HashSet::new(),
            }];
        }

        app.world_mut()
            .resource_mut::<Messages<crate::ai_plugin::AiEntityAttacked>>()
            .write(crate::ai_plugin::AiEntityAttacked {
                entity_uuid: "marker-uuid".into(),
                attacker_uuid: uuid::Uuid::parse_str("cccccccc-0000-0000-0000-000000000001")
                    .unwrap(),
            });

        app.update();
        app.update();

        // name_to_uuid must contain the new entity.
        let uuid = app
            .world()
            .resource::<WorldContentRuntime>()
            .name_to_uuid
            .get("spawned_one")
            .cloned();
        assert!(uuid.is_some(), "SpawnEntity must register name_to_uuid");

        // The ECS must contain an entity with that UUID.
        let uuid_value = uuid.unwrap();
        let mut found = false;
        let mut q = app
            .world_mut()
            .query::<(&EntityUuid, &bevy::prelude::Transform)>();
        for (eu, t) in q.iter(app.world()) {
            if eu.0 == uuid_value {
                found = true;
                assert!((t.translation.x - 7.0).abs() < 1e-3);
                assert!((t.translation.z - 3.0).abs() < 1e-3);
            }
        }
        assert!(found, "spawned entity with the new UUID must exist in ECS");
    }

    /// SpawnEntity action with an `anchor` resolves the anchor against the
    /// base-world `WorldConfig` resource.
    #[test]
    fn spawn_entity_action_with_anchor_resolves_against_base_world() {
        use crate::entities::spawner::EntityUuid;

        let template_path = write_spawn_template_fixture();
        let mut app = ai_trigger_test_app();

        // Insert a WorldConfig with a known anchor.
        let mut wc = crate::world::config::WorldConfig::default();
        wc.anchors.insert("alpha".to_string(), [42.0, 0.0, -5.0]);
        app.world_mut().insert_resource(wc);

        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime
                .name_to_uuid
                .insert("trigger_src".to_string(), "src-uuid".to_string());
            runtime.trigger_states = vec![TriggerState {
                trigger: crate::world::content::Trigger {
                    condition: TriggerCondition::OnDestroyed {
                        entity_name: "trigger_src".to_string(),
                    },
                    actions: vec![TriggerAction::SpawnEntity {
                        template_path: template_path.clone(),
                        name: "anchor_spawn".to_string(),
                        anchor: Some("alpha".to_string()),
                        position: None,
                        rotation: None,
                        scale: None,
                    }],
                    when: None,
                },
                fired: false,
                origin_layer: None,
                seen_destroyed: HashSet::new(),
            }];
        }

        app.world_mut()
            .resource_mut::<Messages<crate::ai_plugin::AiEntityDestroyed>>()
            .write(crate::ai_plugin::AiEntityDestroyed {
                entity_uuid: "src-uuid".into(),
            });

        app.update();
        app.update();

        let uuid = app
            .world()
            .resource::<WorldContentRuntime>()
            .name_to_uuid
            .get("anchor_spawn")
            .cloned()
            .expect("anchor spawn must register name_to_uuid");

        let mut q = app
            .world_mut()
            .query::<(&EntityUuid, &bevy::prelude::Transform)>();
        let mut found = false;
        for (eu, t) in q.iter(app.world()) {
            if eu.0 == uuid {
                found = true;
                assert!((t.translation.x - 42.0).abs() < 1e-3);
                assert!((t.translation.z - -5.0).abs() < 1e-3);
            }
        }
        assert!(found, "anchor-resolved spawn must exist at anchor coords");
    }

    /// SpawnEntity action with an `anchor` resolves against the originating
    /// sub-world layer's anchor table (not the base world).
    #[test]
    fn spawn_entity_action_resolves_anchor_against_origin_layer() {
        use crate::entities::spawner::EntityUuid;

        let template_path = write_spawn_template_fixture();
        let mut app = ai_trigger_test_app();
        app.init_resource::<WorldLayerMap>();

        // Register a fake layer with a custom anchor.
        let layer_path = "fake_layer_path.toml".to_string();
        {
            let mut lm = app.world_mut().resource_mut::<WorldLayerMap>();
            let mut wr = WorldRuntime::default();
            wr.anchors
                .insert("docking_bay".to_string(), [11.0, 0.0, 22.0]);
            lm.0.insert(layer_path.clone(), wr);
        }

        // Also seed a different anchor with the same name in the base world to
        // prove the layer-local one wins.
        let mut wc = crate::world::config::WorldConfig::default();
        wc.anchors
            .insert("docking_bay".to_string(), [-99.0, 0.0, -99.0]);
        app.world_mut().insert_resource(wc);

        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime
                .name_to_uuid
                .insert("layer_trigger".to_string(), "lt-uuid".to_string());
            runtime.trigger_states = vec![TriggerState {
                trigger: crate::world::content::Trigger {
                    condition: TriggerCondition::OnAttacked {
                        entity_name: "layer_trigger".to_string(),
                    },
                    actions: vec![TriggerAction::SpawnEntity {
                        template_path: template_path.clone(),
                        name: "layer_spawn".to_string(),
                        anchor: Some("docking_bay".to_string()),
                        position: None,
                        rotation: None,
                        scale: None,
                    }],
                    when: None,
                },
                fired: false,
                origin_layer: Some(layer_path.clone()),
                seen_destroyed: HashSet::new(),
            }];
        }

        app.world_mut()
            .resource_mut::<Messages<crate::ai_plugin::AiEntityAttacked>>()
            .write(crate::ai_plugin::AiEntityAttacked {
                entity_uuid: "lt-uuid".into(),
                attacker_uuid: uuid::Uuid::parse_str("dddddddd-0000-0000-0000-000000000001")
                    .unwrap(),
            });

        app.update();
        app.update();

        // The layer's spawned_entities list must now have the new entity, and
        // that entity must be at the layer-local anchor.
        let layer = app
            .world()
            .resource::<WorldLayerMap>()
            .0
            .get(&layer_path)
            .expect("layer present")
            .clone();
        assert_eq!(
            layer.spawned_entities.len(),
            1,
            "layer-originated SpawnEntity must attach to the parent layer for cascade unload"
        );

        let uuid = app
            .world()
            .resource::<WorldContentRuntime>()
            .name_to_uuid
            .get("layer_spawn")
            .cloned()
            .expect("layer spawn must register name_to_uuid");
        let mut q = app
            .world_mut()
            .query::<(&EntityUuid, &bevy::prelude::Transform)>();
        let mut found = false;
        for (eu, t) in q.iter(app.world()) {
            if eu.0 == uuid {
                found = true;
                assert!(
                    (t.translation.x - 11.0).abs() < 1e-3,
                    "must use layer anchor (11), not base anchor (-99); got {}",
                    t.translation.x
                );
                assert!((t.translation.z - 22.0).abs() < 1e-3);
            }
        }
        assert!(found);
    }

    /// DestroyEntity action despawns the target entity and emits a destroyed
    /// world event that subsequent triggers can chain on.
    #[test]
    fn destroy_entity_action_despawns_and_emits_chained_event() {
        use crate::entities::spawner::EntityUuid;

        let mut app = ai_trigger_test_app();

        // Spawn a target entity with a known UUID.
        let target_uuid = "doomed-uuid";
        let target_entity = app
            .world_mut()
            .spawn((
                EntityUuid(target_uuid.into()),
                bevy::prelude::Transform::from_xyz(0.0, 0.0, 0.0),
            ))
            .id();

        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime
                .name_to_uuid
                .insert("doomed".to_string(), target_uuid.into());
            runtime
                .name_to_uuid
                .insert("witness".to_string(), "src-uuid".to_string());
            // First trigger: on attack Ã¢â€ â€™ destroy.
            // Second trigger: on destroyed of "doomed" Ã¢â€ â€™ add objective (proves chaining).
            runtime.trigger_states = vec![
                TriggerState {
                    trigger: crate::world::content::Trigger {
                        condition: TriggerCondition::OnAttacked {
                            entity_name: "witness".into(),
                        },
                        actions: vec![TriggerAction::DestroyEntity {
                            entity: "doomed".into(),
                        }],
                        when: None,
                    },
                    fired: false,
                    origin_layer: None,
                    seen_destroyed: HashSet::new(),
                },
                TriggerState {
                    trigger: crate::world::content::Trigger {
                        condition: TriggerCondition::OnDestroyed {
                            entity_name: "doomed".into(),
                        },
                        actions: vec![TriggerAction::AddObjective {
                            id: "obj-chained".into(),
                            text: "chained".into(),
                            mandatory: false,
                            targets: vec![],
                            directive: crate::messages::AiDirective::None,
                            utility: crate::objectives::UtilityConfig::default(),
                            source: crate::messages::ObjectiveSource::default(),
                        }],
                        when: None,
                    },
                    fired: false,
                    origin_layer: None,
                    seen_destroyed: HashSet::new(),
                },
            ];
        }

        app.world_mut()
            .resource_mut::<Messages<crate::ai_plugin::AiEntityAttacked>>()
            .write(crate::ai_plugin::AiEntityAttacked {
                entity_uuid: "src-uuid".into(),
                attacker_uuid: uuid::Uuid::parse_str("eeeeeeee-0000-0000-0000-000000000001")
                    .unwrap(),
            });

        app.update();
        app.update();

        // Target entity must be gone.
        assert!(
            app.world().get_entity(target_entity).is_err(),
            "DestroyEntity must despawn the target entity"
        );

        // Chained on_destroyed trigger must have fired (objective added).
        let objs = &app.world().resource::<ObjectiveManagerRes>().0;
        assert!(
            objs.sorted_snapshots()
                .iter()
                .any(|o| o.id == "obj-chained"),
            "chained on_destroyed trigger must fire from DestroyEntity action"
        );

        // External consumers must also see the message: DestroyEntity action
        // must emit AiEntityDestroyed via the deferred Commands::queue path,
        // matching combat-induced destruction.
        let msgs = app
            .world()
            .resource::<Messages<crate::ai_plugin::AiEntityDestroyed>>();
        let mut cursor = msgs.get_cursor();
        let emitted: Vec<String> = cursor.read(msgs).map(|m| m.entity_uuid.clone()).collect();
        assert!(
            emitted.iter().any(|u| u == target_uuid),
            "DestroyEntity action must emit AiEntityDestroyed for '{target_uuid}', got {emitted:?}"
        );
    }

    /// DestroyEntity with an unknown entity name is a warned no-op (no panic,
    /// no objective from a chained trigger).
    #[test]
    fn destroy_entity_action_with_unknown_name_is_noop() {
        let mut app = ai_trigger_test_app();
        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime
                .name_to_uuid
                .insert("src".to_string(), "src-uuid".to_string());
            runtime.trigger_states = vec![TriggerState {
                trigger: crate::world::content::Trigger {
                    condition: TriggerCondition::OnAttacked {
                        entity_name: "src".into(),
                    },
                    actions: vec![TriggerAction::DestroyEntity {
                        entity: "does_not_exist".into(),
                    }],
                    when: None,
                },
                fired: false,
                origin_layer: None,
                seen_destroyed: HashSet::new(),
            }];
        }
        app.world_mut()
            .resource_mut::<Messages<crate::ai_plugin::AiEntityAttacked>>()
            .write(crate::ai_plugin::AiEntityAttacked {
                entity_uuid: "src-uuid".into(),
                attacker_uuid: uuid::Uuid::parse_str("ffffffff-0000-0000-0000-000000000001")
                    .unwrap(),
            });
        app.update();
        // No assertion needed beyond "does not panic". Verify objectives empty.
        let objs = &app.world().resource::<ObjectiveManagerRes>().0;
        assert!(objs.sorted_snapshots().is_empty());
    }

    /// UnloadWorld cascades through entities spawned by a layer-origin
    /// SpawnEntity action (not just those spawned at LoadWorld time).
    #[test]
    fn unload_world_cascades_through_spawn_entity_action_results() {
        use crate::entities::spawner::EntityUuid;

        let template_path = write_spawn_template_fixture();
        let mut app = ai_trigger_test_app();
        app.init_resource::<WorldLayerMap>();

        let layer_path = "cascade_layer.toml".to_string();
        {
            let mut lm = app.world_mut().resource_mut::<WorldLayerMap>();
            lm.0.insert(layer_path.clone(), WorldRuntime::default());
        }

        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime
                .name_to_uuid
                .insert("src".to_string(), "src-uuid".to_string());
            runtime.trigger_states = vec![TriggerState {
                trigger: crate::world::content::Trigger {
                    condition: TriggerCondition::OnAttacked {
                        entity_name: "src".into(),
                    },
                    actions: vec![TriggerAction::SpawnEntity {
                        template_path: template_path.clone(),
                        name: "cascade_me".into(),
                        anchor: None,
                        position: Some([1.0, 0.0, 1.0]),
                        rotation: None,
                        scale: None,
                    }],
                    when: None,
                },
                fired: false,
                origin_layer: Some(layer_path.clone()),
                seen_destroyed: HashSet::new(),
            }];
        }

        app.world_mut()
            .resource_mut::<Messages<crate::ai_plugin::AiEntityAttacked>>()
            .write(crate::ai_plugin::AiEntityAttacked {
                entity_uuid: "src-uuid".into(),
                attacker_uuid: uuid::Uuid::parse_str("10101010-0000-0000-0000-000000000001")
                    .unwrap(),
            });

        app.update();
        app.update();

        let spawned: Vec<bevy::prelude::Entity> = app
            .world()
            .resource::<WorldLayerMap>()
            .0
            .get(&layer_path)
            .expect("layer present")
            .spawned_entities
            .clone();
        assert_eq!(spawned.len(), 1, "spawn must attach to the layer entry");

        // Now register unload handler + drive an UnloadWorld through it.
        app.add_systems(Update, apply_world_layer_changes);
        app.init_resource::<PendingWorldLayerChanges>();
        app.world_mut()
            .resource_mut::<PendingWorldLayerChanges>()
            .0
            .push(WorldLayerChange::Unload(layer_path.clone()));
        app.update();
        app.update();

        for e in spawned {
            assert!(
                app.world().get_entity(e).is_err(),
                "entity from SpawnEntity action must be despawned by UnloadWorld"
            );
        }
        let _ = std::any::type_name::<EntityUuid>(); // touch import
    }

    /// `when` predicate gates SpawnEntity just like every other action.
    #[test]
    fn spawn_entity_action_respects_when_predicate() {
        let template_path = write_spawn_template_fixture();
        let mut app = ai_trigger_test_app();

        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime
                .name_to_uuid
                .insert("src".to_string(), "src-uuid".to_string());
            runtime.trigger_states = vec![TriggerState {
                trigger: crate::world::content::Trigger {
                    condition: TriggerCondition::OnAttacked {
                        entity_name: "src".into(),
                    },
                    actions: vec![TriggerAction::SpawnEntity {
                        template_path: template_path.clone(),
                        name: "blocked".into(),
                        anchor: None,
                        position: Some([0.0, 0.0, 0.0]),
                        rotation: None,
                        scale: None,
                    }],
                    when: Some(crate::world::flags::Predicate::Flag {
                        name: "ready".into(),
                    }),
                },
                fired: false,
                origin_layer: None,
                seen_destroyed: HashSet::new(),
            }];
        }

        app.world_mut()
            .resource_mut::<Messages<crate::ai_plugin::AiEntityAttacked>>()
            .write(crate::ai_plugin::AiEntityAttacked {
                entity_uuid: "src-uuid".into(),
                attacker_uuid: uuid::Uuid::parse_str("20202020-0000-0000-0000-000000000001")
                    .unwrap(),
            });
        app.update();
        app.update();

        // Flag was NOT set Ã¢â€ â€™ no registration should appear.
        let has = app
            .world()
            .resource::<WorldContentRuntime>()
            .name_to_uuid
            .contains_key("blocked");
        assert!(
            !has,
            "SpawnEntity must not run while `when` predicate is false"
        );
    }

    /// SpawnEntity action applies optional `rotation` (XYZ Euler radians) and
    /// `scale` (per-axis) to the spawned entity's Transform, mirroring the
    /// static `[[entity]]` TransformConfig semantics.
    #[test]
    fn spawn_entity_action_applies_rotation_and_scale() {
        use crate::entities::spawner::EntityUuid;

        let template_path = write_spawn_template_fixture();
        let mut app = ai_trigger_test_app();

        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime
                .name_to_uuid
                .insert("marker".to_string(), "marker-uuid".to_string());
            runtime.trigger_states = vec![TriggerState {
                trigger: crate::world::content::Trigger {
                    condition: TriggerCondition::OnAttacked {
                        entity_name: "marker".to_string(),
                    },
                    actions: vec![TriggerAction::SpawnEntity {
                        template_path: template_path.clone(),
                        name: "rotated_scaled".to_string(),
                        anchor: None,
                        position: Some([1.0, 2.0, 3.0]),
                        rotation: Some([0.0, std::f32::consts::FRAC_PI_2, 0.0]),
                        scale: Some([2.0, 2.0, 2.0]),
                    }],
                    when: None,
                },
                fired: false,
                origin_layer: None,
                seen_destroyed: HashSet::new(),
            }];
        }

        app.world_mut()
            .resource_mut::<Messages<crate::ai_plugin::AiEntityAttacked>>()
            .write(crate::ai_plugin::AiEntityAttacked {
                entity_uuid: "marker-uuid".into(),
                attacker_uuid: uuid::Uuid::parse_str("cccccccc-0000-0000-0000-000000000002")
                    .unwrap(),
            });

        app.update();
        app.update();

        let uuid = app
            .world()
            .resource::<WorldContentRuntime>()
            .name_to_uuid
            .get("rotated_scaled")
            .cloned()
            .expect("SpawnEntity must register name_to_uuid");

        let expected_quat = Quat::from_euler(EulerRot::XYZ, 0.0, std::f32::consts::FRAC_PI_2, 0.0);
        let expected_scale = Vec3::new(2.0, 2.0, 2.0);
        let expected_translation = Vec3::new(1.0, 2.0, 3.0);

        let mut found = false;
        let mut q = app
            .world_mut()
            .query::<(&EntityUuid, &bevy::prelude::Transform)>();
        for (eu, t) in q.iter(app.world()) {
            if eu.0 == uuid {
                found = true;
                assert!(
                    t.translation.abs_diff_eq(expected_translation, 1e-4),
                    "translation mismatch: got {:?}, expected {:?}",
                    t.translation,
                    expected_translation
                );
                assert!(
                    t.rotation.abs_diff_eq(expected_quat, 1e-4),
                    "rotation mismatch: got {:?}, expected {:?}",
                    t.rotation,
                    expected_quat
                );
                assert!(
                    t.scale.abs_diff_eq(expected_scale, 1e-4),
                    "scale mismatch: got {:?}, expected {:?}",
                    t.scale,
                    expected_scale
                );
            }
        }
        assert!(
            found,
            "spawned entity must exist in ECS with the registered UUID"
        );
    }

    /// (#475) `on_timer` triggers fire when `time.elapsed_secs() -
    /// runtime.world_loaded_at_secs >= after_secs`. Verify the producer
    /// in `handle_ai_events` emits `TimerElapsed` events using the
    /// load-time anchor, and that an `on_timer` trigger correctly fires
    /// a `spawn_entity` action.
    #[test]
    fn on_timer_trigger_fires_spawn_entity_action() {
        use crate::entities::spawner::EntityUuid;

        let template_path = write_spawn_template_fixture();
        let mut app = ai_trigger_test_app();

        // Stamp world load time to `now` and install an on_timer trigger
        // with `after_secs = 0.0` so it should fire on the first tick.
        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime.world_loaded_at_secs = Some(0.0);
            runtime.trigger_states = vec![TriggerState {
                trigger: crate::world::content::Trigger {
                    condition: TriggerCondition::OnTimer { after_secs: 0.0 },
                    actions: vec![TriggerAction::SpawnEntity {
                        template_path: template_path.clone(),
                        name: "wave_now".to_string(),
                        anchor: None,
                        position: Some([1.0, 0.0, 1.0]),
                        rotation: None,
                        scale: None,
                    }],
                    when: None,
                },
                fired: false,
                origin_layer: None,
                seen_destroyed: HashSet::new(),
            }];
        }

        // Tick twice: first runs handle_ai_events which fires the trigger
        // and queues the spawn via Commands; second flushes Commands.
        app.update();
        app.update();

        let uuid = app
            .world()
            .resource::<WorldContentRuntime>()
            .name_to_uuid
            .get("wave_now")
            .cloned();
        assert!(
            uuid.is_some(),
            "on_timer after_secs=0 must have fired its SpawnEntity action â€” \
             handle_ai_events must emit TimerElapsed events when \
             world_loaded_at_secs is set"
        );

        // And the trigger must be marked fired (single-shot).
        let fired = app.world().resource::<WorldContentRuntime>().trigger_states[0].fired;
        assert!(fired, "on_timer trigger must latch fired=true after firing");

        // ECS must contain the spawned entity.
        let uuid_val = uuid.unwrap();
        let mut q = app.world_mut().query::<&EntityUuid>();
        let found = q.iter(app.world()).any(|eu| eu.0 == uuid_val);
        assert!(found, "spawned entity must exist in ECS");
    }

    /// (#475) `on_timer` triggers with `after_secs > now - world_loaded_at`
    /// must NOT fire yet. Pin the elapsed-secs comparison.
    #[test]
    fn on_timer_trigger_does_not_fire_before_after_secs_elapses() {
        let template_path = write_spawn_template_fixture();
        let mut app = ai_trigger_test_app();

        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            // World loaded "in the future" so elapsed will be negative,
            // clamped to 0, never satisfying after_secs = 100.
            runtime.world_loaded_at_secs = Some(0.0);
            runtime.trigger_states = vec![TriggerState {
                trigger: crate::world::content::Trigger {
                    condition: TriggerCondition::OnTimer { after_secs: 100.0 },
                    actions: vec![TriggerAction::SpawnEntity {
                        template_path: template_path.clone(),
                        name: "wave_future".to_string(),
                        anchor: None,
                        position: Some([0.0, 0.0, 0.0]),
                        rotation: None,
                        scale: None,
                    }],
                    when: None,
                },
                fired: false,
                origin_layer: None,
                seen_destroyed: HashSet::new(),
            }];
        }

        app.update();
        app.update();

        let uuid = app
            .world()
            .resource::<WorldContentRuntime>()
            .name_to_uuid
            .get("wave_future")
            .cloned();
        assert!(
            uuid.is_none(),
            "on_timer after_secs=100 must not fire when only a few ms have elapsed"
        );
        let fired = app.world().resource::<WorldContentRuntime>().trigger_states[0].fired;
        assert!(!fired, "trigger must not be marked fired");
    }

    /// SpawnEntity action stamps the trigger `name` onto the spawned entity as
    /// its `EntityName` component.  This is required for `resolve_objective_target`
    /// to match a `AiDirective::Destroy { target: "wave_1" }` against
    /// `AiWorldEntity::name` in `WorldSnapshot` — if the component kept the
    /// template display name ("Harrow Destroyer") the Backfill AI would never
    /// resolve the Destroy target and the ship would stay on its Patrol forever.
    #[test]
    fn spawn_entity_action_stamps_trigger_name_as_entity_name() {
        use crate::entities::spawner::{EntityName, EntityUuid};

        let template_path = write_spawn_template_fixture();
        let mut app = ai_trigger_test_app();

        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime.world_loaded_at_secs = Some(0.0);
            runtime.trigger_states = vec![TriggerState {
                trigger: crate::world::content::Trigger {
                    condition: TriggerCondition::OnTimer { after_secs: 0.0 },
                    actions: vec![TriggerAction::SpawnEntity {
                        template_path: template_path.clone(),
                        name: "wave_1".to_string(),
                        anchor: None,
                        position: Some([50.0, 0.0, 50.0]),
                        rotation: None,
                        scale: None,
                    }],
                    when: None,
                },
                fired: false,
                origin_layer: None,
                seen_destroyed: std::collections::HashSet::new(),
            }];
        }

        app.update();
        app.update();

        // Find the spawned entity by UUID and confirm its EntityName is "wave_1".
        let uuid = app
            .world()
            .resource::<WorldContentRuntime>()
            .name_to_uuid
            .get("wave_1")
            .cloned()
            .expect("SpawnEntity must register name_to_uuid for wave_1");

        let mut q = app
            .world_mut()
            .query::<(&EntityUuid, Option<&EntityName>)>();
        let entity_name = q
            .iter(app.world())
            .find_map(|(eu, name)| (eu.0 == uuid).then(|| name.map(|n| n.0.clone())));

        assert!(
            entity_name.is_some(),
            "spawned entity with UUID for 'wave_1' must exist in ECS"
        );
        assert_eq!(
            entity_name.unwrap().as_deref(),
            Some("wave_1"),
            "EntityName must be the trigger name 'wave_1', not the template display name"
        );
    }

    // -- follow_up_trigger_holds (pure evaluator) -------------------------

    fn name_map(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(n, u)| (n.to_string(), u.to_string()))
            .collect()
    }

    #[test]
    fn follow_up_trigger_holds_fires_immediately_when_trigger_is_none() {
        let n2u = HashMap::new();
        let flags = crate::world::flags::FlagStore::new();
        assert!(follow_up_trigger_holds(
            None,
            0.0,
            &[],
            &n2u,
            &flags,
            &HashSet::new(),
            &HashSet::new(),
        ));
    }

    #[test]
    fn follow_up_trigger_holds_on_world_loaded_always_fires() {
        let n2u = HashMap::new();
        let flags = crate::world::flags::FlagStore::new();
        assert!(follow_up_trigger_holds(
            Some(&TriggerCondition::OnWorldLoaded),
            0.0,
            &[],
            &n2u,
            &flags,
            &HashSet::new(),
            &HashSet::new(),
        ));
    }

    #[test]
    fn follow_up_trigger_holds_on_timer_uses_elapsed_secs_not_world_events() {
        let n2u = HashMap::new();
        let flags = crate::world::flags::FlagStore::new();
        let cond = TriggerCondition::OnTimer { after_secs: 3.0 };

        // Below threshold: does not fire.
        assert!(!follow_up_trigger_holds(
            Some(&cond),
            2.9,
            &[],
            &n2u,
            &flags,
            &HashSet::new(),
            &HashSet::new(),
        ));
        // At/above threshold: fires.
        assert!(follow_up_trigger_holds(
            Some(&cond),
            3.0,
            &[],
            &n2u,
            &flags,
            &HashSet::new(),
            &HashSet::new(),
        ));
        assert!(follow_up_trigger_holds(
            Some(&cond),
            10.0,
            &[],
            &n2u,
            &flags,
            &HashSet::new(),
            &HashSet::new(),
        ));
    }

    #[test]
    fn follow_up_trigger_holds_on_entered_region_fires_when_ship_inside_region() {
        let n2u = name_map(&[("Axiom Dock", "axiom-dock-uuid")]);
        let flags = crate::world::flags::FlagStore::new();
        let cond = TriggerCondition::OnEnteredRegion {
            entity_name: "Axiom Dock".into(),
        };

        // Ship not inside: does NOT fire.
        assert!(!follow_up_trigger_holds(
            Some(&cond),
            0.0,
            &[],
            &n2u,
            &flags,
            &HashSet::new(),
            &HashSet::new(),
        ));
        // Ship inside: fires.
        let mut inside = HashSet::new();
        inside.insert("axiom-dock-uuid".to_string());
        assert!(follow_up_trigger_holds(
            Some(&cond),
            0.0,
            &[],
            &n2u,
            &flags,
            &inside,
            &HashSet::new(),
        ));
    }

    #[test]
    fn follow_up_trigger_holds_on_entered_region_unknown_entity_does_not_fire() {
        // Even if the ship is inside some region, an unmapped entity name
        // never resolves and the trigger never fires.
        let n2u = HashMap::new();
        let flags = crate::world::flags::FlagStore::new();
        let cond = TriggerCondition::OnEnteredRegion {
            entity_name: "Nowhere".into(),
        };
        let mut inside = HashSet::new();
        inside.insert("some-other-uuid".to_string());
        assert!(!follow_up_trigger_holds(
            Some(&cond),
            0.0,
            &[],
            &n2u,
            &flags,
            &inside,
            &HashSet::new(),
        ));
    }

    #[test]
    fn follow_up_trigger_holds_on_exited_region_fires_when_ship_outside() {
        // "Already-true" semantics: a follow-up that needs the player to
        // be OUTSIDE the region fires immediately if they are already
        // outside.
        let n2u = name_map(&[("Trap Zone", "trap-uuid")]);
        let flags = crate::world::flags::FlagStore::new();
        let cond = TriggerCondition::OnExitedRegion {
            entity_name: "Trap Zone".into(),
        };

        // Ship inside: does NOT fire.
        let mut inside = HashSet::new();
        inside.insert("trap-uuid".to_string());
        assert!(!follow_up_trigger_holds(
            Some(&cond),
            0.0,
            &[],
            &n2u,
            &flags,
            &inside,
            &HashSet::new(),
        ));
        // Ship outside: fires.
        assert!(follow_up_trigger_holds(
            Some(&cond),
            0.0,
            &[],
            &n2u,
            &flags,
            &HashSet::new(),
            &HashSet::new(),
        ));
    }

    #[test]
    fn follow_up_trigger_holds_on_flag_set_fires_when_flag_already_set() {
        let n2u = HashMap::new();
        let mut flags = crate::world::flags::FlagStore::new();
        flags.set_flag("aphelion_armed");
        let cond = TriggerCondition::OnFlagSet {
            name: "aphelion_armed".into(),
        };
        assert!(follow_up_trigger_holds(
            Some(&cond),
            0.0,
            &[],
            &n2u,
            &flags,
            &HashSet::new(),
            &HashSet::new(),
        ));
    }

    #[test]
    fn follow_up_trigger_holds_on_flag_set_does_not_fire_when_flag_unset() {
        let n2u = HashMap::new();
        let flags = crate::world::flags::FlagStore::new();
        let cond = TriggerCondition::OnFlagSet {
            name: "aphelion_armed".into(),
        };
        assert!(!follow_up_trigger_holds(
            Some(&cond),
            0.0,
            &[],
            &n2u,
            &flags,
            &HashSet::new(),
            &HashSet::new(),
        ));
    }

    #[test]
    fn follow_up_trigger_holds_on_flag_set_strips_parent_prefix() {
        // Follow-ups don't participate in sub-world layer chains; the
        // evaluator strips any `parent:` prefix so the predicate resolves
        // against the base flag store.
        let n2u = HashMap::new();
        let mut flags = crate::world::flags::FlagStore::new();
        flags.set_flag("aphelion_armed");
        let cond = TriggerCondition::OnFlagSet {
            name: "parent:aphelion_armed".into(),
        };
        assert!(follow_up_trigger_holds(
            Some(&cond),
            0.0,
            &[],
            &n2u,
            &flags,
            &HashSet::new(),
            &HashSet::new(),
        ));
    }

    #[test]
    fn follow_up_trigger_holds_on_flag_cleared_fires_when_flag_already_unset() {
        let n2u = HashMap::new();
        let flags = crate::world::flags::FlagStore::new();
        let cond = TriggerCondition::OnFlagCleared {
            name: "shields_offline".into(),
        };
        // Unset flag is treated as "cleared" â€” fires immediately.
        assert!(follow_up_trigger_holds(
            Some(&cond),
            0.0,
            &[],
            &n2u,
            &flags,
            &HashSet::new(),
            &HashSet::new(),
        ));
    }

    #[test]
    fn follow_up_trigger_holds_on_destroyed_fires_when_entity_already_destroyed() {
        let n2u = name_map(&[("Ironveil", "ironveil-uuid")]);
        let flags = crate::world::flags::FlagStore::new();
        let cond = TriggerCondition::OnDestroyed {
            entity_name: "Ironveil".into(),
        };

        // Ironveil's UUID is registered but NOT in the live set â€” fires.
        assert!(follow_up_trigger_holds(
            Some(&cond),
            0.0,
            &[],
            &n2u,
            &flags,
            &HashSet::new(),
            &HashSet::new(),
        ));
    }

    #[test]
    fn follow_up_trigger_holds_on_destroyed_does_not_fire_when_entity_alive() {
        let n2u = name_map(&[("Ironveil", "ironveil-uuid")]);
        let flags = crate::world::flags::FlagStore::new();
        let cond = TriggerCondition::OnDestroyed {
            entity_name: "Ironveil".into(),
        };
        let mut live = HashSet::new();
        live.insert("ironveil-uuid".to_string());
        assert!(!follow_up_trigger_holds(
            Some(&cond),
            0.0,
            &[],
            &n2u,
            &flags,
            &HashSet::new(),
            &live,
        ));
    }

    #[test]
    fn follow_up_trigger_holds_on_attacked_requires_event() {
        let n2u = name_map(&[("Ironveil", "ironveil-uuid")]);
        let flags = crate::world::flags::FlagStore::new();
        let cond = TriggerCondition::OnAttacked {
            entity_name: "Ironveil".into(),
        };

        // No event: does NOT fire (event-only condition; no "already
        // attacked" state to short-circuit on).
        assert!(!follow_up_trigger_holds(
            Some(&cond),
            0.0,
            &[],
            &n2u,
            &flags,
            &HashSet::new(),
            &HashSet::new(),
        ));
        // Matching event in the snapshot: fires.
        let events = vec![WorldEvent::Attacked {
            uuid: "ironveil-uuid".into(),
            attacker_uuid: "phoenix-uuid".into(),
        }];
        assert!(follow_up_trigger_holds(
            Some(&cond),
            0.0,
            &events,
            &n2u,
            &flags,
            &HashSet::new(),
            &HashSet::new(),
        ));
    }

    #[test]
    fn follow_up_trigger_holds_on_hailed_requires_event() {
        let n2u = name_map(&[("Axiom", "axiom-uuid")]);
        let flags = crate::world::flags::FlagStore::new();
        let cond = TriggerCondition::OnHailed {
            entity_name: "Axiom".into(),
        };
        let events = vec![WorldEvent::Hailed {
            target_uuid: "axiom-uuid".into(),
        }];
        assert!(follow_up_trigger_holds(
            Some(&cond),
            0.0,
            &events,
            &n2u,
            &flags,
            &HashSet::new(),
            &HashSet::new(),
        ));
    }

    #[test]
    fn follow_up_trigger_holds_on_all_destroyed_fires_when_all_uuids_absent() {
        let n2u = name_map(&[("A", "a-uuid"), ("B", "b-uuid"), ("C", "c-uuid")]);
        let flags = crate::world::flags::FlagStore::new();
        let cond = TriggerCondition::OnAllDestroyed {
            entity_names: vec!["A".into(), "B".into(), "C".into()],
        };

        // All three live: does NOT fire.
        let mut live = HashSet::new();
        live.insert("a-uuid".to_string());
        live.insert("b-uuid".to_string());
        live.insert("c-uuid".to_string());
        assert!(!follow_up_trigger_holds(
            Some(&cond),
            0.0,
            &[],
            &n2u,
            &flags,
            &HashSet::new(),
            &live,
        ));

        // A destroyed, B+C still alive: does NOT fire.
        let mut live = HashSet::new();
        live.insert("b-uuid".to_string());
        live.insert("c-uuid".to_string());
        assert!(!follow_up_trigger_holds(
            Some(&cond),
            0.0,
            &[],
            &n2u,
            &flags,
            &HashSet::new(),
            &live,
        ));

        // All three destroyed: fires.
        assert!(follow_up_trigger_holds(
            Some(&cond),
            0.0,
            &[],
            &n2u,
            &flags,
            &HashSet::new(),
            &HashSet::new(),
        ));
    }

    // -- tick_pending_follow_ups: integration of triggered follow-ups ----

    /// Build a minimal app for testing `tick_pending_follow_ups` directly.
    /// Mirrors the existing `delayed_follow_up_replacement_preserves_display_speaker`
    /// shape but exercises the new trigger evaluator.
    fn pending_follow_up_test_app() -> App {
        let mut app = App::new();
        app.add_plugins(bevy::time::TimePlugin)
            .init_resource::<WorldContentRuntime>()
            .init_resource::<CommsInboxRes>()
            .add_message::<CommsChannel2Event>()
            .add_systems(
                Update,
                (tick_pending_follow_ups, handle_comms_channel2).chain(),
            );
        app
    }

    /// Queue a triggered follow-up with a `...` placeholder onto the runtime.
    fn queue_triggered_follow_up(
        app: &mut App,
        body: &str,
        sender_uuid: &str,
        thread_id: &str,
        placeholder_id: &str,
        trigger: TriggerCondition,
    ) {
        let placeholder = CommsMessage {
            id: placeholder_id.into(),
            sender_uuid: sender_uuid.into(),
            sender_name: "Axiom Station".into(),
            subject: "...".into(),
            body: "...".into(),
            responses: vec![],
            selected_response: None,
            is_read: false,
            is_orphaned: false,
            sender_in_range: true,
            thread_id: thread_id.into(),
            is_urgent: false,
        };
        app.world_mut()
            .resource_mut::<CommsInboxRes>()
            .0
            .inject(placeholder);
        app.world_mut()
            .resource_mut::<WorldContentRuntime>()
            .pending_follow_ups
            .push(PendingFollowUp {
                node: CommsDialogueNode {
                    body: body.into(),
                    responses: vec![],
                    speaker: None,
                    trigger: Some(trigger),
                },
                sender_uuid: sender_uuid.into(),
                sender_name: "Axiom Station".into(),
                thread_id: thread_id.into(),
                elapsed_secs: 0.0,
                placeholder_id: Some(placeholder_id.into()),
                urgent: false,
            });
    }

    #[test]
    fn pending_follow_up_with_on_flag_set_trigger_stays_queued_until_flag_is_set() {
        let mut app = pending_follow_up_test_app();
        queue_triggered_follow_up(
            &mut app,
            "Aphelion armed â€” we're committed now.",
            "axiom-uuid",
            "thread-aphelion",
            "placeholder-aphelion",
            TriggerCondition::OnFlagSet {
                name: "aphelion_armed".into(),
            },
        );

        // Tick once with flag unset â€” placeholder stays, follow-up still queued.
        app.update();
        {
            let messages = app.world().resource::<CommsInboxRes>().0.messages();
            assert_eq!(messages.len(), 1);
            assert_eq!(
                messages[0].body, "...",
                "placeholder must remain while the trigger is unsatisfied"
            );
            let runtime = app.world().resource::<WorldContentRuntime>();
            assert_eq!(runtime.pending_follow_ups.len(), 1);
        }

        // Set the flag; next tick must inject the real message.
        app.world_mut()
            .resource_mut::<WorldContentRuntime>()
            .flags
            .set_flag("aphelion_armed");
        app.update();

        let messages = app.world().resource::<CommsInboxRes>().0.messages();
        assert_eq!(
            messages.len(),
            1,
            "placeholder must be replaced by the real message"
        );
        assert_eq!(messages[0].body, "Aphelion armed â€” we're committed now.");
        assert_eq!(messages[0].thread_id, "thread-aphelion");
        let runtime = app.world().resource::<WorldContentRuntime>();
        assert!(runtime.pending_follow_ups.is_empty());
    }

    #[test]
    fn pending_follow_up_with_on_flag_set_fires_immediately_if_flag_already_set() {
        // Critical case for the user request: "or immediately if it's
        // already in range". Set the flag BEFORE queueing the follow-up;
        // the very first tick must inject the real message.
        let mut app = pending_follow_up_test_app();
        app.world_mut()
            .resource_mut::<WorldContentRuntime>()
            .flags
            .set_flag("aphelion_armed");

        queue_triggered_follow_up(
            &mut app,
            "Already-armed acknowledgement.",
            "axiom-uuid",
            "thread-aphelion",
            "placeholder-aphelion",
            TriggerCondition::OnFlagSet {
                name: "aphelion_armed".into(),
            },
        );

        app.update();

        let messages = app.world().resource::<CommsInboxRes>().0.messages();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].body, "Already-armed acknowledgement.");
    }

    #[test]
    fn pending_follow_up_with_on_timer_uses_queue_relative_elapsed_secs() {
        let mut app = pending_follow_up_test_app();
        queue_triggered_follow_up(
            &mut app,
            "Three seconds elapsed.",
            "axiom-uuid",
            "thread-timer",
            "placeholder-timer",
            TriggerCondition::OnTimer { after_secs: 3.0 },
        );

        // Force the queue-relative elapsed_secs past the threshold.
        app.world_mut()
            .resource_mut::<WorldContentRuntime>()
            .pending_follow_ups[0]
            .elapsed_secs = 4.0;
        app.update();

        let messages = app.world().resource::<CommsInboxRes>().0.messages();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].body, "Three seconds elapsed.");
    }

    // -- Issue #506: channel-2 routing tests ------------------------------------

    /// Scenario hail arrives in CommsInboxRes via channel-2 (handle_ai_events
    /// writes to CommsChannel2Event; handle_comms_channel2 injects into inbox).
    #[test]
    fn scenario_hail_arrives_in_inbox_via_channel2() {
        let mut app = ai_trigger_test_app();

        // Install a comms template that fires on WorldLoaded.
        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime
                .name_to_uuid
                .insert("outpost_alpha".to_string(), "outpost-uuid-ch2".to_string());
            runtime.comms_template_states.push(CommsTemplateState {
                template: crate::world::config::CommsTemplate {
                    from: "outpost_alpha".to_string(),
                    trigger: TriggerCondition::OnWorldLoaded,
                    node: CommsDialogueNode {
                        body: "Channel-2 test message.".to_string(),
                        responses: vec![],
                        speaker: Some("Outpost Alpha".to_string()),
                        trigger: None,
                    },
                    thread_id: None,
                    urgent: false,
                    root_follow_up: None,
                },
                fired: false,
            });
            // Queue a WorldLoaded event so handle_ai_events fires the template.
            runtime.pending_world_events.push(WorldEvent::WorldLoaded);
        }

        app.update();

        let messages = app.world().resource::<CommsInboxRes>().0.messages();
        assert_eq!(
            messages.len(),
            1,
            "scenario hail must arrive in inbox after routing through channel-2"
        );
        assert_eq!(messages[0].body, "Channel-2 test message.");
        assert_eq!(messages[0].sender_name, "Outpost Alpha");
    }

    /// When the comms system is AI-operated (`operate_ai = true`),
    /// `handle_comms_channel2` auto-picks the first response (index 0).
    #[test]
    fn ai_auto_respond_on_scenario_hail_via_channel2() {
        let mut app = ai_trigger_test_app();

        // Spawn a Ship entity with comms system set to AI control.
        {
            let mut sources = crate::ship_plugin::ShipSystemControlSources::default();
            sources.0.set(
                crate::system_registry::comms_system_id(),
                crate::control_source::ControlSource::Ai,
            );
            app.world_mut().spawn((
                crate::simulation::Ship,
                crate::simulation::LocalShip,
                sources,
            ));
        }

        // Install a template with a response, fired on WorldLoaded.
        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime
                .name_to_uuid
                .insert("sector_hq".to_string(), "sector-hq-uuid".to_string());
            runtime.comms_template_states.push(CommsTemplateState {
                template: crate::world::config::CommsTemplate {
                    from: "sector_hq".to_string(),
                    trigger: TriggerCondition::OnWorldLoaded,
                    node: CommsDialogueNode {
                        body: "AI auto-respond test.".to_string(),
                        responses: vec![CommsResponse {
                            text: "Acknowledged.".to_string(),
                            actions: vec![],
                            follow_up: None,
                        }],
                        speaker: None,
                        trigger: None,
                    },
                    thread_id: None,
                    urgent: false,
                    root_follow_up: None,
                },
                fired: false,
            });
            runtime.pending_world_events.push(WorldEvent::WorldLoaded);
        }

        app.update();

        let messages = app.world().resource::<CommsInboxRes>().0.messages();
        assert_eq!(messages.len(), 1, "message must be injected into inbox");
        assert_eq!(
            messages[0].selected_response,
            Some(0),
            "AI-operated comms must auto-pick response index 0"
        );
    }
}
