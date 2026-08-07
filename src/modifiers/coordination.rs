use std::collections::HashMap;

use bevy::prelude::*;

use crate::entity_spawner::{EntityUuid, RegionEffectsSection};
use crate::impulse::{ImpulsePhase, ImpulseState, IMPULSE_SPEED_MULTIPLIER};
use crate::messages::FlagKind;
use crate::messages::{ModifierSlot, ModifierSource, PowerGroupId};
use crate::modifiers::{Modifier, ShipModifiers};
use crate::power_plugin::{PowerMultiplierResource, ShipPowerSystem};
use crate::power_system::{
    Channel1Read, PowerReadState, PowerSystem, HELM_POWER_GROUP, SHIELDS_POWER_GROUP,
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
            (PowerGroupId(SHIELDS_POWER_GROUP.into()), d),
        ])
    }

    fn helm() -> PowerGroupId {
        PowerGroupId(HELM_POWER_GROUP.into())
    }
    fn weapons() -> PowerGroupId {
        PowerGroupId(WEAPONS_POWER_GROUP.into())
    }
    fn shields() -> PowerGroupId {
        PowerGroupId(SHIELDS_POWER_GROUP.into())
    }

    #[test]
    fn power_level_2_gives_zero_bonus() {
        let mut mods = ShipModifiers::new();
        let mut power = PowerSystem::default();
        power.set_group_allocation(&helm(), 2).unwrap();
        power.set_group_allocation(&weapons(), 2).unwrap();
        power.set_group_allocation(&shields(), 2).unwrap();
        apply_power_modifiers(&mut mods, &power, &default_multipliers());
        assert_eq!(mods.get(&ModifierSlot::MaxSpeed), 1.0);
        assert_eq!(mods.get(&ModifierSlot::PhaserDamage), 1.0);
        assert_eq!(mods.get(&ModifierSlot::ShieldRegen), 1.0);
    }

    /// **`ModifierSlot::RadarRange` has no power producer since issue #952.**
    ///
    /// The half of the swap that is easy to forget: taking `sensors` out of
    /// `POWER_GROUP_ORDER` also has to take the modifier it wrote with it, or a
    /// stale `PowerGroup("sensors")` entry would sit in the cache for ever —
    /// nothing removes a modifier whose producer stopped running, and
    /// `translate_power_modifiers` re-applies rather than rebuilds.
    #[test]
    fn power_no_longer_writes_the_radar_range_slot() {
        let mut mods = ShipModifiers::new();
        let mut power = PowerSystem::default();
        power.set_group_allocation(&shields(), 4).unwrap();
        apply_power_modifiers(&mut mods, &power, &default_multipliers());
        assert_eq!(
            mods.get(&ModifierSlot::RadarRange),
            1.0,
            "a hull's acquisition horizon must be what its own file authors, at \
             every reactor setting"
        );
        assert!(mods.get(&ModifierSlot::ShieldRegen) > 1.0);
    }

    /// The `shields` group buys regen at the rungs its multiplier table says.
    #[test]
    fn shields_power_drives_the_shield_regen_slot() {
        for (level, expected) in [(1u8, 1.0 / 1.5), (2, 1.0), (3, 1.25), (4, 1.5)] {
            let mut mods = ShipModifiers::new();
            let mut power = PowerSystem::default();
            // Free the budget first so level 4 is not refused by the 8-point cap.
            power.set_group_allocation(&helm(), 1).unwrap();
            power.set_group_allocation(&weapons(), 1).unwrap();
            power.set_group_allocation(&shields(), level).unwrap();
            apply_power_modifiers(&mut mods, &power, &default_multipliers());
            let got = mods.get(&ModifierSlot::ShieldRegen);
            assert!(
                (got - expected).abs() < 1e-5,
                "shields at {level} should give ShieldRegen x{expected}, got {got}"
            );
        }
    }

    /// **Every shipped Alliance hull spends its combat-stations point on WEAPONS
    /// DAMAGE, and its reach does not depend on the reactor at all (#955).**
    ///
    /// This replaces `every_alliance_hull_reaches_its_authored_beam_range_at_combat_stations`
    /// (#923), which asserted the opposite of the second half: that
    /// `beam_range × RadarRange` *equalled* the authored `beam_range` at combat
    /// stations, i.e. that a hull had to SPEND a reactor point to reach the
    /// numbers its own file wrote down. That assertion was pinning a coupling
    /// that should not have existed — the old test could only ever be satisfied
    /// by holding `sensors` at exactly the ×1.0 rung, so it silently forbade the
    /// fleet from ever moving that group — and #955 deleted the multiplication
    /// instead. Reach is now a property of the gun and is not asserted here at
    /// all; it is pinned where it is computed
    /// (`ai::server::tests::direct_fire_reach_ignores_the_radar_range_slot` and
    /// `console::weapons::server_tests::phaser_reach_is_the_authored_beam_range_and_ignores_the_radar_range_slot`).
    ///
    /// What is left for this pin is the half that IS a reactor question, walked
    /// on the SHIPPED files through the include resolver rather than asserted on
    /// a multiplier in isolation:
    ///
    ///   1. seed a `PowerSystem` from the hull's `[power_groups.*]`, in the
    ///      runtime's own order (`authored_power_group_seed`);
    ///   2. resolve every group's channel against the hull's own
    ///      `[power.ai_policy]` over a COMBAT-STATIONS fact snapshot, and apply
    ///      the winning level through `set_group_allocation` — so the silent
    ///      8-point total cap is exercised for real, in emission order, and a
    ///      policy that asks for nine points fails here rather than in a duel;
    ///   3. translate that power state through `apply_power_modifiers_from_read_state`;
    ///   4. assert `ModifierSlot::PhaserDamage` is strictly ABOVE nominal — the
    ///      point #923 moved to `sensors` is back on `weapons`, and it buys
    ///      intensity.
    #[test]
    fn every_alliance_hull_elevates_its_phaser_damage_at_combat_stations() {
        for path in [
            "assets/entities/alliance_battleship.toml",
            "assets/entities/alliance_cruiser.toml",
            "assets/entities/alliance_destroyer.toml",
            "assets/entities/alliance_courier.toml",
        ] {
            let config = crate::entity_includes::load_entity_config(path)
                .unwrap_or_else(|e| panic!("{path}: {e}"));
            let reactor = config
                .power
                .as_ref()
                .unwrap_or_else(|| panic!("{path} authors a [power] reactor"));
            let policy = reactor
                .ai_policy
                .as_ref()
                .unwrap_or_else(|| panic!("{path} authors a [power.ai_policy]"))
                .to_policy()
                .unwrap_or_else(|e| panic!("{path}: {e}"));
            let topology = config
                .ship_config
                .as_ref()
                .unwrap_or_else(|| panic!("{path} authors a ship_config"));

            // (1) The reactor as the spawner seeds it.
            let seed = crate::ship::power::authored_power_group_seed(&topology.power_groups);
            assert!(
                !seed.is_empty(),
                "{path} authors no [power_groups.*]; this pin is about the four-group \
                 Alliance hulls, whose 8-point cap is what makes the red-alert \
                 allocation load-bearing"
            );
            let mut power = PowerSystem::from_authored_groups(reactor.capacity, &seed);

            // (2) Combat stations: red alert, a full battery, under way. The
            // groups are walked in `power.iter()` order because that is the order
            // `ai_power_allocation` emits in, and the total cap makes the order
            // observable.
            let facts = crate::ship::power::seed_power_facts(
                &power,
                100.0, // battery_pct — above every authored reserve
                1.0,   // thrust — above `thrust_threshold`
                true,  // red alert
                Some(0.0),
                None,
                true,
                0,
            );
            let group_ids: Vec<PowerGroupId> = power.iter().map(|(id, _)| id.clone()).collect();
            for id in &group_ids {
                if let Some(crate::ai::policy::AiPolicyVerb::SetPowerGroupAllocation(level)) =
                    policy.resolve_channel(&id.0, &facts, &[])
                {
                    let wanted = *level;
                    power
                        .set_group_allocation(id, wanted)
                        .unwrap_or_else(|e| panic!("{path}: {e:?}"));
                    assert_eq!(
                        power.level_for(id),
                        wanted,
                        "{path}: the authored policy asked for `{}` = {wanted} at combat \
                         stations and the reactor's 8-point total cap refused it (total is \
                         now {}). `PowerSystem::increase` fails SILENTLY, so a policy that \
                         over-spends the budget ships as a group stuck at the wrong level \
                         and a command re-emitted every tick for ever",
                        id.0,
                        power.total()
                    );
                }
            }
            assert!(
                power.total() <= 8,
                "{path}: combat stations totals {} against a cap of 8",
                power.total()
            );

            // (3) Power → modifiers, through the hull's own multiplier table.
            let mut multipliers = default_multipliers();
            if let Some(pm) = config
                .weapons_console
                .as_ref()
                .and_then(|wc| wc.power_multipliers)
            {
                multipliers.insert(PowerGroupId(WEAPONS_POWER_GROUP.into()), pm);
            }
            let mut mods = ShipModifiers::new();
            apply_power_modifiers_from_read_state(&mut mods, &power.read_state(), &multipliers);

            // (4) The claim: the alert buys DAMAGE.
            let damage_mult = mods.get(&ModifierSlot::PhaserDamage);
            assert!(
                damage_mult > 1.0,
                "{path}: at combat stations `weapons` sits at level {} and \
                 `ModifierSlot::PhaserDamage` resolves to x{damage_mult:.3}, i.e. nominal \
                 or worse. #955 put the red-alert reactor point back on this group \
                 precisely so going to combat stations means something; a hull that \
                 elevates nothing has a red alert that changes no number at all",
                power.level_for(&weapons())
            );
        }
    }

    /// Every shipped `assets/entities/*.toml`, as the relative paths the include
    /// resolver keys on. Read off the directory rather than listed, so a new hull
    /// is covered by the invariant below the moment it is added.
    fn shipped_entity_paths() -> Vec<String> {
        let mut out: Vec<String> = std::fs::read_dir("assets/entities")
            .expect("assets/entities must be readable")
            .map(|e| e.expect("readable dir entry").path())
            .filter(|p| p.extension().is_some_and(|x| x == "toml"))
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .collect();
        out.sort();
        out
    }

    /// How much daylight an acquisition horizon must keep beyond the longest
    /// thing the hull can shoot with, as a fraction of that reach.
    ///
    /// A TEST threshold, not a gameplay tunable: no simulation code reads it and
    /// every number it constrains lives in TOML. A fifth is the smallest margin
    /// that is unambiguously a decision rather than a coincidence — before this
    /// pin the battleship's horizon and its bow blaster were the same 50.0, and
    /// a bare `>` would have called that healthy.
    const ACQUISITION_MARGIN: f32 = 1.2;

    /// **Every shipped hull can SEE further than it can SHOOT.**
    ///
    /// The invariant #955 needs and did not have. A LOCK is a precondition for
    /// firing, and the horizon a lock is taken through is
    /// `[weapons_console.radar] range × ModifierSlot::RadarRange`
    /// (`console::weapons::mod::ai_target_selection`, `console::weapons::beam::handle_set_target`).
    /// Decoupling reach from power was only half the fix: if the horizon lands
    /// under the guns, reach is capped again by acquisition instead of by the
    /// multiplier, and just as silently.
    ///
    /// Since issue #952 the slot has no power producer at all — `sensors` is no
    /// longer a power group — so the horizon walked here is simply the authored
    /// `range`. That makes every hull's margin WIDER than when this pin was
    /// written, and the pin is kept anyway: it guards the authoring, and the
    /// authored numbers were chosen against the old ×0.667. When #955 landed the
    /// slot was still driven off SENSORS, no AI-crewed hull ever left
    /// `[power_groups.sensors] default_level = 1`, and the horizon was
    /// permanently two thirds of its file value.
    ///
    /// The battleship shipped exactly that: `75 × 0.667 = 50.000002` against a
    /// `heavy-fore` blaster authoring `range = 50.0` and an artillery envelope
    /// authoring `max_artillery_range = 50.0`. The shadow and reposition legs are
    /// entered on `range_to_target > max_artillery_range` — precisely where
    /// `make_candidate` culls every candidate including the retention one, so the
    /// hull dropped the lock at the instant its doctrine stepped out to reacquire.
    ///
    /// What is deliberately NOT asserted: the DEFENSIVE ring
    /// (`target_direct_fire_range + safe_range_margin`), which is derived from
    /// whoever is being fought rather than authored on this hull, so no static
    /// walk of the shipped files can bound it.
    #[test]
    fn every_hulls_acquisition_horizon_clears_its_longest_gun_at_rest() {
        use crate::ai::policy::AiPolicyVerb;
        use crate::ship::helm_ai::MAX_ARTILLERY_RANGE_PARAM;

        let mut checked: Vec<String> = Vec::new();
        for path in shipped_entity_paths() {
            let config = crate::entity_includes::load_entity_config(&path)
                .unwrap_or_else(|e| panic!("{path}: {e}"));
            let Some(wc) = config.weapons_console.as_ref() else {
                continue;
            };

            // The longest thing this hull can put unguided fire at, read off the
            // FIRING paths rather than off the threat-ring projection: an
            // unauthored `beam_range` reaches the phaser default, and a hull that
            // authors NO `[[weapons_console.phaser_banks]]` at all still fires the
            // implicit legacy bank — `combat_config.0.banks.is_empty()` in both
            // `console::weapons::beam::{handle_fire_phaser, ai_phaser_auto_fire}`
            // shoots at `DEFAULT_PHASER_RANGE`. `ai::server::entity_direct_fire_banks`
            // has no such branch, so reading it instead would understate the
            // courier by 5 and let the next bankless hull through. Torpedoes are
            // absent because a homing round has no bounded reach to clear.
            let mut longest_gun = 0.0f32;
            for bank in &wc.phaser_banks {
                let reach = if bank.beam_range > 0.0 {
                    bank.beam_range
                } else {
                    crate::entity_config::PhaserCombatConfig::DEFAULT_PHASER_RANGE
                };
                longest_gun = longest_gun.max(reach);
            }
            if wc.phaser_banks.is_empty() {
                longest_gun =
                    longest_gun.max(crate::entity_config::PhaserCombatConfig::DEFAULT_PHASER_RANGE);
            }
            for bank in &wc.blaster_banks {
                longest_gun = longest_gun.max(bank.range);
            }
            if longest_gun <= 0.0 {
                continue;
            }

            // The OUTER edge of an authored engagement envelope, where the hull
            // flies one. `max_artillery_range` is the boundary that matters: its
            // doctrine leaves the firing position on
            // `range_to_target > max_artillery_range`, so the hull has to still
            // hold a lock OUTSIDE the envelope or the leg it just entered has
            // nothing left to reposition against.
            let helm = config.helm_console.as_ref();
            let envelope = [
                helm.and_then(|h| h.engines_ai.as_ref()),
                helm.and_then(|h| h.steering_ai.as_ref()),
            ]
            .into_iter()
            .flatten()
            .filter_map(|ai| ai.param.get(MAX_ARTILLERY_RANGE_PARAM).copied())
            .fold(0.0f32, f32::max);
            let required = longest_gun.max(envelope);

            let Some(radar_range) = wc.radar.as_ref().map(|r| r.range) else {
                // No `[weapons_console.radar]` at all. `ai_target_selection`
                // reads that as UNBOUNDED (`range_bounds_targets` is false), so
                // range never culls a candidate and there is no horizon to
                // clear. Every Harrow hull is here.
                continue;
            };

            // The reactor as the spawner seeds it, then AT REST: no red alert,
            // nothing under way, a full battery. Since #952 no reactor setting
            // touches `RadarRange` at all, so this walk is now checking that
            // nothing has quietly re-coupled them as much as it is checking the
            // authored number — which is why the `radar_mult == 1.0` assertion
            // below sits inside the loop rather than being folded away.
            let seed = crate::ship::power::authored_power_group_seed(
                &config
                    .ship_config
                    .as_ref()
                    .map(|s| s.power_groups.clone())
                    .unwrap_or_default(),
            );
            let capacity = config.power.as_ref().map(|p| p.capacity).unwrap_or(100.0);
            let mut power = PowerSystem::from_authored_groups(capacity, &seed);
            if let Some(authored) = config.power.as_ref().and_then(|p| p.ai_policy.as_ref()) {
                let policy = authored
                    .to_policy()
                    .unwrap_or_else(|e| panic!("{path}: {e}"));
                // battery_pct 100 (above every authored reserve), thrust 0
                // (station-keeping), red alert DOWN, no combat in living memory,
                // no enemy in sensor range, no Destroy directive, nothing offline.
                let facts = crate::ship::power::seed_power_facts(
                    &power, 100.0, 0.0, false, None, None, false, 0,
                );
                let group_ids: Vec<PowerGroupId> = power.iter().map(|(id, _)| id.clone()).collect();
                for id in &group_ids {
                    if let Some(AiPolicyVerb::SetPowerGroupAllocation(level)) =
                        policy.resolve_channel(&id.0, &facts, &[])
                    {
                        power
                            .set_group_allocation(id, *level)
                            .unwrap_or_else(|e| panic!("{path}: {e:?}"));
                    }
                }
            }

            let multipliers = default_multipliers();
            let mut mods = ShipModifiers::new();
            apply_power_modifiers_from_read_state(&mut mods, &power.read_state(), &multipliers);
            let radar_mult = mods.get(&ModifierSlot::RadarRange);
            assert_eq!(
                radar_mult, 1.0,
                "{path}: the reactor wrote `ModifierSlot::RadarRange`. Since #952 no \
                 power group produces that slot, so this is a resurrected coupling \
                 rather than a tuning question"
            );
            let horizon = radar_range * radar_mult;

            assert!(
                horizon >= required * ACQUISITION_MARGIN,
                "{path}: at rest this hull acquires out to {horizon:.3} \
                 (`[weapons_console.radar] range` {radar_range} × RadarRange ×{radar_mult:.3}) \
                 but must engage out to {required:.1} (longest gun \
                 {longest_gun:.1}, authored artillery envelope {envelope:.1}). A lock is a \
                 precondition for firing, so a horizon inside the guns caps reach just as \
                 surely as the multiplier #955 deleted, and just as silently. Author \
                 `[weapons_console.radar] range` up to at least {:.1}",
                required * ACQUISITION_MARGIN / radar_mult,
            );
            checked.push(path);
        }

        assert!(
            checked.len() >= 4,
            "only {} shipped hull(s) exercised this invariant ({checked:?}). The four \
             Alliance hulls all author `[weapons_console.radar]` and direct-fire banks; \
             if fewer than that reached the assertion, the walk stopped finding them \
             rather than the fleet having got smaller",
            checked.len()
        );
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
                steering_multiplier: 0.0,
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
