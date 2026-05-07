//! Pure client-side simulation-state model.
//!
//! Mirrors the parts of `SimSnapshot` the captain UI needs to render
//! (red alert state, current view mode), updated by inbound `SimState`
//! messages, and exposes `ClientMessage` builders for the captain
//! buttons. Bevy-free so it can be exhaustively unit-tested on native.

use bevy::prelude::Resource;

use crate::messages::{ClientMessage, ServerMessage, ViewDirection, ViewMode};

/// Subset of `SimSnapshot` the client UI needs. Reset to defaults on
/// `Welcome` (which also clears `LobbyState`) and refreshed every time
/// a `SimState` message arrives.
#[derive(Clone, Debug, PartialEq, Resource)]
pub struct ClientSimState {
    pub red_alert: bool,
    pub view_mode: ViewMode,
}

impl Default for ClientSimState {
    fn default() -> Self {
        Self {
            red_alert: false,
            view_mode: ViewMode::default(),
        }
    }
}

impl ClientSimState {
    /// Apply a single inbound `ServerMessage`. Only `SimState` and the
    /// `Welcome` reset are honoured; everything else is ignored so this
    /// state can be driven from the same event stream as `LobbyState`.
    pub fn apply(&mut self, msg: &ServerMessage) {
        match msg {
            ServerMessage::SimState { snapshot } => {
                self.red_alert = snapshot.red_alert;
                self.view_mode = snapshot.view_mode.clone();
            }
            ServerMessage::Welcome { .. } => {
                *self = Self::default();
            }
            _ => {}
        }
    }

    /// True iff the captain's view direction selector should highlight
    /// the given direction. Radar mode highlights nothing in the cross.
    pub fn is_active_camera_direction(&self, direction: &ViewDirection) -> bool {
        matches!(&self.view_mode, ViewMode::Camera(d) if d == direction)
    }
}

/// `ClientMessage` to send when the captain presses a direction button
/// in the view-selector cross.
pub fn message_for_direction_press(direction: ViewDirection) -> ClientMessage {
    ClientMessage::SetView { mode: ViewMode::Camera(direction) }
}

/// `ClientMessage` to send when the captain presses the Red Alert toggle.
pub fn red_alert_toggle_message() -> ClientMessage {
    ClientMessage::ToggleRedAlert
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::{Console, GamePhase, GameState, Player, SimSnapshot};

    fn snap(red_alert: bool, view_mode: ViewMode) -> SimSnapshot {
        SimSnapshot { red_alert, view_mode, ship_x: 0.0, ship_z: 0.0, ship_yaw: 0.0 }
    }

    #[test]
    fn default_sim_state_is_calm_and_facing_forward() {
        let s = ClientSimState::default();
        assert!(!s.red_alert);
        assert_eq!(s.view_mode, ViewMode::Camera(ViewDirection::Fore));
    }

    #[test]
    fn sim_state_message_updates_red_alert_and_view_mode() {
        let mut s = ClientSimState::default();
        s.apply(&ServerMessage::SimState {
            snapshot: snap(true, ViewMode::Camera(ViewDirection::Aft)),
        });
        assert!(s.red_alert);
        assert_eq!(s.view_mode, ViewMode::Camera(ViewDirection::Aft));
    }

    #[test]
    fn welcome_resets_sim_state_to_defaults() {
        let mut s = ClientSimState {
            red_alert: true,
            view_mode: ViewMode::Radar,
        };
        s.apply(&ServerMessage::Welcome {
            state: GameState {
                phase: GamePhase::Lobby,
                players: vec![],
                world: None,
            },
        });
        assert_eq!(s, ClientSimState::default());
    }

    #[test]
    fn unrelated_messages_do_not_disturb_sim_state() {
        let mut s = ClientSimState {
            red_alert: true,
            view_mode: ViewMode::Camera(ViewDirection::Port),
        };
        let before = s.clone();
        s.apply(&ServerMessage::PlayerJoined {
            player: Player { token: "x".into(), name: "Y".into(), consoles: vec![Console::Helm], connected: true },
        });
        assert_eq!(s, before);
    }

    #[test]
    fn is_active_camera_direction_only_matches_in_camera_mode() {
        let mut s = ClientSimState::default();
        assert!( s.is_active_camera_direction(&ViewDirection::Fore));
        assert!(!s.is_active_camera_direction(&ViewDirection::Aft));

        s.view_mode = ViewMode::Camera(ViewDirection::Port);
        assert!( s.is_active_camera_direction(&ViewDirection::Port));
        assert!(!s.is_active_camera_direction(&ViewDirection::Fore));

        s.view_mode = ViewMode::Radar;
        for d in [ViewDirection::Fore, ViewDirection::Aft, ViewDirection::Port, ViewDirection::Starboard] {
            assert!(!s.is_active_camera_direction(&d), "Radar mode highlights no cross arrow");
        }
    }

    #[test]
    fn direction_press_builds_set_view_camera_message() {
        let msg = message_for_direction_press(ViewDirection::Starboard);
        assert_eq!(
            msg,
            ClientMessage::SetView { mode: ViewMode::Camera(ViewDirection::Starboard) },
        );
    }

    #[test]
    fn red_alert_toggle_message_is_toggle_red_alert() {
        assert_eq!(red_alert_toggle_message(), ClientMessage::ToggleRedAlert);
    }
}
