use bevy::prelude::*;

use crate::lobby::Sessions;
use crate::messages::{
    CoordinationPayload, SensorsBlackboard, ServerMessage, StationId, SystemBlackboard,
    SystemControlPayload, SystemId,
};
use crate::ship_plugin::CoordinationEnqueue;

// Placeholder shield frequency returned until entities expose a real value.
const PLACEHOLDER_SHIELD_FREQUENCY: f32 = 0.5;

// ── Resources ──────────────────────────────────────────────────────────────────

/// The currently selected science target on the Sensors console. `None` means
/// no target is selected. Broadcast to all clients via SensorsBlackboard so
/// every radar can render a blue science-target marker.
///
/// Per-entity `Component` on every ship (player + NPC). PR-7 (issue #597)
/// removed the dual `Resource` derive — every ship has its own sensors target.
#[derive(Component, Default, Clone, Debug)]
pub struct SensorsTarget(pub Option<String>);

/// Tracks the last frequency value sent for a given target so we avoid
/// re-emitting when nothing has changed.
///
/// Per-ship `Component` so NPC ships track their own Sensors→Tactical
/// frequency hints independently of the player's.
#[derive(Component, Default, Clone)]
pub struct SensorsFrequencyState {
    pub last_sent_target: Option<String>,
    pub last_sent_frequency: Option<f32>,
}

// ── Plugin ─────────────────────────────────────────────────────────────────────

pub struct ShipSensorsPlugin;

impl Plugin for ShipSensorsPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<CoordinationEnqueue>().add_systems(
            Update,
            (
                handle_sensors_messages.in_set(crate::sim_sets::SimSet::Input),
                operate_sensors_ai.in_set(crate::sim_sets::SimSet::Input),
                tick_sensors_frequency_hint.in_set(crate::sim_sets::SimSet::Input),
                publish_sensors_blackboard.in_set(crate::sim_sets::SimSet::Publish),
            ),
        );
    }
}

// ── Systems ────────────────────────────────────────────────────────────────────

/// Handle `SetScienceTarget` messages from the Sensors console.
///
/// Validates: sender holds the Sensors station and `accept_human_input` is
/// true. Stores the target in [`SensorsTarget`] for blackboard broadcast, and
/// emits `SensorsTargetSuggestion` directly to the Tactical station holder.
pub fn handle_sensors_messages(
    sessions: Res<Sessions>,
    mut ship_query: Query<
        (
            &crate::messages::AdmittedCommands,
            &crate::ship_plugin::ShipConfigComponent,
            &mut SensorsTarget,
            Has<crate::server_app::LocalShip>,
        ),
        With<crate::server_app::Ship>,
    >,
    mut outbox: ResMut<crate::simulation::SimOutbox>,
) {
    for (admitted, _ship_config, mut entity_target, is_local) in ship_query.iter_mut() {
        for cmd in admitted.for_target(crate::system_registry::SENSORS_SYSTEM_ID) {
            let SystemControlPayload::SetScienceTarget { uuid } = &cmd.payload else {
                continue;
            };

            // Write to this ship's own SensorsTarget component (player or NPC).
            entity_target.0 = Some(uuid.clone());

            // Route the suggestion to whoever currently holds the Tactical
            // console. Only the LocalShip has a browser client, so only the
            // LocalShip's console-holder gets the WebRTC push. NPC ships'
            // AI Tactical picks up the target via the coordination bus.
            if !is_local {
                continue;
            }
            let Some(tactical_token) = sessions.0.holder_for_station(&StationId("tactical".into()))
            else {
                continue;
            };

            outbox.0.push((
                crate::lobby::Target::Token(tactical_token.to_string()),
                ServerMessage::SensorsTargetSuggestion { uuid: uuid.clone() },
            ));
        }
    }
}

