use std::collections::HashMap;

use bevy::prelude::*;

use crate::entity_spawner::{EntityUuid, RegionEffectsSection};
use crate::flag_kind::FlagKind;
use crate::messages::{Console, ModifierSlot, ModifierSource};
use crate::modifiers::{Modifier, ShipModifiers};
use crate::power_system::PowerSystem;
use crate::region_effects::RegionEffectKind;
use crate::region_plugin::{RegionEntered, RegionExited, RegionMembership};
use crate::impulse::{ImpulsePhase, ImpulseState, IMPULSE_SPEED_MULTIPLIER};
use crate::power_plugin::{PowerMultiplierResource, ShipPowerSystem};
use crate::simulation::ShipImpulse;
use crate::ship_plugin::ImpulseConfigResource;

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
        app.init_resource::<ShipModifiers>()
            .add_observer(on_region_entered)
            .add_observer(on_region_exited);
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
    power: Res<ShipPowerSystem>,
    mult_cfg: Option<Res<PowerMultiplierResource>>,
    mut modifiers: ResMut<ShipModifiers>,
) {
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
            RegionEffectKind::DamageZone { .. } | RegionEffectKind::BlocksImpulse | RegionEffectKind::NebulaFog { .. } => {}
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

/// Apply impulse-drive modifiers to `modifiers` based on the current
/// `ImpulseState`.
///
/// When the impulse drive is active (`ImpulsePhase::Active`) it registers a
/// `MaxSpeed` modifier with `ModifierSource::ImpulseDrive` and a bonus that
/// yields `speed_multiplier` × max speed.
///
/// When the drive is idle or charging the modifier is removed, so any
/// previously applied impulse effect is cleaned up.
pub fn apply_impulse_to(modifiers: &mut ShipModifiers, impulse: &ImpulseState, speed_multiplier: f32) {
    if impulse.is_active() {
        modifiers.add_or_update(Modifier {
            source: ModifierSource::ImpulseDrive,
            slot: ModifierSlot::MaxSpeed,
            bonus: speed_multiplier - 1.0,
        });
    } else {
        modifiers.remove(&ModifierSource::ImpulseDrive, &ModifierSlot::MaxSpeed);
    }
}

/// Impulse → modifier translator system.
///
/// Reads `ShipImpulse` each frame and updates `ShipModifiers` via
/// `apply_impulse_to`. Change-detects the impulse phase so it only writes
/// to the modifier table on transitions, avoiding redundant events.
///
/// This is the single routing point for impulse-side modifier writes.
pub fn translate_impulse_modifiers(
    impulse: Res<ShipImpulse>,
    mut modifiers: ResMut<ShipModifiers>,
    impulse_config: Option<Res<ImpulseConfigResource>>,
    mut prev_phase: Local<Option<ImpulsePhase>>,
) {
    let current = impulse.0.phase;
    if Some(current) != *prev_phase {
        *prev_phase = Some(current);
        let speed_multiplier = impulse_config
            .as_deref()
            .map(|c| c.speed_multiplier)
            .unwrap_or(IMPULSE_SPEED_MULTIPLIER);
        apply_impulse_to(&mut modifiers, &impulse.0, speed_multiplier);
    }
}

/// Observer: applies region effects to `ShipModifiers` when the ship enters a region.
fn on_region_entered(
    trigger: On<RegionEntered>,
    region_query: Query<(&EntityUuid, &RegionEffectsSection)>,
    mut modifiers: ResMut<ShipModifiers>,
) {
    let ev = trigger.event();
    let Ok((uuid_comp, effects)) = region_query.get(ev.region_entity) else {
        return;
    };
    let uuid = match uuid::Uuid::parse_str(&uuid_comp.0) {
        Ok(u) => u,
        Err(_) => return,
    };
    apply_region_effects(&mut modifiers, uuid, &effects.0);
}

/// Observer: clears region effects from `ShipModifiers` when the ship exits a region.
fn on_region_exited(
    trigger: On<RegionExited>,
    membership: Res<RegionMembership>,
    mut modifiers: ResMut<ShipModifiers>,
) {
    let ev = trigger.event();
    let uuid_str = match membership.region_uuids.get(&ev.region_entity) {
        Some(s) => s,
        None => return,
    };
    let uuid = match uuid::Uuid::parse_str(uuid_str) {
        Ok(u) => u,
        Err(_) => return,
    };
    modifiers.clear_source(&ModifierSource::RegionEffect { uuid });
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
            RegionEffectKind::DamageZone { dps: 50.0, shield_pierce: 0.0 },
            RegionEffectKind::BlocksImpulse,
        ]);
        assert!((mods.get(&ModifierSlot::MaxSpeed) - 1.0).abs() < 1e-6);
        assert!((mods.get(&ModifierSlot::RadarRange) - 1.0).abs() < 1e-6);
        assert!(!mods.has_flag(&FlagKind::CommsJammed));
        assert!(!mods.has_flag(&FlagKind::SensorBlind));
    }

    // ── apply_impulse_to tests ──────────────────────────────────────────

