use bevy::prelude::Resource;
use crate::messages::{SimSnapshot, ViewDirection};

#[derive(Resource)]
pub struct ShipState {
    red_alert: bool,
    pub view_direction: ViewDirection,
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
            view_direction: ViewDirection::Fore,
            x: 0.0,
            z: 0.0,
            yaw: 0.0,
            forward_speed: 0.0,
        }
    }

    pub fn toggle_red_alert(&mut self) {
        self.red_alert = !self.red_alert;
    }

    pub fn snapshot(&self) -> SimSnapshot {
        SimSnapshot { red_alert: self.red_alert, view_direction: self.view_direction.clone() }
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
    fn snapshot_reflects_current_state() {
        let mut s = ShipState::new();
        assert!(!s.snapshot().red_alert);
        s.toggle_red_alert();
        assert!(s.snapshot().red_alert);
    }

    #[test]
    fn view_direction_defaults_to_fore() {
        let s = ShipState::new();
        assert_eq!(s.view_direction, ViewDirection::Fore);
    }

    #[test]
    fn snapshot_includes_view_direction() {
        let mut s = ShipState::new();
        s.view_direction = ViewDirection::Port;
        assert_eq!(s.snapshot().view_direction, ViewDirection::Port);
    }
}
