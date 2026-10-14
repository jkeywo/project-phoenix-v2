use super::*;

fn group(id: &str) -> PowerGroupId {
    PowerGroupId(id.into())
}
fn bindings(power: &PowerSystem) -> EnergyCoolantBindings {
    EnergyCoolantBindings {
        reactor: "fixture-coolant".into(),
        consumers: power
            .network()
            .consumers
            .keys()
            .map(|id| (id.clone(), "fixture-coolant".into()))
            .collect(),
    }
}
fn tick(power: &mut PowerSystem, stock: i64) -> PowerTickResult {
    power
        .tick_energy(
            1.0,
            &bindings(power),
            &BTreeMap::from([("fixture-coolant".into(), stock)]),
            &mut |_| 0,
        )
        .unwrap()
}

#[test]
fn compatibility_levels_are_views_of_the_same_desired_flow() {
    let mut power = PowerSystem::default();
    assert_eq!(power.level_for(&group("helm")), 2);
    power.set_group_allocation(&group("helm"), 4).unwrap();
    assert_eq!(power.flow_for(&group("helm")), Some(4.0));
    assert_eq!(
        power.network().groups[&group("helm")].desired_flow_per_sec,
        4.0
    );
    power.set_flow(&group("helm"), 1.0).unwrap();
    assert_eq!(power.level_for(&group("helm")), 1);
    assert_eq!(
        power.read_state().battery_charge,
        power.network().reactor.energy
    );
    assert!(!power.read_state().locked);
}

#[test]
fn zero_incoming_flow_neither_restrains_weapons_nor_stops_local_work() {
    let authored = [AuthoredPowerGroup {
        id: group("weapons"),
        level: 2,
        floor: 0,
    }];
    let mut power = PowerSystem::from_authored_groups(&PowerConfig::default(), &authored);
    power.set_group_allocation(&group("weapons"), 0).unwrap();
    assert!(power.is_group_cold(&group("weapons")));
    assert!(!power.weapons_restrained());
    let id = SystemId("fixture-weapons".into());
    assert!(power.try_spend_discrete(&id).unwrap());
    power.set_weapons_restrained(true);
    assert!(power.weapons_restrained());
    assert_eq!(power.flow_for(&group("weapons")), Some(0.0));
    power.set_weapons_restrained(false);
    assert!(!power.weapons_restrained());
}

#[test]
fn requests_can_exceed_supply_without_restoring_the_obsolete_global_budget_lock() {
    let mut power = PowerSystem::default();
    for name in POWER_GROUP_ORDER {
        power.set_group_allocation(&group(name), 4).unwrap();
    }
    assert_eq!(power.total(), 12);
    power.network_mut().reactor.energy = 0.0;
    let before = power.capture_continuation();
    let result = tick(&mut power, 100);
    assert!(!result.energy.cut_groups.is_empty());
    assert!(!power.locked());
    for (id, state) in &power.network().groups {
        assert_eq!(
            state.desired_flow_per_sec,
            before.network.groups[id].desired_flow_per_sec
        );
    }
    power.set_flow(&group("helm"), 1.0).unwrap();
    assert_eq!(power.flow_for(&group("helm")), Some(1.0));
}

#[test]
fn coolant_withdrawal_is_integer_and_saved_credit_funds_the_next_fraction() {
    let mut power = PowerSystem::default();
    for state in power.network_mut().consumers.values_mut() {
        state.heat = 10.0;
    }
    let first = tick(&mut power, 1);
    let used = first.energy.coolant_used["fixture-coolant"] as f64;
    assert!(used > 0.0 && used < 1.0);
    assert_eq!(first.coolant_withdrawals["fixture-coolant"], 1);
    let credit = power.coolant_credit()["fixture-coolant"];
    assert!((credit + used - 1.0).abs() < 1e-12);
    let second = tick(&mut power, 0);
    assert!(!second.coolant_withdrawals.contains_key("fixture-coolant"));
    assert!(
        (power.coolant_credit()["fixture-coolant"]
            + second.energy.coolant_used["fixture-coolant"] as f64
            - credit)
            .abs()
            < 1e-12
    );
}

