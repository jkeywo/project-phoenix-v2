use crate::messages::SimSnapshot;

pub struct ShipState {
    red_alert: bool,
}

impl ShipState {
    pub fn new() -> Self {
        Self { red_alert: false }
    }

    pub fn toggle_red_alert(&mut self) {
        self.red_alert = !self.red_alert;
    }

    pub fn snapshot(&self) -> SimSnapshot {
        SimSnapshot { red_alert: self.red_alert }
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
}
