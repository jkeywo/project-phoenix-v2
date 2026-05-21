use bevy::prelude::*;
use crate::damage::ConsoleHull;
use crate::simulation::{Ship, ShipHullIntegrity};
use std::collections::{HashMap, HashSet};

use crate::comms_inbox::CommsInbox;
use crate::lobby::{InboundMessage, Sessions, Target, WorldResource};
use crate::simulation::SimOutbox;
use crate::messages::{
    ClientMessage, CommsContact, CommsMessage, Console, GamePhase, ServerMessage,
};
use crate::objectives::ObjectiveManager;
use crate::world::content::{
    ActiveDialogue, CommsTemplateState, TriggerAction,
    TriggerState, WorldEvent, comms_template_states_from_world, evaluate_comms_templates,
    evaluate_triggers, trigger_states_from_world,
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
    /// de-duplicate `LoadScenario` actions (no-op if path already active).
    pub loaded_scenario_paths: HashSet<String>,
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
/// When a `TriggerAction::LoadScenario { path }` fires, `handle_ai_events` pushes
/// the path here. The `apply_pending_scenario_loads` system drains it each frame,
/// parses the TOML, and merges the new triggers/comms into the runtime.
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
}

/// Map of `path → WorldRuntime` for sub-worlds loaded via `LoadWorld` / `extra_worlds`.
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
    Load(String),
    Unload(String),
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
                    broadcast_comms_state.in_set(crate::sim_sets::SimSet::Broadcast),
                    broadcast_objective_summary.in_set(crate::sim_sets::SimSet::Broadcast),
                ).chain(),
            )
            .add_systems(Update, handle_ai_events.in_set(crate::sim_sets::SimSet::Physics))
            .add_systems(Update, apply_pending_scenario_loads.in_set(crate::sim_sets::SimSet::Physics))
            .add_systems(Update, apply_world_layer_changes.in_set(crate::sim_sets::SimSet::Physics));
    }
}