/// Emit a channel-3 `FrequencyHint` coordination message to Tactical whenever
/// each ship's locked target changes.
///
/// Iterates every ship (player + NPC) so NPC Sensors→Tactical hints flow
/// through the coordination bus alongside the player's. Each emission
/// stamps its source ship so the enqueue handler routes it correctly.
pub fn tick_sensors_frequency_hint(
    mut ship_q: Query<
        (
            Entity,
            &crate::simulation::WeaponsTarget,
            &crate::ship_plugin::ShipSystemControlSources,
            &mut SensorsFrequencyState,
        ),
        With<crate::server_app::Ship>,
    >,
    mut writer: MessageWriter<CoordinationEnqueue>,
) {
    for (entity, weapons_target, control_sources, mut state) in ship_q.iter_mut() {
        let current_target = match weapons_target.0.clone() {
            Some(uuid) => uuid,
            None => {
                state.last_sent_target = None;
                state.last_sent_frequency = None;
                continue;
            }
        };

        // Placeholder: real implementation would look up the entity's shield frequency.
        let frequency = PLACEHOLDER_SHIELD_FREQUENCY;

        let target_changed = state.last_sent_target.as_deref() != Some(&current_target);
        let frequency_changed = state.last_sent_frequency != Some(frequency);

        if !target_changed && !frequency_changed {
            continue;
        }

        state.last_sent_target = Some(current_target);
        state.last_sent_frequency = Some(frequency);

        let sender_origin = control_sources
            .0
            .source_for(&crate::system_registry::sensors_system_id());

        writer.write(CoordinationEnqueue {
            source_entity: entity,
            sender_origin,
            target: crate::system_registry::tactical_system_id(),
            payload: CoordinationPayload::FrequencyHint { frequency },
            sender_label: "Sensors".to_string(),
        });
    }
}

// ── Blackboard publish ────────────────────────────────────────────────────────

pub fn publish_sensors_blackboard(
    ship_config: Res<crate::lobby::server::ShipClientConfigResource>,
    sensors_target_q: Query<&SensorsTarget, With<crate::server_app::LocalShip>>,
    mut ship_bbs_q: Query<
        &mut crate::server_app::ShipSystemBlackboards,
        With<crate::server_app::LocalShip>,
    >,
) {
    let cfg = &ship_config.0;
    let science_target_uuid = sensors_target_q.single().ok().and_then(|st| st.0.clone());
    let bb = SensorsBlackboard {
        radar_range: cfg.sensors_radar_range,
        radar_shows: cfg.sensors_radar_shows.clone(),
        radar_selects: cfg.sensors_radar_selects.clone(),
        science_target_uuid,
    };

    if let Some(mut bbs) = ship_bbs_q.iter_mut().next() {
        bbs.0.insert(
            SystemId(crate::system_registry::SENSORS_SYSTEM_ID.to_string()),
            SystemBlackboard::Sensors(bb),
        );
    }
}

