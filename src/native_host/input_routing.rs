//! Phoenix focus policy over the reusable platform input router.
#[cfg(test)]
use super::bridge_profile::PaneRect;
use super::host_lobby::HOST_LOBBY_SURFACE_ID;
pub use phoenix_platform::input_routing::*;

/// Keep the host chrome in traversal but initially focus an operator pane.
pub fn focused_on_first_pane(order: Vec<PaneId>) -> FocusRing {
    FocusRing::focused_where(order, |id| id != HOST_LOBBY_SURFACE_ID)
}

#[cfg(test)]
#[path = "input_routing_tests.rs"]
mod tests;
