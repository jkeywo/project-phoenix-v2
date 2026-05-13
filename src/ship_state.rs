use bevy::prelude::Resource;
use crate::flag_kind::FlagKind;
use crate::messages::{SimSnapshot, ViewDirection, ViewMode};

#[derive(Resource)]
pub struct ShipState {
    red_alert: bool,
    pub view_mode: ViewMode,
    /// Ship position (x, z)
    pub x: f32,
    pub z: f32,
    /// Yaw angle in radians (0 = facing negative Z)
    pub yaw: f32,
    /// Current forward speed
    pub forward_speed: f32,
}

impl ShipState {
    pub fn new() -> Self {
        Self {
            red_alert: false,
            view_mode: ViewMode::Camera(ViewDirection::Fore),
            x: 0.0,
            z: 0.0,
            yaw: 0.0,
            forward_speed: 0.0,
        }
    }

    pub fn toggle_red_alert(&mut self) {
        self.red_alert = !self.red_alert;
    }

    /// Read-only accessor for the red-alert flag (the field itself is
    /// private). Used by the viewscreen border plugin to drive the
    /// alert texture swap and vignette pulse.
    pub fn red_alert(&self) -> bool {
        self.red_alert
    }

    pub fn snapshot(&self, hull_integrity: f32, power_levels: (u8, u8, u8), flags: Vec<FlagKind>) -> SimSnapshot {
        SimSnapshot {
            red_alert: self.red_alert,
            view_mode: self.view_mode.clone(),
            ship_x: self.x,
            ship_z: self.z,
            ship_yaw: self.yaw,
            hull_integrity,
            power_levels,
            flags,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggle_red_alert_flips_state() {
        let mut s = ShipState::new();
        assert!(!s.red_alert);
        s.toggle_red_alert();
        assert!(s.red_alert);
    }

    #[test]
    fn double_toggle_restores_original_state() {
        let mut s = ShipState::new();
        s.toggle_red_alert();
        s.toggle_red_alert();
        assert!(!s.red_alert);
    }

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-6
    }

    #[test]
    fn snapshot_reflects_current_state() {
        let mut s = ShipState::new();
        assert!(!s.snapshot(100.0, (2, 2, 2), vec![]).red_alert);
        s.toggle_red_alert();
        assert!(s.snapshot(100.0, (2, 2, 2), vec![]).red_alert);
    }

    #[test]
    fn view_mode_defaults_to_camera_fore() {
        let s = ShipState::new();
        assert_eq!(s.view_mode, ViewMode::Camera(ViewDirection::Fore));
    }

    #[test]
    fn snapshot_includes_view_mode() {
        let mut s = ShipState::new();
        s.view_mode = ViewMode::Camera(ViewDirection::Port);
        assert_eq!(s.snapshot(100.0, (2, 2, 2), vec![]).view_mode, ViewMode::Camera(ViewDirection::Port));
    }

    #[test]
    fn snapshot_includes_ship_position_and_yaw() {
        let mut s = ShipState::new();
        s.x = 3.0;
        s.z = -7.5;
        s.yaw = 1.25;
        let snap = s.snapshot(100.0, (2, 2, 2), vec![]);
        assert_eq!(snap.ship_x, 3.0);
        assert_eq!(snap.ship_z, -7.5);
        assert_eq!(snap.ship_yaw, 1.25);
    }

    #[test]
    fn snapshot_view_mode_radar_round_trips_through_state() {
        let mut s = ShipState::new();
        s.view_mode = ViewMode::Radar;
        assert_eq!(s.snapshot(100.0, (2, 2, 2), vec![]).view_mode, ViewMode::Radar);
    }

    #[test]
    fn snapshot_includes_hull_integrity() {
        let s = ShipState::new();
        assert!(near(s.snapshot(75.0, (2, 2, 2), vec![]).hull_integrity, 75.0));
    }

    #[test]
    fn snapshot_includes_power_levels() {
        let s = ShipState::new();
        assert_eq!(s.snapshot(100.0, (3, 4, 1), vec![]).power_levels, (3, 4, 1));
    }
}
