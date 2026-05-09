//! Pure client-side simulation-state model.
//!
//! Mirrors the parts of `SimSnapshot` the captain UI needs to render
//! (red alert state, current view mode), updated by inbound `SimState`
//! messages, and exposes `ClientMessage` builders for the captain
//! buttons. Bevy-free so it can be exhaustively unit-tested on native.

use bevy::prelude::Resource;

use crate::messages::{ClientMessage, ServerMessage, ViewDirection, ViewMode, WorldData};

/// Subset of `SimSnapshot` the client UI needs. Reset to defaults on
/// `Welcome` (which also clears `LobbyState`) and refreshed every time
/// a `SimState` message arrives.
#[derive(Clone, Debug, PartialEq, Resource)]
pub struct ClientSimState {
    pub red_alert: bool,
    pub view_mode: ViewMode,
    pub ship_x:   f32,
    pub ship_z:   f32,
    pub ship_yaw: f32,
    /// Static world snapshot replayed on `WorldSetup` and on `Welcome`
    /// (when the server includes it). Used by the helm radar.
    pub world: WorldData,
    /// Seconds remaining on the active repair action (if `repair_in_progress`)
    /// or penalty cooldown (if `repair_penalty`).
    pub repair_cooldown_secs: f32,
    /// True while this console is performing an authorized repair.
    pub repair_in_progress: bool,
    /// True while this player has an unauthorized-repair penalty cooldown.
    pub repair_penalty: bool,
}

impl Default for ClientSimState {
    fn default() -> Self {
        Self {
            red_alert: false,
            view_mode: ViewMode::default(),
            ship_x:   0.0,
            ship_z:   0.0,
            ship_yaw: 0.0,
            world: WorldData::default(),
            repair_cooldown_secs: 0.0,
            repair_in_progress: false,
            repair_penalty: false,
        }
    }
}

impl ClientSimState {
    /// Apply a single inbound `ServerMessage`. Drives both the captain
    /// console state (red alert, view mode) and the helm console state
    /// (ship pose, world snapshot for the radar).
    pub fn apply(&mut self, msg: &ServerMessage) {
        match msg {
            ServerMessage::SimState { snapshot } => {
                self.red_alert = snapshot.red_alert;
                self.view_mode = snapshot.view_mode.clone();
                self.ship_x   = snapshot.ship_x;
                self.ship_z   = snapshot.ship_z;
                self.ship_yaw = snapshot.ship_yaw;
            }
            ServerMessage::WorldSetup { world } => {
                self.world = world.clone();
            }
            ServerMessage::Welcome { state } => {
                let preserved_world = state.world.clone().unwrap_or_default();
                *self = Self::default();
                self.world = preserved_world;
            }
            ServerMessage::RepairState { remaining_cooldown_secs, in_progress, penalty } => {
                self.repair_cooldown_secs = *remaining_cooldown_secs;
                self.repair_in_progress = *in_progress;
                self.repair_penalty = *penalty;
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

/// `ClientMessage` for the helm "On Screen" button: switches the server
/// viewscreen to radar mode.
pub fn on_screen_message() -> ClientMessage {
    ClientMessage::SetView { mode: ViewMode::Radar }
}

/// `ClientMessage` for the Repair button: sends a repair request to the server.
pub fn repair_message() -> ClientMessage {
    ClientMessage::Repair { console: crate::messages::Console::Helm }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::{AsteroidInfo, Console, GamePhase, GameState, Player, SimSnapshot};

    fn snap(red_alert: bool, view_mode: ViewMode) -> SimSnapshot {
        SimSnapshot { red_alert, view_mode, ship_x: 0.0, ship_z: 0.0, ship_yaw: 0.0, hull_integrity: 100, authorized_repair_console: None }
    }

    fn snap_pose(x: f32, z: f32, yaw: f32) -> SimSnapshot {
        SimSnapshot {
            red_alert: false,
            view_mode: ViewMode::default(),
            ship_x: x,
            ship_z: z,
            ship_yaw: yaw,
            hull_integrity: 100,
            authorized_repair_console: None,
        }
    }

    #[test]
    fn default_sim_state_is_calm_and_facing_forward() {
        let s = ClientSimState::default();
        assert!(!s.red_alert);
        assert_eq!(s.view_mode, ViewMode::Camera(ViewDirection::Fore));
        assert_eq!(s.ship_x, 0.0);
        assert_eq!(s.ship_z, 0.0);
        assert_eq!(s.ship_yaw, 0.0);
        assert!(s.world.asteroids.is_empty());
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
    fn sim_state_message_updates_ship_pose() {
        let mut s = ClientSimState::default();
        s.apply(&ServerMessage::SimState { snapshot: snap_pose(12.5, -7.25, 1.5) });
        assert_eq!(s.ship_x, 12.5);
        assert_eq!(s.ship_z, -7.25);
        assert_eq!(s.ship_yaw, 1.5);
    }

    #[test]
    fn world_setup_message_populates_world_data() {
        let mut s = ClientSimState::default();
        let world = WorldData {
            asteroids: vec![
                AsteroidInfo { uuid: "a".into(), x:  3.0, z:  4.0, radius: 2.0 },
                AsteroidInfo { uuid: "b".into(), x: -1.5, z:  0.0, radius: 1.0 },
            ],
        };
        s.apply(&ServerMessage::WorldSetup { world: world.clone() });
        assert_eq!(s.world, world);
    }

    #[test]
    fn welcome_resets_sim_state_but_preserves_world_when_present() {
        let mut s = ClientSimState {
            red_alert: true,
            view_mode: ViewMode::Radar,
            ship_x: 9.0, ship_z: 9.0, ship_yaw: 1.0,
            world: WorldData::default(),
            repair_cooldown_secs: 0.0,
            repair_in_progress: false,
            repair_penalty: false,
        };
        let world = WorldData {
            asteroids: vec![AsteroidInfo { uuid: "c".into(), x: 1.0, z: 2.0, radius: 0.5 }],
        };
        s.apply(&ServerMessage::Welcome {
            state: GameState {
                phase: GamePhase::InProgress,
                players: vec![],
                world: Some(world.clone()),
            },
        });
        // Everything except `world` must reset to defaults.
        assert!(!s.red_alert);
        assert_eq!(s.view_mode, ViewMode::default());
        assert_eq!(s.ship_x, 0.0);
        assert_eq!(s.ship_z, 0.0);
        assert_eq!(s.ship_yaw, 0.0);
        assert_eq!(s.world, world, "world from Welcome must be retained");
    }

    #[test]
    fn welcome_without_world_clears_world_to_default() {
        let mut s = ClientSimState {
            red_alert: false,
            view_mode: ViewMode::default(),
            ship_x: 0.0, ship_z: 0.0, ship_yaw: 0.0,
            world: WorldData {
                asteroids: vec![AsteroidInfo { uuid: "d".into(), x: 0.0, z: 0.0, radius: 1.0 }],
            },
            repair_cooldown_secs: 0.0,
            repair_in_progress: false,
            repair_penalty: false,
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
            ship_x: 5.0, ship_z: 6.0, ship_yaw: 0.7,
            world: WorldData {
                asteroids: vec![AsteroidInfo { uuid: "e".into(), x: 0.0, z: 0.0, radius: 1.0 }],
            },
            repair_cooldown_secs: 0.0,
            repair_in_progress: false,
            repair_penalty: false,
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

    #[test]
    fn on_screen_message_is_set_view_radar() {
        assert_eq!(
            on_screen_message(),
            ClientMessage::SetView { mode: ViewMode::Radar },
        );
    }
}
