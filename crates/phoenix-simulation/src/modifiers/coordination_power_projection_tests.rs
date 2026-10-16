// Frozen complete translator and cleanup oracle from 30ad2c1d.
use super::*;
use crate::modifiers::power_system::PowerReadState;
use crate::modifiers::{IntModifier, IntModifierSlot, Modifier, ModifierEvent};
use crate::ship::power::{PowerMultiplierResource, ShipPowerSystem};
use bevy::ecs::system::RunSystemOnce;

fn owned_translate_power_modifiers(
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

        owned_apply_power_modifiers_from_read_state(&mut mods, &read_state, &mult.multipliers);
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
        owned_apply_power_modifiers_from_read_state(&mut mods, &read_state, &mult.multipliers);
    }
}

fn owned_apply_power_modifiers_from_read_state(
    modifiers: &mut ShipModifiers,
    power: &PowerReadState,
    multipliers: &HashMap<PowerGroupId, [f32; 4]>,
) {
    // Incoming flow replenishes local reserves. It never amplifies machinery.
    // Clear contributions left by restored/fixture modifier collections.
    let _ = (power, multipliers);
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

fn initial_modifiers() -> ShipModifiers {
    let mut mods = ShipModifiers::new();
    for (group, slot, bonus) in [
        (HELM_POWER_GROUP, ModifierSlot::MaxSpeed, 0.5),
        (HELM_POWER_GROUP, ModifierSlot::MaxYawRate, -0.25),
        (WEAPONS_POWER_GROUP, ModifierSlot::PhaserDamage, -0.0),
        (SHIELDS_POWER_GROUP, ModifierSlot::ShieldRegen, 0.75),
        // Same source, different slot: this translator must not broaden cleanup.
        (HELM_POWER_GROUP, ModifierSlot::RadarRange, 0.125),
        ("custom-allocation", ModifierSlot::MaxSpeed, -0.125),
    ] {
        mods.add_or_update(Modifier {
            source: ModifierSource::PowerGroup(PowerGroupId(group.into())),
            slot,
            bonus,
        });
    }
    let world_source = ModifierSource::World {
        id: "kept-world".into(),
        tag: "kept-tag".into(),
    };
    for (slot, bonus) in [
        (ModifierSlot::MaxSpeed, 0.25),
        (ModifierSlot::PhaserDamage, -0.5),
        (ModifierSlot::ShieldRegen, 0.125),
        (ModifierSlot::RepairRate, 0.75),
    ] {
        mods.add_or_update(Modifier {
            source: world_source.clone(),
            slot,
            bonus,
        });
    }
    mods.add_or_update_int(IntModifier {
        source: world_source.clone(),
        slot: IntModifierSlot::RepairTeams,
        bonus: 2,
    });
    mods.add_flag(world_source, FlagKind::SensorBlind);
    // Retain genuine existing Added events: cleanup must append, not replace them.
    mods
}

fn actual_power() -> ShipPowerSystem {
    let authored = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    ShipPowerSystem(
        PowerSystem::from_energy(authored.energy.unwrap(), authored.ship_config.unwrap()).unwrap(),
    )
}

// Each pair starts from the SAME cloned owner, including the integer-map seed.
// Full Debug equality therefore also observes every private table/cache/flag and
// pending event, without depending on independently randomized HashMap order.
fn assert_modifiers_equal(actual: &ShipModifiers, expected: &ShipModifiers) {
    assert_eq!(format!("{actual:?}"), format!("{expected:?}"));
    assert_eq!(
        serde_json::to_vec(&actual.debug_payload()).unwrap(),
        serde_json::to_vec(&expected.debug_payload()).unwrap()
    );
    for slot in ModifierSlot::all() {
        assert_eq!(
            actual.get(&slot).to_bits(),
            expected.get(&slot).to_bits(),
            "{slot:?}"
        );
    }
    for slot in IntModifierSlot::all() {
        assert_eq!(actual.get_int(&slot), expected.get_int(&slot));
    }
}

fn scenario(
    power: &ShipPowerSystem,
    seed: &ShipModifiers,
    global: bool,
    global_mult: bool,
    component_mask: u8,
    multiplier_mask: u8,
    with_local: bool,
) -> (World, Vec<Entity>) {
    let mut world = World::new();
    if global {
        world.insert_resource(power.clone());
    }
    if global_mult {
        world.insert_resource(PowerMultiplierResource::default());
    }
    let mut actors = Vec::new();
    for i in 0..3 {
        let mut entity = world.spawn((crate::server_app::Ship, seed.clone()));
        if i == 0 && with_local {
            entity.insert(crate::server_app::LocalShip);
        }
        if i < 2 && component_mask & (1 << i) != 0 {
            entity.insert(power.clone());
        }
        if i < 2 && multiplier_mask & (1 << i) != 0 {
            let mut mult = PowerMultiplierResource::default();
            mult.multipliers.insert(helm(), [-0.75, 0.25, 0.75, 1.0]);
            entity.insert(mult);
        }
        actors.push(entity.id());
    }
    // A power-bearing non-Ship must neither be cleaned nor suppress fallback.
    actors.push(
        world
            .spawn((
                seed.clone(),
                power.clone(),
                PowerMultiplierResource::default(),
            ))
            .id(),
    );
    (world, actors)
}

fn assert_power_unchanged(world: &World, actors: &[Entity], expected: &ShipPowerSystem) {
    let expected = expected.0.capture_continuation();
    if let Some(global) = world.get_resource::<ShipPowerSystem>() {
        assert_eq!(global.0.capture_continuation(), expected);
    }
    for actor in actors {
        if let Some(power) = world.get::<ShipPowerSystem>(*actor) {
            assert_eq!(power.0.capture_continuation(), expected);
        }
    }
}

#[test]
fn power_cleanup_complete_producer_matches_owned_oracle_for_all_fallback_gates() {
    let power = actual_power();
    let seed = initial_modifiers();
    for global in [false, true] {
        for global_mult in [false, true] {
            for component_mask in 0..4 {
                for multiplier_mask in 0..4 {
                    for with_local in [false, true] {
                        let (mut actual, actors) = scenario(
                            &power,
                            &seed,
                            global,
                            global_mult,
                            component_mask,
                            multiplier_mask,
                            with_local,
                        );
                        let (mut expected, expected_actors) = scenario(
                            &power,
                            &seed,
                            global,
                            global_mult,
                            component_mask,
                            multiplier_mask,
                            with_local,
                        );
                        assert_eq!(actors, expected_actors);
                        for pass in 0..2 {
                            actual.run_system_once(translate_power_modifiers).unwrap();
                            expected
                                .run_system_once(owned_translate_power_modifiers)
                                .unwrap();
                            for (i, actor) in actors.iter().enumerate() {
                                let result = actual.get::<ShipModifiers>(*actor).unwrap();
                                assert_modifiers_equal(
                                    result,
                                    expected.get::<ShipModifiers>(*actor).unwrap(),
                                );
                                let cleaned = i < 2 && component_mask & (1 << i) != 0
                                    || i == 0 && component_mask == 0 && global && with_local;
                                if !cleaned {
                                    assert_modifiers_equal(result, &seed);
                                } else {
                                    assert_eq!(
                                        result.pending_events.len(),
                                        seed.pending_events.len() + 4,
                                        "pass {pass}: emit exactly the four actual removals once"
                                    );
                                    let removed: Vec<_> = result.pending_events
                                        [seed.pending_events.len()..]
                                        .iter()
                                        .map(|event| match event {
                                            ModifierEvent::Removed { source, slot } => {
                                                (source.clone(), slot.clone())
                                            }
                                            _ => panic!("cleanup may only append actual removals"),
                                        })
                                        .collect();
                                    assert_eq!(
                                        removed,
                                        vec![
                                            (
                                                ModifierSource::PowerGroup(helm()),
                                                ModifierSlot::MaxSpeed
                                            ),
                                            (
                                                ModifierSource::PowerGroup(helm()),
                                                ModifierSlot::MaxYawRate
                                            ),
                                            (
                                                ModifierSource::PowerGroup(weapons()),
                                                ModifierSlot::PhaserDamage
                                            ),
                                            (
                                                ModifierSource::PowerGroup(shields()),
                                                ModifierSlot::ShieldRegen
                                            ),
                                        ]
                                    );
                                    assert_eq!(
                                        result.get(&ModifierSlot::RadarRange).to_bits(),
                                        seed.get(&ModifierSlot::RadarRange).to_bits()
                                    );
                                    assert!(result.has_flag(&FlagKind::SensorBlind));
                                    assert_eq!(result.get_int(&IntModifierSlot::RepairTeams), 2);
                                }
                            }
                            assert_power_unchanged(&actual, &actors, &power);
                            assert_power_unchanged(&expected, &actors, &power);
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn power_cleanup_public_wrappers_match_original_and_leave_unrelated_sources_intact() {
    let power = actual_power();
    for seed in [ShipModifiers::new(), initial_modifiers()] {
        let mut expected = seed.clone();
        let mult = default_multipliers();
        owned_apply_power_modifiers_from_read_state(&mut expected, &power.0.read_state(), &mult);
        let mut actual = seed.clone();
        apply_power_modifiers(&mut actual, &power.0, &mult);
        assert_modifiers_equal(&actual, &expected);
        let mut projected = seed.clone();
        apply_power_modifiers_from_read_state(
            &mut projected,
            &power.0.read_state(),
            &HashMap::new(),
        );
        assert_modifiers_equal(&projected, &expected);
        // Cleanup remains idempotent even with a deliberately unrelated table.
        let mut unrelated = HashMap::new();
        unrelated.insert(PowerGroupId("unused".into()), [0.75, 0.5, 0.25, 0.0]);
        apply_power_modifiers(&mut actual, &power.0, &unrelated);
        assert_modifiers_equal(&actual, &expected);
    }
}
