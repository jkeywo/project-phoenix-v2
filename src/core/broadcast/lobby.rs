use bevy::prelude::*;

use crate::core::broadcast::broadcaster::{dispatch, BroadcastKind, Broadcaster};
use crate::messages::{DeliveryClass, GamePhase};

/// Marker for the lobby broadcast phase.
///
/// - Delivery: `Reliable` (must arrive; lobby chrome is not resent).
/// - Phase gate: inline — dispatch runs only while `GamePhase::Lobby` is the
///   current state (and skips entirely when no `State<GamePhase>` exists).
/// - Schedule: `FixedUpdate`, after `LobbySystemSet` (which moved to the fixed
///   schedule with the sim in issue #895 — the edge must share its schedule).
pub struct Lobby;

impl BroadcastKind for Lobby {
    fn delivery() -> DeliveryClass {
        DeliveryClass::Reliable
    }

    fn phase_allows(world: &World) -> bool {
        match world.get_resource::<State<GamePhase>>() {
            Some(s) => *s.get() == GamePhase::Lobby,
            None => false,
        }
    }

    fn add_dispatch(app: &mut App) {
        app.add_systems(
            FixedUpdate,
            dispatch::<Lobby>.after(crate::lobby::LobbySystemSet),
        );
    }
}

/// Bevy plugin that broadcasts `ServerMessage`s during the `Lobby` phase.
///
/// Use [`Broadcaster::register`] before adding the plugin to `App` to enqueue
/// producers. Each producer is called at the requested cadence and its output
/// is routed to the `Target` resolved from the `Audience`.
pub type LobbyBroadcaster = Broadcaster<Lobby>;

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::broadcast::audience::Audience;
    use crate::core::broadcast::cadence::Cadence;
    use crate::lobby::{LobbyPlugin, OutboundMessage};
    use crate::messages::{GamePhase, ServerMessage};

    #[derive(Resource, Default)]
    struct Outbox(Vec<OutboundMessage>);

    fn collect(mut reader: MessageReader<OutboundMessage>, mut box_: ResMut<Outbox>) {
        for m in reader.read() {
            box_.0.push(m.clone());
        }
    }

    fn dispatch_app(broadcaster: LobbyBroadcaster) -> App {
        let mut app = App::new();
        app.add_plugins(LobbyPlugin);
        app.add_plugins(bevy::time::TimePlugin);
        app.add_plugins(broadcaster);

        app.world_mut()
            .insert_resource(State::new(GamePhase::Lobby));
        app.init_resource::<Outbox>();
        app.add_systems(PostUpdate, collect);
        // One fixed step per update (issue #895): dispatch runs on the
        // logical tick, and a 1 ms step keeps the Hz cadences' clocks as
        // near-still as the old wall-clock fixture.
        crate::ship::test_support::drive_one_fixed_step_per_update(
            &mut app,
            std::time::Duration::from_millis(1),
        );
        app
    }

    fn tick_and_collect(app: &mut App) -> Vec<OutboundMessage> {
        app.update();
        let msgs = app.world().resource::<Outbox>().0.clone();
        msgs
    }

    #[test]
    fn once_cadence_delivers_message_on_first_tick() {
        let broadcaster =
            LobbyBroadcaster::new().register(Audience::All, Cadence::Once, |_world: &mut World| {
                vec![ServerMessage::GameStarted]
            });
        let mut app = dispatch_app(broadcaster);
        let msgs = tick_and_collect(&mut app);
        assert!(
            msgs.iter()
                .any(|m| matches!(m.msg, ServerMessage::GameStarted)),
            "Cadence::Once should deliver message on first tick"
        );
    }

    #[test]
    fn once_cadence_does_not_fire_again() {
        let broadcaster =
            LobbyBroadcaster::new().register(Audience::All, Cadence::Once, |_world: &mut World| {
                vec![ServerMessage::GameStarted]
            });
        let mut app = dispatch_app(broadcaster);
        let _ = tick_and_collect(&mut app);
        app.world_mut().resource_mut::<Outbox>().0.clear();
        let msgs = tick_and_collect(&mut app);
        assert!(
            !msgs
                .iter()
                .any(|m| matches!(m.msg, ServerMessage::GameStarted)),
            "Cadence::Once should not fire on second tick"
        );
    }

    #[test]
    fn on_event_producer_drains_per_frame() {
        use std::sync::{Arc, Mutex};
        let queue: Arc<Mutex<Vec<ServerMessage>>> = Arc::new(Mutex::new(vec![]));
        let q2 = queue.clone();

        let broadcaster = LobbyBroadcaster::new().register(
            Audience::All,
            Cadence::OnEvent,
            move |_world: &mut World| {
                let mut q = q2.lock().unwrap();
                std::mem::take(&mut *q)
            },
        );

        let mut app = dispatch_app(broadcaster);

        queue.lock().unwrap().push(ServerMessage::GameStarted);
        queue.lock().unwrap().push(ServerMessage::GameStarted);

        let msgs1 = tick_and_collect(&mut app);
        let count1 = msgs1
            .iter()
            .filter(|m| matches!(m.msg, ServerMessage::GameStarted))
            .count();
        assert_eq!(count1, 2, "tick 1: expected 2 messages, got {count1}");

        app.world_mut().resource_mut::<Outbox>().0.clear();
        let msgs2 = tick_and_collect(&mut app);
        let count2 = msgs2
            .iter()
            .filter(|m| matches!(m.msg, ServerMessage::GameStarted))
            .count();
        assert_eq!(
            count2, 0,
            "tick 2: expected 0 messages after drain, got {count2}"
        );
    }

    #[test]
    fn multiple_registrations_all_fire() {
        let broadcaster = LobbyBroadcaster::new()
            .register(Audience::All, Cadence::Once, |_world: &mut World| {
                vec![ServerMessage::GameStarted]
            })
            .register(Audience::All, Cadence::Once, |_world: &mut World| {
                vec![ServerMessage::PlayerLeft { token: "t1".into() }]
            });

        let mut app = dispatch_app(broadcaster);
        let msgs = tick_and_collect(&mut app);
        let game_started = msgs
            .iter()
            .any(|m| matches!(m.msg, ServerMessage::GameStarted));
        let player_left = msgs
            .iter()
            .any(|m| matches!(m.msg, ServerMessage::PlayerLeft { .. }));
        assert!(game_started, "first registration should fire");
        assert!(player_left, "second registration should fire");
    }

    #[test]
    fn dispatch_skips_when_not_in_lobby_phase() {
        let broadcaster =
            LobbyBroadcaster::new().register(Audience::All, Cadence::Once, |_world: &mut World| {
                vec![ServerMessage::GameStarted]
            });
        let mut app = dispatch_app(broadcaster);
        app.world_mut()
            .insert_resource(State::new(GamePhase::InProgress));
        let msgs = tick_and_collect(&mut app);
        assert!(
            !msgs
                .iter()
                .any(|m| matches!(m.msg, ServerMessage::GameStarted)),
            "broadcasts should not fire outside lobby phase"
        );
    }
}
