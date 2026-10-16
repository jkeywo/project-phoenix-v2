//! Ephemeral receipt of the actual once-per-tick Radio listening work.
//! It is not machinery state, another payment, a route or an observation.
use bevy::prelude::Component;
use phoenix_model::identity::SystemId;

#[derive(Component, Clone, Debug, Default)]
pub(crate) struct PaidRadioWork {
    owner: Option<SystemId>,
    tick: Option<u64>,
    fraction: f32,
}
impl PaidRadioWork {
    pub(super) fn record(&mut self, owner: &SystemId, tick: u64, fraction: f32) {
        self.owner = Some(owner.clone());
        self.tick = Some(tick);
        self.fraction = if fraction.is_finite() && (0.0..=1.0).contains(&fraction) {
            fraction
        } else {
            0.0
        };
    }
    /// Raw paid fraction, BEFORE Operation. A late consumer must independently
    /// prove current equipment/availability/Operation and use current poses.
    pub(crate) fn raw(&self, owner: &SystemId, tick: u64) -> f32 {
        if self.owner.as_ref() == Some(owner) && self.tick == Some(tick) {
            self.fraction
        } else {
            0.0
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn raw_receipt_has_exact_owner_clock_and_no_restored_credit() {
        let owner = SystemId("comms".into());
        let mut receipt = PaidRadioWork::default();
        assert_eq!(receipt.raw(&owner, 7), 0.0);
        receipt.record(&owner, 7, 0.4);
        assert_eq!(receipt.raw(&owner, 7), 0.4);
        assert_eq!(receipt.raw(&owner, 8), 0.0);
        assert_eq!(receipt.raw(&SystemId("other".into()), 7), 0.0);
        receipt = PaidRadioWork::default();
        assert_eq!(receipt.raw(&owner, 7), 0.0);
    }
}
