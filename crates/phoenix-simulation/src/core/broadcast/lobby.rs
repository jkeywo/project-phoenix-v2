use bevy::prelude::*;

use crate::core::broadcast::broadcaster::{dispatch, BroadcastKind, Broadcaster};
use crate::core::messages::{DeliveryClass, GamePhase};

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
#[path = "lobby_tests.rs"]
mod tests;
