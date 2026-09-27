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
mod tests {
    use super::*;

    #[test]
    fn exact_charge_pays_once_then_insufficient_attack_shuts_every_weapon_off() {
        let weapon = StrikeWeaponConfig {
            cost: 5.0,
            damage_multiplier: 2.0,
        };
        let mut boost = StrikeBoost::default();
        let mut charge = 5.0;
        boost.set(true);
        assert_eq!(boost.fire(&mut charge, Some(&weapon)), 2.0);
        assert_eq!(charge, 0.0);
        assert_eq!(boost.fire(&mut charge, Some(&weapon)), 1.0);
        assert!(boost.depleted);
        charge = 10.0;
        assert_eq!(boost.fire(&mut charge, Some(&weapon)), 1.0);
        assert_eq!(charge, 10.0, "recharging does not re-enable boost");
        boost.set(true);
        assert_eq!(boost.fire(&mut charge, Some(&weapon)), 2.0);
        assert_eq!(charge, 5.0);
    }

    #[test]
    fn restored_boost_spends_identically_across_weapon_costs() {
        let cheap = StrikeWeaponConfig {
            cost: 3.0,
            damage_multiplier: 1.5,
        };
        let expensive = StrikeWeaponConfig {
            cost: 8.0,
            damage_multiplier: 3.0,
        };
        let mut live = StrikeBoost::default();
        live.set(true);
        let mut restored: StrikeBoost =
            serde_json::from_str(&serde_json::to_string(&live).unwrap()).unwrap();
        let mut live_charge = 10.0;
        let mut restored_charge = live_charge;
        for weapon in [&cheap, &expensive, &cheap] {
            assert_eq!(
                live.fire(&mut live_charge, Some(weapon)),
                restored.fire(&mut restored_charge, Some(weapon))
            );
            assert_eq!(live_charge, restored_charge);
            assert_eq!(live, restored);
        }
        assert_eq!(
            live_charge, 7.0,
            "failed expensive shot disables the later cheap shot"
        );
    }
}
