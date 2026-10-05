//! Attack-time accounting for the ship-wide strike reserve.
//!
//! Call only after a weapon has passed every no-fire gate. A beam calls once
//! when its cycle starts; each emitted projectile calls at its actual launch.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StrikeWeaponConfig {
    pub cost: f32,
    pub damage_multiplier: f32,
}

impl StrikeWeaponConfig {
    pub fn valid(&self) -> bool {
        self.cost.is_finite()
            && self.cost > 0.0
            && self.damage_multiplier.is_finite()
            && self.damage_multiplier >= 1.0
    }
}

/// Continuation state, shared by every firing family on this ship.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct StrikeBoost {
    pub enabled: bool,
    pub depleted: bool,
}

impl StrikeBoost {
    pub fn set(&mut self, enabled: bool) {
        self.enabled = enabled;
        self.depleted = false;
    }

    /// An unauthored attack fires normally. An authored attack that cannot pay
    /// also fires normally and shuts boost off until another explicit order.
    pub fn fire(&mut self, charge: &mut f32, config: Option<&StrikeWeaponConfig>) -> f32 {
        let Some(config) = config.filter(|_| self.enabled) else {
            return 1.0;
        };
        if !config.valid() || !charge.is_finite() || *charge < config.cost {
            self.enabled = false;
            self.depleted = true;
            return 1.0;
        }
        *charge -= config.cost;
        config.damage_multiplier
    }
}

#[cfg(test)]
#[path = "strike_reserve_tests.rs"]
mod tests;