#[test]
fn unused_coolant_credit_is_not_a_second_inventory_or_an_idle_drain() {
    let mut power = PowerSystem::default();
    let mut saved = power.capture_continuation();
    saved.coolant_credit.insert("fixture-coolant".into(), 0.75);
    power
        .restore_continuation_with_capacities(&saved, &BTreeSet::from(["fixture-coolant".into()]))
        .unwrap();
    let mut profile = power.config().clone();
    profile.reactor.reserve.charge_heat_per_unit = 0.0;
    profile.reactor.reserve.discharge_heat_per_unit = 0.0;
    profile.reactor.reserve.passive_loss_at_full_per_sec = 0.0;
    for consumer in &mut profile.consumers {
        consumer.reserve.charge_heat_per_unit = 0.0;
        consumer.reserve.discharge_heat_per_unit = 0.0;
        consumer.reserve.passive_loss_at_full_per_sec = 0.0;
    }
    let mut cool = PowerSystem::from_energy(profile, power.ship_config().clone()).unwrap();
    let mut saved = cool.capture_continuation();
    saved.coolant_credit.insert("fixture-coolant".into(), 0.75);
    cool.restore_continuation_with_capacities(&saved, &BTreeSet::from(["fixture-coolant".into()]))
        .unwrap();
    let result = tick(&mut cool, 0);
    assert_eq!(result.energy.coolant_used["fixture-coolant"], 0.0);
    assert!(result.coolant_withdrawals.is_empty());
    assert_eq!(cool.coolant_credit()["fixture-coolant"], 0.75);
}

#[test]
fn malformed_credit_and_unknown_capacity_refuse_the_whole_restore() {
    let mut power = PowerSystem::default();
    let original = power.capture_continuation();
    let capacities = BTreeSet::from(["fixture-coolant".into()]);
    for invalid in [-0.1, 1.0, f64::NAN, f64::INFINITY] {
        let mut saved = original.clone();
        saved
            .coolant_credit
            .insert("fixture-coolant".into(), invalid);
        assert!(power
            .restore_continuation_with_capacities(&saved, &capacities)
            .is_err());
        assert_eq!(power.capture_continuation(), original);
    }
    let mut saved = original.clone();
    saved.coolant_credit.insert("nonexistent".into(), 0.25);
    assert!(power
        .restore_continuation_with_capacities(&saved, &capacities)
        .unwrap_err()
        .contains("unknown actual capacity"));
    assert_eq!(power.capture_continuation(), original);
}

#[test]
fn invalid_tick_inputs_do_not_move_energy_credit_or_readings() {
    let mut power = PowerSystem::default();
    let original = power.capture_continuation();
    let binding = bindings(&power);
    assert!(power
        .tick_energy(
            f32::NAN,
            &binding,
            &BTreeMap::from([("fixture-coolant".into(), 1)]),
            &mut |_| 0
        )
        .is_err());
    assert!(power
        .tick_energy(
            1.0,
            &binding,
            &BTreeMap::from([("fixture-coolant".into(), -1)]),
            &mut |_| 0
        )
        .is_err());
    assert!(power
        .tick_energy(1.0, &binding, &BTreeMap::new(), &mut |_| 0)
        .is_err());
    assert_eq!(power.capture_continuation(), original);
}

#[test]
fn continuation_carries_one_network_stock_credit_restraint_and_observed_flows() {
    let mut power = PowerSystem::default();
    power.set_mode(EnergyAllocationMode::Minimal);
    power.set_cooling_usage(CoolingUsage::High);
    power.set_priority(&group("weapons"), 10).unwrap();
    power.set_weapons_restrained(true);
    tick(&mut power, 10);
    let saved = power.capture_continuation();
    let json = serde_json::to_value(&saved).unwrap();
    assert!(json.get("network").is_some());
    assert!(json.get("battery_charge").is_none());
    assert!(json.get("allocations").is_none());
    assert!(json.get("locked").is_none());
    let decoded: PowerState = serde_json::from_value(json).unwrap();
    let mut restored = PowerSystem::default();
    restored
        .restore_continuation_with_capacities(&decoded, &BTreeSet::from(["fixture-coolant".into()]))
        .unwrap();
    assert_eq!(
        power.capture_continuation(),
        restored.capture_continuation()
    );
    for _ in 0..5 {
        assert_eq!(tick(&mut power, 10), tick(&mut restored, 10));
        assert_eq!(
            power.capture_continuation(),
            restored.capture_continuation()
        );
    }
}