use crate::impulse::{ImpulseState, IMPULSE_CHARGE_DURATION, IMPULSE_SPEED_MULTIPLIER};

    #[test]
    fn impulse_idle_does_not_write_modifier() {
        let mut mods = ShipModifiers::new();
        let impulse = ImpulseState::new();
        apply_impulse_to(&mut mods, &impulse, IMPULSE_SPEED_MULTIPLIER);
        assert_eq!(mods.get(&ModifierSlot::MaxSpeed), 1.0);
    }

    #[test]
    fn impulse_charging_does_not_write_modifier() {
        let mut mods = ShipModifiers::new();
        let mut impulse = ImpulseState::new();
        impulse.start_charge();
        impulse.tick(1.0, IMPULSE_CHARGE_DURATION);
        apply_impulse_to(&mut mods, &impulse, IMPULSE_SPEED_MULTIPLIER);
        assert_eq!(mods.get(&ModifierSlot::MaxSpeed), 1.0);
    }

    #[test]
    fn impulse_active_writes_maxspeed_with_correct_bonus() {
        let mut mods = ShipModifiers::new();
        let mut impulse = ImpulseState::new();
        impulse.start_charge();
        impulse.tick(IMPULSE_CHARGE_DURATION, IMPULSE_CHARGE_DURATION);
        assert!(impulse.is_active());
        apply_impulse_to(&mut mods, &impulse, IMPULSE_SPEED_MULTIPLIER);
        let expected = 1.0 + (IMPULSE_SPEED_MULTIPLIER - 1.0);
        assert!((mods.get(&ModifierSlot::MaxSpeed) - expected).abs() < 1e-6);
    }

    #[test]
    fn impulse_active_source_is_impulsedrive() {
        let mut mods = ShipModifiers::new();
        let mut impulse = ImpulseState::new();
        impulse.start_charge();
        impulse.tick(IMPULSE_CHARGE_DURATION, IMPULSE_CHARGE_DURATION);
        apply_impulse_to(&mut mods, &impulse, IMPULSE_SPEED_MULTIPLIER);
        // Verify source identity by clearing the ImpulseDrive source
        mods.clear_source(&ModifierSource::ImpulseDrive);
        assert_eq!(mods.get(&ModifierSlot::MaxSpeed), 1.0);
    }

    #[test]
    fn impulse_cancel_removes_modifier() {
        let mut mods = ShipModifiers::new();
        let mut impulse = ImpulseState::new();
        // Activate impulse
        impulse.start_charge();
        impulse.tick(IMPULSE_CHARGE_DURATION, IMPULSE_CHARGE_DURATION);
        apply_impulse_to(&mut mods, &impulse, IMPULSE_SPEED_MULTIPLIER);
        assert!((mods.get(&ModifierSlot::MaxSpeed) - IMPULSE_SPEED_MULTIPLIER).abs() < 1e-6);
        // Cancel impulse
        impulse.cancel_charge();
        apply_impulse_to(&mut mods, &impulse, IMPULSE_SPEED_MULTIPLIER);
        assert_eq!(mods.get(&ModifierSlot::MaxSpeed), 1.0);
    }

    #[test]
    fn impulse_active_does_not_affect_other_slots() {
        let mut mods = ShipModifiers::new();
        let mut impulse = ImpulseState::new();
        impulse.start_charge();
        impulse.tick(IMPULSE_CHARGE_DURATION, IMPULSE_CHARGE_DURATION);
        apply_impulse_to(&mut mods, &impulse, IMPULSE_SPEED_MULTIPLIER);
        assert!((mods.get(&ModifierSlot::MaxSpeed) - IMPULSE_SPEED_MULTIPLIER).abs() < 1e-6);
        assert_eq!(mods.get(&ModifierSlot::MaxYawRate), 1.0);
        assert_eq!(mods.get(&ModifierSlot::RadarRange), 1.0);
        assert_eq!(mods.get(&ModifierSlot::PhaserDamage), 1.0);
        assert_eq!(mods.get(&ModifierSlot::HullDamageTaken), 1.0);
        assert_eq!(mods.get(&ModifierSlot::RepairRate), 1.0);
    }

    /// Verifies that `translate_impulse_modifiers` reads `speed_multiplier`
    /// from `ImpulseConfigResource` rather than the `IMPULSE_SPEED_MULTIPLIER`
    /// const. With a custom 3.0× multiplier (vs. the 10.0× default), the
    /// MaxSpeed modifier must reflect the resource value.
    #[test]
    fn translate_impulse_modifiers_reads_speed_multiplier_from_resource() {
        use crate::ship_plugin::ImpulseConfigResource;
        use crate::simulation::ShipImpulse;
        use crate::impulse::ImpulseState;

        let mut app = App::new();
        app.init_resource::<ShipModifiers>();

        // Activate the impulse drive directly.
        let mut impulse = ImpulseState::new();
        impulse.start_charge();
        impulse.tick(IMPULSE_CHARGE_DURATION, IMPULSE_CHARGE_DURATION);
        assert!(impulse.is_active(), "test fixture: impulse should be active");
        app.insert_resource(ShipImpulse(impulse));

        // Configure a non-default speed multiplier (3.0 instead of 10.0).
        // The MaxSpeed modifier must reflect this — proving the system
        // reads the resource rather than the const fallback.
        app.insert_resource(ImpulseConfigResource {
            charge_duration: IMPULSE_CHARGE_DURATION,
            speed_multiplier: 3.0,
            acceleration_multiplier: 1.0,
        });

        app.add_systems(Update, translate_impulse_modifiers);
        app.update();

        let mods = app.world().resource::<ShipModifiers>();
        let max_speed = mods.get(&ModifierSlot::MaxSpeed);
        assert!(
            (max_speed - 3.0).abs() < 1e-6,
            "expected MaxSpeed=3.0 from resource speed_multiplier, got {max_speed}"
        );
        assert!(
            (max_speed - IMPULSE_SPEED_MULTIPLIER).abs() > 0.5,
            "MaxSpeed must not fall back to IMPULSE_SPEED_MULTIPLIER const"
        );
    }
}
