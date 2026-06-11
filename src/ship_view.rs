use bevy::prelude::*;
use crate::messages::{Console, ConsoleHullStatus, ServerMessage, ViewMode};

/// Shared ship state broadcast to all consoles every 10Hz via `SimState`.
///
/// This is the client-side mirror of the fields every console needs: pose,
/// red-alert, view mode, power levels, and impulse charge. Per-console panels
/// read from this resource instead of the `ClientSimState` god-resource.
#[derive(Clone, Debug, PartialEq, Resource)]
pub struct ShipView {
    pub red_alert: bool,
    pub view_mode: ViewMode,
    pub ship_x: f32,
    pub ship_z: f32,
    pub ship_yaw: f32,
    /// Current forward speed in world units per second (negative = reversing).
    pub forward_speed: f32,
    pub hull_fraction: f32,
    pub power_levels: (u8, u8, u8),
    pub impulse_charge_progress: f32,
    /// Per-console hull status, populated from `SimSnapshot`. Empty when the
    /// ship config has no per-console hull entries.
    pub console_hull: Vec<ConsoleHullStatus>,
    /// Optimistic camera direction written by `on_dir_selected` when the
    /// captain presses a direction button.  While `Some`, `ShipView::apply`
    /// will not overwrite `view_mode` from a `SimState` unless the server
    /// confirms the same direction (or a non-camera mode arrives).  This
    /// prevents a stale in-flight `SimState` (generated before the server
    /// processed `SetView`) from reverting the button highlight one frame
    /// after the captain pressed it.
    pub pending_view_mode: Option<ViewMode>,
}

impl Default for ShipView {
    fn default() -> Self {
        Self {
            red_alert: false,
            view_mode: ViewMode::default(),
            ship_x: 0.0,
            ship_z: 0.0,
            ship_yaw: 0.0,
            forward_speed: 0.0,
            hull_fraction: 1.0,
            power_levels: (2, 2, 2),
            impulse_charge_progress: 0.0,
            console_hull: vec![],
            pending_view_mode: None,
        }
    }
}

impl ShipView {
    /// Returns `(current_hp, max_hp)` for the given console, or `None` if the
    /// ship config has no hull entry for it.
    pub fn hull_for(&self, console: &Console) -> Option<(f32, f32)> {
        self.console_hull.iter()
            .find(|s| &s.console == console)
            .map(|s| (s.current, s.max_hp))
    }
}

impl ShipView {
    /// True iff the captain's view direction selector should highlight
    /// the given direction. Radar mode highlights nothing in the cross.
    pub fn is_active_camera_direction(&self, direction: &crate::messages::ViewDirection) -> bool {
        matches!(&self.view_mode, ViewMode::Camera(d) if d == direction)
    }

