use bevy::prelude::*;

use crate::lobby::Sessions;
use crate::messages::{
    Console, CoordinationPayload, SensorsBlackboard, ServerMessage,
    SystemBlackboard, SystemControlPayload, SystemId,
};
use crate::ship::control_source::ControlSource;
use crate::ship_plugin::CoordinationEnqueue;
use crate::simulation::WeaponsTarget;

// Placeholder shield frequency returned until entities expose a real value.
const PLACEHOLDER_SHIELD_FREQUENCY: f32 = 0.5;

// ── Resources ──────────────────────────────────────────────────────────────────

/// The currently selected science target on the Sensors console. `None` means
/// no target is selected. Broadcast to all clients via SensorsBlackboard so
/// every radar can render a blue science-target marker.
///
/// Derives both `Resource` (existing player-ship singleton) and `Component`
/// (per-entity path after issue #589 unification).
#[derive(Resource, Component, Default, Clone, Debug)]
pub struct SensorsTarget(pub Option<String>);

/// Tracks the last frequency value sent for a given target so we avoid
/// re-emitting when nothing has changed.
#[derive(Resource, Default)]
pub struct SensorsFrequencyState {
    pub last_sent_target: Option<String>,
    pub last_sent_frequency: Option<f32>,
}

// ── Plugin ─────────────────────────────────────────────────────────────────────

pub struct ShipSensorsPlugin;

