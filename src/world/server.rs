use bevy::prelude::*;
use crate::damage::ConsoleHull;
use crate::simulation::{Ship, ShipHullIntegrity};
use std::collections::HashMap;

use crate::comms_inbox::CommsInbox;
use crate::entity_spawner::{spawn_entity, ScenarioOwner};
use crate::lobby::{InboundMessage, Sessions, Target, WorldResource};
use crate::simulation::SimOutbox;
use crate::messages::{
    ClientMessage, CommsContact, CommsMessage, Console, GamePhase, ServerMessage,
};
use crate::objectives::ObjectiveManager;
use crate::world::content::{
    ActiveDialogue, CommsDialogueNode, CommsTemplate, CommsTemplateState, TriggerAction,
    TriggerState, WorldEvent, evaluate_comms_templates, evaluate_triggers,
    trigger_states_from_config,
};

// ── Resources ──────────────────────────────────────────────────────────────

/// Server-side runtime state for the currently active world content.
///
/// Populated at `Startup` from `WorldContentResource` (which is itself populated
/// by `ConfigCachePlugin` when `wasm_load_world_content` is called before
/// `wasm_init`). When no world content is loaded all vecs/maps are empty and comms
/// systems are no-ops.
#[derive(Resource, Default)]
pub struct WorldContentRuntime {
    /// Mutable per-trigger runtime state (fired flag).
    pub trigger_states: Vec<TriggerState>,
    /// Mutable per-template runtime state (fired flag).
    pub comms_template_states: Vec<CommsTemplateState>,
    /// Active in-flight dialogues keyed by CommsMessage id.
    pub active_dialogues: HashMap<String, ActiveDialogue>,
    /// Named-spawn → UUID mapping (populated from ScenarioConfig).
    pub name_to_uuid: HashMap<String, String>,
    /// Hailable contacts derived from scenario spawns.
    pub contacts: Vec<CommsContact>,
    /// Set to `true` whenever contacts or other scenario-level data changes so
    /// `broadcast_comms_state` knows to push a fresh snapshot even if the
    /// inbox itself hasn't changed.
    pub needs_broadcast: bool,
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

/// Bevy resource wrapping the additive scenario manager.
///
/// Scenarios are keyed by their TOML path string. Loading is additive; the same
/// path loaded twice is a no-op. Unloading removes the scenario and cleans up
/// comms and objectives.
#[derive(Resource, Default)]
pub struct ScenarioManagerRes(pub crate::world::content::ScenarioManager);


pub struct WorldPlugin;

impl Plugin for WorldPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup_world_hardcoded)
            .init_resource::<WorldContentRuntime>()
            .init_resource::<CommsInboxRes>()
            .init_resource::<ObjectiveManagerRes>()
            .init_resource::<ScenarioManagerRes>()
            .add_systems(
                Startup,
                (
                    insert_world_config_resource,
                    spawn_world_entities,
                    spawn_scenario_entities,
                    init_scenario_runtime,
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
            .add_systems(Update, handle_ai_events.in_set(crate::sim_sets::SimSet::Physics));
    }
}

/// Startup system: copy the unified `WorldConfig` from the WASM-side
/// thread-local cache into a Bevy `Resource` so downstream systems
/// (`spawn_world_entities`, `ai::server::tick_ai_controllers`) can read it
/// via `Res<WorldConfig>` (PRD #337/#338 slice 1).
///
/// On native (no WASM bridge) `get_world_config()` returns `None` and this
/// system is a no-op; the legacy `MapConfig`-based fallbacks remain in place.
fn insert_world_config_resource(mut commands: Commands) {
    if let Some(world_config) = crate::config_cache::get_world_config() {
        commands.insert_resource(world_config);
    }
}

/// Startup system: spawn `[[entity]]` instances owned by the unified
/// `WorldConfig` pipeline.
///
/// PRD #337/#338 slice 1 + PRD #339 slice 2: the unified pipeline owns
/// both asteroid-field templates AND any `[[entity]]` carrying a `name`
/// field. The legacy `setup_world_from_config` in `server_app.rs` keeps
/// anonymous non-asteroid entries; the shared `is_owned_by_unified_pipeline`
/// helper guarantees no entry is spawned twice.
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
        let pos = instance_position(entity_inst);
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
        let pos = instance_position(entity_inst);
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

