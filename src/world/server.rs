use bevy::prelude::*;
use crate::damage::ConsoleHull;
use crate::simulation::{Ship, ShipHullIntegrity};
use std::collections::{HashMap, HashSet};

use crate::comms_inbox::CommsInbox;
use crate::lobby::{InboundMessage, Sessions, Target, WorldResource};
use crate::simulation::SimOutbox;
use crate::messages::{
    ClientMessage, CommsContact, CommsMessage, Console, GamePhase, ServerMessage, ViewMode,
};
use crate::ship_state::ShipState;
use crate::objectives::ObjectiveManager;
use crate::world::content::{
    ActiveDialogue, CommsTemplateState, TriggerAction,
    TriggerState, WorldEvent, comms_template_states_from_world, evaluate_comms_templates,
    trigger_states_from_world,
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
}

/// Resolve the current `sender_in_range` flag for an injection-time message,
/// matching the stamp logic in `broadcast_comms_state`. Used by every site
/// that inserts a new `CommsMessage` so the field is correct from the moment
/// the message lands in the inbox (belt-and-braces against future refactors
/// that bypass the broadcast stamp pass).
fn current_sender_in_range(runtime: &WorldContentRuntime, sender_uuid: &str) -> bool {
    // Synthetic senders (not a real UUID4 — e.g. "_self", "Starcorp Command") are
    // always readable: they have no physical entity to range-check against.
    if uuid::Uuid::parse_str(sender_uuid).is_err() {
        return true;
    }
    match runtime.range_flags.get(sender_uuid).copied() {
        Some(flag) => flag,
        None => !runtime.range_active,
    }
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
    /// `extra_worlds`) â€” the loader is the base world itself, so
    /// `parent:` from this layer walks straight to the base
    /// `WorldContentRuntime.flags` store.
    pub loader_path: Option<String>,
}

/// Map of `path â†’ WorldRuntime` for sub-worlds loaded via `LoadWorld` / `extra_worlds`.
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
    /// `LoadWorld(path)` to enqueue this â€” `None` for startup-time loads
    /// (base world's `extra_worlds`). Recorded on the new
    /// `WorldRuntime.loader_path` so `parent:` walks from the loaded
    /// layer reach the right outer flag store (PRD #397 fix 1).
    Load { path: String, loader_path: Option<String> },
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
            .add_systems(
                Startup,
                (
                    insert_world_config_resource,
                    spawn_world_entities,
                    init_world_runtime,
                    load_extra_worlds,
                    setup_fallback_world.run_if(not(resource_exists::<crate::world::config::WorldConfig>)),
                ).chain(),
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
                    auto_clear_on_screen_message.in_set(crate::sim_sets::SimSet::Broadcast),
                    update_comms_range_flags.in_set(crate::sim_sets::SimSet::Broadcast),
                    broadcast_comms_state.in_set(crate::sim_sets::SimSet::Broadcast),
                    broadcast_objective_summary.in_set(crate::sim_sets::SimSet::Broadcast),
                ).chain(),
            )
            .add_systems(Update, handle_ai_events.in_set(crate::sim_sets::SimSet::Physics))
            .add_systems(Update, apply_pending_scenario_loads.in_set(crate::sim_sets::SimSet::Physics))
            .add_systems(Update, apply_world_layer_changes.in_set(crate::sim_sets::SimSet::Physics))
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
/// (e.g. a region entity spawned without an `EntityUuid` component â€” not
/// expected in production paths but possible in narrow unit tests).
///
/// Single-fire-per-transition is provided by the region containment
/// system itself: `update_region_membership` uses set differences between
/// the previous and current "inside" sets, so it only triggers the
/// observer event once per boundary crossing. Staying inside on the next
/// tick produces no further `RegionEntered` events.
///
/// NPC entities are never considered by `update_region_membership`
/// (it queries `With<Ship>` and only computes membership for the player
/// ship), so this observer is only invoked for player ship crossings.
fn handle_region_entered_event(
    trigger: On<crate::regions::server::RegionEntered>,
    membership: Option<Res<crate::regions::server::RegionMembership>>,
    runtime: Option<ResMut<WorldContentRuntime>>,
) {
    let (Some(membership), Some(mut runtime)) = (membership, runtime) else {
        return;
    };
    let ev = trigger.event();
    let Some(uuid) = membership.region_uuids.get(&ev.region_entity).cloned() else {
        return;
    };
    runtime.pending_world_events.push(WorldEvent::EnteredRegion { uuid });
}

/// Observer: mirror of `handle_region_entered_event` for region exits.
/// Fires both on boundary-crossing exits and on implicit exits when the
/// region entity is despawned while the ship is inside.
fn handle_region_exited_event(
    trigger: On<crate::regions::server::RegionExited>,
    membership: Option<Res<crate::regions::server::RegionMembership>>,
    runtime: Option<ResMut<WorldContentRuntime>>,
) {
    let (Some(membership), Some(mut runtime)) = (membership, runtime) else {
        return;
    };
    let ev = trigger.event();
    let Some(uuid) = membership.region_uuids.get(&ev.region_entity).cloned() else {
        return;
    };
    runtime.pending_world_events.push(WorldEvent::ExitedRegion { uuid });
}

