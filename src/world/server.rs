use bevy::prelude::*;
use crate::damage::HullIntegrity;
use crate::lobby::WorldResource;
use crate::simulation::{Ship, ShipHullIntegrity};
use bevy::prelude::*;
use std::collections::HashMap;

use crate::comms_inbox::CommsInbox;
use crate::entity_spawner::spawn_entity;
use crate::lobby::{InboundMessage, Sessions, Target};
use crate::simulation::SimOutbox;
use crate::messages::{
    ClientMessage, CommsContact, CommsMessage, Console, ObjectiveSnapshot,
    ServerMessage,
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
    /// Stable scenario ID string (used to scope inbox messages and objectives).
    pub scenario_id: String,
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


pub struct WorldPlugin;

impl Plugin for WorldPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup_world_hardcoded)
            .init_resource::<WorldContentRuntime>()
            .init_resource::<CommsInboxRes>()
            .init_resource::<ObjectiveManagerRes>()
            .add_systems(
                Startup,
                (spawn_scenario_entities, init_scenario_runtime),
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
        hull: Some(crate::entity_config::HullConfig { hull_integrity: 100.0 }),
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
    };
    let ship_uuid = crate::entity_loader::assign_uuid();
    let ship_entity = crate::entity_spawner::spawn_entity(
        &mut commands, &ship_config, Vec3::ZERO, ship_uuid, Some("player-ship".to_string()),
    );
    commands.entity(ship_entity).insert(Ship);
    commands.insert_resource(ShipHullIntegrity(HullIntegrity::with_hp(100.0)));
}


// ── Startup systems ─────────────────────────────────────────────────────────

/// Startup system: resolves all scenario spawn positions and spawns each entity
/// through the shared `spawn_entity` helper.
fn spawn_scenario_entities(mut commands: Commands) {
    let scenario_config = match crate::config_cache::get_world_content_config() {
        Some(s) => s,
        None => return, // No scenario loaded — nothing to do.
    };

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

        spawn_entity(
            &mut commands,
            config,
            position,
            spawn.uuid.clone(),
            Some(spawn.name.clone()),
        );

        bevy::log::info!(
            "WorldPlugin: spawned '{}' at {:?} uuid={}",
            spawn.name,
            spawn.position,
            spawn.uuid
        );
    }
}

/// Startup system: initialises `WorldContentRuntime`, `CommsInboxRes`, and
/// `ObjectiveManagerRes` from the loaded `ScenarioConfig` (if any).
fn init_scenario_runtime(
    mut runtime: ResMut<WorldContentRuntime>,
    mut inbox: ResMut<CommsInboxRes>,
) {
    let scenario_config = match crate::config_cache::get_world_content_config() {
        Some(s) => s,
        None => return,
    };

    runtime.scenario_id = "default".to_string();
    runtime.name_to_uuid = scenario_config.name_to_uuid.clone();
    runtime.comms_template_states =
        crate::world::content::comms_template_states_from_config(&scenario_config);
    runtime.trigger_states = trigger_states_from_config(&scenario_config);

    // Build contacts list from comms templates: any entity referenced as
    // `from` in a comms template is a hailable contact (provided we have
    // its UUID).
    let mut contacts: Vec<CommsContact> = Vec::new();
    for tmpl in &scenario_config.comms {
        let uuid = match scenario_config.name_to_uuid.get(&tmpl.from) {
            Some(u) => u.clone(),
            None => continue,
        };
        // Avoid duplicates
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

            inbox.0.inject(msg, &runtime.scenario_id);

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
                    objectives.0.add(id, text, *mandatory, &runtime.scenario_id);
                }
                TriggerAction::CompleteObjective { id } => {
                    objectives.0.complete(id);
                }
                TriggerAction::FailObjective { id } => {
                    objectives.0.fail(id);
                }
                TriggerAction::LoadScenario { .. } => {
                    // Scenario loading is handled by the scenario manager; not yet
                    // wired into this system. No-op for now.
                }
                TriggerAction::SetAiState { .. } => {
                    // SetAiState is handled by the AI-event trigger system, not
                    // the comms response path. No-op here.
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

            inbox.0.inject(new_msg, &runtime.scenario_id);

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

// ── AI-event trigger system ─────────────────────────────────────────────────

/// Read `AiEntityAttacked` and `AiEntityDestroyed` messages, translate them
/// into `WorldEvent`s, evaluate the scenario trigger table, and execute the
/// resulting actions (including `SetAiState`).
fn handle_ai_events(
    mut runtime: ResMut<WorldContentRuntime>,
    mut objectives: ResMut<ObjectiveManagerRes>,
    mut attacked_reader: MessageReader<crate::ai_plugin::AiEntityAttacked>,
    mut destroyed_reader: MessageReader<crate::ai_plugin::AiEntityDestroyed>,
    mut ai_query: Query<(&EntityUuid, &mut AiControllerComponent, &BehaviourSection)>,
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
    let fired = evaluate_triggers(&mut runtime.trigger_states, &world_events, &name_to_uuid);

    for ft in fired {
        for action in &ft.actions {
            match action {
                TriggerAction::AddObjective { id, text, mandatory } => {
                    objectives.0.add(id.clone(), text.clone(), *mandatory, runtime.scenario_id.clone());
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
                TriggerAction::LoadScenario { .. } => {
                    // Not yet implemented at the plugin layer.
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
        runtime.scenario_id = "test".to_string();
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
            runtime.scenario_id = "test".to_string();
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

        // Set up trigger: on attacked → SetAiState to "chase"
        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime.scenario_id = "test".to_string();
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
}
