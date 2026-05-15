use bevy::prelude::*;

use crate::lobby_handler;
pub use crate::lobby_handler::Target;
use crate::messages::{ClientMessage, GamePhase, GameState, ServerMessage, WorldData};
use crate::session::SessionManager;
use crate::stations_config::ShipStations;

/// Cached `GameState` snapshot derived from `Sessions` + `CurrentPhase` each frame.
/// Renderer systems read this instead of accessing `Sessions` directly.
#[derive(Resource, Clone)]
pub struct GameStateCache(pub GameState);

/// Pending outbound messages produced by lobby systems.
/// Drained each frame by the `LobbyBroadcaster` dispatch.
#[derive(Resource, Default)]
pub struct LobbyOutbox(pub Vec<(Target, ServerMessage)>);

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
            complexity: std::collections::HashMap::new(),
            world: None,
        });
        app.insert_resource(Sessions(SessionManager::new()))
            .insert_resource(CurrentPhase(GamePhase::Lobby))
            .insert_resource(initial_cache)
            .insert_resource(LobbyOutbox::default())
            .add_message::<InboundMessage>()
            .add_message::<OutboundMessage>()
            .add_message::<PlayerDisconnected>()
            .add_systems(Startup, update_session_with_config)
            .add_systems(Update, (process_lobby, handle_disconnect, update_game_state_cache));
    }
}

/// Update the Sessions resource with available consoles from the ship's EntityConfig.
fn update_session_with_config(
    mut sessions: ResMut<Sessions>,
) {
    // Get the config cache from thread-local storage
    if let Some(ship_config) = crate::config_cache::get_config_cache()
        .get("assets/entities/player_ship.toml")
    {
        sessions.0 = SessionManager::new_with_config(ship_config);
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
    mut sessions: ResMut<Sessions>,
    mut phase: ResMut<CurrentPhase>,
    mut outbox: ResMut<LobbyOutbox>,
    world: Option<Res<WorldResource>>,
    ship_stations: Option<Res<ShipStations>>,
) {
    // Only consume inbound messages during the Lobby phase.  In InProgress the
    // simulation systems own the message queue; draining here would silently
    // discard HelmInput and other sim messages before they can be processed.
    if phase.0 != GamePhase::Lobby {
        return;
    }
    let default_stations = ShipStations::default();
    let stations = ship_stations.as_ref().map(|s| s.as_ref()).unwrap_or(&default_stations);
    let world_data = world.as_ref().map(|w| &w.0);
    for ev in inbound.read() {
        let result = lobby_handler::process_message(
            &ev.token,
            &ev.msg,
            &mut sessions.0,
            phase.0.clone(),
            world_data,
            stations,
        );
        apply_result(result, &mut outbox, &mut phase);
    }
}

fn handle_disconnect(
    mut events: MessageReader<PlayerDisconnected>,
    mut sessions: ResMut<Sessions>,
    mut phase: ResMut<CurrentPhase>,
    mut outbox: ResMut<LobbyOutbox>,
    ship_stations: Option<Res<ShipStations>>,
) {
    for ev in events.read() {
        let result = if let Some(stations) = ship_stations.as_ref() {
            lobby_handler::process_disconnect_with_stations(&ev.token, &mut sessions.0, stations)
        } else {
            lobby_handler::process_disconnect(&ev.token, &mut sessions.0)
        };
        apply_result(result, &mut outbox, &mut phase);
    }
}

fn apply_result(
    result: lobby_handler::LobbyHandlerResult,
    outbox: &mut ResMut<LobbyOutbox>,
    phase: &mut ResMut<CurrentPhase>,
) {
    if let Some(new_phase) = result.new_phase {
        phase.0 = new_phase;
    }
    outbox.0.extend(result.outbound);
}

// ── Broadcaster helper ─────────────────────────────────────────────────────

/// Returns a [`LobbyBroadcaster`] pre-configured with a producer that drains
/// [`LobbyOutbox`] each frame and writes each entry as an `OutboundMessage`.
///
/// Uses `Cadence::OnEvent` so the producer fires every frame.  When the outbox
/// is empty the producer returns an empty `Vec` and no messages are emitted.
/// When populated (by `process_lobby` or `handle_disconnect`) the queued
/// entries are flushed directly to `OutboundMessage` with their original
/// `Target` routing.
///
/// This must be registered once (typically in `bridge.rs`) alongside
/// `LobbyPlugin`.  Multiple registrations are safe because
/// `LobbyBroadcaster::is_unique` returns `false`.
pub fn lobby_outbox_broadcaster() -> crate::core::broadcast::LobbyBroadcaster {
    use crate::core::broadcast::{Audience, Cadence, LobbyBroadcaster};
    LobbyBroadcaster::new().register(
        Audience::All,
        Cadence::OnEvent,
        |world: &mut bevy::prelude::World| {
            let mut outbox = world.resource_mut::<LobbyOutbox>();
            let entries = std::mem::take(&mut outbox.0);
            for (target, msg) in entries {
                world.write_message(OutboundMessage { target, msg });
            }
            vec![]
        },
    )
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
            .add_plugins(lobby_outbox_broadcaster())
            .add_plugins(bevy::time::TimePlugin)
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
