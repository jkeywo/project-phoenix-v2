use crate::messages::{ViewDirection, ViewMode};
use crate::ship::viewscreen::{source_system_for_view_mode, ViewscreenArbiter, ViewscreenRequest};
use bevy::prelude::Resource;

#[derive(Resource)]
pub struct ShipState {
    red_alert: bool,
    pub view_mode: ViewMode,
    captain_view_direction: ViewDirection,
    viewscreen: ViewscreenArbiter,
    /// Ship position (x, z)
    pub x: f32,
    pub z: f32,
    /// Yaw angle in radians (0 = facing negative Z)
    pub yaw: f32,
    /// Current forward speed
    pub forward_speed: f32,
    /// Current visual banking roll angle in radians (leans into turns).
    pub roll: f32,
    /// Current phaser emitter frequency (0.0–1.0). Changed by `SetPhaserFrequency`.
    pub phaser_frequency: f32,
}

impl Default for ShipState {
    fn default() -> Self {
        Self::new()
    }
}

impl ShipState {
    pub fn new() -> Self {
        Self {
            red_alert: false,
            view_mode: ViewMode::Camera(ViewDirection::Fore),
            captain_view_direction: ViewDirection::Fore,
            viewscreen: ViewscreenArbiter::new(),
            x: 0.0,
            z: 0.0,
            yaw: 0.0,
            forward_speed: 0.0,
            roll: 0.0,
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

    pub fn request_view_mode(&mut self, mode: ViewMode) {
        let requester = source_system_for_view_mode(&mode);
        self.request_view_mode_from(requester, mode);
    }

    pub fn request_view_mode_from(&mut self, requester: crate::messages::SystemId, mode: ViewMode) {
        let resolution = self
            .viewscreen
            .request_channel_2(ViewscreenRequest { requester, mode });
        self.view_mode = resolution.mode;
        self.captain_view_direction = self.viewscreen.captain_view_direction();
    }

    pub fn show_view_mode(&mut self, mode: ViewMode) {
        let requester = source_system_for_view_mode(&mode);
        let resolution = self
            .viewscreen
            .show_channel_2(ViewscreenRequest { requester, mode });
        self.view_mode = resolution.mode;
        self.captain_view_direction = self.viewscreen.captain_view_direction();
    }

    pub fn restore_captain_view(&mut self) {
        let resolution = self.viewscreen.restore_captain_view();
        self.view_mode = resolution.mode;
        self.captain_view_direction = self.viewscreen.captain_view_direction();
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

    #[test]
    fn view_mode_defaults_to_camera_fore() {
        let s = ShipState::new();
        assert_eq!(s.view_mode, ViewMode::Camera(ViewDirection::Fore));
    }

    #[test]
    fn non_camera_request_toggles_back_to_last_captain_camera() {
        let mut s = ShipState::new();
        s.request_view_mode(ViewMode::Camera(ViewDirection::Aft));
        s.request_view_mode(ViewMode::Radar);
        assert_eq!(s.view_mode, ViewMode::Radar);
        s.request_view_mode(ViewMode::Radar);
        assert_eq!(s.view_mode, ViewMode::Camera(ViewDirection::Aft));
    }

    #[test]
    fn captain_camera_request_updates_restore_target_even_from_overlay() {
        let mut s = ShipState::new();
        s.request_view_mode(ViewMode::Radar);
        s.request_view_mode(ViewMode::Camera(ViewDirection::Port));
        s.request_view_mode(ViewMode::NavigationChart);
        s.request_view_mode(ViewMode::NavigationChart);
        assert_eq!(s.view_mode, ViewMode::Camera(ViewDirection::Port));
    }

}
