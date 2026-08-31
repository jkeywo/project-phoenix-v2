pub mod handler;
/// First-valid-wins scenario + player-ship selection (issue #755's arbiter,
/// ported to Rust for issue #1326) — pure, Bevy-free, and a deliberate
/// transcription of `gui/scenario-arbiter.js` rather than a second design.
pub mod scenario_arbiter;
pub mod server;
pub mod session;
pub mod stations_config;

pub use server::{
    lobby_outbox_broadcaster, CountdownTimer, GameStateCache, InboundMessage, LobbyOutbox,
    LobbyPlugin, LobbySystemSet, OutboundMessage, PlayerDisconnected, SelectedShipResource,
    Sessions, Target, WorldResource,
};
