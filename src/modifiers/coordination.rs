use std::collections::HashMap;

use bevy::prelude::*;

use crate::messages::{Console, ModifierSlot, ModifierSource};
use crate::modifiers::{Modifier, ShipModifiers};
use crate::power_system::PowerSystem;

/// Single owner of `ShipModifiers` lifecycle.
///
/// Registers `ShipModifiers` as the sole `init_resource` call site, replacing
/// the duplicate registrations that existed in `SimulationPlugin` and
/// `RegionPlugin`. All other plugins read `Res<ShipModifiers>` or write
/// `ResMut<ShipModifiers>` after this plugin has initialised the resource.
pub struct ModifierCoordinationPlugin;

impl Plugin for ModifierCoordinationPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ShipModifiers>();
    }
}

/// Apply power-level modifiers to `modifiers` based on the current `PowerSystem`
/// state and per-console multiplier config.
///
/// Registers one `Modifier` per console slot using `ModifierSource::Console`.
/// Re-registration replaces the previous entry (no stacking). Multiplier
/// arrays are indexed by power level 1–4 (1 maps to index 0).
pub fn apply_power_modifiers(
    modifiers: &mut ShipModifiers,
    power: &PowerSystem,
    multipliers: &HashMap<Console, [f32; 4]>,
) {
    let default_mult = [-0.5, 0.0, 0.25, 0.5];

    let helm_level = (power.helm as usize).saturating_sub(1).min(3);
    let helm_bonus = multipliers.get(&Console::Helm).unwrap_or(&default_mult)[helm_level];
    modifiers.add_or_update(Modifier {
        source: ModifierSource::Console(Console::Helm),
        slot: ModifierSlot::MaxSpeed,
        bonus: helm_bonus,
    });
    modifiers.add_or_update(Modifier {
        source: ModifierSource::Console(Console::Helm),
        slot: ModifierSlot::MaxYawRate,
        bonus: helm_bonus,
    });

    let weapons_level = (power.weapons as usize).saturating_sub(1).min(3);
    let weapons_bonus = multipliers.get(&Console::Tactical).unwrap_or(&default_mult)[weapons_level];
    modifiers.add_or_update(Modifier {
        source: ModifierSource::Console(Console::Tactical),
        slot: ModifierSlot::PhaserDamage,
        bonus: weapons_bonus,
    });

    let sensors_level = (power.sensors as usize).saturating_sub(1).min(3);
    let sensors_bonus = multipliers.get(&Console::Sensors).unwrap_or(&default_mult)[sensors_level];
    modifiers.add_or_update(Modifier {
        source: ModifierSource::Console(Console::Sensors),
        slot: ModifierSlot::RadarRange,
        bonus: sensors_bonus,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_multipliers() -> HashMap<Console, [f32; 4]> {
        let d = [-0.5f32, 0.0, 0.25, 0.5];
        HashMap::from([
            (Console::Helm, d),
            (Console::Tactical, d),
            (Console::Sensors, d),
        ])
    }

    #[test]
    fn power_level_2_gives_zero_bonus() {
        let mut mods = ShipModifiers::new();
        let mut power = PowerSystem::default();
        power.helm = 2;
        power.weapons = 2;
        power.sensors = 2;
        apply_power_modifiers(&mut mods, &power, &default_multipliers());
        assert_eq!(mods.get(&ModifierSlot::MaxSpeed), 1.0);
        assert_eq!(mods.get(&ModifierSlot::PhaserDamage), 1.0);
        assert_eq!(mods.get(&ModifierSlot::RadarRange), 1.0);
    }

    #[test]
    fn helm_power_4_gives_positive_bonus() {
        let mut mods = ShipModifiers::new();
        let mut power = PowerSystem::default();
        power.helm = 4;
        apply_power_modifiers(&mut mods, &power, &default_multipliers());
        assert!((mods.get(&ModifierSlot::MaxSpeed) - 1.5).abs() < 1e-6);
    }

    #[test]
    fn helm_power_1_gives_negative_bonus() {
        let mut mods = ShipModifiers::new();
        let mut power = PowerSystem::default();
        power.helm = 1;
        apply_power_modifiers(&mut mods, &power, &default_multipliers());
        // Negative bonus uses 1/(1+|bonus|): -0.5 → 1/1.5 ≈ 0.667
        let expected = 1.0 / 1.5f32;
        assert!((mods.get(&ModifierSlot::MaxSpeed) - expected).abs() < 1e-5);
    }

    #[test]
    fn apply_twice_does_not_stack() {
        let mut mods = ShipModifiers::new();
        let mut power = PowerSystem::default();
        power.helm = 4;
        let mult = default_multipliers();
        apply_power_modifiers(&mut mods, &power, &mult);
        apply_power_modifiers(&mut mods, &power, &mult);
        assert!((mods.get(&ModifierSlot::MaxSpeed) - 1.5).abs() < 1e-6);
    }
}