/// Startup system: copy the unified `WorldConfig` from the WASM-side
/// thread-local cache into a Bevy `Resource` so downstream systems
/// (`spawn_world_entities`, `ai::server::tick_ai_controllers`) can read it
/// via `Res<WorldConfig>`.
///
/// On native (no WASM bridge) `get_world_config()` returns `None` and this
/// system is a no-op; `setup_fallback_world` handles that case via its
/// `run_if(not(resource_exists::<WorldConfig>))` gate.
fn insert_world_config_resource(mut commands: Commands) {
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
        let config = match crate::entity_loader::resolve_entity(entity_inst, config_cache) {
            Ok(c) => c,
            Err(e) => {
                bevy::log::error!(
                    "spawn_world_entities: failed to resolve asteroid field '{}': {}",
                    entity_inst.template_path, e
                );
                continue;
            }
        };
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
    // entity. A missing registration is a programmer error — log and skip
    // rather than allocate a fresh UUID (which would silently desync).
    for entity_inst in named {
        let name = entity_inst.name.as_ref().expect("partition guarantees Some");
        let uuid = match world_config.name_to_uuid.get(name) {
            Some(u) => u.clone(),
            None => {
                bevy::log::error!(
                    "spawn_world_entities: named entity '{}' has no UUID in WorldConfig.name_to_uuid — skipping",
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

#[allow(dead_code)]
fn instance_position(entity_inst: &crate::world::config::WorldEntity) -> Vec3 {
    if entity_inst.position.len() >= 3 {
        Vec3::new(
            entity_inst.position[0],
            entity_inst.position[1],
            entity_inst.position[2],
        )
    } else {
        Vec3::ZERO
    }
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
        science_console: None,
            sensors_console: None,
        shields_console: None,
        torpedoes: None,
        repair: None,
        star: None,
        planet: None,
        asteroid_field: None,
        shape: None,
        effects: None,
        station: None,
        faction: None,
        behaviour: None,
        radar_appearance: None,
        mesh: None,
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
            });
        }
    }
    runtime.contacts = contacts;
    runtime.needs_broadcast = true;

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
        pending.0.push(WorldLayerChange::Load(path.clone()));
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

        // Evaluate matching on_hailed comms templates.
        let world_events = vec![WorldEvent::Hailed {
            target_uuid: target_uuid.clone(),
        }];

        let name_to_uuid = runtime.name_to_uuid.clone();
        let fired = evaluate_comms_templates(
            &mut runtime.comms_template_states,
            &world_events,
            &name_to_uuid,
        );

        for f in fired {
            // Build a CommsMessage and inject it.
            let msg_id = uuid::Uuid::new_v4().to_string();
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
            };

            inbox.0.inject(msg);

            // Record the active dialogue.
            runtime.active_dialogues.insert(
                msg_id,
                ActiveDialogue {
                    message_id: String::new(), // not needed in the map key
                    current_node: f.node.clone(),
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
    _commands: Commands,
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

        let responses = &dialogue.current_node.responses;
        if *response_index >= responses.len() {
            continue;
        }

        let response = &responses[*response_index];

        // Fire response actions.
        for action in &response.actions {
            match action {
                TriggerAction::AddObjective { id, text, mandatory } => {
                    objectives.0.add(id, text, *mandatory);
                }
                TriggerAction::CompleteObjective { id } => {
                    objectives.0.complete(id);
                }
                TriggerAction::FailObjective { id } => {
                    objectives.0.fail(id);
                }
                TriggerAction::SetAiState { .. } => {
                    // SetAiState is handled by the AI-event trigger system, not
                    // the comms response path. No-op here.
                }
                TriggerAction::ApplyModifier { .. }
                | TriggerAction::RemoveModifier { .. }
                | TriggerAction::ApplyFlag { .. }
                | TriggerAction::RemoveFlag { .. }
                | TriggerAction::ApplyIntModifier { .. }
                | TriggerAction::RemoveIntModifier { .. }
                | TriggerAction::GameOver { .. }
                | TriggerAction::LoadScenario { .. }
                | TriggerAction::LoadWorld { .. }
                | TriggerAction::UnloadWorld { .. } => {
                    // Modifier/flag/game-over/load-scenario/load-world/unload-world
                    // actions are handled by the AI-event trigger or damage systems.
                    // No-op in the comms response path.
                }
            }
        }

        // Record the chosen response on the inbox message.
        inbox.0.record_response(message_id, *response_index);

        // Advance to follow-up node if present.
        if let Some(follow_up) = &response.follow_up {
            // Inject a new message for the follow-up node.
            let new_msg_id = uuid::Uuid::new_v4().to_string();
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
                sender_uuid,
                sender_name,
                subject: follow_up.body.chars().take(40).collect(),
                body: follow_up.body.clone(),
                responses: new_responses,
                selected_response: None,
                is_read: false,
                is_orphaned: false,
            };

            inbox.0.inject(new_msg);

            // Record the follow-up dialogue.
            runtime.active_dialogues.insert(
                new_msg_id,
                ActiveDialogue {
                    message_id: String::new(),
                    current_node: follow_up.clone(),
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

    let messages = inbox.0.messages();
    let objectives_snap = objectives.0.sorted_snapshots();
    let contacts = runtime.contacts.clone();

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
    _commands: Commands,
    mut attacked_reader: MessageReader<crate::ai_plugin::AiEntityAttacked>,
    mut destroyed_reader: MessageReader<crate::ai_plugin::AiEntityDestroyed>,
    mut ai_query: Query<(&EntityUuid, &mut AiControllerComponent, &BehaviourSection)>,
    mut modifiers: Option<ResMut<crate::modifiers::ShipModifiers>>,
    mut next_state: Option<ResMut<NextState<GamePhase>>>,
    mut game_over_reason: Option<ResMut<crate::simulation::GameOverReason>>,
    mut pending: Option<ResMut<PendingScenarioLoad>>,
    mut pending_layers: Option<ResMut<PendingWorldLayerChanges>>,
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
    if world_events.is_empty() {
        return;
    }

    let name_to_uuid = runtime.name_to_uuid.clone();

    // Auto-fire comms templates that match the world events (e.g. on_attacked distress calls).
    // These are injected without any player hailing — they are broadcast messages.
    let fired_comms = evaluate_comms_templates(
        &mut runtime.comms_template_states,
        &world_events,
        &name_to_uuid,
    );
    for fc in fired_comms {
        let msg_id = uuid::Uuid::new_v4().to_string();
        let sender_name = fc.from.clone();
        let sender_uuid = name_to_uuid
            .get(&fc.from)
            .cloned()
            .unwrap_or_else(|| fc.from.clone());
        let responses: Vec<String> = fc.node.responses.iter().map(|r| r.text.clone()).collect();
        let msg = crate::messages::CommsMessage {
            id: msg_id.clone(),
            sender_uuid,
            sender_name,
            subject: fc.node.body.chars().take(40).collect(),
            body: fc.node.body.clone(),
            responses,
            selected_response: None,
            is_read: false,
            is_orphaned: false,
        };
        inbox.0.inject(msg);
        runtime.active_dialogues.insert(
            msg_id,
            ActiveDialogue {
                message_id: String::new(),
                current_node: fc.node.clone(),
            },
        );
    }

    let fired = evaluate_triggers(&mut runtime.trigger_states, &world_events, &name_to_uuid);

    for ft in fired {
        for action in &ft.actions {
            match action {
                TriggerAction::AddObjective { id, text, mandatory } => {
                    objectives.0.add(id.clone(), text.clone(), *mandatory);
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
                        // Build the new AiState from the behaviour config.
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
                TriggerAction::LoadScenario { path } => {
                    if let Some(ref mut p) = pending {
                        p.0.push(path.clone());
                    }
                }
                TriggerAction::LoadWorld { path } => {
                    if let Some(ref mut lc) = pending_layers {
                        lc.0.push(WorldLayerChange::Load(path.clone()));
                    }
                }
                TriggerAction::UnloadWorld { path } => {
                    if let Some(ref mut lc) = pending_layers {
                        lc.0.push(WorldLayerChange::Unload(path.clone()));
                    }
                }
            }
        }
    }
}

use crate::ai_plugin::AiControllerComponent;
use crate::entity_spawner::{BehaviourSection, EntityUuid};

// ── Pending scenario load system ─────────────────────────────────────────────

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

// ── World layer system (LoadWorld / UnloadWorld) ──────────────────────────────

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
                            "build_layer_config_cache: failed to parse '{}' — entity will be skipped",
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
/// of the underlying `Trigger`/`CommsTemplate` clone identity — we use indices
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
            WorldLayerChange::Load(path) => {
                if layer_map.0.contains_key(&path) {
                    // Already loaded — de-duplicate, no-op.
                    continue;
                }
                let toml_str_opt = load_scenario_toml(&path);
                match toml_str_opt {
                    None => {
                        // WASM: re-queue until the fetch completes.
                        pending.0.push(WorldLayerChange::Load(path));
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
                                let trigger_states =
                                    trigger_states_from_world(&scenario_config);
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
                                        });
                                    }
                                }

                                runtime.needs_broadcast = true;

                                layer_map.0.insert(
                                    path,
                                    WorldRuntime {
                                        trigger_states,
                                        comms_template_states,
                                        spawned_entities,
                                    },
                                );
                            }
                        }
                    }
                }
            }
            WorldLayerChange::Unload(path) => {
                let Some(layer) = layer_map.0.remove(&path) else {
                    continue; // Not loaded — no-op.
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
    // `WorldConfig` is loaded the fallback must be skipped — the
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
        // Run only the Startup schedule — WorldPlugin's Update systems
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
             already loaded — the [[entity]] pipeline owns ship spawning"
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
                    broadcast_comms_state,
                    broadcast_objective_summary,
                ).chain(),
            )
            .add_systems(PostUpdate, collect);
        app
    }

    fn push_msg(app: &mut App, token: &str, msg: ClientMessage) {
        use bevy::ecs::system::RunSystemOnce;
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
        let mut orphaned = CommsMessage {
            id: "orphaned-001".into(),
            sender_uuid: station_uuid.into(),
            sender_name: "Starbase Alpha".into(),
            subject: "Old message".into(),
            body: "Old message body".into(),
            responses: vec![],
            selected_response: None,
            is_read: false,
            is_orphaned: true,
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
            },
            fired: false,
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
                },
                fired: false,
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
                },
                fired: false,
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
        // resolution needed — the helper that mutates `name_to_uuid` runs
        // independently of the asteroid-field spawning path).
        let mut world_cfg = UnifiedWorldConfig::default();
        world_cfg.entities.push(WorldEntity {
            template_path: "assets/entities/station_outpost.toml".into(),
            name: Some("starbase_alpha".into()),
            position: vec![500.0, 0.0, 0.0],
            ..Default::default()
        });
        world_cfg.entities.push(WorldEntity {
            template_path: "assets/entities/star_sun.toml".into(),
            name: None,
            position: vec![0.0, 0.0, 0.0],
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
        // folds `WorldConfig.name_to_uuid` in) must NOT overwrite those —
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
        // spawned as real Bevy entities — otherwise triggers / comms resolve
        // to a UUID that has no Transform behind it. The spawned entity's
        // `EntityUuid` component must equal the UUID already registered in
        // `WorldConfig.name_to_uuid` for that name (single source of truth —
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
            position: vec![500.0, 0.0, 0.0],
            ..Default::default()
        });
        // An anonymous entry must NOT be spawned by the unified pipeline
        // (the complementary `setup_world` in `server_app.rs` owns it).
        world_cfg.entities.push(WorldEntity {
            template_path: "fixture/star.toml".into(),
            position: vec![0.0, 0.0, 0.0],
            ..Default::default()
        });

        // Pre-populate name_to_uuid as `spawn_world_entities`'s
        // assign-uuid pass would have.
        world_cfg
            .name_to_uuid
            .insert("starbase_alpha".into(), "stable-station-uuid".into());

        // Build a fixture ConfigCache with the templates referenced above.
        // Empty EntityConfig is sufficient — no asteroid_field section, so
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

        // Its EntityUuid must equal the registered UUID — not a fresh one.
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
            anchor: Some("patrol_alpha".into()),
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
        // [behaviour] block must end up with a BehaviourSection — the
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
            anchor: Some("patrol_alpha".into()),
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

    // ── extra_worlds + LoadWorld / UnloadWorld (issue #352) ──────────────────

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
            matches!(&pending.0[0], WorldLayerChange::Load(p) if p == "assets/worlds/patrol.toml")
        );
        assert!(
            matches!(&pending.0[1], WorldLayerChange::Load(p) if p == "assets/worlds/side.toml")
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
                },
                fired: false,
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
            matches!(&pending.0[0], WorldLayerChange::Load(p) if p == "assets/worlds/patrol.toml")
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
                },
                fired: false,
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
            .push(WorldLayerChange::Load("assets/worlds/patrol.toml".into()));

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
            .push(WorldLayerChange::Load("assets/worlds/patrol.toml".into()));
        app.update();

        let trigger_count_after_first = app
            .world()
            .resource::<WorldContentRuntime>()
            .trigger_states
            .len();

        // Load again — must not double-add.
        app.world_mut()
            .resource_mut::<PendingWorldLayerChanges>()
            .0
            .push(WorldLayerChange::Load("assets/worlds/patrol.toml".into()));
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
            .push(WorldLayerChange::Load("assets/worlds/patrol.toml".into()));
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

    // ── Entity spawn / despawn via LoadWorld / UnloadWorld (issue #352) ───────

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
            .push(WorldLayerChange::Load(world_path.clone()));

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
            .push(WorldLayerChange::Load(world_path.clone()));
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
}
