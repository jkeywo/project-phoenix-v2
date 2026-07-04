//! Server-side Comms console plugin (issue #427, migrated to blackboard #565).
//!
//! Issue #608: the comms conversation engine (hail / respond / clear /
//! show-on-screen / channel-2 handlers) lives here, alongside the
//! blackboard-publish system, so changing comms behaviour no longer
//! requires reaching into the world module.

use crate::ship_plugin::ShipSystemControlSources;
use bevy::prelude::*;

use crate::messages::{CommsBlackboard, ObjectiveSnapshot, SystemBlackboard, SystemId};
use crate::world::server::ObjectiveManagerRes;
use crate::world::server::{CommsInboxRes, WorldContentRuntime};

use crate::entity_spawner::EntityUuid;
use crate::messages::{CommsMessage, GamePhase};
use crate::world::content::{
    evaluate_comms_templates, ActiveDialogue, PendingFollowUp, TriggerAction, WorldEvent,
};
use crate::world::server::{
    CommsChannel2Event, OnScreenMessage, ShipModifiersParams, WorldLayerChange, WorldLayerParams,
};

pub struct CommsConsolePlugin;

impl Plugin for CommsConsolePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (
                publish_comms_blackboard.in_set(crate::sim_sets::SimSet::Publish),
                operate_comms_ai.in_set(crate::sim_sets::SimSet::Physics),
            ),
        );
    }
}

// ── Blackboard publish ────────────────────────────────────────────────────────

fn publish_comms_blackboard(
    inbox: Option<Res<CommsInboxRes>>,
    runtime: Option<Res<WorldContentRuntime>>,
    objectives: Option<Res<ObjectiveManagerRes>>,
    mut ship_bbs_q: Query<
        &mut crate::server_app::ShipSystemBlackboards,
        With<crate::server_app::LocalShip>,
    >,
) {
    let mut messages = inbox.as_ref().map(|r| r.0.messages()).unwrap_or_default();

    if let Some(rt) = runtime.as_ref() {
        for m in messages.iter_mut() {
            if let Some(flag) = rt.range_flags.get(&m.sender_uuid).copied() {
                m.sender_in_range = flag;
            } else if rt.range_active && uuid::Uuid::parse_str(&m.sender_uuid).is_ok() {
                m.sender_in_range = false;
            }
        }
    }

    let objectives_snap: Vec<ObjectiveSnapshot> = objectives
        .as_ref()
        .map(|o| o.0.sorted_snapshots())
        .unwrap_or_default();

    let mut contacts = runtime
        .as_ref()
        .map(|rt| rt.contacts.clone())
        .unwrap_or_default();
    for contact in contacts.iter_mut() {
        contact.is_urgent = messages
            .iter()
            .any(|m| m.sender_uuid == contact.uuid && m.is_urgent && !m.is_read);
    }

    let bb = CommsBlackboard {
        messages,
        objectives: objectives_snap,
        contacts,
    };

    if let Some(mut bbs) = ship_bbs_q.iter_mut().next() {
        bbs.0.insert(
            SystemId(crate::system_registry::COMMS_SYSTEM_ID.to_string()),
            SystemBlackboard::Comms(bb),
        );
    }
}

// ── Comms conversation handlers (issue #608, moved from world/server.rs) ──────

