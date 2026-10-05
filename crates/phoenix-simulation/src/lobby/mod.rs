pub mod crew_replication;
pub mod handler;
/// First-valid-wins scenario + player-ship selection (issue #755's arbiter,
/// ported to Rust for issue #1326) — pure, Bevy-free, and a deliberate
/// transcription of `gui/scenario-arbiter.js` rather than a second design.
pub mod scenario_arbiter;
pub mod server;
pub mod session;
pub mod start_policy;
pub mod stations_config;

pub use server::{
    apply_fleet_lobby_inputs, lobby_outbox_broadcaster, CountdownTimer, FleetLobbyInput,
    FleetManagedLobby, GameStateCache, InboundMessage, LobbyOutbox, LobbyPlugin, LobbySystemSet,
    OutboundMessage, PendingStartGrants, PlayerDisconnected, SelectedShipResource, Sessions,
    StartGrantResults, Target, WorldResource,
};
pub use start_policy::{ReadinessTally, StartGrant, StartGrantResult};
