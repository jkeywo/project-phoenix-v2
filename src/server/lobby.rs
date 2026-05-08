use bevy::prelude::*;

use crate::server::lobby_handler::{self, LobbyHandlerResult};
use crate::shared::messages::{ClientMessage, GamePhase, GameState, ServerMessage, WorldData};
use crate::server::session::SessionManager;

/// Cached `GameState` snapshot derived from `Sessions` + `CurrentPhase` each frame.
/// Renderer systems read this instead of accessing `Sessions` directly.
#[derive(Resource, Clone)]
pub struct GameStateCache(pub GameState);

pub use crate::server::lobby_handler::Target;

// ── Resources ──────────────────────────────────────────────────────────────

#[derive(Resource)]
pub struct Sessions(pub SessionManager);

#[derive(Resource)]
pub struct CurrentPhase(pub GamePhase);

/// Server's authoritative copy of the world layout — populated once during
/// world setup and broadcast to clients via `WorldSetup` after `StartGame`,
/// and replayed inside `Welcome` for mid-game reconnects.
#[derive(Resource, Clone, Default)]
pub struct WorldResource(pub WorldData);

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

// ── Plugin ─────────────────────────────────────────────────────────────────

pub struct LobbyPlugin;

impl Plugin for LobbyPlugin {
    fn build(&self, app: &mut App) {
        let initial_cache = GameStateCache(GameState {
            phase: GamePhase::Lobby,
            players: vec![],
            world: None,
        });
        app.insert_resource(Sessions(SessionManager::new()))
            .insert_resource(CurrentPhase(GamePhase::Lobby))
            .insert_resource(initial_cache)
            .add_message::<InboundMessage>()
            .add_message::<OutboundMessage>()
            .add_message::<PlayerDisconnected>()
            .add_systems(Update, (process_lobby, handle_disconnect, update_game_state_cache));
    }
}

pub fn update_game_state_cache(
    sessions: Res<Sessions>,
    phase: Res<CurrentPhase>,
    world: Option<Res<WorldResource>>,
    mut cache: ResMut<GameStateCache>,
) {
    if !sessions.is_changed() && !phase.is_changed() {
        return;
    }
    let world_data = world.as_ref().map(|w| &w.0);
    cache.0 = lobby_handler::derive_game_state(&sessions.0, &phase.0, world_data);
}

// ── Systems ────────────────────────────────────────────────────────────────

pub fn process_lobby(
    mut inbound: MessageReader<InboundMessage>,
    mut outbound: MessageWriter<OutboundMessage>,
    mut sessions: ResMut<Sessions>,
    mut phase: ResMut<CurrentPhase>,
    world: Option<Res<WorldResource>>,
) {
    let world_data = world.as_ref().map(|w| &w.0);
    for ev in inbound.read() {
        let result = lobby_handler::process_message(
            &ev.token,
            &ev.msg,
            &mut sessions.0,
            phase.0.clone(),
            world_data,
        );
        apply_result(result, &mut outbound, &mut phase);
    }
}

fn handle_disconnect(
    mut events: MessageReader<PlayerDisconnected>,
    mut outbound: MessageWriter<OutboundMessage>,
    mut sessions: ResMut<Sessions>,
    mut phase: ResMut<CurrentPhase>,
) {
    for ev in events.read() {
        let result = lobby_handler::process_disconnect(&ev.token, &mut sessions.0);
        apply_result(result, &mut outbound, &mut phase);
    }
}

fn apply_result(
    result: LobbyHandlerResult,
    outbound: &mut MessageWriter<OutboundMessage>,
    phase: &mut ResMut<CurrentPhase>,
) {
    if let Some(new_phase) = result.new_phase {
        phase.0 = new_phase;
    }
    for (target, msg) in result.outbound {
        outbound.write(OutboundMessage { target, msg });
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
    fn identify_arrives_via_inbound_message_and_welcome_is_sent_via_outbound() {
        let mut app = test_app();
        push(&mut app, "peer-id", ClientMessage::Identify { token: "t1".into(), name: "Alice".into() });
        let out = tick(&mut app);
        assert!(out.iter().any(|m| matches!(&m.msg, ServerMessage::Welcome { .. })));
    }
}
