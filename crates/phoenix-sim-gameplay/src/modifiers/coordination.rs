use std::collections::HashMap;

use bevy::prelude::*;

use crate::core::messages::FlagKind;
use crate::core::messages::{ModifierSlot, ModifierSource, PowerGroupId};
use crate::entities::spawner::{EntityUuid, RegionEffectsSection};
use crate::modifiers::power_system::{
    PowerReadState, PowerSystem, HELM_POWER_GROUP, SHIELDS_POWER_GROUP, WEAPONS_POWER_GROUP,
};
use crate::modifiers::{Modifier, ShipModifiers};
use crate::regions::effects::RegionEffectKind;
use crate::regions::server::{RegionEntered, RegionExited, RegionMembership};
use crate::ship::impulse::ImpulseState;
use crate::ship::power::{PowerMultiplierResource, ShipPowerSystem};

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
/// Iterates every ship (`With<Ship>`) — player + NPC — and clears its
/// retired fixed allocation contributions from `ShipModifiers`.
/// Incoming energy pays machinery directly; it never amplifies these slots.
/// This is the single routing point for this compatibility cleanup;
/// `handle_power_messages` and `tick_power_system` no longer touch
/// `ShipModifiers` directly.
///
/// The system is registered by `SimulationPlugin` (not by
/// `ModifierCoordinationPlugin`) so it can be chained after the power‑handling
/// systems with explicit `.after()` ordering.
pub fn translate_power_modifiers(
    power_res: Option<Res<ShipPowerSystem>>,
    // Retain the existing declared read access, without materialising its table.
    _mult_res: Option<Res<PowerMultiplierResource>>,
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

    for (power_comp, _mult_comp, mut mods, _is_local) in ships_q.iter_mut() {
        // Only translate for ships that carry the per-entity power state.
        let Some(_) = power_comp else {
            continue;
        };
        any_ship_had_component = true;
        clear_retired_power_modifiers(&mut mods);
    }

    // Resource-only fallback for tests that don't spawn any ship entity
    // with a per-entity `ShipPowerSystem` component. A global power owner
    // permits cleanup of the first LocalShip's per-entity modifiers.
    if any_ship_had_component {
        return;
    }
    let Some(_) = power_res.as_deref() else {
        return;
    };
    if let Some(mut mods) = ships_q
        .iter_mut()
        .find(|(_, _, _, is_local)| *is_local)
        .map(|(_, _, mods, _)| mods)
    {
        clear_retired_power_modifiers(&mut mods);
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
/// slot (also written by region dampening — see `apply_region_effects`; the
/// Sensors power group wrote it too until issue #952 retired that group) since
/// that slot already gates the tactical console's live radar blips and
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
        (
            Option<&crate::entities::spawner::EntitySystemHull>,
            Option<&crate::ship::physical_damage::ShipDamage>,
            &mut ShipModifiers,
            Option<&crate::ship::components::ShipSystemControlSources>,
            Option<&crate::ship::components::ShipConfigComponent>,
        ),
        With<crate::server_app::Ship>,
    >,
) {
    use crate::ship::damage::DamageTier;
    use crate::ship::system_registry::{
        helm_radar_system_id, sensor_radar_system_id, tactical_radar_system_id,
    };

    for (hull, damage, mut mods, sources, config) in ships_q.iter_mut() {
        for (kind, fallback, slot) in [
            (
                crate::ship::system_registry::HELM_RADAR_KIND,
                helm_radar_system_id(),
                ModifierSlot::HelmRadarRange,
            ),
            (
                crate::ship::system_registry::TACTICAL_RADAR_KIND,
                tactical_radar_system_id(),
                ModifierSlot::RadarRange,
            ),
            (
                crate::ship::system_registry::SENSOR_RADAR_KIND,
                sensor_radar_system_id(),
                ModifierSlot::SensorRadarRange,
            ),
        ] {
            let mut ids: Vec<_> = config
                .into_iter()
                .flat_map(|config| &config.0.systems)
                .filter(|system| system.kind == kind)
                .map(|system| system.id.clone())
                .collect();
            if ids.is_empty() {
                ids.push(fallback);
            }
            for sid in ids {
                let disabled_source = ModifierSource::SystemDisabled(sid.clone());
                if sources.is_some_and(|sources| sources.0.is_gm_disabled(&sid)) {
                    mods.add_or_update(Modifier {
                        source: disabled_source,
                        slot: slot.clone(),
                        bonus: 0.0,
                    });
                } else {
                    mods.remove(&disabled_source, &slot);
                }
                let bonus = if let Some(damage) = damage {
                    let factor =
                        damage.factor(&sid, phoenix_model::damage::DamageBinding::Operation);
                    if factor == 0.0 {
                        RADAR_DESTROYED_BONUS
                    } else {
                        factor - 1.0
                    }
                } else {
                    match hull.map_or(DamageTier::Operational, |hull| hull.0.tier_for(&sid)) {
                        DamageTier::Operational => 0.0,
                        DamageTier::Damaged | DamageTier::Disabled => {
                            -hull.map_or(0.0, |hull| hull.0.debuff_magnitude_for(&sid))
                        }
                        DamageTier::Destroyed => RADAR_DESTROYED_BONUS,
                    }
                };
                mods.add_or_update(Modifier {
                    source: ModifierSource::SystemDamage(sid),
                    slot: slot.clone(),
                    bonus,
                });
            }
        }
    }
}

/// Clear the former fixed allocation contributions without projecting power state.
/// Incoming flow replenishes local reserves; it never amplifies machinery.
/// The legacy power and multiplier arguments remain accepted for existing callers.
pub fn apply_power_modifiers(
    modifiers: &mut ShipModifiers,
    _power: &PowerSystem,
    _multipliers: &HashMap<PowerGroupId, [f32; 4]>,
) {
    clear_retired_power_modifiers(modifiers);
}

pub fn apply_power_modifiers_from_read_state(
    modifiers: &mut ShipModifiers,
    _power: &PowerReadState,
    _multipliers: &HashMap<PowerGroupId, [f32; 4]>,
) {
    clear_retired_power_modifiers(modifiers);
}

fn clear_retired_power_modifiers(modifiers: &mut ShipModifiers) {
    // Incoming flow replenishes local reserves. It never amplifies machinery.
    // Clear contributions left by restored/fixture modifier collections.
    for (group, slots) in [
        (
            HELM_POWER_GROUP,
            &[ModifierSlot::MaxSpeed, ModifierSlot::MaxYawRate][..],
        ),
        (WEAPONS_POWER_GROUP, &[ModifierSlot::PhaserDamage][..]),
        (SHIELDS_POWER_GROUP, &[ModifierSlot::ShieldRegen][..]),
    ] {
        for slot in slots {
            modifiers.remove(
                &ModifierSource::PowerGroup(PowerGroupId(group.into())),
                slot,
            );
        }
    }
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

/// Refresh one actual field's existing modifier source without stacking or
/// remove/re-add events on every unchanged fixed step.
pub fn apply_weighted_region_effects(
    modifiers: &mut ShipModifiers,
    region_uuid: uuid::Uuid,
    effects: &[RegionEffectKind],
    exposure: f32,
) {
    let source = ModifierSource::RegionEffect { uuid: region_uuid };
    if !exposure.is_finite() || exposure <= 0.0 {
        modifiers.clear_source(&source);
        return;
    }
    let mut radar = false;
    let mut thrust = false;
    let mut yaw = false;
    let mut comms = false;
    let mut blind = false;
    let weighted: Vec<_> = effects
        .iter()
        .map(|effect| match effect {
            RegionEffectKind::RadarDampening { multiplier } => {
                radar = true;
                RegionEffectKind::RadarDampening {
                    multiplier: multiplier * exposure,
                }
            }
            RegionEffectKind::SlowZone {
                thrust_modifier,
                yaw_rate_modifier,
            } => {
                thrust |= thrust_modifier.is_some();
                yaw |= yaw_rate_modifier.is_some();
                RegionEffectKind::SlowZone {
                    thrust_modifier: thrust_modifier.map(|value| value * exposure),
                    yaw_rate_modifier: yaw_rate_modifier.map(|value| value * exposure),
                }
            }
            RegionEffectKind::CommsJam => {
                comms = true;
                effect.clone()
            }
            RegionEffectKind::SensorBlind => {
                blind = true;
                effect.clone()
            }
            _ => effect.clone(),
        })
        .collect();
    for (present, slot) in [
        (radar, ModifierSlot::RadarRange),
        (thrust, ModifierSlot::MaxSpeed),
        (yaw, ModifierSlot::MaxYawRate),
    ] {
        if !present {
            modifiers.remove(&source, &slot);
        }
    }
    if !comms {
        modifiers.remove_flag(source.clone(), FlagKind::CommsJammed);
    }
    if !blind {
        modifiers.remove_flag(source.clone(), FlagKind::SensorBlind);
    }
    apply_region_effects(modifiers, region_uuid, &weighted);
}

/// Apply impulse-drive modifiers to `modifiers` based on the current
/// `ImpulseState`.
///
/// When the impulse drive is active (`Active`) it registers a
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
/// Derives every ship's keyed speed contribution from its own phase and
/// configuration. Reapplying is event-idempotent and also repairs a cache
/// replaced during snapshot adoption without depending on a phase transition.
/// LocalShip selects presentation and must not select authoritative modifiers.
pub fn translate_impulse_modifiers(
    mut ships: Query<&mut ShipModifiers, With<crate::server_app::Ship>>,
) {
    for mut modifiers in &mut ships {
        // Paid impulse work is resolved by the sole physics integrator. Clear
        // this former unconditional effect so a charged intent cannot act free.
        modifiers.remove(&ModifierSource::ImpulseDrive, &ModifierSlot::MaxSpeed);
    }
}

/// Observer: applies region effects to `ShipModifiers` when the ship enters a region.
fn on_region_entered(
    trigger: On<RegionEntered>,
    region_query: Query<(
        &EntityUuid,
        &RegionEffectsSection,
        Option<&crate::regions::field::RegionFieldState>,
    )>,
    mut modifiers_q: Query<&mut ShipModifiers>,
) {
    let ev = trigger.event();
    let Ok((uuid_comp, effects, field)) = region_query.get(ev.region_entity) else {
        return;
    };
    if field.is_some() {
        return;
    } // Continuously refreshed by the sole Region owner.
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
