// Bevy plugin: reads the loaded ScenarioConfig and dispatches spawn actions
// through the existing entity-spawn pipeline. Also owns the server-side
// scenario runtime — comms template evaluation, inbox management, objective
// tracking, and the associated broadcast systems.

use bevy::prelude::*;
use std::collections::HashMap;

use crate::comms_inbox::CommsInbox;
use crate::entity_spawner::spawn_entity;
use crate::lobby::{CurrentPhase, InboundMessage, OutboundMessage, Sessions, Target};
use crate::messages::{
    ClientMessage, CommsContact, CommsMessage, Console, GamePhase, ObjectiveSnapshot,
    ServerMessage,
};
use crate::objectives::ObjectiveManager;
use crate::scenario::{
    ActiveDialogue, CommsDialogueNode, CommsTemplate, CommsTemplateState, TriggerAction,
    WorldEvent, evaluate_comms_templates,
};

// ── Resources ──────────────────────────────────────────────────────────────

/// Server-side runtime state for the currently active scenario.
///
/// Populated at `Startup` from `ScenarioResource` (which is itself populated
/// by `ConfigCachePlugin` when `wasm_load_scenario` is called before
/// `wasm_init`). When no scenario is loaded all vecs/maps are empty and comms
/// systems are no-ops.
#[derive(Resource, Default)]
pub struct ScenarioRuntime {
    /// Stable scenario ID string (used to scope inbox messages and objectives).
    pub scenario_id: String,
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

// ── Plugin ─────────────────────────────────────────────────────────────────

/// Bevy plugin that spawns entities declared in the active scenario and wires
/// up the server-side comms/objective systems.
pub struct ScenarioPlugin;

impl Plugin for ScenarioPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ScenarioRuntime>()
            .init_resource::<CommsInboxRes>()
            .init_resource::<ObjectiveManagerRes>()
            .add_systems(
                Startup,
                (spawn_scenario_entities, init_scenario_runtime),
            )
            .add_systems(
                Update,
                (
                    handle_hail,
                    handle_respond_to_message,
                    handle_clear_comms,
                    broadcast_comms_state,
                    broadcast_objective_summary,
                ).chain(),
            );
    }
}

// ── Startup systems ─────────────────────────────────────────────────────────

/// Startup system: resolves all scenario spawn positions and spawns each entity
/// through the shared `spawn_entity` helper.
fn spawn_scenario_entities(mut commands: Commands) {
    let scenario_config = match crate::config_cache::get_scenario_config() {
        Some(s) => s,
        None => return, // No scenario loaded — nothing to do.
    };

    let map_config = crate::config_cache::get_map_config();
    let anchors = map_config
        .as_ref()
        .map(|mc| mc.anchors.clone())
        .unwrap_or_default();

    let config_cache = crate::config_cache::get_config_cache();

    let resolved = match crate::scenario::resolve_positions(&scenario_config, &anchors) {
        Ok(r) => r,
        Err(e) => {
            bevy::log::error!("ScenarioPlugin: failed to resolve spawn positions: {e}");
            return;
        }
    };

    for spawn in &resolved {
        let config = config_cache.get(&spawn.entity_path);

        let Some(config) = config else {
            bevy::log::warn!(
                "ScenarioPlugin: no config found for entity path '{}' (spawn '{}') — skipping",
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
            "ScenarioPlugin: spawned '{}' at {:?} uuid={}",
            spawn.name,
            spawn.position,
            spawn.uuid
        );
    }
}

/// Startup system: initialises `ScenarioRuntime`, `CommsInboxRes`, and
/// `ObjectiveManagerRes` from the loaded `ScenarioConfig` (if any).
fn init_scenario_runtime(
    mut runtime: ResMut<ScenarioRuntime>,
    mut inbox: ResMut<CommsInboxRes>,
) {
    let scenario_config = match crate::config_cache::get_scenario_config() {
        Some(s) => s,
        None => return,
    };

    runtime.scenario_id = "default".to_string();
    runtime.name_to_uuid = scenario_config.name_to_uuid.clone();
    runtime.comms_template_states =
        crate::scenario::comms_template_states_from_config(&scenario_config);

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
    phase: Res<CurrentPhase>,
    mut runtime: ResMut<ScenarioRuntime>,
    mut inbox: ResMut<CommsInboxRes>,
) {
    if phase.0 != GamePhase::InProgress {
        return;
    }

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
    phase: Res<CurrentPhase>,
    mut runtime: ResMut<ScenarioRuntime>,
    mut inbox: ResMut<CommsInboxRes>,
    mut objectives: ResMut<ObjectiveManagerRes>,
) {
    if phase.0 != GamePhase::InProgress {
        return;
    }

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
    phase: Res<CurrentPhase>,
    mut inbox: ResMut<CommsInboxRes>,
) {
    if phase.0 != GamePhase::InProgress {
        return;
    }

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
/// or `ScenarioRuntime::needs_broadcast` is set.
fn broadcast_comms_state(
    phase: Res<CurrentPhase>,
    sessions: Res<Sessions>,
    mut runtime: ResMut<ScenarioRuntime>,
    mut inbox: ResMut<CommsInboxRes>,
    objectives: Res<ObjectiveManagerRes>,
    mut writer: MessageWriter<OutboundMessage>,
) {
    if phase.0 != GamePhase::InProgress {
        return;
    }

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

    writer.write(OutboundMessage {
        target: Target::Token(comms_token.to_string()),
        msg: ServerMessage::CommsState {
            messages,
            objectives: objectives_snap,
            contacts,
        },
    });

    inbox.0.mark_clean();
    runtime.needs_broadcast = false;
}

/// Broadcast `ObjectiveSummary` to the Captain when objectives change.
fn broadcast_objective_summary(
    phase: Res<CurrentPhase>,
    sessions: Res<Sessions>,
    mut objectives: ResMut<ObjectiveManagerRes>,
    mut writer: MessageWriter<OutboundMessage>,
) {
    if phase.0 != GamePhase::InProgress {
        return;
    }

    if !objectives.0.is_dirty() {
        return;
    }

    let Some(captain_token) = sessions.0.console_holder(Console::CaptainChair) else {
        objectives.0.mark_clean();
        return;
    };

    let objectives_snap = objectives.0.sorted_snapshots();

    writer.write(OutboundMessage {
        target: Target::Token(captain_token.to_string()),
        msg: ServerMessage::ObjectiveSummary {
            objectives: objectives_snap,
        },
    });

    objectives.0.mark_clean();
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lobby::{LobbyPlugin, OutboundMessage};
    use crate::messages::*;
    use crate::scenario::{CommsResponse, CommsTemplateState, TriggerCondition};

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
            .init_resource::<ScenarioRuntime>()
            .init_resource::<CommsInboxRes>()
            .init_resource::<ObjectiveManagerRes>()
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
        let msgs = app.world().resource::<Outbox>().0.clone();
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
        let runtime = &mut app.world_mut().resource_mut::<ScenarioRuntime>();
        runtime.name_to_uuid.insert("starbase_alpha".into(), station_uuid.into());
        runtime.contacts.push(CommsContact {
            uuid: station_uuid.into(),
            name: "Starbase Alpha".into(),
        });
        runtime.comms_template_states.push(CommsTemplateState {
            template: crate::scenario::CommsTemplate {
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
}