    /// Apply one inbound server message to this view, updating whichever
    /// fields the message carries.
    pub fn apply(&mut self, msg: &ServerMessage) {
        match msg {
            ServerMessage::SimState { snapshot } => {
                self.red_alert = snapshot.red_alert;
                // Reconcile view_mode against any pending optimistic selection.
                //
                // If the captain pressed a direction button before the server
                // had a chance to confirm it, `pending_view_mode` holds the
                // desired direction.  We skip overwriting `view_mode` until
                // either:
                //   (a) the server confirms the same direction, or
                //   (b) a non-camera mode arrives (e.g. Helm sets Radar) —
                //       external mode changes always win.
                let server_is_camera = matches!(snapshot.view_mode, ViewMode::Camera(_));
                match &self.pending_view_mode {
                    None => {
                        self.view_mode = snapshot.view_mode.clone();
                    }
                    Some(pending) => {
                        if !server_is_camera || snapshot.view_mode == *pending {
                            // Confirmed or superseded — clear pending and accept.
                            self.pending_view_mode = None;
                            self.view_mode = snapshot.view_mode.clone();
                        }
                        // else: stale SimState still showing old camera direction;
                        // keep the optimistic view_mode until the server catches up.
                    }
                }
                self.ship_x = snapshot.ship_x;
                self.ship_z = snapshot.ship_z;
                self.ship_yaw = snapshot.ship_yaw;
                self.forward_speed = snapshot.forward_speed;
                self.power_levels = snapshot.power_levels;
                self.impulse_charge_progress = snapshot.impulse_charge_progress;
                let total_max: f32 = snapshot.console_hull.iter().map(|c| c.max_hp).sum();
                let total_cur: f32 = snapshot.console_hull.iter().map(|c| c.current).sum();
                self.hull_fraction = if total_max > 0.0 {
                    (total_cur / total_max).clamp(0.0, 1.0)
                } else {
                    1.0
                };
                self.console_hull = snapshot.console_hull.clone();
            }
            ServerMessage::Welcome { .. } => {
                let _ = std::mem::replace(self, ShipView::default());
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::{RadarStateSnapshot, ShipClientConfig, SimSnapshot, ViewDirection};

    fn base_snapshot() -> SimSnapshot {
        SimSnapshot {
            red_alert: false,
            view_mode: ViewMode::default(),
            ship_x: 0.0,
            ship_z: 0.0,
            ship_yaw: 0.0,
            forward_speed: 0.0,
            power_levels: (2, 2, 2),
            impulse_charge_progress: 0.0,
            flags: vec![],
            entity_states: vec![],
            radar_state: RadarStateSnapshot::default(),
            engine_thrust: 0.0,
            console_hull: vec![],
        }
    }

    #[test]
    fn sim_state_updates_red_alert() {
        let mut view = ShipView::default();
        view.apply(&ServerMessage::SimState {
            snapshot: SimSnapshot { red_alert: true, ..base_snapshot() },
        });
        assert!(view.red_alert);
    }

    #[test]
    fn sim_state_clears_red_alert() {
        let mut view = ShipView { red_alert: true, ..Default::default() };
        view.apply(&ServerMessage::SimState {
            snapshot: SimSnapshot { red_alert: false, ..base_snapshot() },
        });
        assert!(!view.red_alert);
    }

    #[test]
    fn welcome_resets_to_default() {
        let mut view = ShipView { red_alert: true, ship_x: 100.0, ..Default::default() };
        view.apply(&ServerMessage::Welcome {
            state: crate::messages::GameState {
                phase: crate::messages::GamePhase::InProgress,
                players: vec![],
                complexity: std::collections::HashMap::new(),
                world: None,
            },
            ship_stations: crate::stations_config::ShipStations::default(),
            ship_config: ShipClientConfig::default(),
        });
        assert!(!view.red_alert);
        assert_eq!(view.ship_x, 0.0);
    }

    #[test]
    fn sim_state_updates_position_and_power() {
        let mut view = ShipView::default();
        view.apply(&ServerMessage::SimState {
            snapshot: SimSnapshot {
                ship_x: 12.5,
                ship_z: -7.25,
                power_levels: (3, 2, 4),
                ..base_snapshot()
            },
        });
        assert_eq!(view.ship_x, 12.5);
        assert_eq!(view.ship_z, -7.25);
        assert_eq!(view.power_levels, (3, 2, 4));
    }

    #[test]
    fn sim_state_updates_forward_speed() {
        let mut view = ShipView::default();
        assert_eq!(view.forward_speed, 0.0);
        view.apply(&ServerMessage::SimState {
            snapshot: SimSnapshot {
                forward_speed: 18.5,
                ..base_snapshot()
            },
        });
        assert!((view.forward_speed - 18.5).abs() < 1e-6);
    }

    #[test]
    fn sim_state_updates_forward_speed_negative_propagates() {
        let mut view = ShipView::default();
        view.apply(&ServerMessage::SimState {
            snapshot: SimSnapshot {
                forward_speed: -5.0,
                ..base_snapshot()
            },
        });
        assert!((view.forward_speed - (-5.0)).abs() < 1e-6);
    }

    // ── pending_view_mode reconciliation ────────────────────────────────────

    #[test]
    fn sim_state_does_not_overwrite_pending_camera_direction() {
        // Simulate: user pressed Port (optimistic), stale SimState still says Fore.
        let mut view = ShipView {
            view_mode: ViewMode::Camera(ViewDirection::Port),
            pending_view_mode: Some(ViewMode::Camera(ViewDirection::Port)),
            ..Default::default()
        };
        view.apply(&ServerMessage::SimState {
            snapshot: SimSnapshot {
                view_mode: ViewMode::Camera(ViewDirection::Fore),
                ..base_snapshot()
            },
        });
        assert_eq!(
            view.view_mode,
            ViewMode::Camera(ViewDirection::Port),
            "stale SimState must not overwrite pending optimistic direction"
        );
        assert!(view.pending_view_mode.is_some(), "pending should still be set");
    }

    #[test]
    fn sim_state_clears_pending_when_server_confirms_direction() {
        let mut view = ShipView {
            view_mode: ViewMode::Camera(ViewDirection::Port),
            pending_view_mode: Some(ViewMode::Camera(ViewDirection::Port)),
            ..Default::default()
        };
        // Server confirms Port.
        view.apply(&ServerMessage::SimState {
            snapshot: SimSnapshot {
                view_mode: ViewMode::Camera(ViewDirection::Port),
                ..base_snapshot()
            },
        });
        assert_eq!(view.view_mode, ViewMode::Camera(ViewDirection::Port));
        assert!(view.pending_view_mode.is_none(), "pending should be cleared after confirmation");
    }

    #[test]
    fn non_camera_sim_state_overrides_pending_camera() {
        // Helm sets Radar while a pending Camera(Port) is outstanding.
        let mut view = ShipView {
            view_mode: ViewMode::Camera(ViewDirection::Port),
            pending_view_mode: Some(ViewMode::Camera(ViewDirection::Port)),
            ..Default::default()
        };
        view.apply(&ServerMessage::SimState {
            snapshot: SimSnapshot {
                view_mode: ViewMode::Radar,
                ..base_snapshot()
            },
        });
        assert_eq!(view.view_mode, ViewMode::Radar, "Radar from Helm must win over pending Camera");
        assert!(view.pending_view_mode.is_none(), "pending cleared when non-camera mode arrives");
    }

    #[test]
    fn sim_state_updates_view_mode_normally_when_no_pending() {
        let mut view = ShipView::default(); // pending_view_mode is None
        view.apply(&ServerMessage::SimState {
            snapshot: SimSnapshot {
                view_mode: ViewMode::Camera(ViewDirection::Aft),
                ..base_snapshot()
            },
        });
        assert_eq!(view.view_mode, ViewMode::Camera(ViewDirection::Aft));
        assert!(view.pending_view_mode.is_none());
    }

    #[test]
    fn is_active_camera_direction_only_matches_in_camera_mode() {
        use crate::messages::ViewDirection;
        let mut view = ShipView::default();
        // Default view mode is Camera(Fore).
        assert!( view.is_active_camera_direction(&ViewDirection::Fore));
        assert!(!view.is_active_camera_direction(&ViewDirection::Aft));

        view.view_mode = ViewMode::Camera(ViewDirection::Port);
        assert!( view.is_active_camera_direction(&ViewDirection::Port));
        assert!(!view.is_active_camera_direction(&ViewDirection::Fore));

        view.view_mode = ViewMode::Radar;
        for d in [ViewDirection::Fore, ViewDirection::Aft, ViewDirection::Port, ViewDirection::Starboard] {
            assert!(!view.is_active_camera_direction(&d), "Radar mode highlights no cross arrow");
        }
    }
}