#[test]
fn old_scalar_save_payload_is_explicitly_incompatible() {
    assert!(serde_json::from_str::<PowerState>(
        r#"{"battery_charge":50.0,"allocations":[],"locked":true}"#
    )
    .is_err());
}

#[test]
fn flow_readings_are_actual_intake_and_drain_reading_uses_measured_energy() {
    let mut power = PowerSystem::default();
    let initial = power.battery_charge();
    let result = tick(&mut power, 10);
    assert_eq!(
        power.reactor_energy_rate(),
        power.battery_charge() - initial
    );
    assert_eq!(
        power.is_draining(&PowerConfig::default()),
        power.battery_charge() < initial
    );
    for (id, report) in result.energy.consumer_reports {
        assert_eq!(power.last_intake_per_sec(&id), Some(report.accepted_energy));
    }
    let saved = power.capture_continuation();
    let mut restored = PowerSystem::default();
    restored.restore_continuation(&saved).unwrap();
    assert_eq!(
        restored.last_intake_per_sec(&SystemId("fixture-helm".into())),
        power.last_intake_per_sec(&SystemId("fixture-helm".into()))
    );
}

#[test]
fn mismatched_saved_consumer_readings_are_rejected_before_restore() {
    let mut power = PowerSystem::default();
    let original = power.capture_continuation();
    let mut bad = original.clone();
    bad.last_intake_per_sec
        .remove(&SystemId("fixture-helm".into()));
    assert!(power.restore_continuation(&bad).is_err());
    assert_eq!(power.capture_continuation(), original);
    let mut bad = original.clone();
    bad.last_reactor_energy_delta_per_sec = f32::NAN;
    assert!(power.restore_continuation(&bad).is_err());
    assert_eq!(power.capture_continuation(), original);
}

#[test]
fn strike_output_spends_the_same_reactor_reserve_and_adds_discharge_heat() {
    let config = PowerConfig {
        strike_reserve: Some(StrikeReserveConfig {
            ai_enable_at: Some(5.0),
            group: "weapons".into(),
            units_per_level: 1.0,
            weapons: BTreeMap::from([(
                "phaser-fore".into(),
                crate::modifiers::strike_reserve::StrikeWeaponConfig {
                    cost: 10.0,
                    damage_multiplier: 2.0,
                },
            )]),
        }),
        ..PowerConfig::default()
    };
    let mut power = PowerSystem::new(&config);
    assert!(power.set_strike_boost(true));
    let before = power.battery_charge();
    let heat = power.network().reactor.heat;
    assert_eq!(power.fire_strike_weapon("phaser-fore"), 2.0);
    assert_eq!(power.battery_charge(), before - 10.0);
    assert_eq!(power.network().reactor.heat, heat + 1.0);
    power.network_mut().reactor.energy = 5.0;
    assert_eq!(power.fire_strike_weapon("phaser-fore"), 1.0);
    assert_eq!(power.battery_charge(), 5.0);
    assert!(power.strike_boost().unwrap().depleted);
    assert!(!power.strike_boost().unwrap().enabled);
}

#[test]
fn administrative_bid_adapter_retains_requests_without_a_hidden_rung_budget() {
    let power = PowerSystem::default();
    let bids: Vec<_> = POWER_GROUP_ORDER
        .iter()
        .map(|id| AllocationBid {
            group: group(id),
            want: 4,
            max_level: 4,
            floor: 0,
            rule_priority: 0,
        })
        .collect();
    let plan = plan_allocation(&power, &bids);
    assert_eq!(plan.len(), 3);
    assert!(plan.iter().all(|(_, level)| *level == 4));
}

#[test]
fn control_setters_validate_before_changing_canonical_requests() {
    let mut power = PowerSystem::default();
    let original = power.capture_continuation();
    assert!(power.set_flow(&group("helm"), f32::NAN).is_err());
    assert!(power.set_cooling(&group("helm"), 50.0).is_err());
    assert!(power.set_output(100.0).is_err());
    assert!(power.set_reactor_cooling(f32::INFINITY).is_err());
    assert!(matches!(
        power.set_group_allocation(&group("unknown"), 2),
        Err(PowerAllocationError::UnknownGroup(_))
    ));
    assert_eq!(power.capture_continuation(), original);
}
