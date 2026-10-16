use super::*;
use crate::resources::energy_network::EnergyHardwareEffects;
#[path = "power_transaction_reference.rs"]
mod reference;

fn bindings(power: &PowerSystem) -> EnergyCoolantBindings {
    EnergyCoolantBindings {
        reactor: "fixture-coolant".into(),
        consumers: power
            .network
            .consumers
            .keys()
            .map(|id| (id.clone(), "fixture-coolant".into()))
            .collect(),
    }
}
fn unchanged_bytes(power: &PowerSystem) -> String {
    // Includes receipt/strike fields outside persisted continuation and preserves
    // signed zero; whole-owner equality below also checks the immutable definition.
    serde_json::to_string(&(
        power.capture_continuation(),
        &power.work_emissions,
        &power.strike_config,
    ))
    .unwrap()
}
#[allow(clippy::too_many_arguments)]
fn compare_tick(
    actual: &mut PowerSystem,
    expected: &mut PowerSystem,
    dt: f32,
    links: &EnergyCoolantBindings,
    stocks: &BTreeMap<String, i64>,
    effects: &EnergyHardwareEffects,
    invalid_choice: bool,
    succeeds: bool,
) {
    let before = unchanged_bytes(actual);
    let before_owner = actual.clone();
    let mut actual_calls = Vec::new();
    let mut expected_calls = Vec::new();
    let result = actual.tick_energy_with_effects(dt, links, stocks, effects, &mut |count| {
        actual_calls.push(count);
        if invalid_choice && actual_calls.len() > 1 {
            count
        } else {
            count.saturating_sub(1)
        }
    });
    let oracle = expected.tick_energy_reference(dt, links, stocks, effects, &mut |count| {
        expected_calls.push(count);
        if invalid_choice && expected_calls.len() > 1 {
            count
        } else {
            count.saturating_sub(1)
        }
    });
    assert_eq!(result.is_ok(), succeeds, "unexpected result: {result:?}");
    assert_eq!(result, oracle);
    assert_eq!(actual_calls, expected_calls);
    assert_eq!(actual, expected);
    assert_eq!(unchanged_bytes(actual), unchanged_bytes(expected));
    assert_eq!(actual.read_state(), expected.read_state());
    if result.is_err() {
        assert_eq!(&*actual, &before_owner);
        assert_eq!(unchanged_bytes(actual), before);
    }
    if invalid_choice {
        assert!(
            actual_calls.len() > 1,
            "must reject after a prior routing choice has changed the candidate"
        );
    }
}
fn observed_power() -> PowerSystem {
    let mut power = PowerSystem::default()
        .with_strike_reserve(Some(StrikeReserveConfig {
            group: "reserve".into(),
            units_per_level: 1.0,
            ai_enable_at: None,
            weapons: [(
                "weapon".into(),
                crate::modifiers::strike_reserve::StrikeWeaponConfig {
                    cost: 1.0,
                    damage_multiplier: 2.0,
                },
            )]
            .into(),
        }))
        .unwrap();
    power.set_strike_boost(true);
    power.weapons_restrained = true;
    power.last_reactor_energy_delta_per_sec = -0.0;
    power
        .last_intake_per_sec
        .values_mut()
        .for_each(|value| *value = -0.0);
    power
        .last_work_fraction
        .values_mut()
        .for_each(|value| *value = -0.0);
    power
        .work_emissions
        .insert(SystemId("prior-work".into()), -0.0);
    power.coolant_credit.insert("fixture-coolant".into(), -0.0);
    power
}
#[test]
fn staged_power_transaction_matches_clone_oracle_through_refit_restore_and_zero_time() {
    let mut actual = observed_power();
    let mut expected = actual.clone();
    let stocks = BTreeMap::from([("fixture-coolant".into(), 100)]);
    for dt in [0.0, -0.0, 0.1, 0.25, 0.0] {
        let links = bindings(&actual);
        compare_tick(
            &mut actual,
            &mut expected,
            dt,
            &links,
            &stocks,
            &EnergyHardwareEffects::default(),
            false,
            true,
        );
    }
    for power in [&mut actual, &mut expected] {
        let mut hardware = power.config().clone();
        hardware.reactor.ramp_per_sec *= 1.1;
        hardware.reactor.reserve.charge_heat_per_unit *= 0.9;
        power.reconfigure_energy(hardware).unwrap();
        let saved: PowerState =
            serde_json::from_str(&serde_json::to_string(&power.capture_continuation()).unwrap())
                .unwrap();
        power.restore_continuation(&saved).unwrap();
        power.network.reactor.heat = 40.0;
        power
            .network
            .consumers
            .values_mut()
            .for_each(|state| state.heat = 20.0);
    }
    let links = bindings(&actual);
    for dt in [0.0, 0.5, 1.0] {
        compare_tick(
            &mut actual,
            &mut expected,
            dt,
            &links,
            &stocks,
            &EnergyHardwareEffects::default(),
            false,
            true,
        );
    }
}
#[test]
fn staged_power_transaction_preserves_failures_and_routing_rng_without_partial_commit() {
    for case in 0..7 {
        let mut actual = observed_power();
        let mut links = bindings(&actual);
        let mut stocks = BTreeMap::from([("fixture-coolant".into(), 100)]);
        let mut effects = EnergyHardwareEffects::default();
        let dt = if case == 0 { f32::NAN } else { 0.25 };
        match case {
            1 => {
                stocks.insert("fixture-coolant".into(), -1);
            }
            2 => {
                actual.coolant_credit.insert("missing".into(), 0.5);
            }
            3 => {
                actual.network.consumers.pop_first();
            }
            4 => {
                links.consumers.clear();
            }
            5 => {
                effects.reactor.intake = -1.0;
            }
            6 => {
                actual.network.generation_enabled = false;
                actual.network.reactor.energy = 0.0;
            }
            _ => {}
        }
        let mut expected = actual.clone();
        compare_tick(
            &mut actual,
            &mut expected,
            dt,
            &links,
            &stocks,
            &effects,
            case == 6,
            false,
        );
    }
}