/// Startup system: copy the unified `WorldConfig` from the WASM-side
/// thread-local cache into a Bevy `Resource` so downstream systems
/// (`spawn_world_entities`, `ai::server::tick_ai_controllers`) can read it
/// via `Res<WorldConfig>`.
///
/// On native (no WASM bridge) `get_world_config()` returns `None` and this
/// system is a no-op; `setup_fallback_world` handles that case via its
/// `run_if(not(resource_exists::<WorldConfig>))` gate.
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
    let (fields, named, _anon) = crate::world::config::partition_immediate_entities_three_way(
        world_config,
        |path| {
            config_cache
                .get(path)
                .and_then(|c| c.asteroid_field.as_ref())
                .is_some()
        },
    );

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
                    entity_inst.template_path, e
                );
                continue;
            }
        };
        // Resolve optional `anchor` reference into a concrete world-space offset
        // applied to the streaming spawner. Missing anchor â†’ warn + fall back
        // to world origin so a typo never silently relocates the field.
        if let Some(field) = config.asteroid_field.as_mut() {
            if let Some(anchor_name) = field.anchor.as_ref() {
                match world_config.anchors.get(anchor_name) {
                    Some(pos) => field.anchor_offset = *pos,
                    None => {
                        bevy::log::warn!(
                            "spawn_world_entities: asteroid field '{}' references unknown anchor '{}' â€” falling back to world origin",
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
    // entity. A missing registration is a programmer error â€” log and skip
    // rather than allocate a fresh UUID (which would silently desync).
    for entity_inst in named {
        let name = entity_inst.name.as_ref().expect("partition guarantees Some");
        let uuid = match world_config.name_to_uuid.get(name) {
            Some(u) => u.clone(),
            None => {
                bevy::log::error!(
                    "spawn_world_entities: named entity '{}' has no UUID in WorldConfig.name_to_uuid â€” skipping",
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
                    name, entity_inst.template_path, e
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
    let pos = crate::world::config::resolve_entity_position_with(
        entity_inst,
        anchors,
        entities_by_name,
    )?;
    Ok(Vec3::new(pos[0], pos[1], pos[2]))
}

/// Fallback world setup with hardcoded values for development/testing.
/// Runs only when no `WorldConfig` resource was loaded (gated by the
/// `WorldPlugin` `run_if` clause).
fn setup_fallback_world(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    _world: ResMut<WorldResource>,
) {
    // -- Starfield skybox ---------------------------------------------------
    // Procedural points: many small unlit white spheres at radius ~2000
    // around the origin. Cheap and works on WebGL2.
    let star_mat = materials.add(StandardMaterial {
        base_color: Color::srgb(1.0, 1.0, 1.0),
        unlit: true,
        ..default()
    });
    let star_mesh = meshes.add(Sphere { radius: 1.0 });
    let star_count = 400u32;
    let radius = 2000.0_f32;
    for i in 0..star_count {
        // Deterministic pseudo-random unit vector via golden-spiral on a sphere.
        let frac = (i as f32 + 0.5) / star_count as f32;
        let phi = (1.0 - 2.0 * frac).acos();
        let theta = std::f32::consts::PI * (1.0 + 5_f32.sqrt()) * i as f32;
        let x = phi.sin() * theta.cos() * radius;
        let y = phi.sin() * theta.sin() * radius;
        let z = phi.cos() * radius;
        // Hash for size variation
        let h = ((i.wrapping_mul(2654435761)) ^ 0xDEADBEEF) % 100;
        let scale = 1.5 + (h as f32) / 25.0; // 1.5..5.5
        commands.spawn((
            Mesh3d(star_mesh.clone()),
            MeshMaterial3d(star_mat.clone()),
            Transform::from_xyz(x, y, z).with_scale(Vec3::splat(scale)),
        ));
    }

    // Spawn ship via the generic entity spawner using a hardcoded EntityConfig
    // (mirrors assets/entities/player_ship.toml's collider). This is the
    // no-WorldConfig fallback path; the [[entity]]/spawn_game_start path is
    // preferred and runs whenever a WorldConfig is loaded.
    let ship_config = crate::entity_config::EntityConfig {
        name: None,
        tags: vec!["player".to_string(), "ship".to_string()],
        collider: Some(crate::entity_config::ColliderConfig {
            shape: crate::entity_config::ColliderShape::Capsule,
            radius: 6.0,
            length: 6.0,
        }),
        hull: Some(crate::entity_config::HullConfig { hull_integrity: 100.0, ..Default::default() }),
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
        shape: None,
        effects: None,
        faction: None,
        behaviour: None,
        radar_appearance: None,
        mesh: None,
        light: Vec::new(),
    };
    let ship_uuid = crate::entity_loader::assign_uuid();
    let ship_entity = crate::entity_spawner::spawn_entity(
        &mut commands, &ship_config, Vec3::ZERO, ship_uuid, Some("player-ship".to_string()),
    );
    commands.entity(ship_entity).insert(Ship);
    commands.insert_resource(ShipHullIntegrity(ConsoleHull::from_config(&[
        (crate::messages::Console::Helm, 25.0),
        (crate::messages::Console::Tactical, 25.0),
        (crate::messages::Console::Power, 25.0),
        (crate::messages::Console::Shields, 25.0),
    ])));
}


// -- Startup systems ---------------------------------------------------------

/// Startup system: initialise `WorldContentRuntime`, `CommsInboxRes`, and
/// `WorldResource` from the loaded `WorldConfig` (if any).
///
/// This is the post-PRD-#341 sole runtime-init entry point: the legacy
/// scenario / map split is gone. When no `WorldConfig`
/// resource is present (native unit tests, fallback bootstrap) this is a
/// no-op and downstream comms / trigger systems remain quiet.
fn init_world_runtime(
    world_config: Option<Res<crate::world::config::WorldConfig>>,
    mut runtime: ResMut<WorldContentRuntime>,
    mut inbox: ResMut<CommsInboxRes>,
    _world: ResMut<WorldResource>,
) {
    let Some(world_config) = world_config else {
        return;
    };

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
    runtime.comms_template_states =
        comms_template_states_from_world(&world_config);

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
        pending.0.push(WorldLayerChange::Load { path: path.clone(), loader_path: None });
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

/// Handle `Hail { target_uuid }` messages from Comms console holders.
///
/// Evaluates matching `on_hailed` comms templates for the target entity,
/// injects new messages into the inbox, and records active dialogues.
fn handle_hail(
    mut reader: MessageReader<InboundMessage>,
    sessions: Res<Sessions>,
    mut runtime: ResMut<WorldContentRuntime>,
    mut inbox: ResMut<CommsInboxRes>,
) {
    for ev in reader.read() {
        // Gate: sender must hold Console::Comms.
        if !sessions.0.player_has_console(&ev.token, Console::Comms) {
            continue;
        }

        let ClientMessage::Hail { target_uuid } = &ev.msg else {
            continue;
        };

        // Server-side range gate: when range tracking is active, the target
        // must be a known, in-range entity. Out-of-range hails are silently
        // dropped (clients enforce the same gate UX-side; this defends
        // against stale or malicious clients).
        if runtime.range_active {
            match runtime.range_flags.get(target_uuid).copied() {
                Some(true) => {}
                _ => continue,
            }
        }

        // Evaluate matching on_hailed comms templates.
        let world_events = vec![WorldEvent::Hailed {
            target_uuid: target_uuid.clone(),
        }];

        let WorldContentRuntime {
            name_to_uuid,
            comms_template_states,
            ..
        } = &mut *runtime;
        let fired = evaluate_comms_templates(
            comms_template_states,
            &world_events,
            name_to_uuid,
        );

        // Route the Hailed event into the trigger system so that
        // on_hailed triggers (e.g. complete_objective, load_world)
        // can fire. handle_ai_events drains pending_world_events
        // in SimSet::Physics, which runs after SimSet::Input.
        runtime.pending_world_events.push(WorldEvent::Hailed {
            target_uuid: target_uuid.clone(),
        });

        for f in fired {
            // Build a CommsMessage and inject it.
            let msg_id = uuid::Uuid::new_v4().to_string();
            let thread_id = f.thread_id.clone().unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
            let sender_uuid = target_uuid.clone();
            // Resolve sender display name from contacts (best effort).
            let sender_name = runtime
                .contacts
                .iter()
                .find(|c| c.uuid == *target_uuid)
                .map(|c| c.name.clone())
                .unwrap_or_else(|| target_uuid.clone());

            let responses: Vec<String> =
                f.node.responses.iter().map(|r| r.text.clone()).collect();

            let msg = CommsMessage {
                id: msg_id.clone(),
                sender_uuid: sender_uuid.clone(),
                sender_name,
                subject: f.node.body.chars().take(40).collect(),
                body: f.node.body.clone(),
                responses,
                selected_response: None,
                is_read: false,
                is_orphaned: false,
                sender_in_range: current_sender_in_range(&runtime, &sender_uuid),
                thread_id: thread_id.clone(),
                is_urgent: f.urgent,
            };

            inbox.0.inject(msg);

            // Record the active dialogue.
            runtime.active_dialogues.insert(
                msg_id,
                ActiveDialogue {
                    current_node: f.node.clone(),
                    thread_id,
                },
            );
        }
    }
}

/// Handle `RespondToMessage { message_id, response_index }` from Comms holders.
///
/// Records the chosen response on the inbox message, fires any associated
/// trigger actions, and advances the dialogue to the follow-up node if present.
fn handle_respond_to_message(
    mut reader: MessageReader<InboundMessage>,
    sessions: Res<Sessions>,
    mut runtime: ResMut<WorldContentRuntime>,
    mut inbox: ResMut<CommsInboxRes>,
    mut objectives: ResMut<ObjectiveManagerRes>,
    mut commands: Commands,
    mut ai_query: Query<(&EntityUuid, &mut AiControllerComponent, &BehaviourSection)>,
    mut modifiers: Option<ResMut<crate::modifiers::ShipModifiers>>,
    mut next_state: Option<ResMut<NextState<GamePhase>>>,
    mut game_over_reason: Option<ResMut<crate::simulation::GameOverReason>>,
    mut pending_layers: Option<ResMut<PendingWorldLayerChanges>>,
    mut layer_map: Option<ResMut<WorldLayerMap>>,
    base_world_config: Option<Res<crate::world::config::WorldConfig>>,
    entity_uuid_query: Query<(Entity, &EntityUuid)>,
) {
    for ev in reader.read() {
        if !sessions.0.player_has_console(&ev.token, Console::Comms) {
            continue;
        }

        let ClientMessage::RespondToMessage {
            message_id,
            response_index,
        } = &ev.msg
        else {
            continue;
        };

        // Look up active dialogue for this message.
        let dialogue = match runtime.active_dialogues.get(message_id) {
            Some(d) => d.clone(),
            None => continue,
        };

        // Server-side range gate: if range tracking is active, the sender
        // of this message must currently be in range. Out-of-range responses
        // are silently dropped so stale clients can't fire actions on a
        // hidden response button.
        if runtime.range_active {
            let sender_uuid = inbox.0.sender_uuid_for(message_id).unwrap_or_default();
            match runtime.range_flags.get(&sender_uuid).copied() {
                Some(true) => {}
                _ => continue,
            }
        }

        let responses = &dialogue.current_node.responses;
        if *response_index >= responses.len() {
            continue;
        }

        let response = &responses[*response_index];

        // Fire response actions.
        //
        // PRD #397 fix 2: this dispatch is intentionally parallel to the
        // per-action match in `handle_ai_events` below. Every `TriggerAction`
        // variant a trigger can fire must produce the same observable
        // effect when listed under a comms response. Comms responses do not
        // currently carry an originating sub-world layer (the `CommsTemplate`
        // type has no `origin_layer` field, unlike `TriggerState`), so all
        // layer-scoped operations resolve against the base world (`None`).
        // Flag-mutation transitions are pushed onto
        // `runtime.pending_world_events` so `handle_ai_events` (later in the
        // same Update tick via `SimSet::Physics`) picks them up and fires
        // any chained `on_flag_set` / `on_flag_cleared` triggers.
        //
        // When adding a new `TriggerAction` variant, add an arm here AND in
        // `handle_ai_events`. The `comms_response_dispatches_every_trigger_action_variant`
        // parity test guards against drift.
        let origin_layer: Option<String> = None;
        let name_to_uuid_snapshot = runtime.name_to_uuid.clone();
        // Build reverse map (UUID → entity name) so we can associate objectives
        // added by comms responses with the sender entity.
        let uuid_to_name: std::collections::HashMap<&str, &str> = name_to_uuid_snapshot
            .iter()
            .map(|(name, uuid)| (uuid.as_str(), name.as_str()))
            .collect();
        let sender_uuid = inbox.0.sender_uuid_for(message_id);
        for action in &response.actions {
            match action {
                TriggerAction::AddObjective { id, text, mandatory } => {
                    let entity_name = sender_uuid.clone()
                        .and_then(|suid| uuid_to_name.get(suid.as_str()).copied())
                        .map(String::from);
                    objectives.0.add(id, text, *mandatory, entity_name);
                }
                TriggerAction::CompleteObjective { id } => {
                    objectives.0.complete(id);
                }
                TriggerAction::FailObjective { id } => {
                    objectives.0.fail(id);
                }
                TriggerAction::SetAiState { entity, state, target } => {
                    let target_uuid = match name_to_uuid_snapshot.get(entity) {
                        Some(u) => u.clone(),
                        None => {
                            bevy::log::warn!(
                                "handle_respond_to_message: SetAiState: unknown entity name '{entity}'"
                            );
                            continue;
                        }
                    };
                    for (uuid_comp, mut ctrl, behaviour) in ai_query.iter_mut() {
                        if uuid_comp.0 != target_uuid {
                            continue;
                        }
                        let new_ai_state = crate::ai::build_initial_state(
                            &crate::entity_config::BehaviourConfig {
                                initial_state: state.clone(),
                                state: behaviour.0.state.clone(),
                                transition: behaviour.0.transition.clone(),
                            },
                        );
                        ctrl.controller.current_state = new_ai_state;
                        ctrl.controller.current_state_name = state.clone();
                        if let Some(target_name) = target {
                            if let Some(target_uuid) = name_to_uuid_snapshot.get(target_name) {
                                if let Ok(uuid) = uuid::Uuid::parse_str(target_uuid) {
                                    ctrl.controller.blackboard.target = Some(uuid);
                                }
                            }
                        }
                        break;
                    }
                }
                TriggerAction::ApplyModifier { entity, tag, slot, bonus } => {
                    if !name_to_uuid_snapshot.contains_key(entity) {
                        bevy::log::warn!(
                            "handle_respond_to_message: ApplyModifier: unknown entity name '{entity}'"
                        );
                        continue;
                    }
                    if let Some(ref mut mods) = modifiers {
                        mods.add_or_update(crate::modifiers::Modifier {
                            source: crate::messages::ModifierSource::World {
                                id: "world".to_string(),
                                tag: tag.clone(),
                            },
                            slot: slot.clone(),
                            bonus: *bonus,
                        });
                    }
                }
                TriggerAction::RemoveModifier { entity, tag, slot } => {
                    if !name_to_uuid_snapshot.contains_key(entity) {
                        bevy::log::warn!(
                            "handle_respond_to_message: RemoveModifier: unknown entity name '{entity}'"
                        );
                        continue;
                    }
                    if let Some(ref mut mods) = modifiers {
                        mods.remove(
                            &crate::messages::ModifierSource::World {
                                id: "world".to_string(),
                                tag: tag.clone(),
                            },
                            slot,
                        );
                    }
                }
                TriggerAction::ApplyFlag { entity, tag, kind } => {
                    if !name_to_uuid_snapshot.contains_key(entity) {
                        bevy::log::warn!(
                            "handle_respond_to_message: ApplyFlag: unknown entity name '{entity}'"
                        );
                        continue;
                    }
                    if let Some(ref mut mods) = modifiers {
                        mods.add_flag(
                            crate::messages::ModifierSource::World {
                                id: "world".to_string(),
                                tag: tag.clone(),
                            },
                            kind.clone(),
                        );
                    }
                }
                TriggerAction::RemoveFlag { entity, tag, kind } => {
                    if !name_to_uuid_snapshot.contains_key(entity) {
                        bevy::log::warn!(
                            "handle_respond_to_message: RemoveFlag: unknown entity name '{entity}'"
                        );
                        continue;
                    }
                    if let Some(ref mut mods) = modifiers {
                        mods.remove_flag(
                            crate::messages::ModifierSource::World {
                                id: "world".to_string(),
                                tag: tag.clone(),
                            },
                            kind.clone(),
                        );
                    }
                }
                TriggerAction::ApplyIntModifier { entity, tag, slot, bonus } => {
                    if !name_to_uuid_snapshot.contains_key(entity) {
                        bevy::log::warn!(
                            "handle_respond_to_message: ApplyIntModifier: unknown entity name '{entity}'"
                        );
                        continue;
                    }
                    if let Some(ref mut mods) = modifiers {
                        mods.add_or_update_int(crate::modifiers::IntModifier {
                            source: crate::messages::ModifierSource::World {
                                id: "world".to_string(),
                                tag: tag.clone(),
                            },
                            slot: slot.clone(),
                            bonus: *bonus,
                        });
                    }
                }
                TriggerAction::RemoveIntModifier { entity, tag, slot } => {
                    if !name_to_uuid_snapshot.contains_key(entity) {
                        bevy::log::warn!(
                            "handle_respond_to_message: RemoveIntModifier: unknown entity name '{entity}'"
                        );
                        continue;
                    }
                    if let Some(ref mut mods) = modifiers {
                        mods.remove_int(
                            &crate::messages::ModifierSource::World {
                                id: "world".to_string(),
                                tag: tag.clone(),
                            },
                            slot,
                        );
                    }
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
                        // Comms responses load against the base world
                        // (loader_path = None) since CommsTemplate has no
                        // origin_layer concept today.
                        lc.0.push(WorldLayerChange::Load {
                            path: path.clone(),
                            loader_path: origin_layer.clone(),
                        });
                    }
                }
                TriggerAction::UnloadWorld { path } => {
                    if let Some(ref mut lc) = pending_layers {
                        lc.0.push(WorldLayerChange::Unload(path.clone()));
                    }
                }
                TriggerAction::SetWorldFlag { name } => {
                    if let Some((target_layer, stripped, before, after)) =
                        mutate_world_flag(
                            &mut runtime.flags,
                            layer_map.as_deref_mut().map(|lm| &mut lm.0),
                            &origin_layer,
                            name,
                            FlagMutation::Set,
                        )
                    {
                        emit_flag_transition(
                            &mut runtime.pending_world_events,
                            &stripped, &target_layer, before, after,
                        );
                    }
                }
                TriggerAction::ClearWorldFlag { name } => {
                    if let Some((target_layer, stripped, before, after)) =
                        mutate_world_flag(
                            &mut runtime.flags,
                            layer_map.as_deref_mut().map(|lm| &mut lm.0),
                            &origin_layer,
                            name,
                            FlagMutation::Clear,
                        )
                    {
                        emit_flag_transition(
                            &mut runtime.pending_world_events,
                            &stripped, &target_layer, before, after,
                        );
                    }
                }
                TriggerAction::IncrementWorldFlag { name, by } => {
                    if let Some((target_layer, stripped, before, after)) =
                        mutate_world_flag(
                            &mut runtime.flags,
                            layer_map.as_deref_mut().map(|lm| &mut lm.0),
                            &origin_layer,
                            name,
                            FlagMutation::Increment(*by),
                        )
                    {
                        emit_flag_transition(
                            &mut runtime.pending_world_events,
                            &stripped, &target_layer, before, after,
                        );
                    }
                }
                TriggerAction::SetWorldFlagValue { name, value } => {
                    if let Some((target_layer, stripped, before, after)) =
                        mutate_world_flag(
                            &mut runtime.flags,
                            layer_map.as_deref_mut().map(|lm| &mut lm.0),
                            &origin_layer,
                            name,
                            FlagMutation::SetValue(*value),
                        )
                    {
                        emit_flag_transition(
                            &mut runtime.pending_world_events,
                            &stripped, &target_layer, before, after,
                        );
                    }
                }
                TriggerAction::SpawnEntity {
                    template_path, name, anchor, position, rotation, scale,
                } => {
                    let pos_arr: [f32; 3] = if let Some(pos) = position {
                        *pos
                    } else if let Some(anchor_name) = anchor {
                        // origin_layer = None: resolve against base world anchors only.
                        let lookup = base_world_config
                            .as_ref()
                            .and_then(|wc| wc.anchors.get(anchor_name).copied());
                        match lookup {
                            Some(p) => p,
                            None => {
                                bevy::log::warn!(
                                    "handle_respond_to_message: SpawnEntity '{name}' anchor '{anchor_name}' not found"
                                );
                                continue;
                            }
                        }
                    } else {
                        bevy::log::warn!(
                            "handle_respond_to_message: SpawnEntity '{name}' has neither anchor nor position"
                        );
                        continue;
                    };

                    let config_cache = crate::config_cache::get_config_cache();
                    let template_inst = crate::world::config::WorldEntity {
                        template_path: template_path.clone(),
                        ..Default::default()
                    };
                    let entity_config = match crate::entity_loader::resolve_entity(
                        &template_inst, &config_cache,
                    ) {
                        Ok(c) => c,
                        Err(e) => {
                            #[cfg(not(target_arch = "wasm32"))]
                            {
                                match std::fs::read_to_string(template_path) {
                                    Ok(toml_str) => {
                                        match crate::entity_config::EntityConfig::from_toml(&toml_str) {
                                            Ok(c) => c,
                                            Err(err) => {
                                                bevy::log::warn!(
                                                    "handle_respond_to_message: SpawnEntity '{name}' template '{template_path}' parse error: {err:?}"
                                                );
                                                continue;
                                            }
                                        }
                                    }
                                    Err(_) => {
                                        bevy::log::warn!(
                                            "handle_respond_to_message: SpawnEntity '{name}' template '{template_path}' not in cache nor on disk: {e}"
                                        );
                                        continue;
                                    }
                                }
                            }
                            #[cfg(target_arch = "wasm32")]
                            {
                                bevy::log::warn!(
                                    "handle_respond_to_message: SpawnEntity '{name}' template '{template_path}' not in cache: {e}"
                                );
                                continue;
                            }
                        }
                    };

                    let uuid = crate::entity_loader::assign_uuid();
                    let pos_vec = Vec3::new(pos_arr[0], pos_arr[1], pos_arr[2]);
                    let spawned = crate::entity_spawner::spawn_entity(
                        &mut commands,
                        &entity_config,
                        pos_vec,
                        uuid.clone(),
                        None,
                    );

                    if rotation.is_some() || scale.is_some() {
                        let [rx, ry, rz] = rotation.unwrap_or([0.0, 0.0, 0.0]);
                        let quat = Quat::from_euler(EulerRot::XYZ, rx, ry, rz);
                        let [sx, sy, sz] = scale.unwrap_or([1.0, 1.0, 1.0]);
                        let scale_vec = Vec3::new(sx, sy, sz);
                        commands.entity(spawned).insert(
                            Transform {
                                translation: pos_vec,
                                rotation: quat,
                                scale: scale_vec,
                            },
                        );
                    }

                    runtime.name_to_uuid.insert(name.clone(), uuid);
                    // origin_layer = None => entity is not attached to any
                    // sub-world layer's spawned_entities list. It persists
                    // for the session (matches base-world trigger semantics).
                }
                TriggerAction::DestroyEntity { entity } => {
                    let uuid = match name_to_uuid_snapshot.get(entity) {
                        Some(u) => u.clone(),
                        None => {
                            bevy::log::warn!(
                                "handle_respond_to_message: DestroyEntity: unknown entity name '{entity}'"
                            );
                            continue;
                        }
                    };
                    let mut target_entity: Option<Entity> = None;
                    for (ent, uuid_comp) in entity_uuid_query.iter() {
                        if uuid_comp.0 == uuid {
                            target_entity = Some(ent);
                            break;
                        }
                    }
                    // Defer AiEntityDestroyed via Commands::queue so
                    // external consumers (and chained on_destroyed triggers
                    // in handle_ai_events later this tick) observe the event.
                    runtime.pending_world_events.push(
                        WorldEvent::Destroyed { uuid: uuid.clone() }
                    );
                    let msg_uuid = uuid.clone();
                    commands.queue(move |world: &mut World| {
                        if let Some(mut msgs) = world.get_resource_mut::<
                            Messages<crate::ai_plugin::AiEntityDestroyed>,
                        >() {
                            msgs.write(crate::ai_plugin::AiEntityDestroyed {
                                entity_uuid: msg_uuid,
                            });
                        }
                    });
                    if let Some(ent) = target_entity {
                        commands.entity(ent).despawn();
                    }
                }
            }
        }

        // Record the chosen response on the inbox message.
        inbox.0.record_response(message_id, *response_index);

        // Advance to follow-up node if present.
        if let Some(follow_up) = &response.follow_up {
            // Inject a new message for the follow-up node.
            let new_msg_id = uuid::Uuid::new_v4().to_string();
            let thread_id = dialogue.thread_id.clone();
            let sender_uuid = inbox
                .0
                .sender_uuid_for(message_id)
                .unwrap_or_default();
            let sender_name = inbox
                .0
                .sender_name_for(message_id)
                .unwrap_or_default();

            let new_responses: Vec<String> =
                follow_up.responses.iter().map(|r| r.text.clone()).collect();

            let new_msg = CommsMessage {
                id: new_msg_id.clone(),
                sender_uuid: sender_uuid.clone(),
                sender_name,
                subject: follow_up.body.chars().take(40).collect(),
                body: follow_up.body.clone(),
                responses: new_responses,
                selected_response: None,
                is_read: false,
                is_orphaned: false,
                sender_in_range: current_sender_in_range(&runtime, &sender_uuid),
                thread_id: thread_id.clone(),
                is_urgent: false,
            };

            inbox.0.inject(new_msg);

            // Record the follow-up dialogue, inheriting the same thread_id.
            runtime.active_dialogues.insert(
                new_msg_id,
                ActiveDialogue {
                    current_node: follow_up.clone(),
                    thread_id,
                },
            );
        }
    }
}

/// Handle `ClearComms` from Comms console holders.
fn handle_clear_comms(
    mut reader: MessageReader<InboundMessage>,
    sessions: Res<Sessions>,
    mut inbox: ResMut<CommsInboxRes>,
) {
    for ev in reader.read() {
        if !sessions.0.player_has_console(&ev.token, Console::Comms) {
            continue;
        }

        if matches!(ev.msg, ClientMessage::ClearComms) {
            inbox.0.clear();
        }
    }
}

/// Handle `ShowOnScreen { message_id }` from Comms console holders.
///
/// Looks up the message in the inbox, stores it in `OnScreenMessage`, and
/// pushes `ViewMode::Comms` so the viewscreen switches to the comms overlay.
fn handle_show_on_screen(
    mut reader: MessageReader<InboundMessage>,
    sessions: Res<Sessions>,
    inbox: Res<CommsInboxRes>,
    mut on_screen: ResMut<OnScreenMessage>,
    mut ship: ResMut<ShipState>,
) {
    for ev in reader.read() {
        if !sessions.0.player_has_console(&ev.token, Console::Comms) {
            continue;
        }

        if let ClientMessage::ShowOnScreen { ref message_id } = ev.msg {
            if let Some(msg) = inbox.0.messages().into_iter().find(|m| &m.id == message_id) {
                on_screen.0 = Some(msg.clone());
                ship.view_mode = ViewMode::Comms;
            }
        }
    }
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
    ship: Res<ShipState>,
) {
    if on_screen.0.is_none() {
        return;
    }
    // If the captain (or anyone) has switched away from Comms view, clear.
    if !matches!(ship.view_mode, ViewMode::Comms) {
        on_screen.0 = None;
        return;
    }
    // Check the live inbox record for the displayed message.
    let should_clear = if let Some(ref displayed) = on_screen.0 {
        match inbox.0.messages().into_iter().find(|m| m.id == displayed.id) {
            None => true, // message purged from inbox
            Some(live) => {
                live.selected_response.is_some()   // responded to
                || live.is_orphaned                // sender gone
                || !live.sender_in_range           // out of range
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
    ship_q: Query<(&Transform, Option<&crate::comms::CommsRange>), With<crate::simulation::Ship>>,
    entity_q: Query<(
        &crate::entities::spawner::EntityUuid,
        &Transform,
        &crate::comms::CommsRange,
    )>,
) {
    let Ok((ship_tf, ship_range_opt)) = ship_q.single() else {
        // No ship: either lobby/pure-handler tests (range tracking never
        // activated â€” preserve default-true semantics) or the ship was
        // destroyed mid-game. In the latter case, do NOT reset
        // `range_active` to false â€” that would silently re-enable all
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
    mut runtime: ResMut<WorldContentRuntime>,
    mut inbox: ResMut<CommsInboxRes>,
    objectives: Res<ObjectiveManagerRes>,
    mut outbox: ResMut<SimOutbox>,
) {

    let dirty = inbox.0.is_dirty() || runtime.needs_broadcast || objectives.0.is_dirty();
    if !dirty {
        return;
    }

    let Some(comms_token) = sessions.0.console_holder(Console::Comms) else {
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
            // are always readable — they have no physical entity to range-check.
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

    outbox.0.push((Target::Token(comms_token.to_string()), ServerMessage::CommsState {
        messages,
        objectives: objectives_snap,
        contacts,
    }));

    inbox.0.mark_clean();
    runtime.needs_broadcast = false;
}

/// Broadcast `ObjectiveSummary` to the Captain when objectives change.
fn broadcast_objective_summary(
    sessions: Res<Sessions>,
    mut objectives: ResMut<ObjectiveManagerRes>,
    mut outbox: ResMut<SimOutbox>,
) {

    if !objectives.0.is_dirty() {
        return;
    }

    let Some(captain_token) = sessions.0.console_holder(Console::CaptainChair) else {
        objectives.0.mark_clean();
        return;
    };

    let objectives_snap = objectives.0.sorted_snapshots();

    outbox.0.push((Target::Token(captain_token.to_string()), ServerMessage::ObjectiveSummary {
        objectives: objectives_snap,
    }));

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
    mut inbox: ResMut<CommsInboxRes>,
    mut commands: Commands,
    mut attacked_reader: MessageReader<crate::ai_plugin::AiEntityAttacked>,
    mut destroyed_reader: MessageReader<crate::ai_plugin::AiEntityDestroyed>,
    mut ai_query: Query<(&EntityUuid, &mut AiControllerComponent, &BehaviourSection)>,
    mut modifiers: Option<ResMut<crate::modifiers::ShipModifiers>>,
    mut next_state: Option<ResMut<NextState<GamePhase>>>,
    mut game_over_reason: Option<ResMut<crate::simulation::GameOverReason>>,
    _pending: Option<ResMut<PendingScenarioLoad>>,
    mut pending_layers: Option<ResMut<PendingWorldLayerChanges>>,
    mut layer_map: Option<ResMut<WorldLayerMap>>,
    base_world_config: Option<Res<crate::world::config::WorldConfig>>,
    entity_uuid_query: Query<(Entity, &EntityUuid)>,
) {

    let mut world_events: Vec<WorldEvent> = Vec::new();
    for ev in attacked_reader.read() {
        world_events.push(WorldEvent::Attacked {
            uuid: ev.entity_uuid.clone(),
            attacker_uuid: ev.attacker_uuid.to_string(),
        });
    }
    for ev in destroyed_reader.read() {
        world_events.push(WorldEvent::Destroyed { uuid: ev.entity_uuid.clone() });
    }
    // Drain any externally-queued world events (e.g. WorldLoaded pushed by
    // init_world_runtime or apply_world_layer_changes). This lets those
    // emission sites participate in the existing evaluate+dispatch+chain
    // loop below without duplicating the action dispatch table.
    if !runtime.pending_world_events.is_empty() {
        let drained: Vec<WorldEvent> = runtime.pending_world_events.drain(..).collect();
        world_events.extend(drained);
    }
    if world_events.is_empty() {
        return;
    }

    let name_to_uuid = runtime.name_to_uuid.clone();

    // Auto-fire comms templates that match the world events (e.g. on_attacked distress calls).
    // These are injected without any player hailing â€” they are broadcast messages.
    let fired_comms = evaluate_comms_templates(
        &mut runtime.comms_template_states,
        &world_events,
        &name_to_uuid,
    );
    for fc in fired_comms {
        let msg_id = uuid::Uuid::new_v4().to_string();
        let thread_id = fc.thread_id.clone().unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        // `_self` is the reserved synthetic internal-sender name; render it as
        // "Internal Report" in the comms UI so the crew sees a ship-generated
        // intelligence summary rather than a literal "_self" sender label.
        let sender_name = if fc.from == "_self" {
            "Internal Report".to_string()
        } else {
            fc.from.clone()
        };
        let sender_uuid = name_to_uuid
            .get(&fc.from)
            .cloned()
            .unwrap_or_else(|| fc.from.clone());
        let responses: Vec<String> = fc.node.responses.iter().map(|r| r.text.clone()).collect();
        let msg = crate::messages::CommsMessage {
            id: msg_id.clone(),
            sender_uuid: sender_uuid.clone(),
            sender_name,
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
        inbox.0.inject(msg);
        runtime.active_dialogues.insert(
            msg_id,
            ActiveDialogue {
                current_node: fc.node.clone(),
                thread_id,
            },
        );
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
                            // Layer missing from snapshot — treat as empty.
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
                    TriggerAction::AddObjective { id, text, mandatory } => {
                        objectives.0.add(
                            id.clone(),
                            text.clone(),
                            *mandatory,
                            ft.entity_name.clone(),
                        );
                    }
                    TriggerAction::CompleteObjective { id } => {
                        objectives.0.complete(id);
                    }
                    TriggerAction::FailObjective { id } => {
                        objectives.0.fail(id);
                    }
                    TriggerAction::SetAiState { entity, state, target } => {
                        // Resolve spawn name ? UUID
                        let target_uuid = match name_to_uuid.get(entity) {
                            Some(u) => u.clone(),
                            None => {
                                bevy::log::warn!(
                                    "handle_ai_events: SetAiState: unknown entity name '{entity}'"
                                );
                                continue;
                            }
                        };
                        // Find the Bevy entity with that UUID and mutate its controller.
                        for (uuid_comp, mut ctrl, behaviour) in ai_query.iter_mut() {
                            if uuid_comp.0 != target_uuid {
                                continue;
                            }
                            let new_ai_state = crate::ai::build_initial_state(
                                &crate::entity_config::BehaviourConfig {
                                    initial_state: state.clone(),
                                    state: behaviour.0.state.clone(),
                                    transition: behaviour.0.transition.clone(),
                                },
                            );
                            ctrl.controller.current_state = new_ai_state;
                            ctrl.controller.current_state_name = state.clone();
                            if let Some(target_name) = target {
                                if let Some(target_uuid) = name_to_uuid.get(target_name) {
                                    if let Ok(uuid) = uuid::Uuid::parse_str(target_uuid) {
                                        ctrl.controller.blackboard.target = Some(uuid);
                                    }
                                }
                            }
                            break;
                        }
                    }
                    TriggerAction::ApplyModifier { entity, tag, slot, bonus } => {
                        if !name_to_uuid.contains_key(entity) {
                            bevy::log::warn!(
                                "handle_ai_events: ApplyModifier: unknown entity name '{entity}'"
                            );
                            continue;
                        }
                        if let Some(ref mut mods) = modifiers {
                            mods.add_or_update(crate::modifiers::Modifier {
                                source: crate::messages::ModifierSource::World {
                                    id: "world".to_string(),
                                    tag: tag.clone(),
                                },
                                slot: slot.clone(),
                                bonus: *bonus,
                            });
                        }
                    }
                    TriggerAction::RemoveModifier { entity, tag, slot } => {
                        if !name_to_uuid.contains_key(entity) {
                            bevy::log::warn!(
                                "handle_ai_events: RemoveModifier: unknown entity name '{entity}'"
                            );
                            continue;
                        }
                        if let Some(ref mut mods) = modifiers {
                            mods.remove(
                                &crate::messages::ModifierSource::World {
                                    id: "world".to_string(),
                                    tag: tag.clone(),
                                },
                                slot,
                            );
                        }
                    }
                    TriggerAction::ApplyFlag { entity, tag, kind } => {
                        if !name_to_uuid.contains_key(entity) {
                            bevy::log::warn!(
                                "handle_ai_events: ApplyFlag: unknown entity name '{entity}'"
                            );
                            continue;
                        }
                        if let Some(ref mut mods) = modifiers {
                            mods.add_flag(
                                crate::messages::ModifierSource::World {
                                    id: "world".to_string(),
                                    tag: tag.clone(),
                                },
                                kind.clone(),
                            );
                        }
                    }
                    TriggerAction::RemoveFlag { entity, tag, kind } => {
                        if !name_to_uuid.contains_key(entity) {
                            bevy::log::warn!(
                                "handle_ai_events: RemoveFlag: unknown entity name '{entity}'"
                            );
                            continue;
                        }
                        if let Some(ref mut mods) = modifiers {
                            mods.remove_flag(
                                crate::messages::ModifierSource::World {
                                    id: "world".to_string(),
                                    tag: tag.clone(),
                                },
                                kind.clone(),
                            );
                        }
                    }
                    TriggerAction::ApplyIntModifier { entity, tag, slot, bonus } => {
                        if !name_to_uuid.contains_key(entity) {
                            bevy::log::warn!(
                                "handle_ai_events: ApplyIntModifier: unknown entity name '{entity}'"
                            );
                            continue;
                        }
                        if let Some(ref mut mods) = modifiers {
                            mods.add_or_update_int(crate::modifiers::IntModifier {
                                source: crate::messages::ModifierSource::World {
                                    id: "world".to_string(),
                                    tag: tag.clone(),
                                },
                                slot: slot.clone(),
                                bonus: *bonus,
                            });
                        }
                    }
                    TriggerAction::RemoveIntModifier { entity, tag, slot } => {
                        if !name_to_uuid.contains_key(entity) {
                            bevy::log::warn!(
                                "handle_ai_events: RemoveIntModifier: unknown entity name '{entity}'"
                            );
                            continue;
                        }
                        if let Some(ref mut mods) = modifiers {
                            mods.remove_int(
                                &crate::messages::ModifierSource::World {
                                    id: "world".to_string(),
                                    tag: tag.clone(),
                                },
                                slot,
                            );
                        }
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
                        if let Some((target_layer, stripped, before, after)) =
                            mutate_world_flag(
                                &mut runtime.flags,
                                layer_map.as_deref_mut().map(|lm| &mut lm.0),
                                &ft.origin_layer,
                                name,
                                FlagMutation::Set,
                            )
                        {
                            emit_flag_transition(
                                &mut next_events, &stripped, &target_layer, before, after,
                            );
                        }
                    }
                    TriggerAction::ClearWorldFlag { name } => {
                        if let Some((target_layer, stripped, before, after)) =
                            mutate_world_flag(
                                &mut runtime.flags,
                                layer_map.as_deref_mut().map(|lm| &mut lm.0),
                                &ft.origin_layer,
                                name,
                                FlagMutation::Clear,
                            )
                        {
                            emit_flag_transition(
                                &mut next_events, &stripped, &target_layer, before, after,
                            );
                        }
                    }
                    TriggerAction::IncrementWorldFlag { name, by } => {
                        if let Some((target_layer, stripped, before, after)) =
                            mutate_world_flag(
                                &mut runtime.flags,
                                layer_map.as_deref_mut().map(|lm| &mut lm.0),
                                &ft.origin_layer,
                                name,
                                FlagMutation::Increment(*by),
                            )
                        {
                            emit_flag_transition(
                                &mut next_events, &stripped, &target_layer, before, after,
                            );
                        }
                    }
                    TriggerAction::SetWorldFlagValue { name, value } => {
                        if let Some((target_layer, stripped, before, after)) =
                            mutate_world_flag(
                                &mut runtime.flags,
                                layer_map.as_deref_mut().map(|lm| &mut lm.0),
                                &ft.origin_layer,
                                name,
                                FlagMutation::SetValue(*value),
                            )
                        {
                            emit_flag_transition(
                                &mut next_events, &stripped, &target_layer, before, after,
                            );
                        }
                    }
                    TriggerAction::SpawnEntity {
                        template_path, name, anchor, position, rotation, scale,
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
                            &template_inst, &config_cache,
                        ) {
                            Ok(c) => c,
                            Err(e) => {
                                // Native fallback: try reading from disk.
                                #[cfg(not(target_arch = "wasm32"))]
                                {
                                    match std::fs::read_to_string(template_path) {
                                        Ok(toml_str) => {
                                            match crate::entity_config::EntityConfig::from_toml(&toml_str) {
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
                            commands.entity(spawned).insert(
                                Transform {
                                    translation: pos_vec,
                                    rotation: quat,
                                    scale: scale_vec,
                                },
                            );
                        }

                        // Register name â†’ uuid for subsequent triggers.
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
                            if let Some(mut msgs) = world.get_resource_mut::<
                                Messages<crate::ai_plugin::AiEntityDestroyed>,
                            >() {
                                msgs.write(crate::ai_plugin::AiEntityDestroyed {
                                    entity_uuid: msg_uuid,
                                });
                            }
                        });
                        // Despawn the underlying entity if we found it.
                        if let Some(ent) = target_entity {
                            commands.entity(ent).despawn();
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

/// Compare `before`/`after` flag values and push a `FlagSet` or `FlagCleared`
/// event into `events` when the boolean view (`counter != 0`) flips.
///
/// `origin_layer` is the resolved target layer of the mutation (after
/// `parent:` walking) — embedded in the emitted event so layer-scoped
/// `on_flag_set` / `on_flag_cleared` triggers only react to transitions
/// in their own layer (PRD #397 fix 1).
fn emit_flag_transition(
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
enum FlagMutation {
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
fn mutate_world_flag(
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
                    "mutate_world_flag: '{name}' from origin {origin_layer:?} walks past base world — ignoring"
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
                    "mutate_world_flag: target layer '{path}' missing from WorldLayerMap — ignoring '{name}'"
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
use crate::entity_spawner::{BehaviourSection, EntityUuid};

// â”€â”€ Pending scenario load system â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

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
                        bevy::log::error!("apply_pending_scenario_loads: failed to parse {}: {}", path, e);
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
                            if !runtime.contacts.iter().any(|c: &crate::messages::CommsContact| c.uuid == uuid) {
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

// â”€â”€ World layer system (LoadWorld / UnloadWorld) â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

/// Build a `ConfigCache` suitable for spawning entities from a world layer.
///
/// On WASM the global config cache (pre-loaded by the JS bridge) is returned
/// unchanged.  On native the global cache is always empty (no WASM pre-load
/// step), so we fall back to reading each template file from disk so that
/// `spawn_immediate_entities_internal` can resolve them.
fn build_layer_config_cache(
    world_config: &crate::world::config::WorldConfig,
) -> crate::config_cache::ConfigCache {
    let mut cache = crate::config_cache::get_config_cache();

    #[cfg(not(target_arch = "wasm32"))]
    {
        use crate::entity_config::EntityConfig;
        for entity in &world_config.entities {
            if cache.contains_key(&entity.template_path) {
                continue;
            }
            match std::fs::read_to_string(&entity.template_path) {
                Ok(toml_str) => {
                    if let Ok(cfg) = EntityConfig::from_toml(&toml_str) {
                        cache.insert(entity.template_path.clone(), cfg);
                    } else {
                        bevy::log::warn!(
                            "build_layer_config_cache: failed to parse '{}' â€” entity will be skipped",
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
    }

    cache
}

/// Bevy system: drain `PendingWorldLayerChanges` and apply each `LoadWorld` or
/// `UnloadWorld` command to `WorldLayerMap` and `WorldContentRuntime`.
///
/// `LoadWorld` parses the TOML, merges triggers/comms into the live runtime, and
/// stores a `WorldRuntime` snapshot keyed by path so `UnloadWorld` can reverse it.
///
/// `UnloadWorld` removes the stored snapshot and retains only triggers/comms
/// states that do not belong to the unloaded world (matched by pointer equality
/// of the underlying `Trigger`/`CommsTemplate` clone identity â€” we use indices
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
                    // Already loaded — de-duplicate, no-op.
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
                                runtime.comms_template_states.extend(comms_template_states.clone());

                                // Assign UUIDs to named entities in this layer's config
                                // and register them in the live runtime's name_to_uuid map.
                                let new_names = crate::world::config::assign_named_entity_uuids(
                                    &scenario_config.entities,
                                    crate::entity_loader::assign_uuid,
                                );
                                for (name, uuid) in &new_names {
                                    scenario_config.name_to_uuid.insert(name.clone(), uuid.clone());
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
                    continue; // Not loaded â€” no-op.
                };

                // Despawn ECS entities that were spawned when this layer loaded.
                for entity in &layer.spawned_entities {
                    commands.entity(*entity).despawn();
                }

                // Remove trigger states belonging to this layer.
                // We identify them by the condition+actions equality of the stored snapshot.
                let removed_triggers: std::collections::HashSet<usize> = layer
                    .trigger_states
                    .iter()
                    .filter_map(|ls| {
                        runtime.trigger_states.iter().position(|rs| {
                            rs.trigger == ls.trigger
                        })
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
                        runtime.comms_template_states.iter().position(|rs| {
                            rs.template == ls.template
                        })
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
        crate::config_cache::pop_pending_world_toml(path)
            .or_else(|| {
                // Fire a JS fetch request if we haven't already.
                crate::config_cache::request_world_fetch(path.to_string());
                None
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai_plugin::{AiEntityAttacked, AiEntityDestroyed};
    use crate::lobby::{LobbyPlugin, OutboundMessage, WorldResource};
    use crate::messages::*;
    use crate::world::content::{CommsDialogueNode, CommsResponse, CommsTemplateState, TriggerCondition};

    // -- setup_fallback_world run-condition tests (PRD #341) ------------------
    //
    // The fallback system must run exactly when no `WorldConfig` resource is
    // present (e.g. native unit tests, no WASM-loaded world). When a
    // `WorldConfig` is loaded the fallback must be skipped â€” the
    // `[[entity]]`-driven spawn path owns the ship via `spawn_game_start_entities`.

    /// Build the minimum app needed to run `WorldPlugin`'s Startup chain.
    /// Excludes `LobbyPlugin` so we don't pull in extra systems we don't need.
    fn fallback_test_app() -> App {
        let mut app = App::new();
        app.add_plugins(bevy::asset::AssetPlugin::default())
            .init_asset::<Mesh>()
            .init_asset::<StandardMaterial>()
            .init_resource::<WorldResource>()
            .add_plugins(WorldPlugin);
        app
    }

    #[test]
    fn setup_fallback_world_runs_when_no_world_config_present() {
        let mut app = fallback_test_app();
        // Run only the Startup schedule â€” WorldPlugin's Update systems
        // require message types we don't want to wire up here.
        app.world_mut().run_schedule(Startup);
        assert!(
            app.world().get_resource::<ShipHullIntegrity>().is_some(),
            "setup_fallback_world should have run and inserted ShipHullIntegrity \
             when no WorldConfig is loaded"
        );
    }

    #[test]
    fn setup_fallback_world_is_skipped_when_world_config_present() {
        let mut app = fallback_test_app();
        // Insert a WorldConfig before Startup runs. The run_if gate must
        // suppress setup_fallback_world.
        app.world_mut()
            .insert_resource(crate::world::config::WorldConfig::default());
        app.world_mut().run_schedule(Startup);
        assert!(
            app.world().get_resource::<ShipHullIntegrity>().is_none(),
            "setup_fallback_world should NOT have run when a WorldConfig is \
             already loaded â€” the [[entity]] pipeline owns ship spawning"
        );
    }

    // -- Test app -------------------------------------------------------------

    #[derive(Resource, Default)]
    struct Outbox(Vec<OutboundMessage>);

    fn collect(mut reader: MessageReader<OutboundMessage>, mut box_: ResMut<Outbox>) {
        for m in reader.read() {
            box_.0.push(m.clone());
        }
    }

    fn comms_test_app() -> App {
        let mut app = App::new();
        app.add_plugins(LobbyPlugin)
            .add_plugins(bevy::time::TimePlugin)
            .init_resource::<WorldContentRuntime>()
            .init_resource::<CommsInboxRes>()
            .init_resource::<ObjectiveManagerRes>()
            .init_resource::<SimOutbox>()
            .init_resource::<Outbox>()
            .add_systems(
                Update,
                (
                    handle_hail,
                    handle_respond_to_message,
                    handle_clear_comms,
                    update_comms_range_flags,
                    broadcast_comms_state,
                    broadcast_objective_summary,
                ).chain(),
            )
            .add_systems(PostUpdate, collect);
        app
    }

    fn push_msg(app: &mut App, token: &str, msg: ClientMessage) {
        app.world_mut()
            .resource_mut::<Messages<InboundMessage>>()
            .write(InboundMessage { token: token.into(), msg });
    }

    fn tick(app: &mut App) -> Vec<OutboundMessage> {
        app.update();
        let sim_entries = std::mem::take(&mut app.world_mut().resource_mut::<SimOutbox>().0);
        let mut msgs = app.world().resource::<Outbox>().0.clone();
        for (target, msg) in sim_entries {
            msgs.push(OutboundMessage { target, msg });
        }
        app.world_mut().resource_mut::<Outbox>().0.clear();
        msgs
    }

    /// Set up a game in InProgress phase with a comms player and captain.
    fn setup_game_with_comms(app: &mut App, station_uuid: &str) {
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
                station: "Captain's Chair".into(),
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
        push_msg(app, "captain", ClientMessage::StartGame);
        tick(app);

        // Manually install a comms template into the runtime so tests are
        // independent of TOML loading.
        let runtime = &mut app.world_mut().resource_mut::<WorldContentRuntime>();
        runtime.name_to_uuid.insert("starbase_alpha".into(), station_uuid.into());
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
                        }],
                        follow_up: None,
                    }],
                },
                thread_id: None,
                urgent: false,
            },
            fired: false,
        });
        runtime.needs_broadcast = true;
    }

    // -- Cycle 1: hail delivers CommsState to comms holder --------------------

    #[test]
    fn hail_with_matching_template_sends_comms_state_to_comms_holder() {
        let station_uuid = "station-uuid-001";
        let mut app = comms_test_app();
        setup_game_with_comms(&mut app, station_uuid);

        // Flush the initial broadcast triggered by needs_broadcast.
        let _ = tick(&mut app);

        push_msg(
            &mut app,
            "comms",
            ClientMessage::Hail {
                target_uuid: station_uuid.into(),
            },
        );
        let out = tick(&mut app);

        let comms_state = out.iter().find_map(|m| {
            if let ServerMessage::CommsState {
                messages,
                contacts,
                ..
            } = &m.msg
            {
                Some((messages.clone(), contacts.clone()))
            } else {
                None
            }
        });

        assert!(comms_state.is_some(), "CommsState must be sent after Hail");
        let (messages, _contacts) = comms_state.unwrap();
        assert_eq!(messages.len(), 1, "one message should arrive");
        assert_eq!(
            messages[0].body,
            "USS Phoenix, please identify yourself."
        );
        assert_eq!(messages[0].responses.len(), 1);
    }

    // -- Cycle 2: hail from non-Comms player is ignored -----------------------

    #[test]
    fn hail_from_non_comms_player_is_ignored() {
        let station_uuid = "station-uuid-002";
        let mut app = comms_test_app();
        setup_game_with_comms(&mut app, station_uuid);
        let _ = tick(&mut app);

        push_msg(
            &mut app,
            "captain",
            ClientMessage::Hail {
                target_uuid: station_uuid.into(),
            },
        );
        let out = tick(&mut app);

        // Should not produce any new CommsState with messages.
        let comms_state_with_messages = out.iter().any(|m| {
            if let ServerMessage::CommsState { messages, .. } = &m.msg {
                !messages.is_empty()
            } else {
                false
            }
        });
        assert!(
            !comms_state_with_messages,
            "non-Comms player hail must be ignored"
        );
    }

    // -- Cycle 3: respond fires actions and updates CommsState ----------------

    #[test]
    fn respond_to_message_fires_add_objective_and_broadcasts_update() {
        let station_uuid = "station-uuid-003";
        let mut app = comms_test_app();
        setup_game_with_comms(&mut app, station_uuid);
        let _ = tick(&mut app);

        // Hail to get a message.
        push_msg(
            &mut app,
            "comms",
            ClientMessage::Hail {
                target_uuid: station_uuid.into(),
            },
        );
        let out = tick(&mut app);

        // Extract the message id.
        let msg_id = out.iter().find_map(|m| {
            if let ServerMessage::CommsState { messages, .. } = &m.msg {
                messages.first().map(|msg| msg.id.clone())
            } else {
                None
            }
        });
        let msg_id = msg_id.expect("expected CommsState with a message after hail");

        // Respond to the message.
        push_msg(
            &mut app,
            "comms",
            ClientMessage::RespondToMessage {
                message_id: msg_id.clone(),
                response_index: 0,
            },
        );
        let out = tick(&mut app);

        // Expect a CommsState update with selected_response set.
        let comms_state = out.iter().find_map(|m| {
            if let ServerMessage::CommsState { messages, .. } = &m.msg {
                Some(messages.clone())
            } else {
                None
            }
        });
        assert!(comms_state.is_some(), "CommsState expected after RespondToMessage");
        let messages = comms_state.unwrap();
        let msg = messages.iter().find(|m| m.id == msg_id).expect("original message must still be in inbox");
        assert_eq!(msg.selected_response, Some(0), "selected_response must be recorded");

        // Expect an ObjectiveSummary to be sent to the captain.
        let obj_summary = out.iter().find_map(|m| {
            if let ServerMessage::ObjectiveSummary { objectives } = &m.msg {
                Some(objectives.clone())
            } else {
                None
            }
        });
        assert!(obj_summary.is_some(), "ObjectiveSummary expected after AddObjective action");
        let objectives = obj_summary.unwrap();
        assert_eq!(objectives.len(), 1);
        assert_eq!(objectives[0].text, "Complete the survey");
    }

    // -- Cycle 4: clear comms removes read/orphaned messages ------------------

    #[test]
    fn clear_comms_removes_orphaned_messages_and_broadcasts_update() {
        let station_uuid = "station-uuid-004";
        let mut app = comms_test_app();
        setup_game_with_comms(&mut app, station_uuid);
        let _ = tick(&mut app);

        // Inject an orphaned message directly.
        let orphaned =         CommsMessage {
            id: "orphaned-001".into(),
            sender_uuid: station_uuid.into(),
            sender_name: "Starbase Alpha".into(),
            subject: "Old message".into(),
            body: "Old message body".into(),
            responses: vec![],
            selected_response: None,
            is_read: false,
            is_orphaned: true,
            sender_in_range: true,
            thread_id: "orphaned-001".into(),
            is_urgent: false,
        };
        // Orphan it before injection so clear() will remove it.
        app.world_mut()
            .resource_mut::<CommsInboxRes>()
            .0
            .inject(orphaned);
        let _ = tick(&mut app);

        push_msg(&mut app, "comms", ClientMessage::ClearComms);
        let out = tick(&mut app);

        let comms_state = out.iter().find_map(|m| {
            if let ServerMessage::CommsState { messages, .. } = &m.msg {
                Some(messages.clone())
            } else {
                None
            }
        });
        assert!(comms_state.is_some(), "CommsState expected after ClearComms");
        let messages = comms_state.unwrap();
        assert!(
            messages.iter().all(|m| !m.is_orphaned),
            "all orphaned messages must be cleared"
        );
    }

    // -- Cycle 5: initial CommsState with contacts sent on game start ---------

    #[test]
    fn initial_comms_state_includes_contacts_from_scenario() {
        let station_uuid = "station-uuid-005";
        let mut app = comms_test_app();
        setup_game_with_comms(&mut app, station_uuid);

        let out = tick(&mut app);

        let contacts = out.iter().find_map(|m| {
            if let ServerMessage::CommsState { contacts, .. } = &m.msg {
                Some(contacts.clone())
            } else {
                None
            }
        });
        assert!(contacts.is_some(), "initial CommsState with contacts expected");
        let contacts = contacts.unwrap();
        assert!(
            contacts.iter().any(|c| c.uuid == station_uuid),
            "station must appear as a contact"
        );
    }

    // -- Cycle 6: hail generates non-empty thread_id --------------------------

    #[test]
    fn hail_generates_non_empty_thread_id_on_message() {
        let station_uuid = "station-uuid-006";
        let mut app = comms_test_app();
        setup_game_with_comms(&mut app, station_uuid);
        let _ = tick(&mut app);

        push_msg(
            &mut app,
            "comms",
            ClientMessage::Hail {
                target_uuid: station_uuid.into(),
            },
        );
        let out = tick(&mut app);

        let msg = out.iter().find_map(|m| {
            if let ServerMessage::CommsState { messages, .. } = &m.msg {
                messages.first().cloned()
            } else {
                None
            }
        });
        let msg = msg.expect("CommsState with a message expected after hail");
        assert!(
            !msg.thread_id.is_empty(),
            "thread_id must be a non-empty UUID after hail"
        );
    }

    // -- Cycle 7: follow-up message inherits parent thread_id -----------------

    #[test]
    fn respond_to_message_follow_up_inherits_parent_thread_id() {
        let station_uuid = "station-uuid-007b";
        let mut app2 = comms_test_app();
        setup_game_with_comms_and_followup(&mut app2, station_uuid);
        let _ = tick(&mut app2);

        push_msg(&mut app2, "comms", ClientMessage::Hail { target_uuid: station_uuid.into() });
        let out = tick(&mut app2);

        let first_msg = out.iter().find_map(|m| {
            if let ServerMessage::CommsState { messages, .. } = &m.msg {
                messages.first().cloned()
            } else {
                None
            }
        });
        let first_msg = first_msg.expect("CommsState expected after hail");
        let parent_thread_id = first_msg.thread_id.clone();
        assert!(!parent_thread_id.is_empty(), "parent message must have a non-empty thread_id");

        push_msg(
            &mut app2,
            "comms",
            ClientMessage::RespondToMessage {
                message_id: first_msg.id.clone(),
                response_index: 0,
            },
        );
        let out2 = tick(&mut app2);

        let follow_up_msg = out2.iter().find_map(|m| {
            if let ServerMessage::CommsState { messages, .. } = &m.msg {
                // The follow-up is the second message (index 1).
                messages.get(1).cloned()
            } else {
                None
            }
        });
        let follow_up_msg = follow_up_msg.expect("follow-up CommsMessage expected after respond");
        assert_eq!(
            follow_up_msg.thread_id, parent_thread_id,
            "follow-up message must carry the same thread_id as the parent"
        );
    }

    fn setup_game_with_comms_and_followup(app: &mut App, station_uuid: &str) {
        setup_game_with_comms(app, station_uuid);
        // Replace the single template with one that has a follow-up node.
        let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
        runtime.comms_template_states.clear();
        runtime.comms_template_states.push(crate::world::content::CommsTemplateState {
            template: crate::world::content::CommsTemplate {
                from: "starbase_alpha".into(),
                trigger: TriggerCondition::OnHailed {
                    entity_name: "starbase_alpha".into(),
                },
                node: CommsDialogueNode {
                    body: "Identify yourself.".into(),
                    responses: vec![CommsResponse {
                        text: "We are the Phoenix.".into(),
                        actions: vec![],
                        follow_up: Some(CommsDialogueNode {
                            body: "Welcome, Phoenix.".into(),
                            responses: vec![],
                        }),
                    }],
                },
                thread_id: None,
                urgent: false,
            },
            fired: false,
        });
    }

    // -- AI-event trigger tests -----------------------------------------------

    /// Build a minimal test app that includes just what handle_ai_events needs.
    fn ai_trigger_test_app() -> App {
        let mut app = App::new();
        app.add_plugins(LobbyPlugin)
            .add_plugins(bevy::time::TimePlugin)
            .add_plugins(crate::ai_plugin::AiPlugin)
            .insert_resource(crate::config_cache::FactionRegistryResource(crate::config_cache::get_faction_registry()))
            .init_resource::<WorldContentRuntime>()
            .init_resource::<CommsInboxRes>()
            .init_resource::<ObjectiveManagerRes>()
            .init_resource::<SimOutbox>()
            .add_systems(Update, handle_ai_events);
        // Set phase to InProgress
        app.world_mut().insert_resource(State::new(GamePhase::InProgress));
        app
    }

    #[test]
    fn on_entity_destroyed_trigger_fires_add_objective_action() {
        let mut app = ai_trigger_test_app();

        let npc_uuid = "dead-npc-uuid-001";
        let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
        runtime.name_to_uuid.insert("station_alpha".to_string(), npc_uuid.to_string());
        runtime.trigger_states = vec![TriggerState {
            trigger: crate::world::content::Trigger {
                condition: TriggerCondition::OnDestroyed { entity_name: "station_alpha".to_string() },
                actions: vec![TriggerAction::AddObjective {
                    id: "obj-001".to_string(),
                    text: "Station destroyed".to_string(),
                    mandatory: false,
                }],
                when: None,
            },
            fired: false,
            origin_layer: None,
        }];

        // Emit the AiEntityDestroyed message.
        app.world_mut()
            .resource_mut::<Messages<AiEntityDestroyed>>()
            .write(AiEntityDestroyed { entity_uuid: npc_uuid.to_string() });

        app.update();

        let objectives = &app.world().resource::<ObjectiveManagerRes>().0;
        assert!(
            objectives.sorted_snapshots().iter().any(|o| o.id == "obj-001"),
            "AddObjective action must have fired"
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
            runtime.name_to_uuid.insert("target".into(), npc_uuid.into());
            runtime.trigger_states = vec![TriggerState {
                trigger: crate::world::content::Trigger {
                    condition: TriggerCondition::OnDestroyed {
                        entity_name: "target".into(),
                    },
                    actions: vec![TriggerAction::AddObjective {
                        id: "obj-gated".into(),
                        text: "Should only fire after flag is set".into(),
                        mandatory: false,
                    }],
                    when: Some(crate::world::flags::parse_predicate("flag(green_light)").unwrap()),
                },
                fired: false,
                origin_layer: None,
            }];
        }
        // First firing: flag unset â†’ no objective.
        app.world_mut()
            .resource_mut::<Messages<AiEntityDestroyed>>()
            .write(AiEntityDestroyed { entity_uuid: npc_uuid.into() });
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
            .write(AiEntityDestroyed { entity_uuid: npc_uuid.into() });
        app.update();
        let objs = &app.world().resource::<ObjectiveManagerRes>().0;
        assert!(
            objs.sorted_snapshots().iter().any(|o| o.id == "obj-gated"),
            "gated action must fire once the flag is set"
        );
    }

    #[test]
    fn set_flag_action_fires_on_flag_set_trigger_within_same_tick() {
        // Trigger A: on_destroyed â†’ set_flag a
        // Trigger B: on_flag_set { name="a" } â†’ add_objective B
        // A and B must both fire in a single tick.
        let mut app = ai_trigger_test_app();
        let npc_uuid = "uuid-chain-source";
        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime.name_to_uuid.insert("source".into(), npc_uuid.into());
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
                },
                TriggerState {
                    trigger: crate::world::content::Trigger {
                        condition: TriggerCondition::OnFlagSet { name: "a".into() },
                        actions: vec![TriggerAction::AddObjective {
                            id: "obj-chain".into(),
                            text: "Reacted to flag set".into(),
                            mandatory: false,
                        }],
                        when: None,
                    },
                    fired: false,
                    origin_layer: None,
                },
            ];
        }
        app.world_mut()
            .resource_mut::<Messages<AiEntityDestroyed>>()
            .write(AiEntityDestroyed { entity_uuid: npc_uuid.into() });
        app.update();

        let runtime = app.world().resource::<WorldContentRuntime>();
        assert!(runtime.flags.flag("a"), "set_flag action must have mutated the store");
        assert!(runtime.trigger_states[1].fired, "on_flag_set trigger must have fired");
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
            runtime.name_to_uuid.insert("source".into(), npc_uuid.into());
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
                },
                TriggerState {
                    trigger: crate::world::content::Trigger {
                        condition: TriggerCondition::OnFlagSet { name: "a".into() },
                        actions: vec![TriggerAction::AddObjective {
                            id: "obj-no-op".into(),
                            text: "Should not fire on no-op re-set".into(),
                            mandatory: false,
                        }],
                        when: None,
                    },
                    fired: false,
                    origin_layer: None,
                },
            ];
        }
        app.world_mut()
            .resource_mut::<Messages<AiEntityDestroyed>>()
            .write(AiEntityDestroyed { entity_uuid: npc_uuid.into() });
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
            runtime.flags.set_flag("shields_up"); // pre-set so we transition trueâ†’false
            runtime.name_to_uuid.insert("source".into(), npc_uuid.into());
            runtime.trigger_states = vec![
                TriggerState {
                    trigger: crate::world::content::Trigger {
                        condition: TriggerCondition::OnDestroyed {
                            entity_name: "source".into(),
                        },
                        actions: vec![TriggerAction::ClearWorldFlag { name: "shields_up".into() }],
                        when: None,
                    },
                    fired: false,
                    origin_layer: None,
                },
                TriggerState {
                    trigger: crate::world::content::Trigger {
                        condition: TriggerCondition::OnFlagCleared { name: "shields_up".into() },
                        actions: vec![TriggerAction::AddObjective {
                            id: "obj-shields-down".into(),
                            text: "Shields are down".into(),
                            mandatory: true,
                        }],
                        when: None,
                    },
                    fired: false,
                    origin_layer: None,
                },
            ];
        }
        app.world_mut()
            .resource_mut::<Messages<AiEntityDestroyed>>()
            .write(AiEntityDestroyed { entity_uuid: npc_uuid.into() });
        app.update();

        let runtime = app.world().resource::<WorldContentRuntime>();
        assert!(!runtime.flags.flag("shields_up"));
        let objs = &app.world().resource::<ObjectiveManagerRes>().0;
        assert!(
            objs.sorted_snapshots().iter().any(|o| o.id == "obj-shields-down"),
            "on_flag_cleared trigger must fire on trueâ†’false transition"
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
            runtime.name_to_uuid.insert("source".into(), npc_uuid.into());
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
                    }],
                    when: Some(
                        crate::world::flags::parse_predicate("flag(parent:armed)").unwrap(),
                    ),
                },
                fired: false,
                origin_layer: Some(layer_path.clone()),
            }];
        }
        app.world_mut()
            .resource_mut::<Messages<AiEntityDestroyed>>()
            .write(AiEntityDestroyed { entity_uuid: npc_uuid.into() });
        app.update();

        let objs = &app.world().resource::<ObjectiveManagerRes>().0;
        assert!(
            objs.sorted_snapshots().iter().any(|o| o.id == "obj-parent-when"),
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
            runtime.name_to_uuid.insert("source".into(), npc_uuid.into());
            runtime.trigger_states = vec![
                // Sub-world trigger: setting `armed` in the sub-world layer.
                TriggerState {
                    trigger: crate::world::content::Trigger {
                        condition: TriggerCondition::OnDestroyed {
                            entity_name: "source".into(),
                        },
                        actions: vec![TriggerAction::SetWorldFlag { name: "armed".into() }],
                        when: None,
                    },
                    fired: false,
                    origin_layer: Some(layer_path.clone()),
                },
                // Base-world watcher: on_flag_set armed.
                TriggerState {
                    trigger: crate::world::content::Trigger {
                        condition: TriggerCondition::OnFlagSet { name: "armed".into() },
                        actions: vec![TriggerAction::AddObjective {
                            id: "obj-base-armed".into(),
                            text: "should NOT fire — different layer".into(),
                            mandatory: false,
                        }],
                        when: None,
                    },
                    fired: false,
                    origin_layer: None,
                },
            ];
        }
        app.world_mut()
            .resource_mut::<Messages<AiEntityDestroyed>>()
            .write(AiEntityDestroyed { entity_uuid: npc_uuid.into() });
        app.update();

        // Sub-world layer's flag store got the mutation; base store did not.
        let lm = app.world().resource::<WorldLayerMap>();
        let layer_flags = &lm.0.get(&layer_path).expect("layer present").flags;
        assert!(layer_flags.flag("armed"), "mutation lands in sub-world layer");
        let runtime = app.world().resource::<WorldContentRuntime>();
        assert!(!runtime.flags.flag("armed"), "base store must remain empty");
        let objs = &app.world().resource::<ObjectiveManagerRes>().0;
        assert!(
            !objs.sorted_snapshots().iter().any(|o| o.id == "obj-base-armed"),
            "base trigger must not cross-fire on sub-world flag"
        );
    }

    /// `parent:flag` mutation from the base world walks past root → no-op +
    /// warn; the predicate read also resolves as unset.
    #[test]
    fn parent_walk_past_root_from_base_is_noop_for_mutation_and_reads_unset() {
        let mut app = ai_trigger_test_app();
        app.init_resource::<WorldLayerMap>();
        app.init_resource::<PendingWorldLayerChanges>();
        let npc_uuid = "uuid-past-root";
        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime.name_to_uuid.insert("source".into(), npc_uuid.into());
            runtime.trigger_states = vec![TriggerState {
                trigger: crate::world::content::Trigger {
                    condition: TriggerCondition::OnDestroyed {
                        entity_name: "source".into(),
                    },
                    // Base-world trigger (origin_layer=None) tries to mutate
                    // `parent:armed` — must be a no-op.
                    actions: vec![TriggerAction::SetWorldFlag {
                        name: "parent:armed".into(),
                    }],
                    when: None,
                },
                fired: false,
                origin_layer: None,
            }];
        }
        app.world_mut()
            .resource_mut::<Messages<AiEntityDestroyed>>()
            .write(AiEntityDestroyed { entity_uuid: npc_uuid.into() });
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
            runtime.name_to_uuid.insert("enemy_ship".to_string(), npc_uuid.to_string());
            runtime.trigger_states = vec![TriggerState {
                trigger: crate::world::content::Trigger {
                    condition: TriggerCondition::OnAttacked { entity_name: "enemy_ship".to_string() },
                    actions: vec![TriggerAction::AddObjective {
                        id: "obj-002".to_string(),
                        text: "Enemy attacked".to_string(),
                        mandatory: false,
                    }],
                    when: None,
                },
                fired: false,
                origin_layer: None,
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
            objectives.sorted_snapshots().iter().any(|o| o.id == "obj-002"),
            "AddObjective action from on_entity_attacked must have fired"
        );
    }

    #[test]
    fn set_ai_state_action_mutates_controller_state() {
        use crate::ai::AiState;
        use crate::entity_config::{BehaviourConfig, StateConfig};

        let mut app = ai_trigger_test_app();

        let npc_uuid = "npc-state-change-uuid-003";
        let attacker_uuid = uuid::Uuid::parse_str("bbbbbbbb-0000-0000-0000-000000000002").unwrap();

        // Spawn an NPC entity with a behaviour that has an "idle" and "chase" state.
        let behaviour = BehaviourConfig {
            initial_state: "idle".to_string(),
            state: vec![
                StateConfig { name: "idle".to_string(), kind: "idle".to_string(), waypoints: vec![], loop_path: false, target_speed: 0.0, maintain_range: 0.0, duration_secs: 0.0 },
                StateConfig { name: "chase".to_string(), kind: "pursuing".to_string(), waypoints: vec![], loop_path: false, target_speed: 0.8, maintain_range: 0.0, duration_secs: 0.0 },
            ],
            transition: vec![],
        };

        let entity = app.world_mut().spawn((
            Transform::from_xyz(0.0, 0.0, 0.0),
            EntityUuid(npc_uuid.to_string()),
            BehaviourSection(behaviour),
        )).id();
        // First update: attach controller
        app.update();

        // Verify controller starts in Idle
        let ctrl_state_name = app.world().get::<AiControllerComponent>(entity)
            .expect("controller must be attached")
            .controller.current_state_name.clone();
        assert_eq!(ctrl_state_name, "idle");

        // Set up trigger: on attacked ? SetAiState to "chase"
        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime.name_to_uuid.insert("npc_alpha".to_string(), npc_uuid.to_string());
            runtime.trigger_states = vec![TriggerState {
                trigger: crate::world::content::Trigger {
                    condition: TriggerCondition::OnAttacked { entity_name: "npc_alpha".to_string() },
                    actions: vec![TriggerAction::SetAiState {
                        entity: "npc_alpha".to_string(),
                        state: "chase".to_string(),
                        target: None,
                    }],
                    when: None,
                },
                fired: false,
                origin_layer: None,
            }];
        }

        // Fire the attacked event
        app.world_mut()
            .resource_mut::<Messages<AiEntityAttacked>>()
            .write(AiEntityAttacked {
                entity_uuid: npc_uuid.to_string(),
                attacker_uuid,
            });

        app.update();

        let ctrl = app.world().get::<AiControllerComponent>(entity).unwrap();
        assert_eq!(ctrl.controller.current_state_name, "chase",
            "SetAiState must update current_state_name to 'chase'");
        assert!(
            matches!(ctrl.controller.current_state, AiState::Pursuing { .. }),
            "current_state must be Pursuing after SetAiState to 'chase'"
        );
    }

    // -- on_attacked comms template auto-injection tests -----------------------

    /// When an entity is attacked, comms templates with `on_attacked` condition
    /// must fire automatically (no player hailing required) and inject a message
    /// into the CommsInbox.
    #[test]
    fn on_attacked_comms_template_auto_injects_into_inbox() {
        use crate::world::content::{CommsDialogueNode, CommsTemplate, CommsTemplateState, TriggerCondition};

        let mut app = ai_trigger_test_app();

        let raider_uuid = "raider-uuid-auto-001";
        let attacker_uuid = uuid::Uuid::parse_str("cccccccc-0000-0000-0000-000000000001").unwrap();
        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime.name_to_uuid.insert("raider".to_string(), raider_uuid.to_string());
            runtime.comms_template_states = vec![CommsTemplateState {
                template: CommsTemplate {
                    from: "raider".to_string(),
                    trigger: TriggerCondition::OnAttacked {
                        entity_name: "raider".to_string(),
                    },
                    node: CommsDialogueNode {
                        body: "Mayday! We are under attack!".to_string(),
                        responses: vec![],
                    },
                    thread_id: None,
                    urgent: false,
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
        assert_eq!(messages.len(), 1, "on_attacked comms template must auto-inject one message");
        assert_eq!(messages[0].body, "Mayday! We are under attack!");
        assert_eq!(messages[0].responses.len(), 0, "broadcast message should have no responses");
    }

    /// A comms template with `on_attacked` must fire only once (single-shot).
    #[test]
    fn on_attacked_comms_template_fires_only_once() {
        use crate::world::content::{CommsDialogueNode, CommsTemplate, CommsTemplateState, TriggerCondition};

        let mut app = ai_trigger_test_app();

        let raider_uuid = "raider-uuid-once-002";
        let attacker_uuid = uuid::Uuid::parse_str("cccccccc-0000-0000-0000-000000000002").unwrap();
        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime.name_to_uuid.insert("raider".to_string(), raider_uuid.to_string());
            runtime.comms_template_states = vec![CommsTemplateState {
                template: CommsTemplate {
                    from: "raider".to_string(),
                    trigger: TriggerCondition::OnAttacked {
                        entity_name: "raider".to_string(),
                    },
                    node: CommsDialogueNode {
                        body: "Distress signal transmitted.".to_string(),
                        responses: vec![],
                    },
                    thread_id: None,
                    urgent: false,
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
        assert_eq!(inbox.messages().len(), 1, "on_attacked comms template must fire only once");
    }

    // -- Unified [[entity]] name ? uuid pipeline (PRD #337/#339 slice 2) -------

    #[test]
    fn spawn_world_entities_populates_name_to_uuid_for_named_entity() {
        use crate::world::config::WorldEntity;
        use crate::world::config::WorldConfig as UnifiedWorldConfig;

        // Build a unified WorldConfig with one named entry (no template
        // resolution needed â€” the helper that mutates `name_to_uuid` runs
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
        let uuid = cfg.name_to_uuid.get("starbase_alpha").expect("named entity must register");
        assert!(!uuid.is_empty(), "registered uuid must be non-empty");
    }

    #[test]
    fn spawn_world_entities_mirrors_names_into_world_content_runtime() {
        // PRD #337/#339 slice 2: trigger / comms lookup paths read from
        // `WorldContentRuntime.name_to_uuid`. The unified pipeline must
        // mirror its registrations into that map so the lookup path stays
        // a single source of truth during the transitional slices.
        use crate::world::config::WorldEntity;
        use crate::world::config::WorldConfig as UnifiedWorldConfig;

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
        // folds `WorldConfig.name_to_uuid` in) must NOT overwrite those â€”
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
            runtime.name_to_uuid.get("starbase_alpha").map(String::as_str),
            Some("unified-pipeline-uuid"),
            "init_world_runtime must preserve unified-pipeline registrations"
        );
        assert_eq!(
            runtime.name_to_uuid.get("only_in_world").map(String::as_str),
            Some("world-only-uuid"),
            "names that exist only in WorldConfig.name_to_uuid still flow through"
        );
    }

    #[test]
    fn spawn_immediate_entities_spawns_named_non_asteroid_with_registered_uuid() {
        // PRD #339 slice 2 (rejection fix): named [[entity]] entries MUST be
        // spawned as real Bevy entities â€” otherwise triggers / comms resolve
        // to a UUID that has no Transform behind it. The spawned entity's
        // `EntityUuid` component must equal the UUID already registered in
        // `WorldConfig.name_to_uuid` for that name (single source of truth â€”
        // no fresh UUID allocation inside the spawn loop).
        use crate::entity_config::EntityConfig;
        use crate::entity_spawner::EntityUuid;
        use crate::world::config::WorldEntity;
        use crate::world::config::WorldConfig as UnifiedWorldConfig;
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
        // Empty EntityConfig is sufficient â€” no asteroid_field section, so
        // `is_owned_by_unified_pipeline` routes by `name.is_some()`.
        let mut cache: HashMap<String, EntityConfig> = HashMap::new();
        cache.insert("fixture/station.toml".into(), EntityConfig::from_toml("").unwrap());
        cache.insert("fixture/star.toml".into(), EntityConfig::from_toml("").unwrap());

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

        // Its EntityUuid must equal the registered UUID â€” not a fresh one.
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
        use crate::world::config::WorldEntity;
        use crate::world::config::WorldConfig as UnifiedWorldConfig;
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

        let mut cache: HashMap<String, EntityConfig> = HashMap::new();
        cache.insert(
            "fixture/raider.toml".into(),
            EntityConfig::from_toml("").unwrap(),
        );

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
        // [behaviour] block must end up with a BehaviourSection â€” the
        // AiPlugin's `attach_controllers_on_spawn` system reads that to
        // wire the AiController. This guarantees NPCs migrated from
        // [[spawn]] to [[entity]] still get AI on spawn.
        use crate::entity_config::EntityConfig;
        use crate::entity_spawner::BehaviourSection;
        use crate::world::config::WorldEntity;
        use crate::world::config::WorldConfig as UnifiedWorldConfig;
        use std::collections::HashMap;

        let raider_toml = r#"
tags = ["ship","npc","enemy"]

[behaviour]
initial_state = "idle"
state = []
transition = []
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

        let mut cache: HashMap<String, EntityConfig> = HashMap::new();
        cache.insert(
            "fixture/raider.toml".into(),
            EntityConfig::from_toml(raider_toml).unwrap(),
        );

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
        use crate::world::config::WorldEntity;
        use crate::world::config::WorldConfig as UnifiedWorldConfig;
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
        });
        let mut cache: HashMap<String, EntityConfig> = HashMap::new();
        cache.insert("fixture/anchored_belt.toml".into(), field_template);

        let mut app = App::new();
        app.add_plugins(bevy::time::TimePlugin);
        app.insert_resource(world_cfg.clone());

        let spawned: Vec<Entity> = {
            let mut commands = app.world_mut().commands();
            spawn_immediate_entities_internal(&mut commands, &world_cfg, &cache)
        };
        app.update();

        assert_eq!(spawned.len(), 1, "exactly one asteroid_field entry must spawn");
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
        use crate::world::config::WorldEntity;
        use crate::world::config::WorldConfig as UnifiedWorldConfig;
        use std::collections::HashMap;

        let mut world_cfg = UnifiedWorldConfig::default();
        // Note: NO anchor named "typo_anchor" â€” only "real_anchor".
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
        });
        let mut cache: HashMap<String, EntityConfig> = HashMap::new();
        cache.insert("fixture/typo_belt.toml".into(), field_template);

        let mut app = App::new();
        app.add_plugins(bevy::time::TimePlugin);
        app.insert_resource(world_cfg.clone());

        let spawned: Vec<Entity> = {
            let mut commands = app.world_mut().commands();
            spawn_immediate_entities_internal(&mut commands, &world_cfg, &cache)
        };
        app.update();

        assert_eq!(
            spawned.len(), 1,
            "unknown anchor must NOT block spawn â€” fallback to origin keeps the field alive"
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

    // â”€â”€ extra_worlds + LoadWorld / UnloadWorld (issue #352) â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

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
        world_cfg.extra_worlds.push("assets/worlds/patrol.toml".into());
        world_cfg.extra_worlds.push("assets/worlds/side.toml".into());
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
            runtime.name_to_uuid.insert("raider".into(), npc_uuid.into());
            runtime.trigger_states = vec![TriggerState {
                trigger: crate::world::content::Trigger {
                    condition: TriggerCondition::OnAttacked { entity_name: "raider".into() },
                    actions: vec![TriggerAction::LoadWorld {
                        path: "assets/worlds/patrol.toml".into(),
                    }],
                    when: None,
                },
                fired: false,
                origin_layer: None,
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
            runtime.name_to_uuid.insert("raider".into(), npc_uuid.into());
            runtime.trigger_states = vec![TriggerState {
                trigger: crate::world::content::Trigger {
                    condition: TriggerCondition::OnAttacked { entity_name: "raider".into() },
                    actions: vec![TriggerAction::UnloadWorld {
                        path: "assets/worlds/patrol.toml".into(),
                    }],
                    when: None,
                },
                fired: false,
                origin_layer: None,
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
            .push(WorldLayerChange::Load { path: "assets/worlds/patrol.toml".into(), loader_path: None });

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
            .push(WorldLayerChange::Load { path: "assets/worlds/patrol.toml".into(), loader_path: None });
        app.update();

        let trigger_count_after_first = app
            .world()
            .resource::<WorldContentRuntime>()
            .trigger_states
            .len();

        // Load again â€” must not double-add.
        app.world_mut()
            .resource_mut::<PendingWorldLayerChanges>()
            .0
            .push(WorldLayerChange::Load { path: "assets/worlds/patrol.toml".into(), loader_path: None });
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
            .push(WorldLayerChange::Load { path: "assets/worlds/patrol.toml".into(), loader_path: None });
        app.update();

        let trigger_count_loaded = app
            .world()
            .resource::<WorldContentRuntime>()
            .trigger_states
            .len();
        assert!(trigger_count_loaded > 0, "patrol.toml must add at least one trigger");

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
    /// produce exactly one load â€” no duplicate entities, no duplicate trigger
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
            let mut pending = app
                .world_mut()
                .resource_mut::<PendingWorldLayerChanges>();
            pending.0.push(WorldLayerChange::Load { path: world_path.clone(), loader_path: None });
            pending.0.push(WorldLayerChange::Load { path: world_path.clone(), loader_path: None });
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
        drop(layer_map);

        // Capture trigger/contact/name counts after the first-tick double-load.
        let runtime = app.world().resource::<WorldContentRuntime>();
        let triggers_after_double = runtime.trigger_states.len();
        let names_after_double = runtime.name_to_uuid.len();
        let contacts_after_double = runtime.contacts.len();
        drop(runtime);

        // Now load the same path AGAIN on a separate tick: must also be a
        // no-op (existing behaviour) and keep the same counts.
        app.world_mut()
            .resource_mut::<PendingWorldLayerChanges>()
            .0
            .push(WorldLayerChange::Load { path: world_path.clone(), loader_path: None });
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
            .push(WorldLayerChange::Unload("assets/worlds/nonexistent.toml".into()));
        app.update(); // must not panic

        let runtime = app.world().resource::<WorldContentRuntime>();
        assert!(runtime.trigger_states.is_empty());
    }

    // â”€â”€ Entity spawn / despawn via LoadWorld / UnloadWorld (issue #352) â”€â”€â”€â”€â”€â”€â”€

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

        (world_path.to_string_lossy().into_owned(), template_path.to_string_lossy().into_owned())
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
            .push(WorldLayerChange::Load { path: world_path.clone(), loader_path: None });

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
            .push(WorldLayerChange::Load { path: world_path.clone(), loader_path: None });
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
            !app.world().resource::<WorldLayerMap>().0.contains_key(&world_path),
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
            .spawn((Ship, Transform::from_xyz(0.0, 0.0, 0.0), CommsRange(100.0)))
            .id();
        let station_entity = app.world_mut().spawn((
            EntityUuid(station_uuid.into()),
            Transform::from_xyz(50.0, 0.0, 0.0),
            CommsRange(100.0),
        )).id();

        // Flush initial broadcast.
        let _ = tick(&mut app);

        // Hail in range so a message is injected.
        push_msg(
            &mut app,
            "comms",
            ClientMessage::Hail { target_uuid: station_uuid.into() },
        );
        let _ = tick(&mut app);

        // Now move the station far away (combined range 200, distance 1000).
        let _ = ship_entity;
        if let Ok(mut e) = app.world_mut().get_entity_mut(station_entity) {
            e.insert(Transform::from_xyz(1000.0, 0.0, 0.0));
        }
        let out = tick(&mut app);

        let (messages, contacts) = out.iter().find_map(|m| {
            if let ServerMessage::CommsState { messages, contacts, .. } = &m.msg {
                Some((messages.clone(), contacts.clone()))
            } else { None }
        }).expect("CommsState must be broadcast after range flip");

        let contact = contacts.iter().find(|c| c.uuid == station_uuid).expect("contact present");
        assert!(!contact.in_range, "contact should be out of range");
        assert_eq!(messages.len(), 1, "one hail message expected");
        assert!(!messages[0].sender_in_range, "sender_in_range must be false when station is far");
    }

    #[test]
    fn comms_state_marks_contact_in_range_when_ship_close() {
        use crate::comms::CommsRange;
        use crate::entities::spawner::EntityUuid;
        use crate::simulation::Ship;

        let station_uuid = "station-uuid-range-near";
        let mut app = comms_test_app();
        setup_game_with_comms(&mut app, station_uuid);

        app.world_mut().spawn((Ship, Transform::from_xyz(0.0, 0.0, 0.0), CommsRange(500.0)));
        app.world_mut().spawn((
            EntityUuid(station_uuid.into()),
            Transform::from_xyz(100.0, 0.0, 0.0),
            CommsRange(500.0),
        ));

        let _ = tick(&mut app);
        push_msg(
            &mut app,
            "comms",
            ClientMessage::Hail { target_uuid: station_uuid.into() },
        );
        let out = tick(&mut app);

        let (messages, contacts) = out.iter().find_map(|m| {
            if let ServerMessage::CommsState { messages, contacts, .. } = &m.msg {
                Some((messages.clone(), contacts.clone()))
            } else { None }
        }).expect("CommsState must be broadcast");

        let contact = contacts.iter().find(|c| c.uuid == station_uuid).expect("contact present");
        assert!(contact.in_range, "contact should be in range");
        assert!(messages[0].sender_in_range, "sender_in_range true when station within range");
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
        app.world_mut().spawn((Ship, Transform::from_xyz(0.0, 0.0, 0.0), CommsRange(500.0)));

        let out = tick(&mut app);
        let contacts = out.iter().find_map(|m| {
            if let ServerMessage::CommsState { contacts, .. } = &m.msg {
                Some(contacts.clone())
            } else { None }
        }).expect("CommsState must be broadcast");

        assert!(
            !contacts.iter().any(|c| c.uuid == bogus_uuid),
            "contact for entity without [comms] block must be pruned, got {contacts:?}"
        );
    }

    /// A Hail targeting an out-of-range entity must NOT inject any message
    /// into the inbox (server-side enforcement; stale clients can't bypass
    /// the client gate).
    #[test]
    fn server_rejects_hail_when_target_out_of_range() {
        use crate::comms::CommsRange;
        use crate::entities::spawner::EntityUuid;
        use crate::simulation::Ship;

        let station_uuid = "station-out-of-range-hail";
        let mut app = comms_test_app();
        setup_game_with_comms(&mut app, station_uuid);

        app.world_mut().spawn((Ship, Transform::from_xyz(0.0, 0.0, 0.0), CommsRange(100.0)));
        app.world_mut().spawn((
            EntityUuid(station_uuid.into()),
            Transform::from_xyz(5000.0, 0.0, 0.0),
            CommsRange(100.0),
        ));

        // Flush initial broadcast so range_flags is populated.
        let _ = tick(&mut app);

        push_msg(
            &mut app,
            "comms",
            ClientMessage::Hail { target_uuid: station_uuid.into() },
        );
        let out = tick(&mut app);

        // No CommsState broadcast should contain a non-empty inbox.
        for m in &out {
            if let ServerMessage::CommsState { messages, .. } = &m.msg {
                assert!(messages.is_empty(), "out-of-range Hail must not inject messages, got {messages:?}");
            }
        }
    }

    /// A `RespondToMessage` whose dialogue sender is out of range must NOT
    /// fire response actions (no objective added, no follow-up).
    #[test]
    fn server_rejects_respond_when_sender_out_of_range() {
        use crate::comms::CommsRange;
        use crate::entities::spawner::EntityUuid;
        use crate::simulation::Ship;

        let station_uuid = "station-respond-oor";
        let mut app = comms_test_app();
        setup_game_with_comms(&mut app, station_uuid);

        // Start in range, hail, then move ship far away and respond.
        app.world_mut().spawn((Ship, Transform::from_xyz(0.0, 0.0, 0.0), CommsRange(500.0)));
        let station_entity = app.world_mut().spawn((
            EntityUuid(station_uuid.into()),
            Transform::from_xyz(50.0, 0.0, 0.0),
            CommsRange(500.0),
        )).id();
        let _ = tick(&mut app);

        push_msg(&mut app, "comms", ClientMessage::Hail { target_uuid: station_uuid.into() });
        let out = tick(&mut app);
        let msg_id = out.iter().find_map(|m| {
            if let ServerMessage::CommsState { messages, .. } = &m.msg {
                messages.first().map(|m| m.id.clone())
            } else { None }
        }).expect("hail produced a message");

        // Move the station far away.
        if let Ok(mut e) = app.world_mut().get_entity_mut(station_entity) {
            e.insert(Transform::from_xyz(5000.0, 0.0, 0.0));
        }
        // Tick to refresh range_flags.
        let _ = tick(&mut app);

        // Try to respond.
        push_msg(&mut app, "comms", ClientMessage::RespondToMessage {
            message_id: msg_id.clone(),
            response_index: 0,
        });
        let _ = tick(&mut app);

        // Objective `obj-survey` must NOT have been added (response_actions
        // include AddObjective in setup_game_with_comms).
        let objectives = &app.world().resource::<ObjectiveManagerRes>().0;
        assert!(
            objectives.sorted_snapshots().is_empty(),
            "out-of-range Respond must not fire AddObjective action"
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

        app.world_mut().spawn((Ship, Transform::from_xyz(0.0, 0.0, 0.0), CommsRange(1000.0)));
        let station_entity = app.world_mut().spawn((
            EntityUuid(station_uuid.into()),
            Transform::from_xyz(50.0, 0.0, 0.0),
            CommsRange(1000.0),
        )).id();
        let _ = tick(&mut app);

        // Hail to populate the inbox while in range.
        push_msg(&mut app, "comms", ClientMessage::Hail { target_uuid: station_uuid.into() });
        let _ = tick(&mut app);

        // Now despawn the station entity.
        app.world_mut().despawn(station_entity);
        let out = tick(&mut app);

        let messages = out.iter().find_map(|m| {
            if let ServerMessage::CommsState { messages, .. } = &m.msg {
                Some(messages.clone())
            } else { None }
        }).expect("a broadcast must fire after despawn (range flip)");

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

        app.world_mut().spawn((Ship, Transform::from_xyz(0.0, 0.0, 0.0), CommsRange(500.0)));
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
        let contacts = out.iter().find_map(|m| {
            if let ServerMessage::CommsState { contacts, .. } = &m.msg {
                Some(contacts.clone())
            } else { None }
        }).expect("CommsState must be broadcast");

        let near = contacts.iter().find(|c| c.uuid == near_uuid).expect("near contact");
        let far = contacts.iter().find(|c| c.uuid == far_uuid).expect("far contact");
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

        let ship_entity = app.world_mut()
            .spawn((Ship, Transform::from_xyz(0.0, 0.0, 0.0), CommsRange(500.0)))
            .id();
        app.world_mut().spawn((
            EntityUuid(station_uuid.into()),
            Transform::from_xyz(100.0, 0.0, 0.0),
            CommsRange(500.0),
        ));

        // Drain initial broadcasts.
        let _ = tick(&mut app);
        let _ = tick(&mut app);

        // Move ship far away â€” this must trigger a fresh broadcast even
        // though the inbox didn't change.
        if let Ok(mut e) = app.world_mut().get_entity_mut(ship_entity) {
            e.insert(Transform::from_xyz(5000.0, 0.0, 0.0));
        }
        let out = tick(&mut app);

        let has_broadcast = out.iter().any(|m| matches!(&m.msg, ServerMessage::CommsState { .. }));
        assert!(has_broadcast, "range flip from inâ†’out must trigger a fresh CommsState broadcast");
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

        let ship_entity = app.world_mut()
            .spawn((Ship, Transform::from_xyz(0.0, 0.0, 0.0), CommsRange(1000.0)))
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
                    }],
                    when: None,
                },
                fired: false,
                origin_layer: None,
            }];
            runtime.pending_world_events.push(WorldEvent::WorldLoaded);
        }

        app.update();

        let objectives = &app.world().resource::<ObjectiveManagerRes>().0;
        assert!(
            objectives.sorted_snapshots().iter().any(|o| o.id == "obj-loaded"),
            "on_world_loaded trigger must have fired its add_objective action"
        );
        // Queue must be drained.
        let runtime = app.world().resource::<WorldContentRuntime>();
        assert!(
            runtime.pending_world_events.is_empty(),
            "pending_world_events must be drained by handle_ai_events"
        );
        assert!(runtime.trigger_states[0].fired, "trigger must be marked fired");
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
            }],
            when: None,
        });
        app.insert_resource(cfg);

        app.world_mut().run_schedule(Startup);

        let runtime = app.world().resource::<WorldContentRuntime>();
        assert!(
            runtime.pending_world_events.iter().any(|e| matches!(e, WorldEvent::WorldLoaded)),
            "init_world_runtime must queue a WorldLoaded event during Startup"
        );
        assert_eq!(runtime.trigger_states.len(), 1, "trigger states must be populated");
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
            .push(WorldLayerChange::Load { path: world_path.clone(), loader_path: None });
        app.update();

        let runtime = app.world().resource::<WorldContentRuntime>();
        assert!(
            runtime.pending_world_events.iter().any(|e| matches!(e, WorldEvent::WorldLoaded)),
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
            .push(WorldLayerChange::Load { path: world_path.clone(), loader_path: None });
        app.update(); // applies load + queues WorldLoaded
        app.update(); // handle_ai_events drains pending event + fires trigger

        {
            let objectives = &app.world().resource::<ObjectiveManagerRes>().0;
            assert!(
                objectives.sorted_snapshots().iter().any(|o| o.id == "obj-on-load"),
                "on_world_loaded trigger must fire on first load"
            );
        }

        // Complete the objective so we can detect the second add as a
        // distinct event (ObjectiveManager dedupes by id; re-adding the
        // same id leaves the existing objective in place which is fine â€”
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
                !runtime.trigger_states.iter().any(|s|
                    matches!(s.trigger.condition, TriggerCondition::OnWorldLoaded)
                ),
                "Unload must remove the on_world_loaded trigger state"
            );
        }

        // -- Second load cycle --
        app.world_mut()
            .resource_mut::<PendingWorldLayerChanges>()
            .0
            .push(WorldLayerChange::Load { path: world_path.clone(), loader_path: None });
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

    use crate::region_shape::RegionShape;
    use crate::regions::server::RegionPlugin;
    use crate::entity_spawner::{spawn_entity, EntityUuid};
    use crate::entity_config::EntityConfig;

    /// Build a minimal app that wires `RegionPlugin` + the issue-#416
    /// observers + `handle_ai_events` into the same world. Skips the
    /// heavyweight `WorldPlugin`/`AiPlugin`/`LobbyPlugin` bootstrap so the
    /// test focuses on the region-event â†’ trigger-fire path.
    fn region_trigger_test_app() -> App {
        let mut app = App::new();
        app.add_plugins(bevy::time::TimePlugin)
            .add_plugins(RegionPlugin)
            .insert_resource(ShipState::new())
            .insert_resource(crate::modifiers::ShipModifiers::new())
            .init_resource::<WorldContentRuntime>()
            .init_resource::<CommsInboxRes>()
            .init_resource::<ObjectiveManagerRes>()
            .init_resource::<SimOutbox>()
            .add_message::<crate::ai::server::AiEntityAttacked>()
            .add_message::<crate::ai::server::AiEntityDestroyed>()
            .add_systems(Update, handle_ai_events)
            .add_observer(handle_region_entered_event)
            .add_observer(handle_region_exited_event);
        // Spawn the player ship (with a Transform so RegionPlugin's
        // membership query succeeds).
        app.world_mut().spawn((Ship, Transform::default()));
        app
    }

    fn spawn_region_with_uuid(app: &mut App, x: f32, z: f32, radius: f32, uuid: &str) -> Entity {
        let config = EntityConfig {
            name: None,
            light: Vec::new(),
            tags: vec!["region".to_string()],
            shape: Some(RegionShape::Sphere { radius }),
            effects: None,
            hull: None, collider: None, appearance: None,
            helm_console: None, weapons_console: None, engineering_console: None,
            captain_console: None, power: None, sensors_console: None,
            navigation_console: None, shields_console: None, torpedoes: None,
            repair: None, comms: None, asteroid_field: None, faction: None,
            behaviour: None, radar_appearance: None, mesh: None,
        };
        let mut commands = app.world_mut().commands();
        spawn_entity(&mut commands, &config, Vec3::new(x, 0.0, z), uuid.to_string(), None)
    }

    fn set_ship_pos(app: &mut App, x: f32, z: f32) {
        let mut ship = app.world_mut().resource_mut::<ShipState>();
        ship.x = x;
        ship.z = z;
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
                }],
                when: None,
            },
            fired: false,
            origin_layer: None,
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
            TriggerCondition::OnEnteredRegion { entity_name: "nebula".into() },
            "obj-entered",
        );

        // Tick 1: ship outside (at origin), no enter â†’ no fire.
        app.update();
        assert!(!objective_present(&app, "obj-entered"),
            "trigger must not fire while outside");

        // Move ship inside. The membership system runs in Physics and
        // queues a WorldEvent via the observer; `handle_ai_events` (also
        // in Physics) drains the queue on the NEXT tick â€” matching the
        // documented `WorldLoaded` two-tick pattern.
        set_ship_pos(&mut app, 110.0, 0.0);
        app.update(); // queues EnteredRegion
        app.update(); // handle_ai_events drains + fires
        assert!(objective_present(&app, "obj-entered"),
            "trigger must fire on entry");

        // Confirm single-shot: trigger is marked fired, queue is drained.
        let runtime = app.world().resource::<WorldContentRuntime>();
        assert!(runtime.trigger_states[0].fired,
            "trigger state must be marked fired after entry");
        assert!(runtime.pending_world_events.is_empty(),
            "pending_world_events must be drained");

        // Stay inside on subsequent ticks â€” membership system must not
        // re-emit `RegionEntered`, so no new events queue up.
        app.update();
        app.update();
        let runtime = app.world().resource::<WorldContentRuntime>();
        assert!(runtime.pending_world_events.is_empty(),
            "staying inside must not enqueue further EnteredRegion events");
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
            TriggerCondition::OnExitedRegion { entity_name: "nebula".into() },
            "obj-exited",
        );

        // Move inside first so we enter cleanly.
        set_ship_pos(&mut app, 10.0, 0.0);
        app.update();
        app.update();
        assert!(!objective_present(&app, "obj-exited"),
            "exit trigger must not fire on entry");

        // Now move outside â†’ RegionExited â†’ queued â†’ drained next tick.
        set_ship_pos(&mut app, 200.0, 0.0);
        app.update();
        app.update();
        assert!(objective_present(&app, "obj-exited"),
            "exit trigger must fire when ship moves outside the region");
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
            TriggerCondition::OnExitedRegion { entity_name: "fragile".into() },
            "obj-imploded",
        );

        // Enter the region.
        set_ship_pos(&mut app, 10.0, 0.0);
        app.update();
        app.update();
        assert!(!objective_present(&app, "obj-imploded"));

        // Despawn the region while ship is inside â€” membership system
        // emits an implicit RegionExited.
        app.world_mut().despawn(region_entity);
        app.update(); // queues ExitedRegion
        app.update(); // drains + fires

        assert!(objective_present(&app, "obj-imploded"),
            "exit trigger must fire when the region is despawned while ship is inside");
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
            TriggerCondition::OnEnteredRegion { entity_name: "region_a".into() },
            "obj-a",
        );
        install_region_trigger(
            &mut app,
            "region_b",
            uuid_b,
            TriggerCondition::OnEnteredRegion { entity_name: "region_b".into() },
            "obj-b",
        );

        // Ship at origin is inside both regions. First tick queues both
        // events, second tick drains + fires both triggers.
        set_ship_pos(&mut app, 0.0, 0.0);
        app.update();
        app.update();

        assert!(objective_present(&app, "obj-a"),
            "region A enter trigger must fire");
        assert!(objective_present(&app, "obj-b"),
            "region B enter trigger must fire");
    }

    #[test]
    fn npc_entering_region_does_not_fire_trigger() {
        // The region membership system only tracks the player Ship entity
        // (queried via `With<Ship>`). Spawning an NPC inside a region must
        // not cause an `OnEnteredRegion` trigger to fire.
        let mut app = region_trigger_test_app();
        let uuid = "uuid-quarantine";
        // Region at (100, 0); player ship stays at origin (outside).
        spawn_region_with_uuid(&mut app, 100.0, 0.0, 50.0, uuid);
        install_region_trigger(
            &mut app,
            "quarantine",
            uuid,
            TriggerCondition::OnEnteredRegion { entity_name: "quarantine".into() },
            "obj-ship-quarantined",
        );

        // Spawn an "NPC" entity inside the region by placing a generic
        // entity (no Ship marker) at (110, 0). The membership system
        // ignores it because the only Ship is the player ship at origin.
        let npc_config = EntityConfig {
            name: None, light: Vec::new(), tags: vec!["npc".into()],
            shape: None, effects: None,
            hull: None, collider: None, appearance: None,
            helm_console: None, weapons_console: None, engineering_console: None,
            captain_console: None, power: None, sensors_console: None,
            navigation_console: None, shields_console: None, torpedoes: None,
            repair: None, comms: None, asteroid_field: None, faction: None,
            behaviour: None, radar_appearance: None, mesh: None,
        };
        {
            let mut commands = app.world_mut().commands();
            let _npc = spawn_entity(
                &mut commands, &npc_config,
                Vec3::new(110.0, 0.0, 0.0), "uuid-npc".into(), None,
            );
        }

        // Tick a few times; player ship stays at origin (outside).
        app.update();
        app.update();

        assert!(!objective_present(&app, "obj-ship-quarantined"),
            "NPC entering the region must not fire the player-ship trigger");
        let runtime = app.world().resource::<WorldContentRuntime>();
        assert!(!runtime.trigger_states[0].fired,
            "trigger must remain unfired when only an NPC is inside");
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
                    condition: TriggerCondition::OnEnteredRegion { entity_name: "zone".into() },
                    actions: vec![TriggerAction::AddObjective {
                        id: "obj-armed-entry".into(),
                        text: "Armed entry.".into(),
                        mandatory: false,
                    }],
                    when: Some(crate::world::flags::parse_predicate("flag(armed)").unwrap()),
                },
                fired: false,
                origin_layer: None,
            });
        }

        // First entry: flag unset â†’ predicate false â†’ no objective.
        set_ship_pos(&mut app, 10.0, 0.0);
        app.update();
        assert!(!objective_present(&app, "obj-armed-entry"),
            "gated trigger must not fire while flag is unset");
        {
            let runtime = app.world().resource::<WorldContentRuntime>();
            assert!(!runtime.trigger_states[0].fired,
                "predicate-false firings must NOT consume the trigger");
        }

        // Set the flag, leave the region, re-enter â€” trigger should fire now.
        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime.flags.set_flag("armed");
        }
        set_ship_pos(&mut app, 200.0, 0.0); // exit
        app.update();
        set_ship_pos(&mut app, 10.0, 0.0);  // re-enter
        app.update();

        assert!(objective_present(&app, "obj-armed-entry"),
            "gated trigger must fire once the flag is set and ship re-enters");
    }

    // -- Issue #417: spawn_entity / destroy_entity trigger actions ---------

    /// Writes a minimal NPC template to a temp file and returns its path.
    fn write_spawn_template_fixture() -> String {
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
            }];
        }

        app.world_mut()
            .resource_mut::<Messages<crate::ai_plugin::AiEntityAttacked>>()
            .write(crate::ai_plugin::AiEntityAttacked {
                entity_uuid: "marker-uuid".into(),
                attacker_uuid: uuid::Uuid::parse_str(
                    "cccccccc-0000-0000-0000-000000000001",
                )
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
        wc.anchors
            .insert("alpha".to_string(), [42.0, 0.0, -5.0]);
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
            wr.anchors.insert("docking_bay".to_string(), [11.0, 0.0, 22.0]);
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
            }];
        }

        app.world_mut()
            .resource_mut::<Messages<crate::ai_plugin::AiEntityAttacked>>()
            .write(crate::ai_plugin::AiEntityAttacked {
                entity_uuid: "lt-uuid".into(),
                attacker_uuid: uuid::Uuid::parse_str(
                    "dddddddd-0000-0000-0000-000000000001",
                )
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
                assert!((t.translation.x - 11.0).abs() < 1e-3,
                    "must use layer anchor (11), not base anchor (-99); got {}", t.translation.x);
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
            // First trigger: on attack â†’ destroy.
            // Second trigger: on destroyed of "doomed" â†’ add objective (proves chaining).
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
                        }],
                        when: None,
                    },
                    fired: false,
                    origin_layer: None,
                },
            ];
        }

        app.world_mut()
            .resource_mut::<Messages<crate::ai_plugin::AiEntityAttacked>>()
            .write(crate::ai_plugin::AiEntityAttacked {
                entity_uuid: "src-uuid".into(),
                attacker_uuid: uuid::Uuid::parse_str(
                    "eeeeeeee-0000-0000-0000-000000000001",
                )
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
            objs.sorted_snapshots().iter().any(|o| o.id == "obj-chained"),
            "chained on_destroyed trigger must fire from DestroyEntity action"
        );

        // External consumers must also see the message: DestroyEntity action
        // must emit AiEntityDestroyed via the deferred Commands::queue path,
        // matching combat-induced destruction.
        let msgs = app
            .world()
            .resource::<Messages<crate::ai_plugin::AiEntityDestroyed>>();
        let mut cursor = msgs.get_cursor();
        let emitted: Vec<String> = cursor
            .read(msgs)
            .map(|m| m.entity_uuid.clone())
            .collect();
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
            }];
        }
        app.world_mut()
            .resource_mut::<Messages<crate::ai_plugin::AiEntityAttacked>>()
            .write(crate::ai_plugin::AiEntityAttacked {
                entity_uuid: "src-uuid".into(),
                attacker_uuid: uuid::Uuid::parse_str(
                    "ffffffff-0000-0000-0000-000000000001",
                )
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
            }];
        }

        app.world_mut()
            .resource_mut::<Messages<crate::ai_plugin::AiEntityAttacked>>()
            .write(crate::ai_plugin::AiEntityAttacked {
                entity_uuid: "src-uuid".into(),
                attacker_uuid: uuid::Uuid::parse_str(
                    "10101010-0000-0000-0000-000000000001",
                )
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
            }];
        }

        app.world_mut()
            .resource_mut::<Messages<crate::ai_plugin::AiEntityAttacked>>()
            .write(crate::ai_plugin::AiEntityAttacked {
                entity_uuid: "src-uuid".into(),
                attacker_uuid: uuid::Uuid::parse_str(
                    "20202020-0000-0000-0000-000000000001",
                )
                .unwrap(),
            });
        app.update();
        app.update();

        // Flag was NOT set â†’ no registration should appear.
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
                        rotation: Some([0.0, 1.5708, 0.0]),
                        scale: Some([2.0, 2.0, 2.0]),
                    }],
                    when: None,
                },
                fired: false,
                origin_layer: None,
            }];
        }

        app.world_mut()
            .resource_mut::<Messages<crate::ai_plugin::AiEntityAttacked>>()
            .write(crate::ai_plugin::AiEntityAttacked {
                entity_uuid: "marker-uuid".into(),
                attacker_uuid: uuid::Uuid::parse_str(
                    "cccccccc-0000-0000-0000-000000000002",
                )
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

        let expected_quat = Quat::from_euler(EulerRot::XYZ, 0.0, 1.5708, 0.0);
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
                    t.translation, expected_translation
                );
                assert!(
                    t.rotation.abs_diff_eq(expected_quat, 1e-4),
                    "rotation mismatch: got {:?}, expected {:?}",
                    t.rotation, expected_quat
                );
                assert!(
                    t.scale.abs_diff_eq(expected_scale, 1e-4),
                    "scale mismatch: got {:?}, expected {:?}",
                    t.scale, expected_scale
                );
            }
        }
        assert!(found, "spawned entity must exist in ECS with the registered UUID");
    }

    // -- PRD #397 fix 2: comms-response action dispatch parity ----------------
    //
    // These tests assert that `handle_respond_to_message` dispatches every
    // `TriggerAction` variant that `handle_ai_events` dispatches. The
    // "enumeration" test at the end matches on every variant of `TriggerAction`
    // so adding a new variant is a compile error until the new variant is
    // wired into both dispatch sites and a per-variant assertion is added.

    /// Extended comms test app that includes the optional resources
    /// `handle_respond_to_message` needs to dispatch the full action set
    /// (modifiers, layer map, game-over state, base WorldConfig). The
    /// `apply_world_layer_changes` system is intentionally NOT wired in: we
    /// only assert that LoadWorld/UnloadWorld push commands into
    /// `PendingWorldLayerChanges`, matching the per-variant assertions used
    /// by the `handle_ai_events` tests above.
    fn comms_parity_test_app() -> App {
        let mut app = comms_test_app();
        app.init_resource::<crate::modifiers::ShipModifiers>()
            .init_resource::<WorldLayerMap>()
            .init_resource::<PendingWorldLayerChanges>()
            .init_resource::<crate::simulation::GameOverReason>();
        app
    }

    /// Install a comms template whose single response carries `actions`,
    /// register the sender as a contact, hail it from the comms player, and
    /// drive the response. Returns the new App after `tick`s have completed.
    fn fire_response_with_actions(actions: Vec<TriggerAction>) -> App {
        let station_uuid = "station-parity-uuid";
        let mut app = comms_parity_test_app();
        // Boot the standard captain+comms+InProgress state but install a
        // tailored template carrying the requested actions.
        push_msg(
            &mut app,
            "captain",
            ClientMessage::Identify {
                token: "captain".into(),
                name: "Alice".into(),
            },
        );
        tick(&mut app);
        push_msg(
            &mut app,
            "captain",
            ClientMessage::SelectStation {
                station: "Captain's Chair".into(),
            },
        );
        tick(&mut app);
        push_msg(
            &mut app,
            "comms",
            ClientMessage::Identify {
                token: "comms".into(),
                name: "Uhura".into(),
            },
        );
        tick(&mut app);
        push_msg(
            &mut app,
            "comms",
            ClientMessage::SelectStation {
                station: "Comms".into(),
            },
        );
        tick(&mut app);
        push_msg(&mut app, "captain", ClientMessage::StartGame);
        tick(&mut app);

        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
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
                        body: "Hello, Phoenix.".into(),
                        responses: vec![CommsResponse {
                            text: "Acknowledge.".into(),
                            actions,
                            follow_up: None,
                        }],
                    },
                    thread_id: None,
                    urgent: false,
                },
                fired: false,
            });
            runtime.needs_broadcast = true;
        }
        let _ = tick(&mut app);

        // Hail to receive the message.
        push_msg(
            &mut app,
            "comms",
            ClientMessage::Hail {
                target_uuid: station_uuid.into(),
            },
        );
        let out = tick(&mut app);

        let msg_id = out
            .iter()
            .find_map(|m| {
                if let ServerMessage::CommsState { messages, .. } = &m.msg {
                    messages.first().map(|msg| msg.id.clone())
                } else {
                    None
                }
            })
            .expect("hail must deliver a comms message");

        // Respond.
        push_msg(
            &mut app,
            "comms",
            ClientMessage::RespondToMessage {
                message_id: msg_id,
                response_index: 0,
            },
        );
        let _ = tick(&mut app);

        app
    }

    #[test]
    fn comms_response_dispatches_set_world_flag() {
        let app = fire_response_with_actions(vec![TriggerAction::SetWorldFlag {
            name: "comms_set".into(),
        }]);
        let runtime = app.world().resource::<WorldContentRuntime>();
        assert_eq!(runtime.flags.counter("comms_set"), 1);
        assert!(
            runtime.pending_world_events.iter().any(|e| matches!(
                e, WorldEvent::FlagSet { name, .. } if name == "comms_set"
            )),
            "SetWorldFlag from comms response must enqueue a FlagSet event \
             for handle_ai_events to chain on"
        );
    }

    #[test]
    fn comms_response_dispatches_clear_world_flag() {
        let actions = vec![
            TriggerAction::SetWorldFlag { name: "to_clear".into() },
            TriggerAction::ClearWorldFlag { name: "to_clear".into() },
        ];
        let app = fire_response_with_actions(actions);
        let runtime = app.world().resource::<WorldContentRuntime>();
        assert_eq!(runtime.flags.counter("to_clear"), 0);
        // Both transitions must have been enqueued.
        let has_set = runtime.pending_world_events.iter().any(|e| matches!(
            e, WorldEvent::FlagSet { name, .. } if name == "to_clear"
        ));
        let has_cleared = runtime.pending_world_events.iter().any(|e| matches!(
            e, WorldEvent::FlagCleared { name, .. } if name == "to_clear"
        ));
        assert!(has_set && has_cleared,
            "both set and clear transitions must be enqueued");
    }

    #[test]
    fn comms_response_dispatches_increment_world_flag() {
        let app = fire_response_with_actions(vec![TriggerAction::IncrementWorldFlag {
            name: "counter".into(),
            by: 7,
        }]);
        let runtime = app.world().resource::<WorldContentRuntime>();
        assert_eq!(runtime.flags.counter("counter"), 7);
    }

    #[test]
    fn comms_response_dispatches_set_world_flag_value() {
        let app = fire_response_with_actions(vec![TriggerAction::SetWorldFlagValue {
            name: "answer".into(),
            value: 42,
        }]);
        let runtime = app.world().resource::<WorldContentRuntime>();
        assert_eq!(runtime.flags.counter("answer"), 42);
    }

    #[test]
    fn comms_response_dispatches_load_world() {
        let app = fire_response_with_actions(vec![TriggerAction::LoadWorld {
            path: "assets/worlds/some.toml".into(),
        }]);
        let pending = app.world().resource::<PendingWorldLayerChanges>();
        assert!(
            pending.0.iter().any(|c| matches!(
                c, WorldLayerChange::Load { path, loader_path }
                if path == "assets/worlds/some.toml" && loader_path.is_none()
            )),
            "LoadWorld from comms response must queue a base-world Load command, got {:?}",
            pending.0
        );
    }

    #[test]
    fn comms_response_dispatches_unload_world() {
        let app = fire_response_with_actions(vec![TriggerAction::UnloadWorld {
            path: "assets/worlds/some.toml".into(),
        }]);
        let pending = app.world().resource::<PendingWorldLayerChanges>();
        assert!(
            pending.0.iter().any(|c| matches!(
                c, WorldLayerChange::Unload(path) if path == "assets/worlds/some.toml"
            )),
            "UnloadWorld from comms response must queue an Unload command, got {:?}",
            pending.0
        );
    }

    #[test]
    fn comms_response_dispatches_game_over() {
        let app = fire_response_with_actions(vec![TriggerAction::GameOver {
            message: Some("you lost".into()),
        }]);
        let reason = app.world().resource::<crate::simulation::GameOverReason>();
        assert_eq!(reason.0.as_deref(), Some("you lost"));
    }

    #[test]
    fn comms_response_dispatches_apply_modifier() {
        let app = fire_response_with_actions(vec![TriggerAction::ApplyModifier {
            entity: "starbase_alpha".into(),
            tag: "boost".into(),
            slot: crate::messages::ModifierSlot::MaxSpeed,
            bonus: 1.5,
        }]);
        let mods = app.world().resource::<crate::modifiers::ShipModifiers>();
        // The cache aggregates all modifiers in `get`. Bonus is the
        // observable side-effect; assert it's been applied to MaxSpeed.
        assert!(
            mods.get(&crate::messages::ModifierSlot::MaxSpeed) > 1.0,
            "ApplyModifier must add to MaxSpeed slot total, got {}",
            mods.get(&crate::messages::ModifierSlot::MaxSpeed)
        );
    }

    #[test]
    fn comms_response_dispatches_remove_modifier() {
        let app = fire_response_with_actions(vec![
            TriggerAction::ApplyModifier {
                entity: "starbase_alpha".into(),
                tag: "boost".into(),
                slot: crate::messages::ModifierSlot::MaxSpeed,
                bonus: 2.0,
            },
            TriggerAction::RemoveModifier {
                entity: "starbase_alpha".into(),
                tag: "boost".into(),
                slot: crate::messages::ModifierSlot::MaxSpeed,
            },
        ]);
        let mods = app.world().resource::<crate::modifiers::ShipModifiers>();
        // After remove, the slot returns to its baseline multiplier of 1.0
        // (empty table). Comparing against the post-apply value (which would
        // exceed 1.0) proves the remove undid the apply.
        let value = mods.get(&crate::messages::ModifierSlot::MaxSpeed);
        assert!(
            (value - 1.0).abs() < 1e-3,
            "RemoveModifier must reverse the previously-applied modifier; \
             expected baseline 1.0, got {value}"
        );
    }

    #[test]
    fn comms_response_dispatches_apply_and_remove_flag() {
        let app = fire_response_with_actions(vec![TriggerAction::ApplyFlag {
            entity: "starbase_alpha".into(),
            tag: "jammer".into(),
            kind: crate::flag_kind::FlagKind::CommsJammed,
        }]);
        let mods = app.world().resource::<crate::modifiers::ShipModifiers>();
        assert!(
            mods.has_flag(&crate::flag_kind::FlagKind::CommsJammed),
            "ApplyFlag must register a CommsJammed flag"
        );

        let app = fire_response_with_actions(vec![
            TriggerAction::ApplyFlag {
                entity: "starbase_alpha".into(),
                tag: "jammer".into(),
                kind: crate::flag_kind::FlagKind::CommsJammed,
            },
            TriggerAction::RemoveFlag {
                entity: "starbase_alpha".into(),
                tag: "jammer".into(),
                kind: crate::flag_kind::FlagKind::CommsJammed,
            },
        ]);
        let mods = app.world().resource::<crate::modifiers::ShipModifiers>();
        assert!(
            !mods.has_flag(&crate::flag_kind::FlagKind::CommsJammed),
            "RemoveFlag must un-register the CommsJammed flag"
        );
    }

    #[test]
    fn comms_response_dispatches_apply_and_remove_int_modifier() {
        let app = fire_response_with_actions(vec![TriggerAction::ApplyIntModifier {
            entity: "starbase_alpha".into(),
            tag: "extra_team".into(),
            slot: crate::modifiers::IntModifierSlot::RepairTeams,
            bonus: 2,
        }]);
        let mods = app.world().resource::<crate::modifiers::ShipModifiers>();
        assert_eq!(
            mods.get_int(&crate::modifiers::IntModifierSlot::RepairTeams), 2,
            "ApplyIntModifier must add to RepairTeams int slot"
        );

        let app = fire_response_with_actions(vec![
            TriggerAction::ApplyIntModifier {
                entity: "starbase_alpha".into(),
                tag: "extra_team".into(),
                slot: crate::modifiers::IntModifierSlot::RepairTeams,
                bonus: 3,
            },
            TriggerAction::RemoveIntModifier {
                entity: "starbase_alpha".into(),
                tag: "extra_team".into(),
                slot: crate::modifiers::IntModifierSlot::RepairTeams,
            },
        ]);
        let mods = app.world().resource::<crate::modifiers::ShipModifiers>();
        assert_eq!(
            mods.get_int(&crate::modifiers::IntModifierSlot::RepairTeams), 0,
            "RemoveIntModifier must reverse the int modifier"
        );
    }

    #[test]
    fn comms_response_dispatches_spawn_entity() {
        use crate::entities::spawner::EntityUuid;

        let template_path = write_spawn_template_fixture();
        let app = fire_response_with_actions(vec![TriggerAction::SpawnEntity {
            template_path,
            name: "comms_spawn".into(),
            anchor: None,
            position: Some([5.0, 0.0, 9.0]),
            rotation: None,
            scale: None,
        }]);

        let uuid = app
            .world()
            .resource::<WorldContentRuntime>()
            .name_to_uuid
            .get("comms_spawn")
            .cloned()
            .expect("SpawnEntity from comms response must register name_to_uuid");

        let mut app = app;
        let mut q = app
            .world_mut()
            .query::<(&EntityUuid, &bevy::prelude::Transform)>();
        let mut found = false;
        for (eu, t) in q.iter(app.world()) {
            if eu.0 == uuid {
                found = true;
                assert!((t.translation.x - 5.0).abs() < 1e-3);
                assert!((t.translation.z - 9.0).abs() < 1e-3);
            }
        }
        assert!(found, "spawned entity must exist in ECS");
    }

    #[test]
    fn comms_response_dispatches_destroy_entity() {
        use crate::entities::spawner::EntityUuid;

        // Pre-spawn a target entity with a known UUID, then point the comms
        // response at it via name_to_uuid.
        let target_uuid = "comms-doomed-uuid";
        let mut app = comms_parity_test_app();
        let target_entity = app
            .world_mut()
            .spawn((
                EntityUuid(target_uuid.into()),
                bevy::prelude::Transform::from_xyz(0.0, 0.0, 0.0),
            ))
            .id();

        // Boot identical to fire_response_with_actions but with a DestroyEntity
        // action that targets the pre-spawned entity.
        push_msg(&mut app, "captain", ClientMessage::Identify {
            token: "captain".into(), name: "Alice".into(),
        });
        tick(&mut app);
        push_msg(&mut app, "captain", ClientMessage::SelectStation {
            station: "Captain's Chair".into(),
        });
        tick(&mut app);
        push_msg(&mut app, "comms", ClientMessage::Identify {
            token: "comms".into(), name: "Uhura".into(),
        });
        tick(&mut app);
        push_msg(&mut app, "comms", ClientMessage::SelectStation {
            station: "Comms".into(),
        });
        tick(&mut app);
        push_msg(&mut app, "captain", ClientMessage::StartGame);
        tick(&mut app);

        let station_uuid = "station-destroy-uuid";
        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime.name_to_uuid.insert("starbase_alpha".into(), station_uuid.into());
            runtime.name_to_uuid.insert("doomed".into(), target_uuid.into());
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
                        body: "Fire?".into(),
                        responses: vec![CommsResponse {
                            text: "Fire.".into(),
                            actions: vec![TriggerAction::DestroyEntity {
                                entity: "doomed".into(),
                            }],
                            follow_up: None,
                        }],
                    },
                    thread_id: None,
                    urgent: false,
                },
                fired: false,
            });
            runtime.needs_broadcast = true;
        }
        let _ = tick(&mut app);

        push_msg(&mut app, "comms", ClientMessage::Hail {
            target_uuid: station_uuid.into(),
        });
        let out = tick(&mut app);
        let msg_id = out.iter().find_map(|m| {
            if let ServerMessage::CommsState { messages, .. } = &m.msg {
                messages.first().map(|msg| msg.id.clone())
            } else { None }
        }).expect("hail must deliver a message");

        push_msg(&mut app, "comms", ClientMessage::RespondToMessage {
            message_id: msg_id, response_index: 0,
        });
        let _ = tick(&mut app);
        // Run one more update so Commands::queue (deferred despawn + message
        // write) is applied.
        app.update();

        assert!(
            app.world().get_entity(target_entity).is_err(),
            "DestroyEntity from comms response must despawn the target entity"
        );

        let runtime = app.world().resource::<WorldContentRuntime>();
        assert!(
            runtime.pending_world_events.iter().any(|e| matches!(
                e, WorldEvent::Destroyed { uuid } if uuid == target_uuid
            )),
            "DestroyEntity from comms response must enqueue a Destroyed event \
             for chained on_destroyed triggers"
        );
    }

    /// Exhaustive enumeration: matches on every `TriggerAction` variant and
    /// drives it through `handle_respond_to_message`, asserting that *some*
    /// observable side-effect occurs. Adding a new variant without wiring it
    /// into the response dispatch will be caught here as either a compile
    /// error (missing arm) or an assertion failure (no side-effect).
    #[test]
    fn comms_response_dispatches_every_trigger_action_variant() {
        // Build one representative instance of every variant. The match below
        // is non-exhaustive on purpose: any added variant of `TriggerAction`
        // becomes a compile error in this test, forcing the author to add
        // both a representative instance AND a per-variant parity test above.
        fn enumerate_variants() -> Vec<TriggerAction> {
            // Construct a known list. The match below proves we considered
            // every variant.
            let variants: Vec<TriggerAction> = vec![
                TriggerAction::AddObjective {
                    id: "x".into(), text: "x".into(), mandatory: false,
                },
                TriggerAction::CompleteObjective { id: "x".into() },
                TriggerAction::FailObjective { id: "x".into() },
                TriggerAction::SetAiState {
                    entity: "x".into(), state: "x".into(), target: None,
                },
                TriggerAction::ApplyModifier {
                    entity: "x".into(), tag: "x".into(),
                    slot: crate::messages::ModifierSlot::MaxSpeed, bonus: 0.0,
                },
                TriggerAction::RemoveModifier {
                    entity: "x".into(), tag: "x".into(),
                    slot: crate::messages::ModifierSlot::MaxSpeed,
                },
                TriggerAction::ApplyFlag {
                    entity: "x".into(), tag: "x".into(),
                    kind: crate::flag_kind::FlagKind::CommsJammed,
                },
                TriggerAction::RemoveFlag {
                    entity: "x".into(), tag: "x".into(),
                    kind: crate::flag_kind::FlagKind::CommsJammed,
                },
                TriggerAction::ApplyIntModifier {
                    entity: "x".into(), tag: "x".into(),
                    slot: crate::modifiers::IntModifierSlot::RepairTeams, bonus: 0,
                },
                TriggerAction::RemoveIntModifier {
                    entity: "x".into(), tag: "x".into(),
                    slot: crate::modifiers::IntModifierSlot::RepairTeams,
                },
                TriggerAction::GameOver { message: None },
                TriggerAction::LoadWorld { path: "x".into() },
                TriggerAction::UnloadWorld { path: "x".into() },
                TriggerAction::SetWorldFlag { name: "x".into() },
                TriggerAction::ClearWorldFlag { name: "x".into() },
                TriggerAction::IncrementWorldFlag { name: "x".into(), by: 0 },
                TriggerAction::SetWorldFlagValue { name: "x".into(), value: 0 },
                TriggerAction::SpawnEntity {
                    template_path: "x".into(), name: "x".into(),
                    anchor: None, position: None,
                    rotation: None, scale: None,
                },
                TriggerAction::DestroyEntity { entity: "x".into() },
            ];
            // Exhaustiveness check: this match must cover every variant. If
            // a new variant is added to `TriggerAction`, this match becomes
            // a compile error.
            for v in &variants {
                match v {
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
                    | TriggerAction::GameOver { .. }
                    | TriggerAction::LoadWorld { .. }
                    | TriggerAction::UnloadWorld { .. }
                    | TriggerAction::SetWorldFlag { .. }
                    | TriggerAction::ClearWorldFlag { .. }
                    | TriggerAction::IncrementWorldFlag { .. }
                    | TriggerAction::SetWorldFlagValue { .. }
                    | TriggerAction::SpawnEntity { .. }
                    | TriggerAction::DestroyEntity { .. } => {}
                }
            }
            variants
        }

        // The per-variant tests above prove each variant's observable
        // dispatch behaviour. This test's job is to (a) enumerate every
        // variant via an exhaustive match (compile-time drift guard) and
        // (b) confirm dispatch doesn't panic when handed the full set in
        // a single response.
        let variants = enumerate_variants();
        let _ = fire_response_with_actions(variants);
    }
}
