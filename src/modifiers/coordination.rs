use std::collections::HashMap;

use bevy::prelude::*;

use crate::entity_spawner::{EntityUuid, RegionEffectsSection};
use crate::impulse::{ImpulsePhase, ImpulseState, IMPULSE_SPEED_MULTIPLIER};
use crate::messages::FlagKind;
use crate::messages::{ModifierSlot, ModifierSource, PowerGroupId};
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
/// `ShipModifiers` is a per-entity `Component` inserted on each ship at spawn
/// time (see `entity_spawner`). All other plugins read/write `&ShipModifiers`
/// or `&mut ShipModifiers` via queries on the ship entity — there is no
/// global `Resource` fallback.
///
/// Also owns the power → modifiers translator system.
pub struct ModifierCoordinationPlugin;

impl Plugin for ModifierCoordinationPlugin {
    fn build(&self, app: &mut App) {
        app.add_observer(on_region_entered)
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
pub fn translate_power_modifiers(
    power_res: Option<Res<ShipPowerSystem>>,
    mult_res: Option<Res<PowerMultiplierResource>>,
    mut ships_q: Query<
        (
            Option<&ShipPowerSystem>,
            Option<&PowerMultiplierResource>,
            &mut ShipModifiers,
            bevy::ecs::query::Has<crate::server_app::LocalShip>,
        ),
        With<crate::server_app::Ship>,
    >,
) {
    let mut any_ship_had_component = false;

    for (power_comp, mult_comp, mut mods, _is_local) in ships_q.iter_mut() {
        // Only translate for ships that carry the per-entity power state.
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

        apply_power_modifiers_from_read_state(&mut mods, &read_state, &mult.multipliers);
    }

    // Resource-only fallback for tests that don't spawn any ship entity
    // with a per-entity `ShipPowerSystem` component. Reads the global
    // `ShipPowerSystem` + `PowerMultiplierResource` resources and writes
    // the per-entity `ShipModifiers` on the LocalShip.
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
    if let Some(mut mods) = ships_q
        .iter_mut()
        .find(|(_, _, _, is_local)| *is_local)
        .map(|(_, _, mods, _)| mods)
    {
        apply_power_modifiers_from_read_state(&mut mods, &read_state, &mult.multipliers);
    }
}

/// Bonus applied to a radar's dedicated `ModifierSlot` when its backing
/// system is fully `Destroyed`. `debuff_magnitude_for` returns `0.0` for the
/// `Destroyed` tier (that field is reserved for the graded Damaged/Disabled
/// debuff — see `SystemHull::debuff_magnitude_for`), so a destroyed radar
/// needs its own, much larger, penalty here. With the cache's
/// `1.0 / (1.0 + |bonus|)` formula this yields a multiplier of ~0.001 — for
/// gameplay purposes, dark.
const RADAR_DESTROYED_BONUS: f32 = -999.0;

/// Damage → radar-range modifier translator system.
///
/// Iterates every ship (player + NPC) and keeps each of the three radar
/// systems' (`helm-radar`, `tactical-radar`, `sensor-radar`) contribution to
/// its dedicated `ModifierSlot` in sync with the system's current
/// `DamageTier`:
/// - `Operational` → no penalty (bonus `0.0`).
/// - `Damaged` / `Disabled` → bonus is the system's own `debuff_magnitude`
///   (graded reduction, consistent with every other damageable system).
/// - `Destroyed` → `RADAR_DESTROYED_BONUS` (near-total blackout).
///
/// `tactical-radar` reuses the existing, shared `ModifierSlot::RadarRange`
/// slot (already driven by the Sensors power group and region dampening —
/// see `apply_power_modifiers_from_read_state` / `apply_region_effects`)
/// since that slot already gates the tactical console's live radar blips and
/// weapon engagement range. `helm-radar` and `sensor-radar` get their own
/// dedicated slots so damaging one radar system cannot bleed into another
/// console's radar.
///
/// Registered by `SimulationPlugin` in `SimSet::Modifiers`, after
/// `SimSet::Damage` (so hull tiers reflect this tick's damage) and before
/// `SimSet::Publish` (so the Helm/Weapons/Sensors blackboard publishers read
/// the fresh multiplier the same tick).
pub fn apply_radar_damage_modifiers(
    mut ships_q: Query<
        (&crate::entity_spawner::EntitySystemHull, &mut ShipModifiers),
        With<crate::server_app::Ship>,
    >,
) {
    use crate::damage::DamageTier;
    use crate::system_registry::{
        helm_radar_system_id, sensor_radar_system_id, tactical_radar_system_id,
    };

    for (hull, mut mods) in ships_q.iter_mut() {
        for (sid, slot) in [
            (helm_radar_system_id(), ModifierSlot::HelmRadarRange),
            (tactical_radar_system_id(), ModifierSlot::RadarRange),
            (sensor_radar_system_id(), ModifierSlot::SensorRadarRange),
        ] {
            let bonus = match hull.0.tier_for(&sid) {
                DamageTier::Operational => 0.0,
                DamageTier::Damaged | DamageTier::Disabled => -hull.0.debuff_magnitude_for(&sid),
                DamageTier::Destroyed => RADAR_DESTROYED_BONUS,
            };
            mods.add_or_update(Modifier {
                source: ModifierSource::SystemDamage(sid),
                slot,
                bonus,
            });
        }
    }
}

/// Apply power-level modifiers to `modifiers` based on the current `PowerSystem`
/// state and per-group multiplier config.
///
/// Registers one `Modifier` per power group using
/// [`ModifierSource::PowerGroup`]. Re-registration replaces the previous entry
/// (no stacking). Multiplier arrays are indexed by power level 1–4 (1 maps
/// to index 0).
pub fn apply_power_modifiers(
    modifiers: &mut ShipModifiers,
    power: &PowerSystem,
    multipliers: &HashMap<PowerGroupId, [f32; 4]>,
) {
    apply_power_modifiers_from_read_state(modifiers, &power.read_state(), multipliers);
}

pub fn apply_power_modifiers_from_read_state(
    modifiers: &mut ShipModifiers,
    power: &PowerReadState,
    multipliers: &HashMap<PowerGroupId, [f32; 4]>,
) {
    let default_mult = [-0.5, 0.0, 0.25, 0.5];
    let channel_1 = Channel1Read::new(power);

    let helm_id = PowerGroupId(HELM_POWER_GROUP.into());
    let weapons_id = PowerGroupId(WEAPONS_POWER_GROUP.into());
    let sensors_id = PowerGroupId(SENSORS_POWER_GROUP.into());

    let helm_level = channel_1.power_level(&helm_id).unwrap_or(2);
    let helm_level = (helm_level as usize).saturating_sub(1).min(3);
    let helm_bonus = multipliers.get(&helm_id).unwrap_or(&default_mult)[helm_level];
    modifiers.add_or_update(Modifier {
        source: ModifierSource::PowerGroup(helm_id.clone()),
        slot: ModifierSlot::MaxSpeed,
        bonus: helm_bonus,
    });
    modifiers.add_or_update(Modifier {
        source: ModifierSource::PowerGroup(helm_id),
        slot: ModifierSlot::MaxYawRate,
        bonus: helm_bonus,
    });

    let weapons_level = channel_1.power_level(&weapons_id).unwrap_or(2);
    let weapons_level = (weapons_level as usize).saturating_sub(1).min(3);
    let weapons_bonus = multipliers.get(&weapons_id).unwrap_or(&default_mult)[weapons_level];
    modifiers.add_or_update(Modifier {
        source: ModifierSource::PowerGroup(weapons_id),
        slot: ModifierSlot::PhaserDamage,
        bonus: weapons_bonus,
    });

    let sensors_level = channel_1.power_level(&sensors_id).unwrap_or(2);
    let sensors_level = (sensors_level as usize).saturating_sub(1).min(3);
    let sensors_bonus = multipliers.get(&sensors_id).unwrap_or(&default_mult)[sensors_level];
    modifiers.add_or_update(Modifier {
        source: ModifierSource::PowerGroup(sensors_id),
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
pub fn translate_impulse_modifiers(
    impulse_q: Query<&ShipImpulse, With<crate::server_app::LocalShip>>,
    impulse_cfg_q: Query<&ImpulseConfigResource, With<crate::server_app::LocalShip>>,
    mut modifiers_q: Query<&mut ShipModifiers, With<crate::server_app::LocalShip>>,
    mut prev_phase: Local<Option<ImpulsePhase>>,
) {
    let Some(impulse_state) = impulse_q.single().ok().map(|i| i.0) else {
        return;
    };
    let current = impulse_state.phase;
    if Some(current) != *prev_phase {
        *prev_phase = Some(current);
        let speed_multiplier = impulse_cfg_q
            .single()
            .ok()
            .map(|c| c.speed_multiplier)
            .unwrap_or(IMPULSE_SPEED_MULTIPLIER);
        if let Some(mut mods_comp) = modifiers_q.iter_mut().next() {
            apply_impulse_to(&mut mods_comp, &impulse_state, speed_multiplier);
        }
    }
}

/// Observer: applies region effects to `ShipModifiers` when the ship enters a region.
fn on_region_entered(
    trigger: On<RegionEntered>,
    region_query: Query<(&EntityUuid, &RegionEffectsSection)>,
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
    if let Ok(mut mods_comp) = modifiers_q.get_mut(ev.subject) {
        apply_region_effects(&mut mods_comp, uuid, &effects.0);
    }
}

/// Observer: clears region effects from `ShipModifiers` when the ship exits a region.
fn on_region_exited(
    trigger: On<RegionExited>,
    membership: Res<RegionMembership>,
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
    if let Ok(mut mods_comp) = modifiers_q.get_mut(ev.subject) {
        mods_comp.clear_source(&source);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::field_reassign_with_default)]

    use super::*;

    fn default_multipliers() -> HashMap<PowerGroupId, [f32; 4]> {
        let d = [-0.5f32, 0.0, 0.25, 0.5];
        HashMap::from([
            (PowerGroupId(HELM_POWER_GROUP.into()), d),
            (PowerGroupId(WEAPONS_POWER_GROUP.into()), d),
            (PowerGroupId(SENSORS_POWER_GROUP.into()), d),
        ])
    }

    fn helm() -> PowerGroupId {
        PowerGroupId(HELM_POWER_GROUP.into())
    }
    fn weapons() -> PowerGroupId {
        PowerGroupId(WEAPONS_POWER_GROUP.into())
    }
    fn sensors() -> PowerGroupId {
        PowerGroupId(SENSORS_POWER_GROUP.into())
    }

    #[test]
    fn power_level_2_gives_zero_bonus() {
        let mut mods = ShipModifiers::new();
        let mut power = PowerSystem::default();
        power.set_group_allocation(&helm(), 2).unwrap();
        power.set_group_allocation(&weapons(), 2).unwrap();
        power.set_group_allocation(&sensors(), 2).unwrap();
        apply_power_modifiers(&mut mods, &power, &default_multipliers());
        assert_eq!(mods.get(&ModifierSlot::MaxSpeed), 1.0);
        assert_eq!(mods.get(&ModifierSlot::PhaserDamage), 1.0);
        assert_eq!(mods.get(&ModifierSlot::RadarRange), 1.0);
    }

    #[test]
    fn helm_power_4_gives_positive_bonus() {
        let mut mods = ShipModifiers::new();
        let mut power = PowerSystem::default();
        power.set_group_allocation(&helm(), 4).unwrap();
        apply_power_modifiers(&mut mods, &power, &default_multipliers());
        assert!((mods.get(&ModifierSlot::MaxSpeed) - 1.5).abs() < 1e-6);
    }

    #[test]
    fn helm_power_1_gives_negative_bonus() {
        let mut mods = ShipModifiers::new();
        let mut power = PowerSystem::default();
        power.set_group_allocation(&helm(), 1).unwrap();
        apply_power_modifiers(&mut mods, &power, &default_multipliers());
        // Negative bonus uses 1/(1+|bonus|): -0.5 → 1/1.5 ≈ 0.667
        let expected = 1.0 / 1.5f32;
        assert!((mods.get(&ModifierSlot::MaxSpeed) - expected).abs() < 1e-5);
    }

    #[test]
    fn apply_twice_does_not_stack() {
        let mut mods = ShipModifiers::new();
        let mut power = PowerSystem::default();
        power.set_group_allocation(&helm(), 4).unwrap();
        let mult = default_multipliers();
        apply_power_modifiers(&mut mods, &power, &mult);
        apply_power_modifiers(&mut mods, &power, &mult);
        assert!((mods.get(&ModifierSlot::MaxSpeed) - 1.5).abs() < 1e-6);
    }

    // ── apply_region_effects tests ─────────────────────────────────────

    use crate::messages::FlagKind;
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

        // Activate the impulse drive directly.
        let mut impulse = ImpulseState::new();
        impulse.start_charge();
        impulse.tick(IMPULSE_CHARGE_DURATION, IMPULSE_CHARGE_DURATION);
        assert!(
            impulse.is_active(),
            "test fixture: impulse should be active"
        );
        // Spawn a LocalShip carrying the per-entity components.
        let ship = app
            .world_mut()
            .spawn((
                crate::simulation::LocalShip,
                crate::simulation::Ship,
                ShipImpulse(impulse),
                ShipModifiers::new(),
            ))
            .id();
        // Configure a non-default speed multiplier (3.0 instead of 10.0).
        // The MaxSpeed modifier must reflect this - proving the system
        // reads the per-entity component rather than the const fallback.
        app.world_mut()
            .entity_mut(ship)
            .insert(ImpulseConfigResource {
                charge_duration: IMPULSE_CHARGE_DURATION,
                speed_multiplier: 3.0,
                acceleration_multiplier: 1.0,
                engage_distance: 200.0,
                cancel_distance: 40.0,
            });

        app.add_systems(Update, translate_impulse_modifiers);
        app.update();

        let mods = app
            .world()
            .get::<ShipModifiers>(ship)
            .expect("ShipModifiers component");
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

        // Spawn an NPC ship (Ship marker, no LocalShip). Give it helm=3 and
        // an explicit multipliers table so we can predict the bonus.
        let mut power = PowerSystem::default();
        power.set_group_allocation(&helm(), 3).unwrap();
        let mut mult = PowerMultiplierResource::default();
        mult.multipliers.insert(helm(), [-0.5, 0.0, 1.0, 2.0]);

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

    // ── apply_radar_damage_modifiers ───────────────────────────────────────────

    mod radar_damage {
        use super::*;
        use crate::damage::{ConsoleTierConfig, SystemHull};
        use crate::entity_spawner::EntitySystemHull;
        use crate::messages::SystemId;
        use crate::server_app::Ship;
        use crate::system_registry::{
            helm_radar_system_id, sensor_radar_system_id, tactical_radar_system_id,
        };

        fn tier_config() -> ConsoleTierConfig {
            ConsoleTierConfig {
                damaged_threshold_pct: 0.75,
                disabled_threshold_pct: 0.25,
                debuff_magnitude: 0.20,
            }
        }

        fn spawn_ship_with_hull(app: &mut App, hull: SystemHull) -> bevy::prelude::Entity {
            app.world_mut()
                .spawn((Ship, EntitySystemHull(hull), ShipModifiers::new()))
                .id()
        }

        #[test]
        fn operational_radars_get_no_penalty() {
            let mut app = App::new();
            let hull = SystemHull::from_config_with_tiers(&[
                (helm_radar_system_id(), 20.0, tier_config()),
                (tactical_radar_system_id(), 20.0, tier_config()),
                (sensor_radar_system_id(), 20.0, tier_config()),
            ]);
            let ship = spawn_ship_with_hull(&mut app, hull);

            app.add_systems(Update, apply_radar_damage_modifiers);
            app.update();

            let mods = app.world().get::<ShipModifiers>(ship).unwrap();
            assert_eq!(mods.get(&ModifierSlot::HelmRadarRange), 1.0);
            assert_eq!(mods.get(&ModifierSlot::RadarRange), 1.0);
            assert_eq!(mods.get(&ModifierSlot::SensorRadarRange), 1.0);
        }

        #[test]
        fn damaged_tactical_radar_reduces_shared_radar_range_slot_only() {
            let mut app = App::new();
            let mut hull = SystemHull::from_config_with_tiers(&[
                (helm_radar_system_id(), 20.0, tier_config()),
                (tactical_radar_system_id(), 20.0, tier_config()),
                (sensor_radar_system_id(), 20.0, tier_config()),
            ]);
            // Drop tactical-radar to 50% HP → Damaged tier (below 75% threshold).
            hull.set_hp(&tactical_radar_system_id(), 10.0);
            let ship = spawn_ship_with_hull(&mut app, hull);

            app.add_systems(Update, apply_radar_damage_modifiers);
            app.update();

            let mods = app.world().get::<ShipModifiers>(ship).unwrap();
            // -0.20 bonus → 1 / (1 + 0.20) ≈ 0.833.
            let radar_range = mods.get(&ModifierSlot::RadarRange);
            assert!(
                (radar_range - (1.0 / 1.20)).abs() < 1e-4,
                "expected ~0.833 tactical RadarRange multiplier, got {radar_range}"
            );
            // Helm/Sensor radar are undamaged and must be unaffected —
            // damaging one radar system must not bleed into another's slot.
            assert_eq!(mods.get(&ModifierSlot::HelmRadarRange), 1.0);
            assert_eq!(mods.get(&ModifierSlot::SensorRadarRange), 1.0);
        }

        #[test]
        fn destroyed_sensor_radar_is_near_blackout_and_isolated() {
            let mut app = App::new();
            let mut hull = SystemHull::from_config_with_tiers(&[
                (helm_radar_system_id(), 20.0, tier_config()),
                (tactical_radar_system_id(), 20.0, tier_config()),
                (sensor_radar_system_id(), 20.0, tier_config()),
            ]);
            hull.set_hp(&sensor_radar_system_id(), 0.0);
            let ship = spawn_ship_with_hull(&mut app, hull);

            app.add_systems(Update, apply_radar_damage_modifiers);
            app.update();

            let mods = app.world().get::<ShipModifiers>(ship).unwrap();
            let sensor_range = mods.get(&ModifierSlot::SensorRadarRange);
            assert!(
                sensor_range < 0.01,
                "destroyed sensor-radar should be near-blackout, got {sensor_range}"
            );
            assert_eq!(mods.get(&ModifierSlot::HelmRadarRange), 1.0);
            assert_eq!(mods.get(&ModifierSlot::RadarRange), 1.0);
        }

        #[test]
        fn ship_without_radar_hull_entries_is_unaffected() {
            let mut app = App::new();
            // A ship whose hull declares none of the three radar SystemIds —
            // `tier_for` must fall back to Operational, not panic or default
            // to some other tier.
            let hull = SystemHull::from_config_with_tiers(&[(
                SystemId("helm".into()),
                20.0,
                tier_config(),
            )]);
            let ship = spawn_ship_with_hull(&mut app, hull);

            app.add_systems(Update, apply_radar_damage_modifiers);
            app.update();

            let mods = app.world().get::<ShipModifiers>(ship).unwrap();
            assert_eq!(mods.get(&ModifierSlot::HelmRadarRange), 1.0);
            assert_eq!(mods.get(&ModifierSlot::RadarRange), 1.0);
            assert_eq!(mods.get(&ModifierSlot::SensorRadarRange), 1.0);
        }
    }
}