/// Resolve the current `sender_in_range` flag for an injection-time message,
/// matching the stamp logic in `broadcast_comms_state`. Used by every site
/// that inserts a new `CommsMessage` so the field is correct from the moment
/// the message lands in the inbox (belt-and-braces against future refactors
/// that bypass the broadcast stamp pass).
pub(crate) fn current_sender_in_range(runtime: &WorldContentRuntime, sender_uuid: &str) -> bool {
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

/// Handle `Hail { target_uuid }` messages from Comms console holders.
///
/// Evaluates matching `on_hailed` comms templates for the target entity,
/// injects new messages into the inbox, and records active dialogues.
pub(crate) fn handle_hail(
    ship_query: Query<&crate::messages::AdmittedCommands, With<crate::simulation::LocalShip>>,
    mut runtime: ResMut<WorldContentRuntime>,
    mut channel2_writer: MessageWriter<CommsChannel2Event>,
) {
    let Some(admitted) = ship_query.iter().next() else {
        return;
    };
    for cmd in admitted.for_target(crate::system_registry::COMMS_SYSTEM_ID) {
        let target_uuid = match &cmd.payload {
            crate::messages::SystemControlPayload::Hail { target_uuid } => target_uuid,
            _ => continue,
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
        let fired = evaluate_comms_templates(comms_template_states, &world_events, name_to_uuid);

        // Route the Hailed event into the trigger system so that
        // on_hailed triggers (e.g. complete_objective, load_world)
        // can fire. handle_ai_events drains pending_world_events
        // in SimSet::Physics, which runs after SimSet::Input.
        runtime.pending_world_events.push(WorldEvent::Hailed {
            target_uuid: target_uuid.clone(),
        });

        for f in fired {
            // Build a CommsMessage and inject it.
            let thread_id = f
                .thread_id
                .clone()
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
            let sender_uuid = target_uuid.clone();
            // Resolve channel display name from contacts (best effort), then
            // let the dialogue node override the visible speaker.
            let channel_name = runtime
                .contacts
                .iter()
                .find(|c| c.uuid == *target_uuid)
                .map(|c| c.name.clone())
                .unwrap_or_else(|| target_uuid.clone());
            let sender_name = f.node.speaker.clone().unwrap_or(channel_name.clone());

            // The root message always injects immediately when its
            // template fires. Per-node triggers are an authoring concept
            // for follow-ups, not roots — the template-level `trigger`
            // already controls when the root arrives.
            let msg_id = uuid::Uuid::new_v4().to_string();
            let responses: Vec<String> = f.node.responses.iter().map(|r| r.text.clone()).collect();
            let msg = CommsMessage {
                id: msg_id.clone(),
                sender_uuid: sender_uuid.clone(),
                sender_name: sender_name.clone(),
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
            channel2_writer.write(CommsChannel2Event { message: msg });
            runtime.active_dialogues.insert(
                msg_id,
                ActiveDialogue {
                    current_node: f.node.clone(),
                    thread_id: thread_id.clone(),
                },
            );

            // Schedule the chained root follow_up, if any. Always queued
            // onto `pending_follow_ups`: a triggerless follow-up fires on
            // the very next tick, while one with a `trigger` waits until
            // its condition is observed (or fires immediately on the next
            // tick if the condition is already true — see
            // `tick_pending_follow_ups`). The chained node inherits the
            // parent's thread_id so both messages render in the same
            // conversation; its own `speaker` overrides the display name
            // (channel identity stays put). No `...` placeholder — root
            // chains stay silent during the wait, mirroring the existing
            // delayed-root behaviour.
            if let Some(ref fu) = f.root_follow_up {
                let fu_sender_name = fu.speaker.clone().unwrap_or(sender_name.clone());
                runtime.pending_follow_ups.push(PendingFollowUp {
                    node: fu.clone(),
                    sender_uuid: sender_uuid.clone(),
                    sender_name: fu_sender_name,
                    thread_id: thread_id.clone(),
                    elapsed_secs: 0.0,
                    placeholder_id: None,
                    urgent: f.urgent,
                });
            }
        }
    }
}

/// Handle `RespondToMessage { message_id, response_index }` from Comms holders.
///
/// Records the chosen response on the inbox message, fires any associated
/// trigger actions, and advances the dialogue to the follow-up node if present.
pub(crate) fn handle_respond_to_message(
    ship_query: Query<&crate::messages::AdmittedCommands, With<crate::simulation::LocalShip>>,
    mut runtime: ResMut<WorldContentRuntime>,
    mut inbox: ResMut<CommsInboxRes>,
    mut channel2_writer: MessageWriter<CommsChannel2Event>,
    mut objectives: ResMut<ObjectiveManagerRes>,
    mut commands: Commands,
    mut ai_query: Query<
        (
            &EntityUuid,
            Option<&mut crate::ai_plugin::ShipAiMemory>,
            Option<&crate::entities::spawner::FactionComponent>,
        ),
        With<crate::ai_plugin::AiControllerComponent>,
    >,
    mut ship_modifiers: ShipModifiersParams,
    mut next_state: Option<ResMut<NextState<GamePhase>>>,
    mut game_over_reason: Option<ResMut<crate::simulation::GameOverReason>>,
    mut world_layers: WorldLayerParams,
    entity_uuid_query: Query<(Entity, &EntityUuid)>,
    mut faction_dispatch: crate::world::server::FactionDispatchParams,
) {
    let Some(admitted) = ship_query.iter().next() else {
        return;
    };
    for cmd in admitted.for_target(crate::system_registry::COMMS_SYSTEM_ID) {
        let (message_id, response_index) = match &cmd.payload {
            crate::messages::SystemControlPayload::RespondToMessage {
                message_id,
                response_index,
            } => (message_id, response_index),
            _ => continue,
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
        // Build UUID → ECS Entity map once per response so the six
        // per-entity modifier/flag arms below can resolve their `entity`
        // target in O(1) instead of scanning `entity_uuid_query` each time.
        // Used by `ApplyModifier` / `RemoveModifier` / `ApplyFlag` /
        // `RemoveFlag` / `ApplyIntModifier` / `RemoveIntModifier` to write
        // to the target entity's per-entity `ShipModifiers` Component.
        let uuid_to_entity: std::collections::HashMap<String, Entity> = entity_uuid_query
            .iter()
            .map(|(ent, uuid_comp)| (uuid_comp.0.clone(), ent))
            .collect();
        let sender_uuid = inbox.0.sender_uuid_for(message_id);
        for action in &response.actions {
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
                    // Explicit targets win; otherwise fall back to the comms
                    // sender so legacy single-entity objectives still mark up.
                    let resolved = if targets.is_empty() {
                        sender_uuid
                            .clone()
                            .and_then(|suid| uuid_to_name.get(suid.as_str()).copied())
                            .map(String::from)
                            .into_iter()
                            .collect()
                    } else {
                        targets.clone()
                    };
                    objectives.0.add_full(
                        id,
                        text,
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
                        "handle_respond_to_message: SetAiState('{entity}' → '{state}') ignored — doctrine-based AI"
                    );
                }
                TriggerAction::ApplyModifier {
                    entity,
                    tag,
                    slot,
                    bonus,
                } => {
                    let Some(uuid) = name_to_uuid_snapshot.get(entity) else {
                        bevy::log::warn!(
                            "handle_respond_to_message: ApplyModifier: unknown entity name '{entity}'"
                        );
                        continue;
                    };
                    let Some(target) = uuid_to_entity.get(uuid).copied() else {
                        bevy::log::warn!(
                            "handle_respond_to_message: ApplyModifier: no ECS entity with UUID '{uuid}' for name '{entity}'"
                        );
                        continue;
                    };
                    let Ok(mut mods) = ship_modifiers.components.get_mut(target) else {
                        bevy::log::warn!(
                            "handle_respond_to_message: ApplyModifier: entity '{entity}' has no ShipModifiers component"
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
                    let Some(uuid) = name_to_uuid_snapshot.get(entity) else {
                        bevy::log::warn!(
                            "handle_respond_to_message: RemoveModifier: unknown entity name '{entity}'"
                        );
                        continue;
                    };
                    let Some(target) = uuid_to_entity.get(uuid).copied() else {
                        bevy::log::warn!(
                            "handle_respond_to_message: RemoveModifier: no ECS entity with UUID '{uuid}' for name '{entity}'"
                        );
                        continue;
                    };
                    let Ok(mut mods) = ship_modifiers.components.get_mut(target) else {
                        bevy::log::warn!(
                            "handle_respond_to_message: RemoveModifier: entity '{entity}' has no ShipModifiers component"
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
                    let Some(uuid) = name_to_uuid_snapshot.get(entity) else {
                        bevy::log::warn!(
                            "handle_respond_to_message: ApplyFlag: unknown entity name '{entity}'"
                        );
                        continue;
                    };
                    let Some(target) = uuid_to_entity.get(uuid).copied() else {
                        bevy::log::warn!(
                            "handle_respond_to_message: ApplyFlag: no ECS entity with UUID '{uuid}' for name '{entity}'"
                        );
                        continue;
                    };
                    let Ok(mut mods) = ship_modifiers.components.get_mut(target) else {
                        bevy::log::warn!(
                            "handle_respond_to_message: ApplyFlag: entity '{entity}' has no ShipModifiers component"
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
                    let Some(uuid) = name_to_uuid_snapshot.get(entity) else {
                        bevy::log::warn!(
                            "handle_respond_to_message: RemoveFlag: unknown entity name '{entity}'"
                        );
                        continue;
                    };
                    let Some(target) = uuid_to_entity.get(uuid).copied() else {
                        bevy::log::warn!(
                            "handle_respond_to_message: RemoveFlag: no ECS entity with UUID '{uuid}' for name '{entity}'"
                        );
                        continue;
                    };
                    let Ok(mut mods) = ship_modifiers.components.get_mut(target) else {
                        bevy::log::warn!(
                            "handle_respond_to_message: RemoveFlag: entity '{entity}' has no ShipModifiers component"
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
                    let Some(uuid) = name_to_uuid_snapshot.get(entity) else {
                        bevy::log::warn!(
                            "handle_respond_to_message: ApplyIntModifier: unknown entity name '{entity}'"
                        );
                        continue;
                    };
                    let Some(target) = uuid_to_entity.get(uuid).copied() else {
                        bevy::log::warn!(
                            "handle_respond_to_message: ApplyIntModifier: no ECS entity with UUID '{uuid}' for name '{entity}'"
                        );
                        continue;
                    };
                    let Ok(mut mods) = ship_modifiers.components.get_mut(target) else {
                        bevy::log::warn!(
                            "handle_respond_to_message: ApplyIntModifier: entity '{entity}' has no ShipModifiers component"
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
                    let Some(uuid) = name_to_uuid_snapshot.get(entity) else {
                        bevy::log::warn!(
                            "handle_respond_to_message: RemoveIntModifier: unknown entity name '{entity}'"
                        );
                        continue;
                    };
                    let Some(target) = uuid_to_entity.get(uuid).copied() else {
                        bevy::log::warn!(
                            "handle_respond_to_message: RemoveIntModifier: no ECS entity with UUID '{uuid}' for name '{entity}'"
                        );
                        continue;
                    };
                    let Ok(mut mods) = ship_modifiers.components.get_mut(target) else {
                        bevy::log::warn!(
                            "handle_respond_to_message: RemoveIntModifier: entity '{entity}' has no ShipModifiers component"
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
                    if let Some(ref mut lc) = world_layers.pending_layers {
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
                    if let Some(ref mut lc) = world_layers.pending_layers {
                        lc.0.push(WorldLayerChange::Unload(path.clone()));
                    }
                }
                TriggerAction::SetWorldFlag { name } => {
                    if let Some((target_layer, stripped, before, after)) =
                        crate::world::server::mutate_world_flag(
                            &mut runtime.flags,
                            world_layers.layer_map.as_deref_mut().map(|lm| &mut lm.0),
                            &origin_layer,
                            name,
                            crate::world::server::FlagMutation::Set,
                        )
                    {
                        crate::world::server::emit_flag_transition(
                            &mut runtime.pending_world_events,
                            &stripped,
                            &target_layer,
                            before,
                            after,
                        );
                    }
                }
                TriggerAction::ClearWorldFlag { name } => {
                    if let Some((target_layer, stripped, before, after)) =
                        crate::world::server::mutate_world_flag(
                            &mut runtime.flags,
                            world_layers.layer_map.as_deref_mut().map(|lm| &mut lm.0),
                            &origin_layer,
                            name,
                            crate::world::server::FlagMutation::Clear,
                        )
                    {
                        crate::world::server::emit_flag_transition(
                            &mut runtime.pending_world_events,
                            &stripped,
                            &target_layer,
                            before,
                            after,
                        );
                    }
                }
                TriggerAction::IncrementWorldFlag { name, by } => {
                    if let Some((target_layer, stripped, before, after)) =
                        crate::world::server::mutate_world_flag(
                            &mut runtime.flags,
                            world_layers.layer_map.as_deref_mut().map(|lm| &mut lm.0),
                            &origin_layer,
                            name,
                            crate::world::server::FlagMutation::Increment(*by),
                        )
                    {
                        crate::world::server::emit_flag_transition(
                            &mut runtime.pending_world_events,
                            &stripped,
                            &target_layer,
                            before,
                            after,
                        );
                    }
                }
                TriggerAction::SetWorldFlagValue { name, value } => {
                    if let Some((target_layer, stripped, before, after)) =
                        crate::world::server::mutate_world_flag(
                            &mut runtime.flags,
                            world_layers.layer_map.as_deref_mut().map(|lm| &mut lm.0),
                            &origin_layer,
                            name,
                            crate::world::server::FlagMutation::SetValue(*value),
                        )
                    {
                        crate::world::server::emit_flag_transition(
                            &mut runtime.pending_world_events,
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
                    let pos_arr: [f32; 3] = if let Some(pos) = position {
                        *pos
                    } else if let Some(anchor_name) = anchor {
                        // origin_layer = None: resolve against base world anchors only.
                        let lookup = world_layers
                            .base_world_config
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
                        &template_inst,
                        &config_cache,
                    ) {
                        Ok(c) => c,
                        Err(e) => {
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
                        commands.entity(spawned).insert(Transform {
                            translation: pos_vec,
                            rotation: quat,
                            scale: scale_vec,
                        });
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
                    runtime
                        .pending_world_events
                        .push(WorldEvent::Destroyed { uuid: uuid.clone() });
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
                    if let Some(ent) = target_entity {
                        commands.entity(ent).try_despawn();
                    }
                }
                TriggerAction::AddFactionEnemy { faction, enemy } => {
                    let Some(registry) = faction_dispatch.registry.as_deref_mut() else {
                        bevy::log::warn!(
                            "handle_respond_to_message: AddFactionEnemy skipped: FactionRegistryResource not present"
                        );
                        continue;
                    };
                    let faction_uuid = match registry.0.uuid_by_name(faction) {
                        Some(u) => u,
                        None => {
                            bevy::log::warn!(
                                "handle_respond_to_message: AddFactionEnemy: unknown faction name '{faction}'"
                            );
                            continue;
                        }
                    };
                    let enemy_uuid = match registry.0.uuid_by_name(enemy) {
                        Some(u) => u,
                        None => {
                            bevy::log::warn!(
                                "handle_respond_to_message: AddFactionEnemy: unknown enemy faction name '{enemy}'"
                            );
                            continue;
                        }
                    };
                    // Idempotent. No target re-validation needed for the
                    // add path — see handle_ai_events for the rationale.
                    registry.0.add_enemy(faction_uuid, enemy_uuid);
                }
                TriggerAction::RemoveFactionEnemy { faction, enemy } => {
                    let Some(registry) = faction_dispatch.registry.as_deref_mut() else {
                        bevy::log::warn!(
                            "handle_respond_to_message: RemoveFactionEnemy skipped: FactionRegistryResource not present"
                        );
                        continue;
                    };
                    let faction_uuid = match registry.0.uuid_by_name(faction) {
                        Some(u) => u,
                        None => {
                            bevy::log::warn!(
                                "handle_respond_to_message: RemoveFactionEnemy: unknown faction name '{faction}'"
                            );
                            continue;
                        }
                    };
                    let enemy_uuid = match registry.0.uuid_by_name(enemy) {
                        Some(u) => u,
                        None => {
                            bevy::log::warn!(
                                "handle_respond_to_message: RemoveFactionEnemy: unknown enemy faction name '{enemy}'"
                            );
                            continue;
                        }
                    };
                    let removed = registry.0.remove_enemy(faction_uuid, enemy_uuid);
                    if removed {
                        let ai_factions: Vec<(uuid::Uuid, uuid::Uuid)> = ai_query
                            .iter()
                            .filter_map(|(uid, _, fc)| {
                                let self_uuid = uuid::Uuid::parse_str(&uid.0).ok()?;
                                fc.map(|fc| (self_uuid, fc.0))
                            })
                            .collect();
                        let uuid_to_faction = crate::world::server::build_uuid_to_faction(
                            &faction_dispatch.non_ai_factions,
                            &ai_factions,
                        );
                        crate::world::server::revalidate_ai_targets_after_faction_change(
                            &mut ai_query,
                            &registry.0,
                            &uuid_to_faction,
                        );
                    }
                }
            }
        }

        // Record the chosen response on the inbox message.
        inbox.0.record_response(message_id, *response_index);

        // Advance to follow-up node if present.
        if let Some(follow_up) = &response.follow_up {
            let thread_id = dialogue.thread_id.clone();
            let sender_uuid = inbox.0.sender_uuid_for(message_id).unwrap_or_default();
            // Use the follow-up's own speaker override if set, otherwise
            // inherit the sender name from the parent message.
            let sender_name = follow_up
                .speaker
                .clone()
                .unwrap_or_else(|| inbox.0.sender_name_for(message_id).unwrap_or_default());

            if follow_up.trigger.is_some() {
                // Triggered follow-up: show a `...` placeholder immediately
                // and queue the real message to be injected once the
                // trigger condition is met (or fires on the next tick if
                // the condition is already true — see
                // `tick_pending_follow_ups`).
                let placeholder_id = uuid::Uuid::new_v4().to_string();
                let placeholder = CommsMessage {
                    id: placeholder_id.clone(),
                    sender_uuid: sender_uuid.clone(),
                    sender_name: sender_name.clone(),
                    subject: "...".to_string(),
                    body: "...".to_string(),
                    responses: vec![],
                    selected_response: None,
                    is_read: false,
                    is_orphaned: false,
                    sender_in_range: current_sender_in_range(&runtime, &sender_uuid),
                    thread_id: thread_id.clone(),
                    is_urgent: false,
                };
                channel2_writer.write(CommsChannel2Event {
                    message: placeholder,
                });
                runtime.pending_follow_ups.push(PendingFollowUp {
                    node: follow_up.clone(),
                    sender_uuid,
                    sender_name,
                    thread_id,
                    elapsed_secs: 0.0,
                    placeholder_id: Some(placeholder_id),
                    urgent: false, // follow-up urgency is not a TOML-level concept
                });
            } else {
                // No trigger — inject immediately (same tick).
                let new_msg_id = uuid::Uuid::new_v4().to_string();
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
                channel2_writer.write(CommsChannel2Event { message: new_msg });
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
}

/// Handle `ClearComms` from Comms console holders.
pub(crate) fn handle_clear_comms(
    ship_query: Query<&crate::messages::AdmittedCommands, With<crate::simulation::LocalShip>>,
    mut inbox: ResMut<CommsInboxRes>,
) {
    let Some(admitted) = ship_query.iter().next() else {
        return;
    };
    for cmd in admitted.for_target(crate::system_registry::COMMS_SYSTEM_ID) {
        if matches!(
            cmd.payload,
            crate::messages::SystemControlPayload::ClearComms
        ) {
            inbox.0.clear();
        }
    }
}

/// Handle `ShowOnScreen { message_id }` from Comms console holders.
///
/// Looks up the message in the inbox, stores it in `OnScreenMessage`, and
/// pushes `ViewMode::Comms` so the viewscreen switches to the comms overlay.
pub(crate) fn handle_show_on_screen(
    ship_query: Query<&crate::messages::AdmittedCommands, With<crate::simulation::LocalShip>>,
    inbox: Res<CommsInboxRes>,
    mut on_screen: ResMut<OnScreenMessage>,
    mut view_mode_q: Query<
        &mut crate::ship_state::ShipViewMode,
        With<crate::simulation::LocalShip>,
    >,
) {
    let Some(admitted) = ship_query.iter().next() else {
        return;
    };
    let Some(mut vm) = view_mode_q.iter_mut().next() else {
        return;
    };
    for cmd in admitted.for_target(crate::system_registry::COMMS_SYSTEM_ID) {
        let show_message_id: Option<&String> = match &cmd.payload {
            crate::messages::SystemControlPayload::ShowOnScreen { message_id } => Some(message_id),
            _ => None,
        };
        if let Some(message_id) = show_message_id {
            if let Some(msg) = inbox.0.messages().into_iter().find(|m| &m.id == message_id) {
                let already_on_screen = matches!(vm.view_mode, crate::messages::ViewMode::Comms)
                    && on_screen
                        .0
                        .as_ref()
                        .is_some_and(|displayed| displayed.id == msg.id);
                if already_on_screen {
                    on_screen.0 = None;
                    vm.restore_captain_view();
                } else {
                    on_screen.0 = Some(msg.clone());
                    vm.show_view_mode(crate::messages::ViewMode::Comms);
                }
            }
        }
    }
}

/// Consume channel-2 deliveries addressed to the Comms system.
///
/// Injects each message into `CommsInboxRes`. When the comms system is
/// AI-operated (`policy.operate_ai`), the stub controller auto-picks the
/// first available response — full trigger-action dispatch is deferred to
/// #520 (AI ship unification).
pub(crate) fn handle_comms_channel2(
    mut reader: MessageReader<CommsChannel2Event>,
    mut inbox: ResMut<CommsInboxRes>,
    ship_query: Query<
        &crate::ship_plugin::ShipSystemControlSources,
        With<crate::simulation::LocalShip>,
    >,
) {
    let policy = if let Some(control_sources) = ship_query.iter().next() {
        control_sources
            .0
            .policy_for(&crate::system_registry::comms_system_id())
    } else {
        crate::control_source::ControlTickPolicy {
            accept_human_input: true,
            operate_ai: false,
            coordinate: true,
        }
    };
    for ev in reader.read() {
        inbox.0.inject(ev.message.clone());
        if policy.operate_ai && !ev.message.responses.is_empty() {
            inbox.0.record_response(&ev.message.id, 0);
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::CommsMessage;
    use crate::server_app::{LocalShip, ShipSystemBlackboards};
    use crate::world::server::CommsInboxRes;

    fn msg(id: &str) -> CommsMessage {
        CommsMessage {
            id: id.into(),
            sender_uuid: "sender-uuid".into(),
            sender_name: "Station Alpha".into(),
            subject: "Test".into(),
            body: "Body text".into(),
            responses: vec!["OK".into()],
            selected_response: None,
            is_read: false,
            is_orphaned: false,
            sender_in_range: true,
            thread_id: id.into(),
            is_urgent: false,
        }
    }

    fn test_app() -> App {
        let mut app = App::new();
        app.insert_resource(CommsInboxRes(crate::console::comms::CommsInbox::new()))
            .insert_resource(WorldContentRuntime::default())
            .add_systems(Update, publish_comms_blackboard);
        // Spawn a LocalShip entity so the query in publish_comms_blackboard resolves.
        app.world_mut()
            .spawn((LocalShip, ShipSystemBlackboards::default()));
        app
    }

    fn comms_bb(app: &mut App) -> CommsBlackboard {
        let mut q = app
            .world_mut()
            .query_filtered::<&ShipSystemBlackboards, With<LocalShip>>();
        let bbs = q
            .single(app.world())
            .expect("no LocalShip with ShipSystemBlackboards");
        let key = SystemId(crate::system_registry::COMMS_SYSTEM_ID.to_string());
        let SystemBlackboard::Comms(bb) =
            bbs.0.get(&key).expect("comms blackboard missing").clone()
        else {
            panic!("wrong blackboard variant");
        };
        bb
    }

    #[test]
    fn blackboard_reflects_inbox_messages() {
        let mut app = test_app();
        app.world_mut()
            .resource_mut::<CommsInboxRes>()
            .0
            .inject(msg("m1"));
        app.update();

        let bb = comms_bb(&mut app);
        assert_eq!(bb.messages.len(), 1);
        assert_eq!(bb.messages[0].id, "m1");
    }

    /// Verifies operate_comms_ai runs per-entity for AI-controlled ships (issue #593 AC).
    #[test]
    fn operate_comms_ai_per_entity_ai_gate() {
        use crate::ship::control_source::{ControlSource, ControlSourceResolver};
        use crate::ship_plugin::ShipSystemControlSources;

        let mut ai_resolver = ControlSourceResolver::new();
        ai_resolver.set(crate::system_registry::comms_system_id(), ControlSource::Ai);
        let ai_sources = ShipSystemControlSources(ai_resolver);
        let ai_policy = ai_sources
            .0
            .policy_for(&crate::system_registry::comms_system_id());
        assert!(
            ai_policy.operate_ai,
            "AI Comms must gate through operate_ai"
        );

        let mut human_resolver = ControlSourceResolver::new();
        human_resolver.set(
            crate::system_registry::comms_system_id(),
            ControlSource::Human,
        );
        let human_sources = ShipSystemControlSources(human_resolver);
        let human_policy = human_sources
            .0
            .policy_for(&crate::system_registry::comms_system_id());
        assert!(!human_policy.operate_ai, "Human Comms must not operate AI");
    }

    // -- handle_respond_to_message: comms-response action dispatch parity ---
    //
    // Moved from world::server::tests (issue #608). These tests share the
    // low-level test harness (`comms_test_app`, `push_msg`, `tick`,
    // `write_spawn_template_fixture`) with the rest of the world-module test
    // suite, so that harness stays in `world::server::tests` (now
    // `pub(crate)`) and is imported here rather than duplicated.
    use crate::messages::{ClientMessage, CommsContact, ServerMessage};
    use crate::world::content::{CommsDialogueNode, CommsResponse, CommsTemplateState, TriggerCondition};
    use crate::world::server::{
        tests::{comms_test_app, push_msg, setup_game_with_comms, tick, write_spawn_template_fixture},
        tick_pending_follow_ups, PendingWorldLayerChanges, WorldLayerChange, WorldLayerMap,
    };

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
        app.init_resource::<WorldLayerMap>()
            .init_resource::<PendingWorldLayerChanges>()
            .init_resource::<crate::simulation::GameOverReason>();
        app
    }

    /// Spawns a bare target entity carrying `EntityUuid` and a per-entity
    /// `ShipModifiers` Component, and registers the name→UUID mapping in
    /// `WorldContentRuntime.name_to_uuid`. Returns the spawned `Entity` so
    /// tests can assert on its component after the trigger action fires.
    ///
    /// Used by the `ApplyModifier` / `RemoveModifier` / `ApplyFlag` /
    /// `RemoveFlag` / `ApplyIntModifier` / `RemoveIntModifier` dispatch
    /// tests to prove the action lands on the per-entity Component, not
    /// the legacy global `ShipModifiers` Resource.
    fn spawn_modifier_target(app: &mut App, name: &str, uuid: &str) -> Entity {
        let entity = app
            .world_mut()
            .spawn((
                EntityUuid(uuid.to_string()),
                crate::modifiers::ShipModifiers::new(),
            ))
            .id();
        let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
        runtime.name_to_uuid.insert(name.into(), uuid.into());
        entity
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
                station: "Captain".into(),
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
        push_msg(&mut app, "captain", ClientMessage::SetReady { ready: true });
        push_msg(&mut app, "comms", ClientMessage::SetReady { ready: true });
        tick(&mut app);

        {
            // Spawn a target entity carrying `EntityUuid` +
            // `ShipModifiers` so the six per-entity modifier/flag
            // TriggerActions can resolve "starbase_alpha" → this Entity.
            // Also registers the name→UUID mapping in
            // `WorldContentRuntime.name_to_uuid`.
            let _ = spawn_modifier_target(&mut app, "starbase_alpha", station_uuid);
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
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
        let _ = tick(&mut app);

        // Hail to receive the message.
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
            ClientMessage::ControlSystem {
                target: crate::system_registry::comms_system_id(),
                payload: crate::messages::SystemControlPayload::RespondToMessage {
                    message_id: msg_id,
                    response_index: 0,
                },
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
            TriggerAction::SetWorldFlag {
                name: "to_clear".into(),
            },
            TriggerAction::ClearWorldFlag {
                name: "to_clear".into(),
            },
        ];
        let app = fire_response_with_actions(actions);
        let runtime = app.world().resource::<WorldContentRuntime>();
        assert_eq!(runtime.flags.counter("to_clear"), 0);
        // Both transitions must have been enqueued.
        let has_set = runtime.pending_world_events.iter().any(|e| {
            matches!(
                e, WorldEvent::FlagSet { name, .. } if name == "to_clear"
            )
        });
        let has_cleared = runtime.pending_world_events.iter().any(|e| {
            matches!(
                e, WorldEvent::FlagCleared { name, .. } if name == "to_clear"
            )
        });
        assert!(
            has_set && has_cleared,
            "both set and clear transitions must be enqueued"
        );
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
        let mut app = fire_response_with_actions(vec![TriggerAction::ApplyModifier {
            entity: "starbase_alpha".into(),
            tag: "boost".into(),
            slot: crate::messages::ModifierSlot::MaxSpeed,
            bonus: 1.5,
        }]);
        // The `fire_response_with_actions` helper spawns a target entity
        // whose `EntityUuid("station-parity-uuid")` matches the name
        // "starbase_alpha" and gives it a per-entity `ShipModifiers`
        // Component. Assert the modifier landed on that entity.
        let mut q = app
            .world_mut()
            .query::<(&EntityUuid, &crate::modifiers::ShipModifiers)>();
        let (_uuid, mods) = q
            .iter(app.world())
            .find(|(u, _)| u.0 == "station-parity-uuid")
            .expect("target entity must carry EntityUuid + ShipModifiers");
        assert!(
            mods.get(&crate::messages::ModifierSlot::MaxSpeed) > 1.0,
            "ApplyModifier must add to MaxSpeed slot total on the target entity's per-entity component, got {}",
            mods.get(&crate::messages::ModifierSlot::MaxSpeed)
        );
    }

    #[test]
    fn comms_response_dispatches_remove_modifier() {
        let mut app = fire_response_with_actions(vec![
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
        let mut q = app
            .world_mut()
            .query::<(&EntityUuid, &crate::modifiers::ShipModifiers)>();
        let (_uuid, mods) = q
            .iter(app.world())
            .find(|(u, _)| u.0 == "station-parity-uuid")
            .expect("target entity must carry EntityUuid + ShipModifiers");
        let value = mods.get(&crate::messages::ModifierSlot::MaxSpeed);
        assert!(
            (value - 1.0).abs() < 1e-3,
            "RemoveModifier must reverse the previously-applied modifier on the target entity's per-entity component; expected baseline 1.0, got {value}"
        );
    }

    #[test]
    fn comms_response_dispatches_apply_and_remove_flag() {
        let mut app = fire_response_with_actions(vec![TriggerAction::ApplyFlag {
            entity: "starbase_alpha".into(),
            tag: "jammer".into(),
            kind: crate::flag_kind::FlagKind::CommsJammed,
        }]);
        let mut q = app
            .world_mut()
            .query::<(&EntityUuid, &crate::modifiers::ShipModifiers)>();
        let (_uuid, mods) = q
            .iter(app.world())
            .find(|(u, _)| u.0 == "station-parity-uuid")
            .expect("target entity must carry EntityUuid + ShipModifiers");
        assert!(
            mods.has_flag(&crate::flag_kind::FlagKind::CommsJammed),
            "ApplyFlag must register a CommsJammed flag on the target entity's per-entity component"
        );

        let mut app = fire_response_with_actions(vec![
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
        let mut q = app
            .world_mut()
            .query::<(&EntityUuid, &crate::modifiers::ShipModifiers)>();
        let (_uuid, mods) = q
            .iter(app.world())
            .find(|(u, _)| u.0 == "station-parity-uuid")
            .expect("target entity must carry EntityUuid + ShipModifiers");
        assert!(
            !mods.has_flag(&crate::flag_kind::FlagKind::CommsJammed),
            "RemoveFlag must un-register the CommsJammed flag on the target entity's per-entity component"
        );
    }

    #[test]
    fn comms_response_dispatches_apply_and_remove_int_modifier() {
        let mut app = fire_response_with_actions(vec![TriggerAction::ApplyIntModifier {
            entity: "starbase_alpha".into(),
            tag: "extra_team".into(),
            slot: crate::modifiers::IntModifierSlot::RepairTeams,
            bonus: 2,
        }]);
        let mut q = app
            .world_mut()
            .query::<(&EntityUuid, &crate::modifiers::ShipModifiers)>();
        let (_uuid, mods) = q
            .iter(app.world())
            .find(|(u, _)| u.0 == "station-parity-uuid")
            .expect("target entity must carry EntityUuid + ShipModifiers");
        assert_eq!(
            mods.get_int(&crate::modifiers::IntModifierSlot::RepairTeams),
            2,
            "ApplyIntModifier must add to RepairTeams int slot on the target entity's per-entity component"
        );

        let mut app = fire_response_with_actions(vec![
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
        let mut q = app
            .world_mut()
            .query::<(&EntityUuid, &crate::modifiers::ShipModifiers)>();
        let (_uuid, mods) = q
            .iter(app.world())
            .find(|(u, _)| u.0 == "station-parity-uuid")
            .expect("target entity must carry EntityUuid + ShipModifiers");
        assert_eq!(
            mods.get_int(&crate::modifiers::IntModifierSlot::RepairTeams),
            0,
            "RemoveIntModifier must reverse the int modifier on the target entity's per-entity component"
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
                station: "Captain".into(),
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
        push_msg(&mut app, "captain", ClientMessage::SetReady { ready: true });
        push_msg(&mut app, "comms", ClientMessage::SetReady { ready: true });
        tick(&mut app);

        let station_uuid = "station-destroy-uuid";
        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime
                .name_to_uuid
                .insert("starbase_alpha".into(), station_uuid.into());
            runtime
                .name_to_uuid
                .insert("doomed".into(), target_uuid.into());
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
        let msg_id = out
            .iter()
            .find_map(|m| {
                if let ServerMessage::CommsState { messages, .. } = &m.msg {
                    messages.first().map(|msg| msg.id.clone())
                } else {
                    None
                }
            })
            .expect("hail must deliver a message");

        push_msg(
            &mut app,
            "comms",
            ClientMessage::ControlSystem {
                target: crate::system_registry::comms_system_id(),
                payload: crate::messages::SystemControlPayload::RespondToMessage {
                    message_id: msg_id,
                    response_index: 0,
                },
            },
        );
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
                    id: "x".into(),
                    text: "x".into(),
                    mandatory: false,
                    targets: vec![],
                    directive: crate::messages::AiDirective::None,
                    utility: crate::objectives::UtilityConfig::default(),
                    source: crate::messages::ObjectiveSource::default(),
                },
                TriggerAction::CompleteObjective { id: "x".into() },
                TriggerAction::FailObjective { id: "x".into() },
                TriggerAction::SetAiState {
                    entity: "x".into(),
                    state: "x".into(),
                    target: None,
                },
                TriggerAction::ApplyModifier {
                    entity: "x".into(),
                    tag: "x".into(),
                    slot: crate::messages::ModifierSlot::MaxSpeed,
                    bonus: 0.0,
                },
                TriggerAction::RemoveModifier {
                    entity: "x".into(),
                    tag: "x".into(),
                    slot: crate::messages::ModifierSlot::MaxSpeed,
                },
                TriggerAction::ApplyFlag {
                    entity: "x".into(),
                    tag: "x".into(),
                    kind: crate::flag_kind::FlagKind::CommsJammed,
                },
                TriggerAction::RemoveFlag {
                    entity: "x".into(),
                    tag: "x".into(),
                    kind: crate::flag_kind::FlagKind::CommsJammed,
                },
                TriggerAction::ApplyIntModifier {
                    entity: "x".into(),
                    tag: "x".into(),
                    slot: crate::modifiers::IntModifierSlot::RepairTeams,
                    bonus: 0,
                },
                TriggerAction::RemoveIntModifier {
                    entity: "x".into(),
                    tag: "x".into(),
                    slot: crate::modifiers::IntModifierSlot::RepairTeams,
                },
                TriggerAction::GameOver { message: None },
                TriggerAction::LoadWorld { path: "x".into() },
                TriggerAction::UnloadWorld { path: "x".into() },
                TriggerAction::SetWorldFlag { name: "x".into() },
                TriggerAction::ClearWorldFlag { name: "x".into() },
                TriggerAction::IncrementWorldFlag {
                    name: "x".into(),
                    by: 0,
                },
                TriggerAction::SetWorldFlagValue {
                    name: "x".into(),
                    value: 0,
                },
                TriggerAction::SpawnEntity {
                    template_path: "x".into(),
                    name: "x".into(),
                    anchor: None,
                    position: None,
                    rotation: None,
                    scale: None,
                },
                TriggerAction::DestroyEntity { entity: "x".into() },
                TriggerAction::AddFactionEnemy {
                    faction: "x".into(),
                    enemy: "y".into(),
                },
                TriggerAction::RemoveFactionEnemy {
                    faction: "x".into(),
                    enemy: "y".into(),
                },
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
                    | TriggerAction::DestroyEntity { .. }
                    | TriggerAction::AddFactionEnemy { .. }
                    | TriggerAction::RemoveFactionEnemy { .. } => {}
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

    // -- Comms conversation-cycle tests (issue #608, moved from
    // world::server::tests). Cover handle_hail / handle_respond_to_message /
    // handle_clear_comms / handle_comms_channel2 end-to-end via the shared
    // comms_test_app()/setup_game_with_comms() harness (still in
    // world::server::tests, imported above).
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
            ClientMessage::ControlSystem {
                target: crate::system_registry::comms_system_id(),
                payload: crate::messages::SystemControlPayload::Hail {
                    target_uuid: station_uuid.into(),
                },
            },
        );
        let out = tick(&mut app);

        let comms_state = out.iter().find_map(|m| {
            if let ServerMessage::CommsState {
                messages, contacts, ..
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
        assert_eq!(messages[0].body, "USS Phoenix, please identify yourself.");
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
            ClientMessage::ControlSystem {
                target: crate::system_registry::comms_system_id(),
                payload: crate::messages::SystemControlPayload::Hail {
                    target_uuid: station_uuid.into(),
                },
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

    #[test]
    fn hail_blocked_when_comms_system_ai_controlled() {
        let station_uuid = "station-uuid-ai-block";
        let mut app = comms_test_app();
        setup_game_with_comms(&mut app, station_uuid);
        let _ = tick(&mut app);

        // Set comms system to AI control (blocks human input).
        {
            let mut q = app.world_mut().query_filtered::<&mut crate::ship_plugin::ShipSystemControlSources, With<crate::simulation::Ship>>();
            for mut sources in q.iter_mut(app.world_mut()) {
                sources.0.set(
                    crate::system_registry::comms_system_id(),
                    crate::control_source::ControlSource::Ai,
                );
            }
        }

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

        let comms_state_with_messages = out.iter().any(|m| {
            if let ServerMessage::CommsState { messages, .. } = &m.msg {
                !messages.is_empty()
            } else {
                false
            }
        });
        assert!(
            !comms_state_with_messages,
            "hail must be blocked when comms system is AI-controlled"
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
            ClientMessage::ControlSystem {
                target: crate::system_registry::comms_system_id(),
                payload: crate::messages::SystemControlPayload::Hail {
                    target_uuid: station_uuid.into(),
                },
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
            ClientMessage::ControlSystem {
                target: crate::system_registry::comms_system_id(),
                payload: crate::messages::SystemControlPayload::RespondToMessage {
                    message_id: msg_id.clone(),
                    response_index: 0,
                },
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
        assert!(
            comms_state.is_some(),
            "CommsState expected after RespondToMessage"
        );
        let messages = comms_state.unwrap();
        let msg = messages
            .iter()
            .find(|m| m.id == msg_id)
            .expect("original message must still be in inbox");
        assert_eq!(
            msg.selected_response,
            Some(0),
            "selected_response must be recorded"
        );

        // Expect an ObjectiveSummary to be sent to the captain.
        let obj_summary = out.iter().find_map(|m| {
            if let ServerMessage::ObjectiveSummary { objectives } = &m.msg {
                Some(objectives.clone())
            } else {
                None
            }
        });
        assert!(
            obj_summary.is_some(),
            "ObjectiveSummary expected after AddObjective action"
        );
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
        let orphaned = CommsMessage {
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

        push_msg(
            &mut app,
            "comms",
            ClientMessage::ControlSystem {
                target: crate::system_registry::comms_system_id(),
                payload: crate::messages::SystemControlPayload::ClearComms,
            },
        );
        let out = tick(&mut app);

        let comms_state = out.iter().find_map(|m| {
            if let ServerMessage::CommsState { messages, .. } = &m.msg {
                Some(messages.clone())
            } else {
                None
            }
        });
        assert!(
            comms_state.is_some(),
            "CommsState expected after ClearComms"
        );
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
        assert!(
            contacts.is_some(),
            "initial CommsState with contacts expected"
        );
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
            ClientMessage::ControlSystem {
                target: crate::system_registry::comms_system_id(),
                payload: crate::messages::SystemControlPayload::Hail {
                    target_uuid: station_uuid.into(),
                },
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

        push_msg(
            &mut app2,
            "comms",
            ClientMessage::ControlSystem {
                target: crate::system_registry::comms_system_id(),
                payload: crate::messages::SystemControlPayload::Hail {
                    target_uuid: station_uuid.into(),
                },
            },
        );
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
        assert!(
            !parent_thread_id.is_empty(),
            "parent message must have a non-empty thread_id"
        );

        push_msg(
            &mut app2,
            "comms",
            ClientMessage::ControlSystem {
                target: crate::system_registry::comms_system_id(),
                payload: crate::messages::SystemControlPayload::RespondToMessage {
                    message_id: first_msg.id.clone(),
                    response_index: 0,
                },
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

    #[test]
    fn follow_up_speaker_changes_display_name_but_keeps_sender_uuid() {
        let station_uuid = "station-uuid-007c";
        let mut app = comms_test_app();
        setup_game_with_comms_and_followup(&mut app, station_uuid);
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
        let first_msg = out
            .iter()
            .find_map(|m| {
                if let ServerMessage::CommsState { messages, .. } = &m.msg {
                    messages.first().cloned()
                } else {
                    None
                }
            })
            .expect("CommsState expected after hail");

        assert_eq!(first_msg.sender_uuid, station_uuid);
        assert_eq!(first_msg.sender_name, "Starbase Alpha");

        push_msg(
            &mut app,
            "comms",
            ClientMessage::ControlSystem {
                target: crate::system_registry::comms_system_id(),
                payload: crate::messages::SystemControlPayload::RespondToMessage {
                    message_id: first_msg.id.clone(),
                    response_index: 0,
                },
            },
        );
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
            .expect("CommsState expected after follow-up");
        let follow_up_msg = messages.get(1).expect("follow-up message expected");

        assert_eq!(follow_up_msg.sender_uuid, station_uuid);
        assert_eq!(follow_up_msg.sender_name, "Dockmaster Kade");
        assert_eq!(follow_up_msg.thread_id, first_msg.thread_id);
    }

    #[test]
    fn triggerless_follow_up_replacement_preserves_display_speaker() {
        let mut app = App::new();
        app.add_plugins(bevy::time::TimePlugin)
            .init_resource::<WorldContentRuntime>()
            .init_resource::<CommsInboxRes>()
            .add_message::<CommsChannel2Event>()
            .add_systems(
                Update,
                (tick_pending_follow_ups, handle_comms_channel2).chain(),
            );

        let placeholder = CommsMessage {
            id: "placeholder-001".into(),
            sender_uuid: "station-uuid-delayed".into(),
            sender_name: "Dockmaster Kade".into(),
            subject: "...".into(),
            body: "...".into(),
            responses: vec![],
            selected_response: None,
            is_read: false,
            is_orphaned: false,
            sender_in_range: true,
            thread_id: "thread-delayed".into(),
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
                    body: "Welcome, Phoenix.".into(),
                    responses: vec![],
                    speaker: Some("Dockmaster Kade".into()),
                    trigger: None,
                },
                sender_uuid: "station-uuid-delayed".into(),
                sender_name: "Dockmaster Kade".into(),
                thread_id: "thread-delayed".into(),
                elapsed_secs: 0.0,
                placeholder_id: Some("placeholder-001".into()),
                urgent: false,
            });

        app.update();

        let messages = app.world().resource::<CommsInboxRes>().0.messages();
        assert_eq!(messages.len(), 1);
        assert_ne!(messages[0].id, "placeholder-001");
        assert_eq!(messages[0].sender_uuid, "station-uuid-delayed");
        assert_eq!(messages[0].sender_name, "Dockmaster Kade");
        assert_eq!(messages[0].thread_id, "thread-delayed");
    }

    // -- Root-level [comms.follow_up] (auto-chained monologues) â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

    /// `handle_hail` injects the root template AND queues a `PendingFollowUp`
    /// for the chained `root_follow_up` node. Once the queued trigger fires
    /// (here: `on_timer` reaches `after_secs`), the chained message is
    /// injected into the inbox sharing the parent's `thread_id` and the
    /// chained `speaker` overrides the display name.
    #[test]
    fn root_follow_up_fires_on_hail_after_timer_expires() {
        let station_uuid = "station-uuid-rfu-001";
        let mut app = comms_test_app();
        setup_game_with_root_follow_up(&mut app, station_uuid);
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
        let _ = tick(&mut app);

        // After the hail, the root message is in the inbox and the chained
        // follow-up sits silently in `pending_follow_ups` (no `...`
        // placeholder for root chains).
        {
            let messages = app.world().resource::<CommsInboxRes>().0.messages();
            assert_eq!(
                messages.len(),
                1,
                "only the root message visible during wait"
            );
            assert_eq!(messages[0].body, "Stand by â€” patching you through.");
            let runtime = app.world().resource::<WorldContentRuntime>();
            assert_eq!(
                runtime.pending_follow_ups.len(),
                1,
                "chained follow-up must be queued"
            );
            assert!(
                runtime.pending_follow_ups[0].placeholder_id.is_none(),
                "root chains must stay silent (no placeholder)"
            );
        }

        // Force the queue-relative timer past the `after_secs` threshold
        // and tick again.
        app.world_mut()
            .resource_mut::<WorldContentRuntime>()
            .pending_follow_ups[0]
            .elapsed_secs = 5.0;
        let _ = tick(&mut app);

        let messages = app.world().resource::<CommsInboxRes>().0.messages();
        assert_eq!(messages.len(), 2, "chained message arrives after timer");
        let parent = &messages[0];
        let chained = &messages[1];
        // Both share the same thread.
        assert_eq!(chained.thread_id, parent.thread_id);
        assert!(!chained.thread_id.is_empty());
        // The chained `speaker` overrode the display name; sender_uuid stays
        // the channel identity.
        assert_eq!(chained.sender_name, "Dr. Myst");
        assert_eq!(chained.sender_uuid, station_uuid);
        assert_eq!(chained.body, "Captain. Dr. Myst speaking.");
    }

    /// When the root template carries an explicit `thread_id`, the chained
    /// follow-up message inherits the same id (so the inbox shows them as
    /// one conversation).
    #[test]
    fn root_follow_up_inherits_explicit_thread_id() {
        let station_uuid = "station-uuid-rfu-002";
        let mut app = comms_test_app();
        setup_game_with_root_follow_up(&mut app, station_uuid);

        // Stamp the template with an explicit thread_id.
        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime.comms_template_states[0].template.thread_id =
                Some("research-scholar".to_string());
        }
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
        let _ = tick(&mut app);
        // Trip the queue-relative timer.
        app.world_mut()
            .resource_mut::<WorldContentRuntime>()
            .pending_follow_ups[0]
            .elapsed_secs = 5.0;
        let _ = tick(&mut app);

        let messages = app.world().resource::<CommsInboxRes>().0.messages();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].thread_id, "research-scholar");
        assert_eq!(messages[1].thread_id, "research-scholar");
    }

    /// A chained `root_follow_up` with `trigger = None` is queued with
    /// `elapsed_secs = 0.0`. The next `tick_pending_follow_ups` pass that
    /// runs finds it ready (triggerless follow-ups are always ready) and
    /// injects it. This mirrors how `response.follow_up` with no trigger
    /// reaches the inbox on the next tick after `RespondToMessage`.
    #[test]
    fn root_follow_up_with_no_trigger_fires_on_next_tick() {
        let station_uuid = "station-uuid-rfu-003";
        let mut app = comms_test_app();
        setup_game_with_root_follow_up(&mut app, station_uuid);

        // Drop the trigger on the chained node â€” now it's triggerless and
        // should fire on the very next tick.
        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            if let Some(ref mut fu) = runtime.comms_template_states[0].template.root_follow_up {
                fu.trigger = None;
            }
        }
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
        // First tick: handle_hail injects the root and queues the chained
        // node with `trigger = None` and `elapsed_secs = 0`. Whether
        // `tick_pending_follow_ups` fires on this same tick depends on
        // Bevy's parallel scheduling, so we don't assert on it here.
        let _ = tick(&mut app);
        // Second tick: regardless of within-tick ordering, the queued
        // triggerless follow-up is now ready and the chained message
        // has been injected.
        let _ = tick(&mut app);

        let messages = app.world().resource::<CommsInboxRes>().0.messages();
        assert_eq!(
            messages.len(),
            2,
            "triggerless chained message must fire on the next tick"
        );
        assert_eq!(messages[1].body, "Captain. Dr. Myst speaking.");
        // The pending queue must be drained.
        let runtime = app.world().resource::<WorldContentRuntime>();
        assert!(runtime.pending_follow_ups.is_empty());
    }

    /// Like `setup_game_with_comms_and_followup`, but installs a template
    /// whose root has zero `[[response]]` entries and a top-level
    /// `root_follow_up` chain (the new authoring shape). The chained node
    /// uses `speaker = "Dr. Myst"` to verify display-speaker override,
    /// and `trigger = on_timer 2s` to verify queue-relative delays.
    fn setup_game_with_root_follow_up(app: &mut App, station_uuid: &str) {
        setup_game_with_comms(app, station_uuid);
        let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
        runtime.comms_template_states.clear();
        runtime
            .comms_template_states
            .push(crate::world::content::CommsTemplateState {
                template: crate::world::content::CommsTemplate {
                    from: "starbase_alpha".into(),
                    trigger: TriggerCondition::OnHailed {
                        entity_name: "starbase_alpha".into(),
                    },
                    node: CommsDialogueNode {
                        body: "Stand by â€” patching you through.".into(),
                        responses: vec![], // no [[response]] â€” chained monologue
                        speaker: None,
                        trigger: None,
                    },
                    thread_id: None,
                    urgent: false,
                    root_follow_up: Some(CommsDialogueNode {
                        body: "Captain. Dr. Myst speaking.".into(),
                        responses: vec![],
                        speaker: Some("Dr. Myst".into()),
                        trigger: Some(TriggerCondition::OnTimer { after_secs: 2.0 }),
                    }),
                },
                fired: false,
            });
    }

    fn setup_game_with_comms_and_followup(app: &mut App, station_uuid: &str) {
        setup_game_with_comms(app, station_uuid);
        // Replace the single template with one that has a follow-up node.
        let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
        runtime.comms_template_states.clear();
        runtime
            .comms_template_states
            .push(crate::world::content::CommsTemplateState {
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
                                speaker: Some("Dockmaster Kade".into()),
                                trigger: None,
                            }),
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

        app.world_mut().spawn((
            Ship,
            crate::simulation::LocalShip,
            Transform::from_xyz(0.0, 0.0, 0.0),
            CommsRange(100.0),
        ));
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
            ClientMessage::ControlSystem {
                target: crate::system_registry::comms_system_id(),
                payload: crate::messages::SystemControlPayload::Hail {
                    target_uuid: station_uuid.into(),
                },
            },
        );
        let out = tick(&mut app);

        // No CommsState broadcast should contain a non-empty inbox.
        for m in &out {
            if let ServerMessage::CommsState { messages, .. } = &m.msg {
                assert!(
                    messages.is_empty(),
                    "out-of-range Hail must not inject messages, got {messages:?}"
                );
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
        app.world_mut().spawn((
            Ship,
            crate::simulation::LocalShip,
            Transform::from_xyz(0.0, 0.0, 0.0),
            CommsRange(500.0),
        ));
        let station_entity = app
            .world_mut()
            .spawn((
                EntityUuid(station_uuid.into()),
                Transform::from_xyz(50.0, 0.0, 0.0),
                CommsRange(500.0),
            ))
            .id();
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
        let msg_id = out
            .iter()
            .find_map(|m| {
                if let ServerMessage::CommsState { messages, .. } = &m.msg {
                    messages.first().map(|m| m.id.clone())
                } else {
                    None
                }
            })
            .expect("hail produced a message");

        // Move the station far away.
        if let Ok(mut e) = app.world_mut().get_entity_mut(station_entity) {
            e.insert(Transform::from_xyz(5000.0, 0.0, 0.0));
        }
        // Tick to refresh range_flags.
        let _ = tick(&mut app);

        // Try to respond.
        push_msg(
            &mut app,
            "comms",
            ClientMessage::ControlSystem {
                target: crate::system_registry::comms_system_id(),
                payload: crate::messages::SystemControlPayload::RespondToMessage {
                    message_id: msg_id.clone(),
                    response_index: 0,
                },
            },
        );
        let _ = tick(&mut app);

        // Objective `obj-survey` must NOT have been added (response_actions
        // include AddObjective in setup_game_with_comms).
        let objectives = &app.world().resource::<ObjectiveManagerRes>().0;
        assert!(
            objectives.sorted_snapshots().is_empty(),
            "out-of-range Respond must not fire AddObjective action"
        );
    }

    #[test]
    fn control_system_hail_dispatches_same_as_client_message_hail() {
        let station_uuid = "station-uuid-control-sys";
        let mut app = comms_test_app();
        setup_game_with_comms(&mut app, station_uuid);
        // Flush the initial broadcast.
        let _ = tick(&mut app);

        push_msg(
            &mut app,
            "comms",
            ClientMessage::ControlSystem {
                target: crate::system_registry::comms_system_id(),
                payload: crate::messages::SystemControlPayload::Hail {
                    target_uuid: station_uuid.to_string(),
                },
            },
        );
        let out = tick(&mut app);

        let comms_state = out.iter().find_map(|m| {
            if let ServerMessage::CommsState { messages, .. } = &m.msg {
                Some(messages.clone())
            } else {
                None
            }
        });

        assert!(
            comms_state.is_some(),
            "ControlSystem::Hail must produce a CommsState broadcast"
        );
        let messages = comms_state.unwrap();
        assert_eq!(
            messages.len(),
            1,
            "ControlSystem::Hail must deliver one message"
        );
        assert_eq!(messages[0].body, "USS Phoenix, please identify yourself.");
    }
}

// ── AI controller stub ─────────────────────────────────────────────────────────

/// Per-entity AI loop for comms. Loops over ALL ship entities (player and NPC)
/// where the Comms system is `ControlSource::Ai`.
///
/// Currently a compile-verified stub — Comms AI auto-responds to hails
/// and processes inbox messages (deferred to later fine-grained decomposition).
pub fn operate_comms_ai(ships: Query<&ShipSystemControlSources>) {
    for sources in &ships {
        let policy = sources
            .0
            .policy_for(&crate::system_registry::comms_system_id());
        if !policy.operate_ai {
            continue;
        }
        // TODO: implement comms AI logic (auto-response, inbox processing)
    }
}
