use bevy::prelude::Resource;
use crate::flag_kind::FlagKind;
use crate::messages::{ConsoleHullStatus, EntityStateSnapshot, RadarStateSnapshot, SimSnapshot, ViewDirection, ViewMode};

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
    /// Current phaser emitter frequency (0.0–1.0). Changed by `SetPhaserFrequency`.
    pub phaser_frequency: f32,
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
            phaser_frequency: 0.5,
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

    pub fn snapshot(
        &self,
        hull_integrity: f32,
        power_levels: (u8, u8, u8),
        flags: Vec<FlagKind>,
        entity_states: Vec<EntityStateSnapshot>,
        radar_state: RadarStateSnapshot,
        impulse_charge_progress: f32,
        console_hull: Vec<ConsoleHullStatus>,
    ) -> SimSnapshot {
        SimSnapshot {
            red_alert: self.red_alert,
            view_mode: self.view_mode.clone(),
            ship_x: self.x,
            ship_z: self.z,
            ship_yaw: self.yaw,
            forward_speed: self.forward_speed,
            hull_integrity,
            power_levels,
            flags,
            entity_states,
            radar_state,
            impulse_charge_progress,
            console_hull,
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

    fn empty_entity_states() -> Vec<EntityStateSnapshot> { vec![] }
    fn default_radar() -> RadarStateSnapshot { RadarStateSnapshot::default() }

    fn snap(s: &ShipState, hull: f32, levels: (u8, u8, u8)) -> SimSnapshot {
        s.snapshot(hull, levels, vec![], empty_entity_states(), default_radar(), 0.0, vec![])
    }

    #[test]
    fn snapshot_reflects_current_state() {
        let mut s = ShipState::new();
        assert!(!snap(&s, 100.0, (2, 2, 2)).red_alert);
        s.toggle_red_alert();
        assert!(snap(&s, 100.0, (2, 2, 2)).red_alert);
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
        assert_eq!(snap(&s, 100.0, (2, 2, 2)).view_mode, ViewMode::Camera(ViewDirection::Port));
    }

    #[test]
    fn snapshot_includes_ship_position_and_yaw() {
        let mut s = ShipState::new();
        s.x = 3.0;
        s.z = -7.5;
        s.yaw = 1.25;
        let snap = s.snapshot(100.0, (2, 2, 2), vec![], empty_entity_states(), default_radar(), 0.0, vec![]);
        assert_eq!(snap.ship_x, 3.0);
        assert_eq!(snap.ship_z, -7.5);
        assert_eq!(snap.ship_yaw, 1.25);
    }

    #[test]
    fn snapshot_view_mode_radar_round_trips_through_state() {
        let mut s = ShipState::new();
        s.view_mode = ViewMode::Radar;
        assert_eq!(snap(&s, 100.0, (2, 2, 2)).view_mode, ViewMode::Radar);
    }

    #[test]
    fn snapshot_includes_hull_integrity() {
        let s = ShipState::new();
        assert!(near(snap(&s, 75.0, (2, 2, 2)).hull_integrity, 75.0));
    }

    #[test]
    fn snapshot_includes_power_levels() {
        let s = ShipState::new();
        assert_eq!(snap(&s, 100.0, (3, 4, 1)).power_levels, (3, 4, 1));
    }

    #[test]
    fn snapshot_includes_impulse_charge_progress() {
        let s = ShipState::new();
        let snap = s.snapshot(100.0, (2, 2, 2), vec![], empty_entity_states(), default_radar(), 0.5, vec![]);
        assert!((snap.impulse_charge_progress - 0.5).abs() < 1e-6);
    }

    #[test]
    fn snapshot_includes_forward_speed() {
        let mut s = ShipState::new();
        s.forward_speed = 22.5;
        let snap = snap(&s, 100.0, (2, 2, 2));
        assert!((snap.forward_speed - 22.5).abs() < 1e-6);
    }
}
