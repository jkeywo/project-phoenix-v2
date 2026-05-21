use bevy::prelude::*;

use crate::lobby::{InboundMessage, Sessions};
use crate::messages::{ClientMessage, Console, ServerMessage};
use crate::simulation::SimOutbox;
use crate::lobby::Target;

// ── Plugin ─────────────────────────────────────────────────────────────────────

pub struct SciencePlugin;

impl Plugin for SciencePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, handle_set_science_target.in_set(crate::sim_sets::SimSet::Input));
    }
}

// ── Systems ───────────────────────────────────────────────────────────────────

pub fn handle_set_science_target(
    mut reader: MessageReader<InboundMessage>,
    sessions: Res<Sessions>,
    mut outbox: ResMut<SimOutbox>,
) {
    for ev in reader.read() {
        let ClientMessage::SetScienceTarget { uuid } = &ev.msg else { continue };

        // Only the Sensors console holder may broadcast a target suggestion.
        if sessions.0.console_holder(Console::Sensors) != Some(ev.token.as_str()) {
            continue;
        }

        // Only broadcast if there is a Weapons console player to receive it.
        let Some(weapons_token) = sessions.0.console_holder(Console::Tactical) else {
            continue;
        };

        outbox.0.push((Target::Token(weapons_token.to_string()), ServerMessage::ScienceTargetSuggestion { uuid: uuid.clone() }));
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::damage::ConsoleHull;
    use crate::lobby::{LobbyPlugin, OutboundMessage};
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
            .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
                std::time::Duration::from_millis(200),
            ))
            .insert_resource(crate::ship_state::ShipState::new())
            .insert_resource(ShipHullIntegrity(ConsoleHull::from_config(&[
                (Console::Helm, 25.0),
                (Console::Tactical, 25.0),
                (Console::Power, 25.0),
                (Console::Shields, 25.0),
            ])))
            .insert_resource(ShipImpulse(crate::impulse::ImpulseState::new()))
            .init_resource::<SimOutbox>()
            .init_resource::<Outbox>()
            .add_plugins(SciencePlugin)
            .add_systems(PostUpdate, collect);
        app
    }

    fn push(app: &mut App, token: &str, msg: ClientMessage) {
        app.world_mut()
            .resource_mut::<Messages<InboundMessage>>()
            .write(InboundMessage { token: token.into(), msg });
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

    fn start_game_with_sensors_and_weapons(app: &mut App) {
        push(app, "captain", ClientMessage::Identify { token: "captain".into(), name: "Alice".into() });
        tick(app);
        push(app, "captain", ClientMessage::SelectStation { station: "Captain's Chair".into() });
        tick(app);
        push(app, "sensors", ClientMessage::Identify { token: "sensors".into(), name: "Spock".into() });
        tick(app);
        push(app, "sensors", ClientMessage::SelectStation { station: "Sensors".into() });
        tick(app);
        push(app, "weapons", ClientMessage::Identify { token: "weapons".into(), name: "Bob".into() });
        tick(app);
        push(app, "weapons", ClientMessage::SelectStation { station: "Tactical".into() });
        tick(app);
        push(app, "captain", ClientMessage::StartGame);
        tick(app);
    }

    // ── SetScienceTarget / ScienceTargetSuggestion tests ─────────────────────

    #[test]
    fn sensors_set_science_target_broadcasts_suggestion_to_weapons() {
        let mut app = test_app();
        start_game_with_sensors_and_weapons(&mut app);

        push(&mut app, "sensors", ClientMessage::SetScienceTarget { uuid: "asteroid-42".into() });
        let out = tick(&mut app);

        let suggestion = out.iter().find_map(|m| match &m.msg {
            ServerMessage::ScienceTargetSuggestion { uuid } => Some(uuid.clone()),
            _ => None,
        }).expect("expected a ScienceTargetSuggestion message");
        assert_eq!(suggestion, "asteroid-42");

        // Should be targeted to Weapons console player only.
        let suggestion_msg = out.iter().find(|m| matches!(&m.msg, ServerMessage::ScienceTargetSuggestion { .. }))
            .unwrap();
        assert!(
            matches!(&suggestion_msg.target, Target::Token(t) if t == "weapons"),
            "ScienceTargetSuggestion should be sent only to Weapons console"
        );
    }

    #[test]
    fn non_sensors_player_cannot_send_science_target() {
        let mut app = test_app();
        start_game_with_sensors_and_weapons(&mut app);

        push(&mut app, "captain", ClientMessage::SetScienceTarget { uuid: "asteroid-42".into() });
        let out = tick(&mut app);

        assert!(
            !out.iter().any(|m| matches!(&m.msg, ServerMessage::ScienceTargetSuggestion { .. })),
            "non-Sensors player should not be able to send ScienceTargetSuggestion"
        );
    }

    #[test]
    fn set_science_target_ignored_in_lobby() {
        let mut app = test_app();
        push(&mut app, "sensors", ClientMessage::Identify { token: "sensors".into(), name: "Spock".into() });
        tick(&mut app);
        push(&mut app, "sensors", ClientMessage::SelectStation { station: "Sensors".into() });
        tick(&mut app);

        push(&mut app, "sensors", ClientMessage::SetScienceTarget { uuid: "asteroid-42".into() });
        let out = tick(&mut app);

        assert!(
            !out.iter().any(|m| matches!(&m.msg, ServerMessage::ScienceTargetSuggestion { .. })),
            "SetScienceTarget should be ignored during Lobby phase"
        );
    }
}