fn instance_position(entity_inst: &crate::map_config::EntityInstance) -> Vec3 {
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


/// Bootstrap precedence: scenario → map-config → hardcoded.
///
/// Returns true when the hardcoded fallback should run (no preloaded configs).
pub fn choose_bootstrap() -> bool {
    if crate::config_cache::get_world_content_config().is_some() {
        return false; // scenario-driven spawn handled by spawn_scenario_entities
    }
    if let Some(map) = crate::config_cache::get_map_config() {
        if let Some(path) = map.default_scenario {
            bevy::log::warn!(
                "WorldPlugin: default scenario '{}' not preloaded — using hardcoded bootstrap",
                path
            );
        }
        return false; // map-config-driven setup handled by SimulationPlugin
    }
    true // pure fallback
}

/// Fallback world setup with hardcoded values for development/testing.
/// Runs only when no map config and no scenario was preloaded.
fn setup_world_hardcoded(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    _world: ResMut<WorldResource>,
) {
    if !choose_bootstrap() {
        return;
    }

    // ── Starfield skybox ───────────────────────────────────────────────────
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
    // no-MapConfig fallback path; the [[entity]]/spawn_game_start path is
    // preferred and runs whenever MapConfig is loaded.
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
        star: None,
        planet: None,
        asteroid_field: None,
        shape: None,
        effects: None,
        station: None,
        faction: None,
        behaviour: None,
        radar_appearance: None,
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


// ── Startup systems ─────────────────────────────────────────────────────────

/// Startup system: resolves all scenario spawn positions and spawns each entity
/// through the shared `spawn_entity` helper.
fn spawn_scenario_entities(mut commands: Commands) {
    let scenario_config = match crate::config_cache::get_world_content_config() {
        Some(s) => s,
        None => return, // No scenario loaded — nothing to do.
    };

    // Derive the scenario path from the map config's `default_scenario` field.
    // Falls back to "default" when the path is unavailable (e.g. in native tests).
    let scenario_path = crate::config_cache::wasm_get_world_content_path()
        .unwrap_or_else(|| "default".to_string());

    let map_config = crate::config_cache::get_map_config();
    let anchors = map_config
        .as_ref()
        .map(|mc| mc.anchors.clone())
        .unwrap_or_default();

    let config_cache = crate::config_cache::get_config_cache();

    let resolved = match crate::world::content::resolve_positions(&scenario_config, &anchors) {
        Ok(r) => r,
        Err(e) => {
            bevy::log::error!("WorldPlugin: failed to resolve spawn positions: {e}");
            return;
        }
    };

    for spawn in &resolved {
        let config = config_cache.get(&spawn.entity_path);

        let Some(config) = config else {
                bevy::log::warn!(
                "WorldPlugin: no config found for entity path '{}' (spawn '{}') — skipping",
                spawn.entity_path,
                spawn.name
            );
            continue;
        };

        let position = Vec3::new(
            spawn.position[0],
            spawn.position[1],
            spawn.position[2],
        );

        let entity = spawn_entity(
            &mut commands,
            config,
            position,
            spawn.uuid.clone(),
            Some(spawn.name.clone()),
        );
        // Tag each spawned entity with the real scenario path so that unloading
        // the scenario can target its entities for cleanup.
        commands.entity(entity).insert(ScenarioOwner(scenario_path.clone()));

        bevy::log::info!(
            "WorldPlugin: spawned '{}' at {:?} uuid={}",
            spawn.name,
            spawn.position,
            spawn.uuid
        );
    }
}

/// Fold a parsed `ScenarioConfig` into a live `WorldContentRuntime`.
///
/// PRD #337/#339 slice 2: the unified `[[entity]]` pipeline runs first and
/// may have already registered names in `runtime.name_to_uuid`. This helper
/// merges the legacy scenario `name_to_uuid` in WITHOUT overwriting any
/// existing entries — unified-pipeline registrations win, legacy fills gaps.
/// Same merge policy applies to derived `contacts`.
///
/// Extracted as a pure helper so the merge semantics are testable on native
/// (where `get_world_content_config()` always returns `None`).
pub fn merge_scenario_into_runtime(
    runtime: &mut WorldContentRuntime,
    scenario_config: &crate::world::content::ScenarioConfig,
    scenario_path: &str,
) {
    for (name, uuid) in &scenario_config.name_to_uuid {
        runtime
            .name_to_uuid
            .entry(name.clone())
            .or_insert_with(|| uuid.clone());
    }
    runtime.comms_template_states =
        crate::world::content::comms_template_states_from_config(scenario_config, scenario_path);
    runtime.trigger_states = trigger_states_from_config(scenario_config, scenario_path);

    // Build contacts list using the merged runtime map so unified-pipeline
    // UUIDs are picked up too.
    let mut contacts: Vec<CommsContact> = Vec::new();
    for tmpl in &scenario_config.comms {
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
}

/// Startup system: initialises `WorldContentRuntime`, `CommsInboxRes`, and
/// `ObjectiveManagerRes` from the loaded `ScenarioConfig` (if any).
/// Also populates `WorldResource` with scenario metadata (title, description).
fn init_scenario_runtime(
    mut runtime: ResMut<WorldContentRuntime>,
    mut inbox: ResMut<CommsInboxRes>,
    mut world: ResMut<WorldResource>,
) {
    let scenario_config = match crate::config_cache::get_world_content_config() {
        Some(s) => s,
        None => return,
    };

    let scenario_path = crate::config_cache::wasm_get_world_content_path()
        .unwrap_or_else(|| "default".to_string());

    world.0.scenario_title = scenario_config.title.clone();
    world.0.scenario_description = scenario_config.description.clone();

    merge_scenario_into_runtime(&mut runtime, &scenario_config, &scenario_path);

    // Mark inbox dirty so the first InProgress broadcast fires even though
    // no messages have arrived yet.
    inbox.0.mark_dirty();
}

/// Re-mark the comms runtime dirty when the game enters InProgress.
///
/// `init_scenario_runtime` marks the runtime dirty during Startup so the first
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

// ── Update systems ──────────────────────────────────────────────────────────

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

            inbox.0.inject(msg, &f.scenario_path);

            // Record the active dialogue.
            runtime.active_dialogues.insert(
                msg_id,
                ActiveDialogue {
                    message_id: String::new(), // not needed in the map key
                    current_node: f.node.clone(),
                    scenario_path: f.scenario_path.clone(),
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
    mut scenario_mgr: ResMut<ScenarioManagerRes>,
    mut commands: Commands,
    scenario_owner_query: Query<(Entity, &ScenarioOwner)>,
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
                    objectives.0.add(id, text, *mandatory, &dialogue.scenario_path);
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
                | TriggerAction::GameOver { .. } => {
                    // Modifier/flag/game-over actions are handled by the AI-event trigger
                    // or damage systems. No-op in the comms response path.
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

            inbox.0.inject(new_msg, &dialogue.scenario_path);

            // Record the follow-up dialogue.
            runtime.active_dialogues.insert(
                new_msg_id,
                ActiveDialogue {
                    message_id: String::new(),
                    current_node: follow_up.clone(),
                    scenario_path: dialogue.scenario_path.clone(),
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

// ── AI-event trigger system ─────────────────────────────────────────────────

/// Read `AiEntityAttacked` and `AiEntityDestroyed` messages, translate them
/// into `WorldEvent`s, evaluate the scenario trigger table, and execute the
/// resulting actions (including `SetAiState`, `ApplyModifier`, `RemoveModifier`,
/// `ApplyFlag`, and `RemoveFlag`).
fn handle_ai_events(
    mut runtime: ResMut<WorldContentRuntime>,
    mut objectives: ResMut<ObjectiveManagerRes>,
    mut inbox: ResMut<CommsInboxRes>,
    mut scenario_mgr: ResMut<ScenarioManagerRes>,
    mut commands: Commands,
    scenario_owner_query: Query<(Entity, &ScenarioOwner)>,
    mut attacked_reader: MessageReader<crate::ai_plugin::AiEntityAttacked>,
    mut destroyed_reader: MessageReader<crate::ai_plugin::AiEntityDestroyed>,
    mut ai_query: Query<(&EntityUuid, &mut AiControllerComponent, &BehaviourSection)>,
    mut modifiers: Option<ResMut<crate::modifiers::ShipModifiers>>,
    mut next_state: Option<ResMut<NextState<GamePhase>>>,
    mut game_over_reason: Option<ResMut<crate::simulation::GameOverReason>>,
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
        inbox.0.inject(msg, &fc.scenario_path);
        runtime.active_dialogues.insert(
            msg_id,
            ActiveDialogue {
                message_id: String::new(),
                current_node: fc.node.clone(),
                scenario_path: fc.scenario_path.clone(),
            },
        );
    }

    let fired = evaluate_triggers(&mut runtime.trigger_states, &world_events, &name_to_uuid);

    for ft in fired {
        for action in &ft.actions {
            match action {
                TriggerAction::AddObjective { id, text, mandatory } => {
                    objectives.0.add(id.clone(), text.clone(), *mandatory, ft.scenario_path.clone());
                }
                TriggerAction::CompleteObjective { id } => {
                    objectives.0.complete(id);
                }
                TriggerAction::FailObjective { id } => {
                    objectives.0.fail(id);
                }
                TriggerAction::SetAiState { entity, state, target } => {
                    // Resolve spawn name → UUID
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
                    if name_to_uuid.get(entity).is_none() {
                        bevy::log::warn!(
                            "handle_ai_events: ApplyModifier: unknown entity name '{entity}'"
                        );
                        continue;
                    }
                    if let Some(ref mut mods) = modifiers {
                        mods.add_or_update(crate::modifiers::Modifier {
                            source: crate::messages::ModifierSource::World {
                                id: ft.scenario_path.clone(),
                                tag: tag.clone(),
                            },
                            slot: slot.clone(),
                            bonus: *bonus,
                        });
                    }
                }
                TriggerAction::RemoveModifier { entity, tag, slot } => {
                    if name_to_uuid.get(entity).is_none() {
                        bevy::log::warn!(
                            "handle_ai_events: RemoveModifier: unknown entity name '{entity}'"
                        );
                        continue;
                    }
                    if let Some(ref mut mods) = modifiers {
                        mods.remove(
                            &crate::messages::ModifierSource::World {
                                id: ft.scenario_path.clone(),
                                tag: tag.clone(),
                            },
                            slot,
                        );
                    }
                }
                TriggerAction::ApplyFlag { entity, tag, kind } => {
                    if name_to_uuid.get(entity).is_none() {
                        bevy::log::warn!(
                            "handle_ai_events: ApplyFlag: unknown entity name '{entity}'"
                        );
                        continue;
                    }
                    if let Some(ref mut mods) = modifiers {
                        mods.add_flag(
                            crate::messages::ModifierSource::World {
                                id: ft.scenario_path.clone(),
                                tag: tag.clone(),
                            },
                            kind.clone(),
                        );
                    }
                }
                TriggerAction::RemoveFlag { entity, tag, kind } => {
                    if name_to_uuid.get(entity).is_none() {
                        bevy::log::warn!(
                            "handle_ai_events: RemoveFlag: unknown entity name '{entity}'"
                        );
                        continue;
                    }
                    if let Some(ref mut mods) = modifiers {
                        mods.remove_flag(
                            crate::messages::ModifierSource::World {
                                id: ft.scenario_path.clone(),
                                tag: tag.clone(),
                            },
                            kind.clone(),
                        );
                    }
                }
                TriggerAction::ApplyIntModifier { entity, tag, slot, bonus } => {
                    if name_to_uuid.get(entity).is_none() {
                        bevy::log::warn!(
                            "handle_ai_events: ApplyIntModifier: unknown entity name '{entity}'"
                        );
                        continue;
                    }
                    if let Some(ref mut mods) = modifiers {
                        mods.add_or_update_int(crate::modifiers::IntModifier {
                            source: crate::messages::ModifierSource::World {
                                id: ft.scenario_path.clone(),
                                tag: tag.clone(),
                            },
                            slot: slot.clone(),
                            bonus: *bonus,
                        });
                    }
                }
                TriggerAction::RemoveIntModifier { entity, tag, slot } => {
                    if name_to_uuid.get(entity).is_none() {
                        bevy::log::warn!(
                            "handle_ai_events: RemoveIntModifier: unknown entity name '{entity}'"
                        );
                        continue;
                    }
                    if let Some(ref mut mods) = modifiers {
                        mods.remove_int(
                            &crate::messages::ModifierSource::World {
                                id: ft.scenario_path.clone(),
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
            }
        }
    }
}

use crate::ai_plugin::{AiControllerComponent, AiEntityAttacked, AiEntityDestroyed};
use crate::entity_spawner::{BehaviourSection, EntityUuid};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lobby::{LobbyPlugin, OutboundMessage};
    use crate::messages::*;
    use crate::world::content::{CommsResponse, CommsTemplateState, TriggerCondition, parse_scenario};

    // ── Bootstrap selection tests ─────────────────────────────────────────────

    #[test]
    fn choose_bootstrap_selects_hardcoded_when_nothing_is_preloaded() {
        // In native (non-wasm) builds, get_map_config() and get_world_content_config()
        // always return None, so choose_bootstrap() must return true (use fallback).
        assert!(choose_bootstrap(), "hardcoded fallback should be selected when no configs preloaded");
    }

    #[test]
    fn parse_scenario_with_spawn_entry_produces_expected_entity() {
        let toml = r#"
[[spawn]]
name        = "Test Station"
entity_path = "assets/entities/station_outpost.toml"
position    = [100.0, 0.0, 200.0]
"#;
        let config = parse_scenario(toml).expect("fixture must parse");
        assert_eq!(config.spawns.len(), 1);
        assert_eq!(config.spawns[0].name, "Test Station");
        assert_eq!(config.spawns[0].entity_path, "assets/entities/station_outpost.toml");
    }

    // ── Test app ─────────────────────────────────────────────────────────────

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
            .init_resource::<ScenarioManagerRes>()
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
            scenario_path: "test".to_string(),
        });
        runtime.needs_broadcast = true;
    }

    // ── Cycle 1: hail delivers CommsState to comms holder ────────────────────

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

    // ── Cycle 2: hail from non-Comms player is ignored ───────────────────────

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

    // ── Cycle 3: respond fires actions and updates CommsState ────────────────

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

    // ── Cycle 4: clear comms removes read/orphaned messages ──────────────────

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
            .inject(orphaned, "default");
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

    // ── Cycle 5: initial CommsState with contacts sent on game start ─────────

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

    // ── AI-event trigger tests ───────────────────────────────────────────────

    /// Build a minimal test app that includes just what handle_ai_events needs.
    fn ai_trigger_test_app() -> App {
        let mut app = App::new();
        app.add_plugins(LobbyPlugin)
            .add_plugins(bevy::time::TimePlugin)
            .add_plugins(crate::ai_plugin::AiPlugin)
            .init_resource::<WorldContentRuntime>()
            .init_resource::<CommsInboxRes>()
            .init_resource::<ObjectiveManagerRes>()
            .init_resource::<ScenarioManagerRes>()
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
            scenario_path: "test".to_string(),
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
                scenario_path: "test".to_string(),
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

        // Set up trigger: on attacked → SetAiState to "chase"
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
                scenario_path: "test".to_string(),
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

    // ── ScenarioOwner component tests ─────────────────────────────────────────

    // Entities manually tagged with ScenarioOwner can be queried by scenario path
    #[test]
    fn scenario_owner_component_queryable_by_path() {
        let mut app = App::new();
        app.add_plugins(bevy::time::TimePlugin);

        // Spawn two entities: one scenario-owned, one not
        let owned = app.world_mut().spawn(ScenarioOwner("scenarios/alpha.toml".to_string())).id();
        let _free = app.world_mut().spawn_empty().id();

        // Query all entities with ScenarioOwner matching a specific path
        let mut found = Vec::new();
        for (entity, owner) in app.world_mut()
            .query::<(Entity, &ScenarioOwner)>()
            .iter(app.world())
        {
            if owner.0 == "scenarios/alpha.toml" {
                found.push(entity);
            }
        }
        assert_eq!(found.len(), 1);
        assert_eq!(found[0], owned);
    }

    // After unloading a scenario, ScenarioOwner is removed and entity persists
    #[test]
    fn remove_scenario_owner_entity_persists() {
        let mut app = App::new();
        app.add_plugins(bevy::time::TimePlugin);

        let entity = app.world_mut().spawn(ScenarioOwner("scenarios/alpha.toml".to_string())).id();
        app.update();

        // Remove ScenarioOwner (simulating unload)
        app.world_mut().entity_mut(entity).remove::<ScenarioOwner>();
        app.update();

        // Entity still exists, no longer has ScenarioOwner
        assert!(app.world().get_entity(entity).is_ok(), "entity should persist after owner removal");
        assert!(app.world().get::<ScenarioOwner>(entity).is_none());
    }

    // ── on_attacked comms template auto-injection tests ───────────────────────

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
                scenario_path: "default".to_string(),
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
                scenario_path: "default".to_string(),
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

    // ── Unified [[entity]] name → uuid pipeline (PRD #337/#339 slice 2) ───────

    #[test]
    fn spawn_world_entities_populates_name_to_uuid_for_named_entity() {
        use crate::map_config::EntityInstance;
        use crate::world::config::WorldConfig as UnifiedWorldConfig;

        // Build a unified WorldConfig with one named entry (no template
        // resolution needed — the helper that mutates `name_to_uuid` runs
        // independently of the asteroid-field spawning path).
        let mut world_cfg = UnifiedWorldConfig::default();
        world_cfg.entities.push(EntityInstance {
            template_path: "assets/entities/station_outpost.toml".into(),
            name: Some("starbase_alpha".into()),
            position: vec![500.0, 0.0, 0.0],
            ..Default::default()
        });
        world_cfg.entities.push(EntityInstance {
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
        use crate::map_config::EntityInstance;
        use crate::world::config::WorldConfig as UnifiedWorldConfig;

        let mut world_cfg = UnifiedWorldConfig::default();
        world_cfg.entities.push(EntityInstance {
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
    fn merge_scenario_into_runtime_preserves_existing_name_to_uuid() {
        // PRD #337/#339 slice 2: when the unified pipeline has already
        // registered a name (e.g. "starbase_alpha" via [[entity]] name=..),
        // folding the legacy ScenarioConfig in must NOT overwrite it. The
        // unified pipeline is the source of truth; legacy entries only fill
        // gaps. (Conflicting names mean the world TOML lists the same name
        // in both `[[entity]]` and `[[spawn]]`, which the migration plan
        // forbids — but unified wins so the spawned entity is reachable.)
        use crate::world::content::parse_scenario;

        let mut runtime = WorldContentRuntime::default();
        runtime.name_to_uuid.insert("starbase_alpha".into(), "unified-uuid".into());

        // Build a minimal ScenarioConfig by parsing an empty body and then
        // pushing entries into `name_to_uuid` directly.
        let mut scenario = parse_scenario("").expect("empty scenario must parse");
        scenario.name_to_uuid.insert("starbase_alpha".into(), "legacy-uuid".into());
        scenario.name_to_uuid.insert("legacy_only".into(), "legacy-only-uuid".into());

        merge_scenario_into_runtime(&mut runtime, &scenario, "fixture");

        assert_eq!(
            runtime.name_to_uuid.get("starbase_alpha").map(String::as_str),
            Some("unified-uuid"),
            "unified-pipeline registration must win over legacy ScenarioConfig"
        );
        assert_eq!(
            runtime.name_to_uuid.get("legacy_only").map(String::as_str),
            Some("legacy-only-uuid"),
            "names that exist only in the legacy ScenarioConfig still flow through"
        );
    }

    #[test]
    fn init_scenario_runtime_merges_rather_than_overwrites_existing_names() {
        // PRD #337/#339 slice 2: `spawn_world_entities` runs before
        // `init_scenario_runtime` and writes names from the unified
        // [[entity]] pipeline into `WorldContentRuntime.name_to_uuid`.
        // `init_scenario_runtime` (which folds the legacy ScenarioConfig
        // `name_to_uuid` in) must NOT overwrite those — otherwise trigger
        // and comms lookups for unified-pipeline names would silently
        // disappear. Cover that with a direct mutation of the runtime
        // map followed by an explicit `init_scenario_runtime` call.
        //
        // On native there's no preloaded scenario config so the system
        // early-returns; that early-return is itself the safety net we
        // want — verify any pre-existing entry survives.
        let mut app = App::new();
        app.init_resource::<WorldContentRuntime>();
        app.init_resource::<CommsInboxRes>();
        app.insert_resource(WorldResource(crate::messages::WorldData::default()));
        app.world_mut()
            .resource_mut::<WorldContentRuntime>()
            .name_to_uuid
            .insert("starbase_alpha".into(), "unified-uuid".into());

        app.add_systems(Update, init_scenario_runtime);
        app.update();

        let runtime = app.world().resource::<WorldContentRuntime>();
        assert_eq!(
            runtime.name_to_uuid.get("starbase_alpha").map(String::as_str),
            Some("unified-uuid"),
            "init_scenario_runtime must preserve unified-pipeline registrations"
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
        use crate::map_config::EntityInstance;
        use crate::world::config::WorldConfig as UnifiedWorldConfig;
        use std::collections::HashMap;

        let mut world_cfg = UnifiedWorldConfig::default();
        world_cfg.entities.push(EntityInstance {
            template_path: "fixture/station.toml".into(),
            name: Some("starbase_alpha".into()),
            position: vec![500.0, 0.0, 0.0],
            ..Default::default()
        });
        // An anonymous entry must NOT be spawned by the unified pipeline
        // (legacy `setup_world_from_config` owns it).
        world_cfg.entities.push(EntityInstance {
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

    #[test]
    fn spawn_immediate_entities_spawns_default_world_starbase_and_earth() {
        // PRD #339 slice 2 (rejection fix): with the shipped default.toml
        // (Starbase Alpha + Earth as named [[entity]]), both must end up
        // as real entities. Earth was anonymous in slice 1 and gained a
        // `name` field in slice 2 — regression-guard it.
        use crate::entity_config::EntityConfig;
        use crate::entity_spawner::EntityUuid;
        use crate::world::config::{parse_world, WorldConfig as UnifiedWorldConfig};
        use std::collections::HashMap;

        let toml = include_str!("../../assets/worlds/default.toml");
        let mut world_cfg = parse_world(toml).expect("default.toml must parse");
        // Mirror what the assign-uuid pass does.
        let assigned = crate::world::config::assign_named_entity_uuids(
            &world_cfg.entities,
            crate::entity_loader::assign_uuid,
        );
        for (name, uuid) in &assigned {
            world_cfg.name_to_uuid.insert(name.clone(), uuid.clone());
        }

        // Stub every template referenced in default.toml — empty
        // EntityConfig is enough because we only check Transform + EntityUuid.
        let mut cache: HashMap<String, EntityConfig> = HashMap::new();
        for ent in &world_cfg.entities {
            cache
                .entry(ent.template_path.clone())
                .or_insert_with(|| EntityConfig::from_toml("").unwrap());
        }

        let mut app = App::new();
        app.add_plugins(bevy::time::TimePlugin);
        app.insert_resource(world_cfg.clone());

        let spawned: Vec<Entity> = {
            let mut commands = app.world_mut().commands();
            spawn_immediate_entities_internal(&mut commands, &world_cfg, &cache)
        };
        app.update();

        // Look up each spawned entity's UUID and bucket by registered name.
        let mut spawned_uuids: Vec<String> = spawned
            .iter()
            .map(|e| {
                app.world()
                    .get::<EntityUuid>(*e)
                    .expect("every spawned entity must carry EntityUuid")
                    .0
                    .clone()
            })
            .collect();
        spawned_uuids.sort();

        let starbase_uuid = world_cfg
            .name_to_uuid
            .get("Starbase Alpha")
            .expect("Starbase Alpha must be registered");
        let earth_uuid = world_cfg
            .name_to_uuid
            .get("earth")
            .expect("earth must be registered");

        assert!(
            spawned_uuids.contains(starbase_uuid),
            "Starbase Alpha must be spawned (uuid={starbase_uuid}, spawned={spawned_uuids:?})"
        );
        assert!(
            spawned_uuids.contains(earth_uuid),
            "Earth must be spawned (uuid={earth_uuid}, spawned={spawned_uuids:?})"
        );
    }
}