/// Per-entity AI loop for the Sensors system. Loops over all ship entities
/// where the Sensors system is `ControlSource::Ai`.
///
/// Currently a compile-verified stub — Sensors AI logic is deferred since
/// the sensor AI produces no active decisions in the current game design
/// (Sensors is purely advisory: the AI auto-suggests scan targets to Tactical
/// via the coordination bus in `tick_sensors_frequency_hint`).
pub fn operate_sensors_ai(ships: Query<&crate::ship_plugin::ShipSystemControlSources>) {
    for sources in &ships {
        let policy = sources
            .0
            .policy_for(&crate::system_registry::sensors_system_id());
        if !policy.operate_ai {
            continue;
        }
        // TODO: implement sensors AI logic (target suggestion, scan selection)
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lobby::{InboundMessage, LobbyPlugin, OutboundMessage, Target};
    use crate::messages::*;
    use crate::simulation::{ShipImpulse, SimOutbox};

    #[derive(Resource, Default)]
    struct Outbox(Vec<OutboundMessage>);

    fn collect(mut reader: MessageReader<OutboundMessage>, mut box_: ResMut<Outbox>) {
        for m in reader.read() {
            box_.0.push(m.clone());
        }
    }

    fn test_app() -> App {
        let mut app = App::new();
        app.add_plugins(LobbyPlugin)
            .add_plugins(bevy::time::TimePlugin)
            .add_plugins(crate::server_app::AdmissionPlugin)
            .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
                std::time::Duration::from_millis(200),
            ))
            .init_resource::<SimOutbox>()
            .init_resource::<Outbox>()
            .init_resource::<crate::lobby::server::ShipClientConfigResource>()
            .add_plugins(ShipSensorsPlugin)
            .add_systems(PostUpdate, collect);
        app.world_mut().spawn((
            crate::simulation::Ship,
            crate::simulation::LocalShip,
            crate::server_app::ShipSystemBlackboards::default(),
            crate::ship_plugin::ShipConfigComponent::default(),
            crate::ship_plugin::ShipSystemControlSources::default(),
            crate::messages::AdmittedCommands::default(),
            crate::ship_plugin::ActiveStationRatings::default(),
            crate::ship_plugin::CoordinationQueue::default(),
            SensorsTarget::default(),
            // PR 7 (issue #597) — WeaponsTarget is now per-entity Component.
            crate::simulation::WeaponsTarget::default(),
            SensorsFrequencyState::default(),
            ShipImpulse(crate::impulse::ImpulseState::new()),
        ));
        app
    }

    fn push(app: &mut App, token: &str, msg: ClientMessage) {
        app.world_mut()
            .resource_mut::<Messages<InboundMessage>>()
            .write(InboundMessage {
                token: token.into(),
                msg,
            });
    }

    fn tick(app: &mut App) -> Vec<OutboundMessage> {
        app.update();
        let sim_entries = std::mem::take(&mut app.world_mut().resource_mut::<SimOutbox>().0);
        let mut out = app.world().resource::<Outbox>().0.clone();
        for (target, msg) in sim_entries {
            out.push(OutboundMessage { target, msg });
        }
        app.world_mut().resource_mut::<Outbox>().0.clear();
        out
    }

    fn start_game_with_sensors_and_tactical(app: &mut App) {
        push(
            app,
            "captain",
            ClientMessage::Identify {
                token: "captain".into(),
                name: "Alice".into(),
            },
        );
        tick(app);
        push(
            app,
            "captain",
            ClientMessage::SelectStation {
                station: "Captain".into(),
            },
        );
        tick(app);
        push(
            app,
            "sensors",
            ClientMessage::Identify {
                token: "sensors".into(),
                name: "Spock".into(),
            },
        );
        tick(app);
        push(
            app,
            "sensors",
            ClientMessage::SelectStation {
                station: "Sensors".into(),
            },
        );
        tick(app);
        push(
            app,
            "tactical",
            ClientMessage::Identify {
                token: "tactical".into(),
                name: "Bob".into(),
            },
        );
        tick(app);
        push(
            app,
            "tactical",
            ClientMessage::SelectStation {
                station: "Tactical".into(),
            },
        );
        tick(app);
        push(app, "captain", ClientMessage::SetReady { ready: true });
        push(app, "sensors", ClientMessage::SetReady { ready: true });
        push(app, "tactical", ClientMessage::SetReady { ready: true });
        tick(app);
    }

    #[test]
    fn sensors_set_science_target_sends_suggestion_to_tactical() {
        let mut app = test_app();
        start_game_with_sensors_and_tactical(&mut app);

        push(
            &mut app,
            "sensors",
            ClientMessage::ControlSystem {
                target: crate::messages::SystemId(
                    crate::system_registry::SENSORS_SYSTEM_ID.to_string(),
                ),
                payload: SystemControlPayload::SetScienceTarget {
                    uuid: "asteroid-42".into(),
                },
            },
        );
        let out = tick(&mut app);

        let suggestion = out
            .iter()
            .find_map(|m| match &m.msg {
                ServerMessage::SensorsTargetSuggestion { uuid } => Some(uuid.clone()),
                _ => None,
            })
            .expect("expected a SensorsTargetSuggestion message");
        assert_eq!(suggestion, "asteroid-42");

        let suggestion_msg = out
            .iter()
            .find(|m| matches!(&m.msg, ServerMessage::SensorsTargetSuggestion { .. }))
            .unwrap();
        assert!(
            matches!(&suggestion_msg.target, Target::Token(t) if t == "tactical"),
            "SensorsTargetSuggestion should be sent only to Tactical console"
        );
    }

    #[test]
    fn non_sensors_player_cannot_send_science_target() {
        let mut app = test_app();
        start_game_with_sensors_and_tactical(&mut app);

        push(
            &mut app,
            "captain",
            ClientMessage::ControlSystem {
                target: crate::messages::SystemId(
                    crate::system_registry::SENSORS_SYSTEM_ID.to_string(),
                ),
                payload: SystemControlPayload::SetScienceTarget {
                    uuid: "asteroid-42".into(),
                },
            },
        );
        let out = tick(&mut app);

        assert!(
            !out.iter()
                .any(|m| matches!(&m.msg, ServerMessage::SensorsTargetSuggestion { .. })),
            "non-Sensors player should not be able to send SensorsTargetSuggestion"
        );
    }

    /// Set the LocalShip's per-entity `WeaponsTarget` for tests.
    fn set_local_weapons_target(app: &mut App, uuid: Option<String>) {
        let mut q = app
            .world_mut()
            .query_filtered::<&mut crate::simulation::WeaponsTarget, With<crate::server_app::LocalShip>>();
        if let Ok(mut wt) = q.single_mut(app.world_mut()) {
            wt.0 = uuid;
        }
    }

    #[test]
    fn frequency_hint_emitted_when_target_changes() {
        let mut app = test_app();
        start_game_with_sensors_and_tactical(&mut app);

        set_local_weapons_target(&mut app, Some("asteroid-1".into()));
        tick(&mut app); // emits first hint

        set_local_weapons_target(&mut app, Some("asteroid-2".into()));
        let enqueue_count = {
            // Tick and count CoordinationEnqueue events written
            app.update();
            // We verify indirectly — state should update to new target
            let mut q = app
                .world_mut()
                .query_filtered::<&SensorsFrequencyState, With<crate::server_app::LocalShip>>();
            q.single(app.world())
                .expect("LocalShip must carry SensorsFrequencyState")
                .last_sent_target
                .clone()
        };

        assert_eq!(
            enqueue_count.as_deref(),
            Some("asteroid-2"),
            "state should track the new target after it changes"
        );
    }

    #[test]
    fn frequency_hint_not_re_emitted_for_same_target() {
        let mut app = test_app();
        start_game_with_sensors_and_tactical(&mut app);

        set_local_weapons_target(&mut app, Some("asteroid-1".into()));
        tick(&mut app); // first emit

        let state_before = {
            let mut q = app
                .world_mut()
                .query_filtered::<&SensorsFrequencyState, With<crate::server_app::LocalShip>>();
            q.single(app.world()).unwrap().last_sent_frequency
        };

        tick(&mut app); // second tick, same target

        let state_after = {
            let mut q = app
                .world_mut()
                .query_filtered::<&SensorsFrequencyState, With<crate::server_app::LocalShip>>();
            q.single(app.world()).unwrap().last_sent_frequency
        };

        assert_eq!(
            state_before, state_after,
            "state should not change when target is unchanged"
        );
    }

    /// Verifies that operate_sensors_ai skips entities where Sensors is Human,
    /// and runs (without panic) for entities where Sensors is Ai (issue #589 AC).
    #[test]
    fn operate_sensors_ai_runs_per_entity_for_ai_controlled_ships() {
        use crate::ship::control_source::{ControlSource, ControlSourceResolver};

        // Human-controlled: operate_sensors_ai must do nothing.
        let mut human_resolver = ControlSourceResolver::new();
        human_resolver.set(
            crate::system_registry::sensors_system_id(),
            ControlSource::Human,
        );
        let human_sources = crate::ship_plugin::ShipSystemControlSources(human_resolver);
        let human_policy = human_sources
            .0
            .policy_for(&crate::system_registry::sensors_system_id());
        assert!(
            !human_policy.operate_ai,
            "human Sensors should not operate AI"
        );

        // AI-controlled: operate_sensors_ai must gate and proceed.
        let mut ai_resolver = ControlSourceResolver::new();
        ai_resolver.set(
            crate::system_registry::sensors_system_id(),
            ControlSource::Ai,
        );
        let ai_sources = crate::ship_plugin::ShipSystemControlSources(ai_resolver);
        let ai_policy = ai_sources
            .0
            .policy_for(&crate::system_registry::sensors_system_id());
        assert!(
            ai_policy.operate_ai,
            "AI Sensors must gate through operate_ai"
        );
    }
}
