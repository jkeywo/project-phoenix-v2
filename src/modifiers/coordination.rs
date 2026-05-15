use std::collections::HashMap;

use bevy::prelude::*;

use crate::entity_spawner::{EntityUuid, RegionEffectsSection};
use crate::flag_kind::FlagKind;
use crate::lobby::CurrentPhase;
use crate::messages::{Console, GamePhase, ModifierSlot, ModifierSource};
use crate::modifiers::{Modifier, ShipModifiers};
use crate::power_system::PowerSystem;
use crate::region_effects::RegionEffectKind;
use crate::region_plugin::{RegionEntered, RegionExited, RegionMembership};
use crate::simulation::{PowerMultiplierResource, ShipPowerSystem};

/// Single owner of `ShipModifiers` lifecycle.
///
/// Registers `ShipModifiers` as the sole `init_resource` call site, replacing
/// the duplicate registrations that existed in `SimulationPlugin` and
/// `RegionPlugin`. All other plugins read `Res<ShipModifiers>` or write
/// `ResMut<ShipModifiers>` after this plugin has initialised the resource.
///
/// Also owns the power → modifiers translator system.
pub struct ModifierCoordinationPlugin;

impl Plugin for ModifierCoordinationPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ShipModifiers>();
    }
}

/// Power → modifier translator system.
///
/// Reads the current `ShipPowerSystem` and `PowerMultiplierResource` each
/// frame and writes the corresponding power-level modifiers into
/// `ShipModifiers`.  This is the single routing point for power-side modifier
/// writes — `handle_power_messages` and `tick_power_system` in simulation.rs
/// no longer touch `ShipModifiers` directly.
///
/// The system is registered by `SimulationPlugin` (not by
/// `ModifierCoordinationPlugin`) so it can be chained after the power‑handling
/// systems with explicit `.after()` ordering.
pub fn translate_power_modifiers(
    phase: Res<CurrentPhase>,
    power: Res<ShipPowerSystem>,
    mult_cfg: Option<Res<PowerMultiplierResource>>,
    mut modifiers: ResMut<ShipModifiers>,
) {
    if phase.0 != GamePhase::InProgress {
        return;
    }
    let Some(mult_cfg) = mult_cfg else { return };
    apply_power_modifiers(&mut modifiers, &power.0, &mult_cfg.multipliers);
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

/// Apply region effects from a single region to `ShipModifiers`.
///
/// Called on region enter. Each effect kind maps to a modifier or flag update.
/// `DamageZone` and `BlocksImpulse` are not modifier effects — they are
/// applied directly by the region plugin and are skipped here.
pub fn apply_region_effects(
    modifiers: &mut ShipModifiers,
    region_uuid: uuid::Uuid,
    effects: &[RegionEffectKind],
) {
    let source = ModifierSource::RegionEffect { uuid: region_uuid };
    for effect in effects {
        match effect {
            RegionEffectKind::DamageZone { .. } | RegionEffectKind::BlocksImpulse => {}
            RegionEffectKind::CommsJam => {
                modifiers.add_flag(source.clone(), FlagKind::CommsJammed);
            }
            RegionEffectKind::SensorBlind => {
                modifiers.add_flag(source.clone(), FlagKind::SensorBlind);
            }
            RegionEffectKind::RadarDampening { multiplier } => {
                modifiers.add_or_update(Modifier {
                    source: source.clone(),
                    slot: ModifierSlot::RadarRange,
                    bonus: *multiplier,
                });
            }
            RegionEffectKind::SlowZone { thrust_modifier, yaw_rate_modifier } => {
                if let Some(bonus) = thrust_modifier {
                    modifiers.add_or_update(Modifier {
                        source: source.clone(),
                        slot: ModifierSlot::MaxSpeed,
                        bonus: *bonus,
                    });
                }
                if let Some(bonus) = yaw_rate_modifier {
                    modifiers.add_or_update(Modifier {
                        source: source.clone(),
                        slot: ModifierSlot::MaxYawRate,
                        bonus: *bonus,
                    });
                }
            }
        }
    }
}

/// Region → modifier translator system.
///
/// Reads `RegionEntered` / `RegionExited` events each frame and updates
/// `ShipModifiers` accordingly. On enter it calls `apply_region_effects`;
/// on exit it calls `clear_source` to remove all modifiers and flags for
/// that region's source UUID.
///
/// This is the single routing point for region-side modifier writes.
pub fn translate_region_modifiers(
    mut entered: MessageReader<RegionEntered>,
    mut exited: MessageReader<RegionExited>,
    region_query: Query<(&EntityUuid, &RegionEffectsSection)>,
    membership: Res<RegionMembership>,
    mut modifiers: ResMut<ShipModifiers>,
) {
    for ev in exited.read() {
        let uuid_str = match membership.region_uuids.get(&ev.region_entity) {
            Some(s) => s,
            None => continue,
        };
        let uuid = match uuid::Uuid::parse_str(uuid_str) {
            Ok(u) => u,
            Err(_) => continue,
        };
        modifiers.clear_source(&ModifierSource::RegionEffect { uuid });
    }

    for ev in entered.read() {
        let Ok((uuid_comp, effects)) = region_query.get(ev.region_entity) else {
            continue;
        };
        let uuid = match uuid::Uuid::parse_str(&uuid_comp.0) {
            Ok(u) => u,
            Err(_) => continue,
        };
        apply_region_effects(&mut modifiers, uuid, &effects.0);
    }
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

    // ── apply_region_effects tests ─────────────────────────────────────

    use crate::flag_kind::FlagKind;
    use crate::region_effects::RegionEffectKind;

    #[test]
    fn enter_radar_dampening_adds_radar_range_modifier_with_correct_source() {
        let mut mods = ShipModifiers::new();
        let uuid = uuid::Uuid::from_u128(1);
        apply_region_effects(&mut mods, uuid, &[RegionEffectKind::RadarDampening { multiplier: -0.3 }]);
        let expected = 1.0 / 1.3;
        assert!((mods.get(&ModifierSlot::RadarRange) - expected).abs() < 1e-6);
        // Verify source UUID is correct by removing it
        mods.clear_source(&ModifierSource::RegionEffect { uuid: uuid::Uuid::from_u128(1) });
        assert!((mods.get(&ModifierSlot::RadarRange) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn enter_slow_zone_thrust_registers_maxspeed() {
        let mut mods = ShipModifiers::new();
        let uuid = uuid::Uuid::from_u128(1);
        apply_region_effects(&mut mods, uuid, &[RegionEffectKind::SlowZone { thrust_modifier: Some(-0.5), yaw_rate_modifier: None }]);
        assert!((mods.get(&ModifierSlot::MaxSpeed) - (1.0 / 1.5)).abs() < 1e-6);
        assert!((mods.get(&ModifierSlot::MaxYawRate) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn enter_slow_zone_yaw_registers_maxyawrate() {
        let mut mods = ShipModifiers::new();
        let uuid = uuid::Uuid::from_u128(1);
        apply_region_effects(&mut mods, uuid, &[RegionEffectKind::SlowZone { thrust_modifier: None, yaw_rate_modifier: Some(-0.3) }]);
        assert!((mods.get(&ModifierSlot::MaxYawRate) - (1.0 / 1.3)).abs() < 1e-6);
        assert!((mods.get(&ModifierSlot::MaxSpeed) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn enter_slow_zone_both_fields_registers_both_slots() {
        let mut mods = ShipModifiers::new();
        let uuid = uuid::Uuid::from_u128(1);
        apply_region_effects(&mut mods, uuid, &[RegionEffectKind::SlowZone { thrust_modifier: Some(-0.5), yaw_rate_modifier: Some(-0.3) }]);
        assert!((mods.get(&ModifierSlot::MaxSpeed) - (1.0 / 1.5)).abs() < 1e-6);
        assert!((mods.get(&ModifierSlot::MaxYawRate) - (1.0 / 1.3)).abs() < 1e-6);
    }

    #[test]
    fn enter_comms_jam_sets_flag() {
        let mut mods = ShipModifiers::new();
        apply_region_effects(&mut mods, uuid::Uuid::from_u128(1), &[RegionEffectKind::CommsJam]);
        assert!(mods.has_flag(&FlagKind::CommsJammed));
    }

    #[test]
    fn enter_sensor_blind_sets_flag() {
        let mut mods = ShipModifiers::new();
        apply_region_effects(&mut mods, uuid::Uuid::from_u128(1), &[RegionEffectKind::SensorBlind]);
        assert!(mods.has_flag(&FlagKind::SensorBlind));
    }

    #[test]
    fn multiple_overlapping_regions_or_aggregate_flags() {
        let mut mods = ShipModifiers::new();
        let uuid1 = uuid::Uuid::from_u128(1);
        let uuid2 = uuid::Uuid::from_u128(2);
        apply_region_effects(&mut mods, uuid1, &[RegionEffectKind::CommsJam]);
        apply_region_effects(&mut mods, uuid2, &[RegionEffectKind::CommsJam]);
        assert!(mods.has_flag(&FlagKind::CommsJammed));
        mods.remove_flag(ModifierSource::RegionEffect { uuid: uuid1 }, FlagKind::CommsJammed);
        assert!(mods.has_flag(&FlagKind::CommsJammed), "flag should remain after removing first source");
        mods.remove_flag(ModifierSource::RegionEffect { uuid: uuid2 }, FlagKind::CommsJammed);
        assert!(!mods.has_flag(&FlagKind::CommsJammed), "flag should clear after removing last source");
    }

    #[test]
    fn multiple_effects_in_one_region_all_applied() {
        let mut mods = ShipModifiers::new();
        let uuid = uuid::Uuid::from_u128(1);
        apply_region_effects(&mut mods, uuid, &[
            RegionEffectKind::CommsJam,
            RegionEffectKind::SensorBlind,
            RegionEffectKind::RadarDampening { multiplier: -0.3 },
        ]);
        assert!(mods.has_flag(&FlagKind::CommsJammed));
        assert!(mods.has_flag(&FlagKind::SensorBlind));
        assert!((mods.get(&ModifierSlot::RadarRange) - (1.0 / 1.3)).abs() < 1e-6);
    }

    #[test]
    fn damage_zone_and_blocks_impulse_do_not_write_modifiers() {
        let mut mods = ShipModifiers::new();
        let uuid = uuid::Uuid::from_u128(1);
        apply_region_effects(&mut mods, uuid, &[
            RegionEffectKind::DamageZone { dps: 50.0 },
            RegionEffectKind::BlocksImpulse,
        ]);
        assert!((mods.get(&ModifierSlot::MaxSpeed) - 1.0).abs() < 1e-6);
        assert!((mods.get(&ModifierSlot::RadarRange) - 1.0).abs() < 1e-6);
        assert!(!mods.has_flag(&FlagKind::CommsJammed));
        assert!(!mods.has_flag(&FlagKind::SensorBlind));
    }
}
