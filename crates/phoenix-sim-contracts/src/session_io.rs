//! Shared host input, output and lifecycle records.
use bevy::prelude::*;
use phoenix_model::messages::{ClientMessage, DeliveryClass, ServerMessage};
pub use phoenix_transport::Target;

/// Pending outbound messages produced by lobby systems.
/// Drained each frame by `drain_lobby_outbox`, which runs unconditionally so
/// messages queued on the Lobby→InProgress transition frame (e.g. GameStarted)
/// are not lost.
#[derive(Resource, Default)]
pub struct LobbyOutbox(pub Vec<(Target, ServerMessage)>);

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
    pub delivery: DeliveryClass,
}

/// Ordering anchor for every lobby system (in `FixedUpdate` since issue #895):
/// `handle_disconnect` runs first, then the per-variant message systems
/// (Identify / SetName / ReturnToLobby plus the four station-management
/// systems), then `tick_countdown → update_game_state_cache`. Downstream
/// systems that must observe the post-lobby world state order themselves with
/// `.after(LobbySystemSet)` — which is why they share its schedule.
#[derive(bevy::ecs::schedule::SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct LobbySystemSet;
