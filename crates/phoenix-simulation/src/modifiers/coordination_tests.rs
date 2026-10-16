#![allow(clippy::field_reassign_with_default)]
use crate::core::messages::{ModifierSlot, ModifierSource, PowerGroupId};
use crate::modifiers::power_system::{
    PowerSystem, HELM_POWER_GROUP, SHIELDS_POWER_GROUP, WEAPONS_POWER_GROUP,
};
use crate::modifiers::ShipModifiers;
use crate::server_app::ShipImpulse;
use crate::ship_plugin::ImpulseConfigResource;
use bevy::prelude::*;
use std::collections::HashMap;

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
    assert_eq!(mods.get(&ModifierSlot::ShieldRegen), 1.0);
}

/// Incoming flow never creates a fixed shield regeneration multiplier.
#[test]
fn shields_flow_does_not_write_a_regeneration_multiplier() {
    for (level, expected) in [(1u8, 1.0), (2, 1.0), (3, 1.0), (4, 1.0)] {
        let mut mods = ShipModifiers::new();
        let mut power = PowerSystem::default();
        // Independent incoming-flow requests never buy fixed modifiers.
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

/// Authored local power pays actual work without a fixed performance multiplier.
#[test]
fn alliance_energy_profiles_pay_work_without_fixed_performance_multipliers() {
    for path in [
        "assets/entities/alliance_battleship.toml",
        "assets/entities/alliance_cruiser.toml",
        "assets/entities/alliance_destroyer.toml",
        "assets/entities/alliance_courier.toml",
    ] {
        let config = crate::entities::include_resolve::load_entity_config(path).unwrap();
        let hardware = config.energy.unwrap();
        let ship = config.ship_config.unwrap();
        let mut power = PowerSystem::from_energy(hardware.clone(), ship).unwrap();
        for group in &hardware.groups {
            power.set_flow(&group.id, group.max_flow_per_sec).unwrap();
        }
        let mut mods = ShipModifiers::new();
        apply_power_modifiers_from_read_state(
            &mut mods,
            &power.read_state(),
            &default_multipliers(),
        );
        for slot in [
            ModifierSlot::MaxSpeed,
            ModifierSlot::PhaserDamage,
            ModifierSlot::ShieldRegen,
            ModifierSlot::RadarRange,
        ] {
            assert_eq!(
                mods.get(&slot),
                1.0,
                "{path}: incoming flow cannot amplify {slot:?}"
            );
        }
        let consumer = hardware
            .consumers
            .iter()
            .find(|consumer| {
                power
                    .ship_config()
                    .system(&consumer.system)
                    .is_some_and(|system| {
                        matches!(system.kind.as_str(), "phaser_bank" | "blaster_bank")
                    })
                    && (consumer.continuous_cost_per_sec > 0.0 || consumer.discrete_cost > 0.0)
            })
            .unwrap();
        let before = power.network().consumers[&consumer.system].energy;
        if consumer.continuous_cost_per_sec > 0.0 {
            assert_eq!(
                power
                    .work_continuous(&consumer.system, 0.1, 1.0)
                    .unwrap()
                    .fraction,
                1.0
            );
        } else {
            assert!(power.try_work_discrete(&consumer.system).unwrap());
        }
        assert!(
            power.network().consumers[&consumer.system].energy < before,
            "{path}: actual weapon activity spends its finite local buffer"
        );
    }
}

/// Every shipped `assets/entities/*.toml`, as the relative paths the include
/// resolver keys on. Read off the directory rather than listed, so a new hull
/// is covered by the invariant below the moment it is added.
fn shipped_entity_paths() -> Vec<String> {
    let mut out: Vec<String> = crate::repo_fixtures::fs::read_dir("assets/entities")
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
    use crate::ship::helm_ai::MAX_ARTILLERY_RANGE_PARAM;

    let mut checked: Vec<String> = Vec::new();
    for path in shipped_entity_paths() {
        let config = crate::entities::include_resolve::load_entity_config(&path)
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
                crate::entities::config::PhaserCombatConfig::DEFAULT_PHASER_RANGE
            };
            longest_gun = longest_gun.max(reach);
        }
        if wc.phaser_banks.is_empty() {
            longest_gun =
                longest_gun.max(crate::entities::config::PhaserCombatConfig::DEFAULT_PHASER_RANGE);
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

        // Acquisition bounds retain their authored values, using actual hardware
        // without evaluating the retired pooled-allocation policy.
        let power = PowerSystem::from_energy(
            config.energy.clone().expect("powered radar hardware"),
            config.ship_config.clone().expect("powered ship graph"),
        )
        .unwrap();

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
fn high_helm_flow_does_not_amplify_base_speed() {
    let mut mods = ShipModifiers::new();
    let mut power = PowerSystem::default();
    power.set_group_allocation(&helm(), 4).unwrap();
    apply_power_modifiers(&mut mods, &power, &default_multipliers());
    assert_eq!(mods.get(&ModifierSlot::MaxSpeed), 1.0);
}

#[test]
fn low_helm_flow_does_not_rewrite_base_speed() {
    let mut mods = ShipModifiers::new();
    let mut power = PowerSystem::default();
    power.set_group_allocation(&helm(), 1).unwrap();
    apply_power_modifiers(&mut mods, &power, &default_multipliers());
    let expected = 1.0f32;
    assert!((mods.get(&ModifierSlot::MaxSpeed) - expected).abs() < 1e-5);
}

#[test]
fn repeated_power_translation_preserves_other_sources() {
    let mut mods = ShipModifiers::new();
    let mut power = PowerSystem::default();
    power.set_group_allocation(&helm(), 4).unwrap();
    mods.add_or_update(crate::modifiers::Modifier {
        source: ModifierSource::PowerGroup(helm()),
        slot: ModifierSlot::MaxSpeed,
        bonus: 0.5,
    });
    mods.add_or_update(crate::modifiers::Modifier {
        source: ModifierSource::RegionEffect {
            uuid: uuid::Uuid::from_u128(8),
        },
        slot: ModifierSlot::MaxSpeed,
        bonus: 0.25,
    });
    let mult = default_multipliers();
    apply_power_modifiers(&mut mods, &power, &mult);
    apply_power_modifiers(&mut mods, &power, &mult);
    assert_eq!(mods.get(&ModifierSlot::MaxSpeed), 1.25);
}

// ── apply_region_effects tests ─────────────────────────────────────

use crate::core::messages::FlagKind;
use crate::regions::effects::RegionEffectKind;

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

use crate::ship::impulse::{ImpulseState, IMPULSE_CHARGE_DURATION, IMPULSE_SPEED_MULTIPLIER};

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

/// An active but unfunded Impulse intent cannot retain the old unconditional
/// speed effect. The translator preserves independent modifiers and intent;
/// actual funded speed is resolved by the sole physics integrator.
#[test]
fn translate_impulse_modifiers_clears_free_effect_without_changing_other_sources_or_intent() {
    use crate::modifiers::Modifier;
    let mut app = App::new();
    let mut mods = ShipModifiers::new();
    mods.add_or_update(Modifier {
        source: ModifierSource::ImpulseDrive,
        slot: ModifierSlot::MaxSpeed,
        bonus: 2.0,
    });
    mods.add_or_update(Modifier {
        source: ModifierSource::RegionEffect {
            uuid: uuid::Uuid::from_u128(8),
        },
        slot: ModifierSlot::MaxSpeed,
        bonus: 0.25,
    });
    let ship = app
        .world_mut()
        .spawn((
            crate::server_app::LocalShip,
            crate::server_app::Ship,
            ShipImpulse(ImpulseState {
                phase: crate::ship::impulse::ImpulsePhase::Active,
                charge_progress: 1.0,
            }),
            mods,
            ImpulseConfigResource {
                speed_multiplier: 3.0,
                ..Default::default()
            },
        ))
        .id();
    app.add_systems(Update, translate_impulse_modifiers);
    app.update();
    assert_eq!(
        app.world()
            .get::<ShipModifiers>(ship)
            .unwrap()
            .get(&ModifierSlot::MaxSpeed),
        1.25
    );
    assert!(app.world().get::<ShipImpulse>(ship).unwrap().0.is_active());
}

#[test]
fn unfunded_impulse_intents_cannot_recreate_a_free_effect_after_restore() {
    use crate::ship::impulse::ImpulsePhase;
    let mut app = App::new();
    app.add_systems(Update, translate_impulse_modifiers);
    let ships: Vec<_> = [true, false]
        .into_iter()
        .map(|local| {
            let mut entity = app.world_mut().spawn((
                crate::server_app::Ship,
                ShipImpulse(ImpulseState {
                    phase: ImpulsePhase::Active,
                    charge_progress: 1.0,
                }),
                ShipModifiers::new(),
                ImpulseConfigResource {
                    speed_multiplier: 6.0,
                    ..Default::default()
                },
            ));
            if local {
                entity.insert(crate::server_app::LocalShip);
            }
            entity.id()
        })
        .collect();
    for &ship in &ships {
        app.world_mut()
            .get_mut::<ShipModifiers>(ship)
            .unwrap()
            .add_or_update(crate::modifiers::Modifier {
                source: ModifierSource::ImpulseDrive,
                slot: ModifierSlot::MaxSpeed,
                bonus: 5.0,
            });
    }
    app.update();
    for &ship in &ships {
        assert_eq!(
            app.world()
                .get::<ShipModifiers>(ship)
                .unwrap()
                .get(&ModifierSlot::MaxSpeed),
            1.0
        );
    }
    // Snapshot adoption can replace the derived cache without changing phase.
    for &ship in &ships {
        app.world_mut()
            .entity_mut(ship)
            .insert(ShipModifiers::new());
    }
    app.update();
    for &ship in &ships {
        assert_eq!(
            app.world()
                .get::<ShipModifiers>(ship)
                .unwrap()
                .get(&ModifierSlot::MaxSpeed),
            1.0
        );
    }
    // A remote drive cancels independently of the locally projected drive.
    app.world_mut()
        .get_mut::<ShipImpulse>(ships[1])
        .unwrap()
        .0
        .cancel_charge();
    app.update();
    assert_eq!(
        app.world()
            .get::<ShipModifiers>(ships[0])
            .unwrap()
            .get(&ModifierSlot::MaxSpeed),
        1.0
    );
    assert_eq!(
        app.world()
            .get::<ShipModifiers>(ships[1])
            .unwrap()
            .get(&ModifierSlot::MaxSpeed),
        1.0
    );
    assert!(app
        .world()
        .get::<ShipImpulse>(ships[0])
        .unwrap()
        .0
        .is_active());
    assert!(!app
        .world()
        .get::<ShipImpulse>(ships[1])
        .unwrap()
        .0
        .is_active());
}

/// Regression test for PRD #597 gap-4: NPC ships with per-entity
/// `ShipPowerSystem`, `PowerMultiplierResource`, and `ShipModifiers`
/// components pass through `translate_power_modifiers`, as player ships do.
/// A desired incoming-flow setting must not create a fixed speed multiplier.
#[test]
fn npc_ship_power_translation_does_not_amplify_base_speed() {
    use crate::modifiers::power_system::PowerSystem;
    use crate::server_app::Ship;
    use crate::ship::power::{PowerMultiplierResource, ShipPowerSystem};

    let mut app = App::new();

    // An NPC with an old multiplier table must still obey physical flow semantics.
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

    // Ordinary NPC translation obeys the same no-fixed-multiplier rule.
    let mods_comp = app
        .world()
        .get::<ShipModifiers>(npc)
        .expect("NPC must have ShipModifiers component");
    let max_speed = mods_comp.get(&ModifierSlot::MaxSpeed);
    assert!(
        (max_speed - 1.0).abs() < 1e-6,
        "NPC incoming flow must leave base MaxSpeed unchanged, got {max_speed}"
    );
}

// ── apply_radar_damage_modifiers ───────────────────────────────────────────

mod radar_damage {
    use super::*;
    use crate::core::messages::SystemId;
    use crate::entities::spawner::EntitySystemHull;
    use crate::server_app::Ship;
    use crate::ship::damage::{ConsoleTierConfig, SystemHull};
    use crate::ship::system_registry::{
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
    fn gm_system_disable_suppresses_radar_capability_and_restore_keeps_damage_baseline() {
        use crate::ship::components::ShipSystemControlSources;
        for hp in [20.0, 10.0, 0.0] {
            let mut app = App::new();
            let sid = helm_radar_system_id();
            let mut hull =
                SystemHull::from_config_with_tiers(&[(sid.clone(), 20.0, tier_config())]);
            hull.set_hp(&sid, hp);
            let ship = spawn_ship_with_hull(&mut app, hull);
            app.world_mut()
                .entity_mut(ship)
                .insert(ShipSystemControlSources::default());
            app.add_systems(Update, apply_radar_damage_modifiers);
            app.update();
            let baseline = app
                .world()
                .get::<ShipModifiers>(ship)
                .unwrap()
                .get(&ModifierSlot::HelmRadarRange);
            app.world_mut()
                .get_mut::<ShipSystemControlSources>(ship)
                .unwrap()
                .0
                .set_gm_disabled(sid.clone(), true);
            app.update();
            let mods = app.world().get::<ShipModifiers>(ship).unwrap();
            assert_eq!(mods.get(&ModifierSlot::HelmRadarRange), 0.0);
            assert_eq!(mods.get(&ModifierSlot::SensorRadarRange), 1.0);
            assert_eq!(mods.get(&ModifierSlot::RadarRange), 1.0);
            app.world_mut()
                .get_mut::<ShipSystemControlSources>(ship)
                .unwrap()
                .0
                .set_gm_disabled(sid.clone(), false);
            app.update();
            assert_eq!(
                app.world()
                    .get::<ShipModifiers>(ship)
                    .unwrap()
                    .get(&ModifierSlot::HelmRadarRange),
                baseline
            );
            assert_eq!(app.world().get::<EntitySystemHull>(ship).unwrap().0, {
                let mut expected =
                    SystemHull::from_config_with_tiers(&[(sid.clone(), 20.0, tier_config())]);
                expected.set_hp(&sid, hp);
                expected
            });
        }
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
        let hull =
            SystemHull::from_config_with_tiers(&[(SystemId("helm".into()), 20.0, tier_config())]);
        let ship = spawn_ship_with_hull(&mut app, hull);

        app.add_systems(Update, apply_radar_damage_modifiers);
        app.update();

        let mods = app.world().get::<ShipModifiers>(ship).unwrap();
        assert_eq!(mods.get(&ModifierSlot::HelmRadarRange), 1.0);
        assert_eq!(mods.get(&ModifierSlot::RadarRange), 1.0);
        assert_eq!(mods.get(&ModifierSlot::SensorRadarRange), 1.0);
    }
}

#[path = "coordination_power_projection_tests.rs"]
mod power_cleanup_projection;
