use std::collections::HashMap;

use bevy::prelude::*;

use crate::core::messages::FlagKind;
use crate::core::messages::{ModifierSlot, ModifierSource, PowerGroupId};
use crate::entities::spawner::{EntityUuid, RegionEffectsSection};
use crate::modifiers::power_system::{
    Channel1Read, PowerReadState, PowerSystem, HELM_POWER_GROUP, SHIELDS_POWER_GROUP,
    WEAPONS_POWER_GROUP,
};
use crate::modifiers::{Modifier, ShipModifiers};
use crate::regions::effects::RegionEffectKind;
use crate::regions::server::{RegionEntered, RegionExited, RegionMembership};
use crate::server_app::ShipImpulse;
use crate::ship::impulse::{ImpulseState, IMPULSE_SPEED_MULTIPLIER};
use crate::ship::power::{PowerMultiplierResource, ShipPowerSystem};
use crate::ship_plugin::ImpulseConfigResource;

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
            &crate::entities::spawner::EntitySystemHull,
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

    for (hull, mut mods, sources, config) in ships_q.iter_mut() {
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
                let bonus = match hull.0.tier_for(&sid) {
                    DamageTier::Operational => 0.0,
                    DamageTier::Damaged | DamageTier::Disabled => {
                        -hull.0.debuff_magnitude_for(&sid)
                    }
                    DamageTier::Destroyed => RADAR_DESTROYED_BONUS,
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

/// Apply power-level modifiers to `modifiers` based on the current `PowerSystem`
/// state and per-group multiplier config.
///
/// Registers one `Modifier` per power group using
/// [`ModifierSource::PowerGroup`]. Re-registration replaces the previous entry
/// (no stacking). Multiplier arrays are indexed by power level 1–4 (1 maps
/// to index 0).
///
/// # What each group buys (issues #955, #952)
///
/// * HELM → [`ModifierSlot::MaxSpeed`] + [`ModifierSlot::MaxYawRate`].
/// * WEAPONS → [`ModifierSlot::PhaserDamage`]. Power buys INTENSITY: the beam
///   hurts more. `console::weapons::beam::tick_beams` multiplies each bank's
///   authored `beam_damage_per_sec` by this slot.
/// * SHIELDS → [`ModifierSlot::ShieldRegen`]. Power buys RECOVERY: every arc
///   climbs back faster. `ship::shields::tick_shields` scales each facing's
///   authored `regen_per_sec` by this slot, so level 2 is exactly what the
///   `[[shield_arc]]` blocks say and the rungs either side of it trade a
///   reactor point for how quickly a battered ship gets its screens back.
///
/// Power buys neither REACH nor ACQUISITION any more, and both halves of that
/// took a separate deletion. #955 removed the `beam_range × RadarRange`
/// multiplication from every firing path: a gun reaches what it authors, at
/// every power level. #952 then took `sensors` out of
/// [`crate::modifiers::power_system::POWER_GROUP_ORDER`] entirely, so
/// [`ModifierSlot::RadarRange`] has no power producer at all — a hull acquires
/// through the horizon its `[weapons_console.radar] range` authors, reduced
/// only by radar HULL DAMAGE (`apply_radar_damage_modifiers`) and by
/// `RegionEffectKind::RadarDampening`. Both of those are things done TO the
/// ship rather than choices made at the reactor, which is the right shape for a
/// horizon: the Power officer should not be able to make the ship blind by
/// spending elsewhere.
///
/// A LOCK remains a precondition for firing, so a horizon authored below a
/// hull's own guns would still be a range cap wearing a different name. The
/// fleet keeps its horizons clear of its guns by AUTHORING, pinned by
/// `tests::every_hulls_acquisition_horizon_clears_its_longest_gun_at_rest`.
///
/// # A COLD group (level 0) reads the bottom rung
///
/// The multiplier tables have four rungs, for levels 1 to 4, and none for
/// "off". A group commanded to 0 (issue #1395) therefore takes the level-1
/// entry through the `saturating_sub(1)` below rather than a fifth value, and
/// the table's own authoring stays four numbers per group.
///
/// That is the right shape for helm and shields, whose hulls floor them at 1 so
/// the case never arises. It is deliberately NOT the whole answer for a cold
/// weapons group: what a switched-off gun does is not fire at the level-1
/// damage multiplier, it does not fire at all — and that is a FIRE GATE, read
/// off `PowerSystem::is_group_cold` where the shot is authorised, not a number
/// in this table. Issue #1396 owns it.
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
    let shields_id = PowerGroupId(SHIELDS_POWER_GROUP.into());

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

    // SHIELDS buys RECOVERY (issue #952) — see this function's doc comment.
    // This block took over from the `sensors` → `RadarRange` one: that slot now
    // has no power producer at all, only radar hull damage and region
    // dampening.
    let shields_level = channel_1.power_level(&shields_id).unwrap_or(2);
    let shields_level = (shields_level as usize).saturating_sub(1).min(3);
    let shields_bonus = multipliers.get(&shields_id).unwrap_or(&default_mult)[shields_level];
    modifiers.add_or_update(Modifier {
        source: ModifierSource::PowerGroup(shields_id),
        slot: ModifierSlot::ShieldRegen,
        bonus: shields_bonus,
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
    mut ships: Query<
        (
            &ShipImpulse,
            Option<&ImpulseConfigResource>,
            &mut ShipModifiers,
        ),
        With<crate::server_app::Ship>,
    >,
) {
    for (impulse, config, mut modifiers) in &mut ships {
        let speed_multiplier = config
            .map(|c| c.speed_multiplier)
            .unwrap_or(IMPULSE_SPEED_MULTIPLIER);
        apply_impulse_to(&mut modifiers, &impulse.0, speed_multiplier);
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
