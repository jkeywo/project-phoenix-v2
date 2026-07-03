use std::collections::HashMap;

use bevy::prelude::*;

use crate::entity_spawner::{EntityUuid, RegionEffectsSection};
use crate::flag_kind::FlagKind;
use crate::impulse::{ImpulsePhase, ImpulseState, IMPULSE_SPEED_MULTIPLIER};
use crate::messages::{Console, ModifierSlot, ModifierSource, PowerGroupId};
use crate::modifiers::{Modifier, ShipModifiers};
use crate::power_plugin::{PowerMultiplierResource, ShipPowerSystem};
use crate::power_system::{
    Channel1Read, PowerReadState, PowerSystem, HELM_POWER_GROUP, SENSORS_POWER_GROUP,
    WEAPONS_POWER_GROUP,
};
use crate::region_effects::RegionEffectKind;
use crate::region_plugin::{RegionEntered, RegionExited, RegionMembership};
use crate::ship_plugin::ImpulseConfigResource;
use crate::simulation::ShipImpulse;

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
/// Iterates every ship (`With<Ship>`) — player + NPC — and writes each
/// ship's own power-level modifiers into its own `ShipModifiers` component.
/// This is the single routing point for power-side modifier writes;
/// `handle_power_messages` and `tick_power_system` no longer touch
/// `ShipModifiers` directly.
///
/// The system is registered by `SimulationPlugin` (not by
/// `ModifierCoordinationPlugin`) so it can be chained after the power‑handling
/// systems with explicit `.after()` ordering.
///
/// After PRD #597 gap-4 closure: iterates all ships (player + NPC), using each
/// ship's per-entity `ShipPowerSystem`, `PowerMultiplierResource`, and
/// `ShipModifiers` components. NPCs are equipped with these components at
/// spawn (see `src/entities/spawner.rs`), so their power settings translate
/// into MaxSpeed / PhaserDamage / RadarRange modifiers via the same code
/// path as the player ship. When the LocalShip carries its own components,
/// the global `ShipModifiers` resource is dual-written so legacy
/// Resource-based readers stay in sync.
///
/// Legacy Resource fallback: test fixtures that spawn a `LocalShip` without
/// per-entity power/modifier components still work — the fallback path reads
/// the global `ShipPowerSystem` + `PowerMultiplierResource` resources and
/// writes to the global `ShipModifiers` resource.
pub fn translate_power_modifiers(
    power_res: Option<Res<ShipPowerSystem>>,
    mult_res: Option<Res<PowerMultiplierResource>>,
    modifiers_res: Option<ResMut<ShipModifiers>>,
    mut ships_q: Query<
        (
            Option<&ShipPowerSystem>,
            Option<&PowerMultiplierResource>,
            Option<&mut ShipModifiers>,
            bevy::ecs::query::Has<crate::server_app::LocalShip>,
        ),
        With<crate::server_app::Ship>,
    >,
) {
    let mut modifiers_res = modifiers_res;
    let mut any_ship_had_component = false;
    let mut local_mods_snapshot: Option<ShipModifiers> = None;

    for (power_comp, mult_comp, mods_comp, is_local) in ships_q.iter_mut() {
        // Only translate for ships that carry the per-entity power state.
        // NPCs get ShipPowerSystem via the spawner unconditionally; the
        // LocalShip has it via server_app spawn. Ships without it are
        // legacy test fixtures — the Resource-fallback branch below handles
        // those (only for LocalShip semantics).
        let Some(power) = power_comp else {
            continue;
        };
        any_ship_had_component = true;
        let read_state = power.0.read_state();

        let mult_default;
        let mult: &PowerMultiplierResource = match mult_comp {
            Some(m) => m,
            None => match mult_res.as_deref() {
                Some(m) => m,
                None => {
                    mult_default = PowerMultiplierResource::default();
                    &mult_default
                }
            },
        };

        if let Some(mut mods) = mods_comp {
            apply_power_modifiers_from_read_state(&mut mods, &read_state, &mult.multipliers);
            if is_local {
                local_mods_snapshot = Some(mods.clone());
            }
        }
    }

    // Dual-write: mirror the LocalShip's per-entity ShipModifiers into the
    // global Resource so legacy Resource-based readers stay in sync.
    if let (Some(local_mods), Some(mods_res)) = (local_mods_snapshot, modifiers_res.as_deref_mut())
    {
        *mods_res = local_mods;
        return;
    }

    // Resource-only fallback for tests that don't spawn any ship entity
    // with a per-entity `ShipPowerSystem` component. Reads the global
    // `ShipPowerSystem` + `PowerMultiplierResource` resources and writes
    // the global `ShipModifiers` resource.
    if any_ship_had_component {
        return;
    }
    let Some(power) = power_res.as_deref() else {
        return;
    };
    let read_state = power.0.read_state();
    let mult_default;
    let mult: &PowerMultiplierResource = match mult_res.as_deref() {
        Some(m) => m,
        None => {
            mult_default = PowerMultiplierResource::default();
            &mult_default
        }
    };
    if let Some(mut mods_res) = modifiers_res {
        apply_power_modifiers_from_read_state(&mut mods_res, &read_state, &mult.multipliers);
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
    apply_power_modifiers_from_read_state(modifiers, &power.read_state(), multipliers);
}

pub fn apply_power_modifiers_from_read_state(
    modifiers: &mut ShipModifiers,
    power: &PowerReadState,
    multipliers: &HashMap<Console, [f32; 4]>,
) {
    let default_mult = [-0.5, 0.0, 0.25, 0.5];
    let channel_1 = Channel1Read::new(power);

    let helm_level = channel_1
        .power_level(&PowerGroupId(HELM_POWER_GROUP.into()))
        .unwrap_or(2);
    let helm_level = (helm_level as usize).saturating_sub(1).min(3);
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

    let weapons_level = channel_1
        .power_level(&PowerGroupId(WEAPONS_POWER_GROUP.into()))
        .unwrap_or(2);
    let weapons_level = (weapons_level as usize).saturating_sub(1).min(3);
    let weapons_bonus = multipliers.get(&Console::Tactical).unwrap_or(&default_mult)[weapons_level];
    modifiers.add_or_update(Modifier {
        source: ModifierSource::Console(Console::Tactical),
        slot: ModifierSlot::PhaserDamage,
        bonus: weapons_bonus,
    });

    let sensors_level = channel_1
        .power_level(&PowerGroupId(SENSORS_POWER_GROUP.into()))
        .unwrap_or(2);
    let sensors_level = (sensors_level as usize).saturating_sub(1).min(3);
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
            RegionEffectKind::DamageZone { .. }
            | RegionEffectKind::BlocksImpulse
            | RegionEffectKind::NebulaFog { .. } => {}
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
            RegionEffectKind::SlowZone {
                thrust_modifier,
                yaw_rate_modifier,
            } => {
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
pub fn apply_impulse_to(
    modifiers: &mut ShipModifiers,
    impulse: &ImpulseState,
    speed_multiplier: f32,
) {
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
///
/// After PR 6 (PRD #597): prefers the per-entity `ShipModifiers` component on
/// the LocalShip entity, dual-writing to the global Resource when both exist.
pub fn translate_impulse_modifiers(
    impulse_q: Query<&ShipImpulse, With<crate::server_app::LocalShip>>,
    impulse_res: Option<Res<ShipImpulse>>,
    modifiers_res: Option<ResMut<ShipModifiers>>,
    // Per-entity component takes priority over the Resource fallback (PR 4).
    impulse_cfg_q: Query<&ImpulseConfigResource, With<crate::server_app::LocalShip>>,
    impulse_config: Option<Res<ImpulseConfigResource>>,
    mut modifiers_q: Query<&mut ShipModifiers, With<crate::server_app::LocalShip>>,
    mut prev_phase: Local<Option<ImpulsePhase>>,
) {
    // Prefer per-entity Component on LocalShip; fall back to Resource for
    // legacy test paths that still insert a global ShipImpulse.
    let impulse_state = impulse_q
        .single()
        .ok()
        .map(|i| i.0.clone())
        .or_else(|| impulse_res.as_deref().map(|r| r.0.clone()));
    let Some(impulse_state) = impulse_state else {
        return;
    };
    let current = impulse_state.phase;
    if Some(current) != *prev_phase {
        *prev_phase = Some(current);
        let speed_multiplier = impulse_cfg_q
            .single()
            .map(|c| c.speed_multiplier)
            .or_else(|_| {
                impulse_config
                    .as_deref()
                    .map(|c| c.speed_multiplier)
                    .ok_or(())
            })
            .unwrap_or(IMPULSE_SPEED_MULTIPLIER);
        let mut modifiers_res = modifiers_res;
        match modifiers_q.iter_mut().next() {
            Some(mut mods_comp) => {
                apply_impulse_to(&mut mods_comp, &impulse_state, speed_multiplier);
                if let Some(mods_res) = modifiers_res.as_deref_mut() {
                    *mods_res = mods_comp.clone();
                }
            }
            None => {
                if let Some(mut mods_res) = modifiers_res {
                    apply_impulse_to(&mut mods_res, &impulse_state, speed_multiplier);
                }
            }
        }
    }
}

/// Observer: applies region effects to `ShipModifiers` when the ship enters a region.
///
/// After PR 6 (PRD #597): applies to the subject entity's per-entity
/// `ShipModifiers` component when present; falls back to the global Resource
/// (dual-writing when both exist).
fn on_region_entered(
    trigger: On<RegionEntered>,
    region_query: Query<(&EntityUuid, &RegionEffectsSection)>,
    modifiers_res: Option<ResMut<ShipModifiers>>,
    mut modifiers_q: Query<&mut ShipModifiers>,
) {
    let ev = trigger.event();
    let Ok((uuid_comp, effects)) = region_query.get(ev.region_entity) else {
        return;
    };
    let uuid = match uuid::Uuid::parse_str(&uuid_comp.0) {
        Ok(u) => u,
        Err(_) => return,
    };
    let mut modifiers_res = modifiers_res;
    match modifiers_q.get_mut(ev.subject) {
        Ok(mut mods_comp) => {
            apply_region_effects(&mut mods_comp, uuid, &effects.0);
            if let Some(mods_res) = modifiers_res.as_deref_mut() {
                *mods_res = mods_comp.clone();
            }
        }
        Err(_) => {
            if let Some(mut mods_res) = modifiers_res {
                apply_region_effects(&mut mods_res, uuid, &effects.0);
            }
        }
    }
}

/// Observer: clears region effects from `ShipModifiers` when the ship exits a region.
///
/// After PR 6 (PRD #597): clears from the subject entity's per-entity
/// `ShipModifiers` component when present; falls back to the global Resource
/// (dual-writing when both exist).
fn on_region_exited(
    trigger: On<RegionExited>,
    membership: Res<RegionMembership>,
    modifiers_res: Option<ResMut<ShipModifiers>>,
    mut modifiers_q: Query<&mut ShipModifiers>,
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
    let source = ModifierSource::RegionEffect { uuid };
    let mut modifiers_res = modifiers_res;
    match modifiers_q.get_mut(ev.subject) {
        Ok(mut mods_comp) => {
            mods_comp.clear_source(&source);
            if let Some(mods_res) = modifiers_res.as_deref_mut() {
                *mods_res = mods_comp.clone();
            }
        }
        Err(_) => {
            if let Some(mut mods_res) = modifiers_res {
                mods_res.clear_source(&source);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::field_reassign_with_default)]

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
        apply_region_effects(
            &mut mods,
            uuid,
            &[RegionEffectKind::RadarDampening { multiplier: -0.3 }],
        );
        let expected = 1.0 / 1.3;
        assert!((mods.get(&ModifierSlot::RadarRange) - expected).abs() < 1e-6);
        // Verify source UUID is correct by removing it
        mods.clear_source(&ModifierSource::RegionEffect {
            uuid: uuid::Uuid::from_u128(1),
        });
        assert!((mods.get(&ModifierSlot::RadarRange) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn enter_slow_zone_thrust_registers_maxspeed() {
        let mut mods = ShipModifiers::new();
        let uuid = uuid::Uuid::from_u128(1);
        apply_region_effects(
            &mut mods,
            uuid,
            &[RegionEffectKind::SlowZone {
                thrust_modifier: Some(-0.5),
                yaw_rate_modifier: None,
            }],
        );
        assert!((mods.get(&ModifierSlot::MaxSpeed) - (1.0 / 1.5)).abs() < 1e-6);
        assert!((mods.get(&ModifierSlot::MaxYawRate) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn enter_slow_zone_yaw_registers_maxyawrate() {
        let mut mods = ShipModifiers::new();
        let uuid = uuid::Uuid::from_u128(1);
        apply_region_effects(
            &mut mods,
            uuid,
            &[RegionEffectKind::SlowZone {
                thrust_modifier: None,
                yaw_rate_modifier: Some(-0.3),
            }],
        );
        assert!((mods.get(&ModifierSlot::MaxYawRate) - (1.0 / 1.3)).abs() < 1e-6);
        assert!((mods.get(&ModifierSlot::MaxSpeed) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn enter_slow_zone_both_fields_registers_both_slots() {
        let mut mods = ShipModifiers::new();
        let uuid = uuid::Uuid::from_u128(1);
        apply_region_effects(
            &mut mods,
            uuid,
            &[RegionEffectKind::SlowZone {
                thrust_modifier: Some(-0.5),
                yaw_rate_modifier: Some(-0.3),
            }],
        );
        assert!((mods.get(&ModifierSlot::MaxSpeed) - (1.0 / 1.5)).abs() < 1e-6);
        assert!((mods.get(&ModifierSlot::MaxYawRate) - (1.0 / 1.3)).abs() < 1e-6);
    }

    #[test]
    fn enter_comms_jam_sets_flag() {
        let mut mods = ShipModifiers::new();
        apply_region_effects(
            &mut mods,
            uuid::Uuid::from_u128(1),
            &[RegionEffectKind::CommsJam],
        );
        assert!(mods.has_flag(&FlagKind::CommsJammed));
    }

    #[test]
    fn enter_sensor_blind_sets_flag() {
        let mut mods = ShipModifiers::new();
        apply_region_effects(
            &mut mods,
            uuid::Uuid::from_u128(1),
            &[RegionEffectKind::SensorBlind],
        );
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
        mods.remove_flag(
            ModifierSource::RegionEffect { uuid: uuid1 },
            FlagKind::CommsJammed,
        );
        assert!(
            mods.has_flag(&FlagKind::CommsJammed),
            "flag should remain after removing first source"
        );
        mods.remove_flag(
            ModifierSource::RegionEffect { uuid: uuid2 },
            FlagKind::CommsJammed,
        );
        assert!(
            !mods.has_flag(&FlagKind::CommsJammed),
            "flag should clear after removing last source"
        );
    }

    #[test]
    fn multiple_effects_in_one_region_all_applied() {
        let mut mods = ShipModifiers::new();
        let uuid = uuid::Uuid::from_u128(1);
        apply_region_effects(
            &mut mods,
            uuid,
            &[
                RegionEffectKind::CommsJam,
                RegionEffectKind::SensorBlind,
                RegionEffectKind::RadarDampening { multiplier: -0.3 },
            ],
        );
        assert!(mods.has_flag(&FlagKind::CommsJammed));
        assert!(mods.has_flag(&FlagKind::SensorBlind));
        assert!((mods.get(&ModifierSlot::RadarRange) - (1.0 / 1.3)).abs() < 1e-6);
    }

    #[test]
    fn damage_zone_and_blocks_impulse_do_not_write_modifiers() {
        let mut mods = ShipModifiers::new();
        let uuid = uuid::Uuid::from_u128(1);
        apply_region_effects(
            &mut mods,
            uuid,
            &[
                RegionEffectKind::DamageZone {
                    dps: 50.0,
                    shield_pierce: 0.0,
                },
                RegionEffectKind::BlocksImpulse,
            ],
        );
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
        use crate::impulse::ImpulseState;
        use crate::ship_plugin::ImpulseConfigResource;
        use crate::simulation::ShipImpulse;

        let mut app = App::new();
        app.init_resource::<ShipModifiers>();

        // Activate the impulse drive directly.
        let mut impulse = ImpulseState::new();
        impulse.start_charge();
        impulse.tick(IMPULSE_CHARGE_DURATION, IMPULSE_CHARGE_DURATION);
        assert!(
            impulse.is_active(),
            "test fixture: impulse should be active"
        );
        // Spawn a LocalShip carrying the impulse Component so
        // `translate_impulse_modifiers` (which prefers the per-entity
        // Component post ship-parity audit) can read it.
        app.world_mut().spawn((
            crate::simulation::LocalShip,
            crate::simulation::Ship,
            ShipImpulse(impulse),
            ShipModifiers::new(),
        ));
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

    /// Regression test for PRD #597 gap-4: NPC ships with per-entity
    /// `ShipPowerSystem`, `PowerMultiplierResource`, and `ShipModifiers`
    /// components must have their power settings translated into modifiers by
    /// `translate_power_modifiers`, the same way the player ship does.
    ///
    /// Spawns an NPC ship (Ship marker, no LocalShip) with helm=3 and asserts
    /// that after one tick the ship's own `ShipModifiers` component carries a
    /// MaxSpeed bonus > 1.0.
    #[test]
    fn npc_ship_helm_power_translates_to_max_speed_modifier() {
        use crate::modifiers::power_system::PowerSystem;
        use crate::power_plugin::{PowerMultiplierResource, ShipPowerSystem};
        use crate::server_app::Ship;

        let mut app = App::new();
        app.init_resource::<ShipModifiers>();

        // Spawn an NPC ship (Ship marker, no LocalShip). Give it helm=3 and
        // an explicit multipliers table so we can predict the bonus.
        let mut power = PowerSystem::default();
        power.helm = 3;
        let mut mult = PowerMultiplierResource::default();
        mult.multipliers
            .insert(crate::messages::Console::Helm, [-0.5, 0.0, 1.0, 2.0]);

        let npc = app
            .world_mut()
            .spawn((Ship, ShipPowerSystem(power), mult, ShipModifiers::new()))
            .id();

        app.add_systems(Update, translate_power_modifiers);
        app.update();

        // The NPC's own per-entity ShipModifiers must reflect helm=3 → +1.0.
        let mods_comp = app
            .world()
            .get::<ShipModifiers>(npc)
            .expect("NPC must have ShipModifiers component");
        let max_speed = mods_comp.get(&ModifierSlot::MaxSpeed);
        assert!(
            (max_speed - 2.0).abs() < 1e-6,
            "NPC helm=3 should give MaxSpeed multiplier 2.0, got {max_speed}"
        );
    }
}
