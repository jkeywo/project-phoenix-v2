use bevy::prelude::*;

use crate::messages::{ClientMessage, Console, GamePhase, GameState, ServerMessage};
use crate::session::SessionManager;

// ── Resources ──────────────────────────────────────────────────────────────

#[derive(Resource)]
pub struct Sessions(pub SessionManager);

#[derive(Resource)]
pub struct CurrentPhase(pub GamePhase);

// ── Messages (Bevy 0.18 pull-based message system) ─────────────────────────

/// A decoded ClientMessage received from one peer, tagged with the sender's
/// session token.
#[derive(Message, Clone)]
pub struct InboundMessage {
    pub token: String,
    pub msg: ClientMessage,
}

/// A lifecycle event signalled by the transport layer when a peer disconnects.
#[derive(Message, Clone)]
pub struct PlayerDisconnected {
    pub token: String,
}

/// A ServerMessage to be forwarded to one or all peers by the JS bridge.
#[derive(Message, Clone)]
pub struct OutboundMessage {
    pub target: Target,
    pub msg: ServerMessage,
}

#[derive(Clone, Debug)]
pub enum Target {
    All,
    Token(String),
    AllExcept(String),
}

// ── Plugin ─────────────────────────────────────────────────────────────────

pub struct LobbyPlugin;

impl Plugin for LobbyPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Sessions(SessionManager::new()))
            .insert_resource(CurrentPhase(GamePhase::Lobby))
            .add_message::<InboundMessage>()
            .add_message::<OutboundMessage>()
            .add_message::<PlayerDisconnected>()
            .add_systems(Update, (process_lobby, handle_disconnect));
    }
}

// ── Systems ────────────────────────────────────────────────────────────────

fn game_state(sessions: &SessionManager, phase: &GamePhase) -> GameState {
    GameState { phase: phase.clone(), players: sessions.players().to_vec() }
}

fn process_lobby(
    mut inbound: MessageReader<InboundMessage>,
    mut outbound: MessageWriter<OutboundMessage>,
    mut sessions: ResMut<Sessions>,
    mut phase: ResMut<CurrentPhase>,
) {
    for ev in inbound.read() {
        match ev.msg.clone() {
            ClientMessage::Identify { token, name } => {
                if let Some(player) = sessions.0.reconnect(&token) {
                    let player = player.clone();
                    let state = game_state(&sessions.0, &phase.0);
                    outbound.write(OutboundMessage {
                        target: Target::Token(token.clone()),
                        msg: ServerMessage::Welcome { state },
                    });
                    outbound.write(OutboundMessage {
                        target: Target::AllExcept(token),
                        msg: ServerMessage::PlayerJoined { player },
                    });
                } else if let Ok(player) = sessions.0.register(token.clone(), name) {
                    let player = player.clone();
                    let state = game_state(&sessions.0, &phase.0);
                    outbound.write(OutboundMessage {
                        target: Target::Token(token.clone()),
                        msg: ServerMessage::Welcome { state },
                    });
                    outbound.write(OutboundMessage {
                        target: Target::AllExcept(token),
                        msg: ServerMessage::PlayerJoined { player },
                    });
                }
            }
            ClientMessage::SetName { name } => {
                sessions.0.set_name(&ev.token, name.clone());
                outbound.write(OutboundMessage {
                    target: Target::All,
                    msg: ServerMessage::NameChanged { token: ev.token.clone(), name },
                });
            }
            ClientMessage::SelectConsole { console } => {
                if sessions.0.select_console(&ev.token, console.clone()).is_ok() {
                    outbound.write(OutboundMessage {
                        target: Target::All,
                        msg: ServerMessage::ConsoleSelected {
                            token: ev.token.clone(),
                            console,
                        },
                    });
                }
            }
            ClientMessage::ClearConsole => {
                sessions.0.clear_console(&ev.token);
                outbound.write(OutboundMessage {
                    target: Target::All,
                    msg: ServerMessage::ConsoleCleared { token: ev.token.clone() },
                });
            }
            ClientMessage::StartGame => {
                if sessions.0.captain_token() == Some(ev.token.as_str())
                    && phase.0 == GamePhase::Lobby
                {
                    phase.0 = GamePhase::InProgress;
                    outbound.write(OutboundMessage {
                        target: Target::All,
                        msg: ServerMessage::GameStarted,
                    });
                }
            }
            ClientMessage::ToggleRedAlert => {}
        }
    }
}