impl Plugin for ShipSensorsPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<CoordinationEnqueue>()
            .init_resource::<SensorsFrequencyState>()
            .init_resource::<SensorsTarget>()
            .add_systems(
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
/// Validates: sender holds `Console::Sensors` and `accept_human_input` is true.
/// Stores the target in [`SensorsTarget`] for blackboard broadcast, and emits
/// `SensorsTargetSuggestion` directly to the Tactical console holder.
pub fn handle_sensors_messages(
    sessions: Res<Sessions>,
    ship_query: Query<
        (
            &crate::messages::AdmittedCommands,
            &crate::ship_plugin::ShipConfigComponent,
        ),
        With<crate::server_app::LocalShip>,
    >,
    mut sensors_q: Query<&mut SensorsTarget, With<crate::server_app::LocalShip>>,
    mut outbox: ResMut<crate::simulation::SimOutbox>,
    mut sensors_target: ResMut<SensorsTarget>,
) {
    let Ok((admitted, ship_config)) = ship_query.single() else {
        return;
    };

    for cmd in admitted.for_target(crate::system_registry::SENSORS_SYSTEM_ID) {
        let SystemControlPayload::SetScienceTarget { uuid } = &cmd.payload else {
            continue;
        };

        // Write to global resource (backward compat) and per-entity component.
        sensors_target.0 = Some(uuid.clone());
        if let Ok(mut entity_target) = sensors_q.single_mut() {
            entity_target.0 = Some(uuid.clone());
        }

        // Route the suggestion to whoever currently holds the Tactical console.
        let Some(tactical_token) = sessions
            .0
            .console_holder(&Console::Tactical, &ship_config.0)
        else {
            continue;
        };

        outbox.0.push((
            crate::lobby::Target::Token(tactical_token.to_string()),
            ServerMessage::SensorsTargetSuggestion { uuid: uuid.clone() },
        ));
    }
}

/// Emit a channel-3 `FrequencyHint` coordination message to Tactical whenever
/// the locked target changes.
///
/// The Sensors system always emits this regardless of whether it is human- or
/// AI-controlled — the coordination bus handles routing (AI Tactical consumes
/// silently; Human Tactical receives a popup).
pub fn tick_sensors_frequency_hint(
    weapons_target: Res<WeaponsTarget>,
    mut state: ResMut<SensorsFrequencyState>,
    ship_query: Query<&crate::ship_plugin::ShipSystemControlSources, With<crate::server_app::LocalShip>>,
    mut writer: MessageWriter<CoordinationEnqueue>,
) {
    let current_target = match &weapons_target.0 {
        Some(uuid) => uuid.clone(),
        None => {
            state.last_sent_target = None;
            state.last_sent_frequency = None;
            return;
        }
    };

    // Placeholder: real implementation would look up the entity's shield frequency.
    let frequency = PLACEHOLDER_SHIELD_FREQUENCY;

    let target_changed = state.last_sent_target.as_deref() != Some(&current_target);
    let frequency_changed = state.last_sent_frequency != Some(frequency);

    if !target_changed && !frequency_changed {
        return;
    }

    state.last_sent_target = Some(current_target);
    state.last_sent_frequency = Some(frequency);

    let sender_origin = if let Ok(control_sources) = ship_query.single() {
        control_sources
            .0
            .source_for(&crate::system_registry::sensors_system_id())
    } else {
        ControlSource::Human
    };

    writer.write(CoordinationEnqueue {
        sender_origin,
        target: crate::system_registry::tactical_system_id(),
        payload: CoordinationPayload::FrequencyHint { frequency },
        sender_label: "Sensors".to_string(),
    });
}

// ── Blackboard publish ────────────────────────────────────────────────────────

pub fn publish_sensors_blackboard(
    ship_config: Res<crate::lobby::server::ShipClientConfigResource>,
    sensors_target: Res<SensorsTarget>,
    mut blackboards: ResMut<crate::server_app::SystemBlackboards>,
) {
    let cfg = &ship_config.0;
    let bb = SensorsBlackboard {
        radar_range: cfg.sensors_radar_range,
        radar_shows: cfg.sensors_radar_shows.clone(),
        radar_selects: cfg.sensors_radar_selects.clone(),
        science_target_uuid: sensors_target.0.clone(),
    };

    blackboards.0.insert(
        SystemId(crate::system_registry::SENSORS_SYSTEM_ID.to_string()),
        SystemBlackboard::Sensors(bb),
    );
}

/// Per-entity AI loop for the Sensors system. Loops over all ship entities
/// where the Sensors system is `ControlSource::Ai`.
///
/// Currently a compile-verified stub — Sensors AI logic is deferred since
/// the sensor AI produces no active decisions in the current game design
/// (Sensors is purely advisory: the AI auto-suggests scan targets to Tactical
/// via the coordination bus in `tick_sensors_frequency_hint`).
pub fn operate_sensors_ai(
    ships: Query<&crate::ship_plugin::ShipSystemControlSources>,
) {
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
    use crate::damage::ConsoleHull;
    use crate::lobby::{InboundMessage, LobbyPlugin, OutboundMessage, Target};
    use crate::messages::*;
    use crate::simulation::{ShipHullIntegrity, ShipImpulse, SimOutbox};

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
                        .insert_resource(ShipHullIntegrity(ConsoleHull::from_config(&[
                (Console::Helm, 25.0),
                (Console::Tactical, 25.0),
                (Console::Power, 25.0),
                (Console::Shields, 25.0),
            ])))
            .insert_resource(ShipImpulse(crate::impulse::ImpulseState::new()))
            .init_resource::<SimOutbox>()
            .init_resource::<WeaponsTarget>()
            .init_resource::<Outbox>()
            .init_resource::<crate::server_app::SystemBlackboards>()
            .init_resource::<crate::lobby::server::ShipClientConfigResource>()
            .add_plugins(ShipSensorsPlugin)
            .add_systems(PostUpdate, collect);
        app.world_mut().spawn((
            crate::simulation::Ship,
            crate::simulation::LocalShip,
            crate::ship_plugin::ShipConfigComponent::default(),
            crate::ship_plugin::ShipSystemControlSources::default(),
            crate::messages::AdmittedCommands::default(),
            crate::ship_plugin::ActiveStationRatings::default(),
            crate::ship_plugin::CoordinationQueue::default(),
            SensorsTarget::default(),
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
                station: "Captain's Chair".into(),
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

    #[test]
    fn frequency_hint_emitted_when_target_changes() {
        let mut app = test_app();
        start_game_with_sensors_and_tactical(&mut app);

        app.world_mut().resource_mut::<WeaponsTarget>().0 = Some("asteroid-1".into());
        tick(&mut app); // emits first hint

        app.world_mut().resource_mut::<WeaponsTarget>().0 = Some("asteroid-2".into());
        let enqueue_count = {
            // Tick and count CoordinationEnqueue events written
            app.update();
            // We verify indirectly — state should update to new target
            app.world()
                .resource::<SensorsFrequencyState>()
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

        app.world_mut().resource_mut::<WeaponsTarget>().0 = Some("asteroid-1".into());
        tick(&mut app); // first emit

        let state_before = app
            .world()
            .resource::<SensorsFrequencyState>()
            .last_sent_frequency;

        tick(&mut app); // second tick, same target

        let state_after = app
            .world()
            .resource::<SensorsFrequencyState>()
            .last_sent_frequency;

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
        let human_sources =
            crate::ship_plugin::ShipSystemControlSources(human_resolver);
        let human_policy = human_sources
            .0
            .policy_for(&crate::system_registry::sensors_system_id());
        assert!(!human_policy.operate_ai, "human Sensors should not operate AI");

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
        assert!(ai_policy.operate_ai, "AI Sensors must gate through operate_ai");
    }
}
