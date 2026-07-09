use crate::messages::{CameraView, ViewMode};
use crate::ship::viewscreen::{source_system_for_view_mode, ViewscreenArbiter, ViewscreenRequest};
use bevy::prelude::Component;

/// Per-entity physics state component for every ship entity (player and NPC).
///
/// Replaces the `x`, `z`, `yaw`, `forward_speed`, and `roll` fields that were
/// previously on the singleton `ShipState` resource. Both the player ship and
/// NPC ships carry this component; the physics tick reads/writes it uniformly.
///
/// Pure per-entity Component post ship-parity audit; the legacy `Resource`
/// derive has been dropped since no production code reads a global
/// `Res<ShipPhysics>`.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq)]
pub struct ShipPhysics {
    /// X position in world space.
    pub x: f32,
    /// Z position in world space.
    pub z: f32,
    /// Yaw angle in radians (0 = facing negative Z).
    pub yaw: f32,
    /// Current forward speed (positive = forward, negative = reverse).
    pub forward_speed: f32,
    /// Current visual banking roll angle in radians (leans into turns).
    pub roll: f32,
}

/// Per-entity red-alert state for every ship entity (player and NPC).
///
/// Replaces the `red_alert` field that was previously on the singleton
/// `ShipState` resource. Added in issue #591.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ShipRedAlert(pub bool);

impl ShipRedAlert {
    pub fn toggle(&mut self) {
        self.0 = !self.0;
    }
}

/// Per-entity viewscreen mode state for every ship entity (player and NPC).
///
/// Replaces the `view_mode` field that was previously on the singleton
/// `ShipState` resource. Added in issue #591.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct ShipViewMode {
    pub view_mode: ViewMode,
    captain_view: CameraView,
    pub viewscreen: ViewscreenArbiter,
}

impl Default for ShipViewMode {
    fn default() -> Self {
        Self {
            view_mode: ViewMode::Camera(CameraView::default()),
            captain_view: CameraView::default(),
            viewscreen: ViewscreenArbiter::new(),
        }
    }
}

impl ShipViewMode {
    pub fn request_view_mode(&mut self, mode: ViewMode) {
        let requester = source_system_for_view_mode(&mode);
        self.request_view_mode_from(requester, mode);
    }

    pub fn request_view_mode_from(&mut self, requester: crate::messages::SystemId, mode: ViewMode) {
        let resolution = self
            .viewscreen
            .request_channel_2(ViewscreenRequest { requester, mode });
        self.view_mode = resolution.mode;
        self.captain_view = self.viewscreen.captain_view();
    }

    pub fn show_view_mode(&mut self, mode: ViewMode) {
        let requester = source_system_for_view_mode(&mode);
        let resolution = self
            .viewscreen
            .show_channel_2(ViewscreenRequest { requester, mode });
        self.view_mode = resolution.mode;
        self.captain_view = self.viewscreen.captain_view();
    }

    pub fn restore_captain_view(&mut self) {
        let resolution = self.viewscreen.restore_captain_view();
        self.view_mode = resolution.mode;
        self.captain_view = self.viewscreen.captain_view();
    }
}

/// Per-entity phaser emitter frequency (0.0–1.0).
///
/// Replaces the `phaser_frequency` field that was previously on the singleton
/// `ShipState` resource.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct ShipPhaserFrequency(pub f32);

impl Default for ShipPhaserFrequency {
    fn default() -> Self {
        Self(0.5)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ship_red_alert_toggle() {
        let mut ra = ShipRedAlert(false);
        assert!(!ra.0);
        ra.toggle();
        assert!(ra.0);
        ra.toggle();
        assert!(!ra.0);
    }

    #[test]
    fn ship_view_mode_defaults_to_camera() {
        let vm = ShipViewMode::default();
        assert_eq!(vm.view_mode, ViewMode::Camera(CameraView::default()));
    }

    #[test]
    fn ship_view_mode_request_toggles_correctly() {
        let mut vm = ShipViewMode::default();
        vm.request_view_mode(ViewMode::Camera(CameraView::new("camera_aft")));
        vm.request_view_mode(ViewMode::Radar);
        assert_eq!(vm.view_mode, ViewMode::Radar);
        vm.request_view_mode(ViewMode::Radar);
        assert_eq!(
            vm.view_mode,
            ViewMode::Camera(CameraView::new("camera_aft"))
        );
    }
}