fn handle_disconnect(
    mut events: MessageReader<PlayerDisconnected>,
    mut outbound: MessageWriter<OutboundMessage>,
    mut sessions: ResMut<Sessions>,
) {
    for ev in events.read() {
        sessions.0.disconnect(&ev.token);
        outbound.write(OutboundMessage {
            target: Target::All,
            msg: ServerMessage::PlayerLeft { token: ev.token.clone() },
        });
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Resource, Default)]
    struct Outbox(Vec<OutboundMessage>);

    fn collect(mut reader: MessageReader<OutboundMessage>, mut outbox: ResMut<Outbox>) {
        for ev in reader.read() {
            outbox.0.push(ev.clone());
        }
    }

    fn test_app() -> App {
        let mut app = App::new();
        app.add_plugins(LobbyPlugin)
            .init_resource::<Outbox>()
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
        let msgs = app.world().resource::<Outbox>().0.clone();
        app.world_mut().resource_mut::<Outbox>().0.clear();
        msgs
    }

    #[test]
    fn identify_sends_welcome_to_new_player() {
        let mut app = test_app();
        push(&mut app, "t1", ClientMessage::Identify { token: "t1".into(), name: "Alice".into() });
        let out = tick(&mut app);
        assert!(out.iter().any(|m| matches!(&m.msg, ServerMessage::Welcome { .. })));
    }

    #[test]
    fn second_player_gets_welcome_others_get_player_joined() {
        let mut app = test_app();
        push(&mut app, "t1", ClientMessage::Identify { token: "t1".into(), name: "Alice".into() });
        tick(&mut app);
        push(&mut app, "t2", ClientMessage::Identify { token: "t2".into(), name: "Bob".into() });
        let out = tick(&mut app);
        assert!(out.iter().any(|m| matches!(&m.msg, ServerMessage::Welcome { .. })));
        assert!(out.iter().any(|m| matches!(&m.msg, ServerMessage::PlayerJoined { .. })));
    }

    #[test]
    fn select_console_updates_session_and_broadcasts() {
        let mut app = test_app();
        push(&mut app, "t1", ClientMessage::Identify { token: "t1".into(), name: "Alice".into() });
        tick(&mut app);
        push(&mut app, "t1", ClientMessage::SelectConsole { console: Console::CaptainChair });
        let out = tick(&mut app);
        assert!(out.iter().any(|m| matches!(&m.msg, ServerMessage::ConsoleSelected { .. })));
        assert_eq!(
            app.world().resource::<Sessions>().0.players()[0].console,
            Some(Console::CaptainChair)
        );
    }

    #[test]
    fn clear_console_removes_assignment_and_broadcasts() {
        let mut app = test_app();
        push(&mut app, "t1", ClientMessage::Identify { token: "t1".into(), name: "Alice".into() });
        tick(&mut app);
        push(&mut app, "t1", ClientMessage::SelectConsole { console: Console::CaptainChair });
        tick(&mut app);
        push(&mut app, "t1", ClientMessage::ClearConsole);
        let out = tick(&mut app);
        assert!(out.iter().any(|m| matches!(&m.msg, ServerMessage::ConsoleCleared { .. })));
        assert!(app.world().resource::<Sessions>().0.players()[0].console.is_none());
    }

    #[test]
    fn set_name_broadcasts_name_changed() {
        let mut app = test_app();
        push(&mut app, "t1", ClientMessage::Identify { token: "t1".into(), name: "Alice".into() });
        tick(&mut app);
        push(&mut app, "t1", ClientMessage::SetName { name: "Alicia".into() });
        let out = tick(&mut app);
        assert!(out.iter().any(|m| {
            matches!(&m.msg, ServerMessage::NameChanged { name, .. } if name == "Alicia")
        }));
    }

    #[test]
    fn captain_starts_game_and_transitions_to_in_progress() {
        let mut app = test_app();
        push(&mut app, "t1", ClientMessage::Identify { token: "t1".into(), name: "Alice".into() });
        tick(&mut app);
        push(&mut app, "t1", ClientMessage::StartGame);
        let out = tick(&mut app);
        assert!(out.iter().any(|m| matches!(&m.msg, ServerMessage::GameStarted)));
        assert_eq!(app.world().resource::<CurrentPhase>().0, GamePhase::InProgress);
    }

    fn disconnect(app: &mut App, token: &str) {
        app.world_mut()
            .resource_mut::<Messages<PlayerDisconnected>>()
            .write(PlayerDisconnected { token: token.into() });
    }

    #[test]
    fn disconnect_of_captain_makes_next_player_captain() {
        let mut app = test_app();
        push(&mut app, "t1", ClientMessage::Identify { token: "t1".into(), name: "Alice".into() });
        tick(&mut app);
        push(&mut app, "t2", ClientMessage::Identify { token: "t2".into(), name: "Bob".into() });
        tick(&mut app);
        disconnect(&mut app, "t1");
        tick(&mut app);
        // t2 is now captain — only t2 should be able to start the game
        push(&mut app, "t2", ClientMessage::StartGame);
        let out = tick(&mut app);
        assert!(out.iter().any(|m| matches!(&m.msg, ServerMessage::GameStarted)));
    }

    #[test]
    fn disconnect_releases_console_so_another_can_claim_it() {
        let mut app = test_app();
        push(&mut app, "t1", ClientMessage::Identify { token: "t1".into(), name: "Alice".into() });
        tick(&mut app);
        push(&mut app, "t1", ClientMessage::SelectConsole { console: Console::CaptainChair });
        tick(&mut app);
        disconnect(&mut app, "t1");
        tick(&mut app);
        push(&mut app, "t2", ClientMessage::Identify { token: "t2".into(), name: "Bob".into() });
        tick(&mut app);
        push(&mut app, "t2", ClientMessage::SelectConsole { console: Console::CaptainChair });
        let out = tick(&mut app);
        assert!(out.iter().any(|m| matches!(&m.msg, ServerMessage::ConsoleSelected { .. })));
    }

    #[test]
    fn disconnect_broadcasts_player_left() {
        let mut app = test_app();
        push(&mut app, "t1", ClientMessage::Identify { token: "t1".into(), name: "Alice".into() });
        tick(&mut app);
        disconnect(&mut app, "t1");
        let out = tick(&mut app);
        assert!(out.iter().any(|m| matches!(&m.msg, ServerMessage::PlayerLeft { token } if token == "t1")));
    }

    #[test]
    fn reconnect_broadcasts_player_joined_to_others() {
        let mut app = test_app();
        // t1 joins, disconnects (simulated by registering then re-identifying)
        push(&mut app, "t1", ClientMessage::Identify { token: "t1".into(), name: "Alice".into() });
        tick(&mut app);
        push(&mut app, "t2", ClientMessage::Identify { token: "t2".into(), name: "Bob".into() });
        tick(&mut app);
        // t1 reconnects — sends Identify with same token
        push(&mut app, "t1", ClientMessage::Identify { token: "t1".into(), name: "Alice".into() });
        let out = tick(&mut app);
        // The reconnecting player gets Welcome
        assert!(out.iter().any(|m| matches!(&m.target, Target::Token(t) if t == "t1")
            && matches!(&m.msg, ServerMessage::Welcome { .. })));
        // Other players get PlayerJoined
        assert!(out.iter().any(|m| matches!(&m.target, Target::AllExcept(t) if t == "t1")
            && matches!(&m.msg, ServerMessage::PlayerJoined { .. })));
    }

    #[test]
    fn non_captain_cannot_start_game() {
        let mut app = test_app();
        push(&mut app, "t1", ClientMessage::Identify { token: "t1".into(), name: "Alice".into() });
        tick(&mut app);
        push(&mut app, "t2", ClientMessage::Identify { token: "t2".into(), name: "Bob".into() });
        tick(&mut app);
        push(&mut app, "t2", ClientMessage::StartGame);
        let out = tick(&mut app);
        assert!(!out.iter().any(|m| matches!(&m.msg, ServerMessage::GameStarted)));
        assert_eq!(app.world().resource::<CurrentPhase>().0, GamePhase::Lobby);
    }
}
