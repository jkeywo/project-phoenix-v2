use super::*;

#[test]
fn accepted_discrete_amount_uses_actual_reserve_and_rejects_malformed_or_missing_owner_without_writes(
) {
    let mut power = PowerSystem::default();
    let consumer = power.config().consumers[0].clone();
    let owner = consumer.system;
    power
        .network_mut()
        .consumers
        .get_mut(&owner)
        .unwrap()
        .energy = consumer.reserve.capacity;
    let before = power.clone();
    for amount in [-1.0, f32::NAN, f32::INFINITY] {
        assert!(power.try_work_discrete_amount(&owner, amount).is_err());
        assert_eq!(power, before);
    }
    assert!(power
        .try_work_discrete_amount(&SystemId("absent-drive".into()), 1.0)
        .is_err());
    assert_eq!(power, before);
    assert!(power.try_work_discrete_amount(&owner, 0.5).unwrap());
    assert_eq!(
        power.network().consumers[&owner].energy,
        consumer.reserve.capacity - 0.5
    );
    assert_eq!(power.emitted_work(&owner), 0.5);
    power
        .network_mut()
        .consumers
        .get_mut(&owner)
        .unwrap()
        .thermal_stopped = true;
    let stopped = power.clone();
    assert!(!power.try_work_discrete_amount(&owner, 0.5).unwrap());
    assert_eq!(power, stopped);
}

fn group(id: &str) -> PowerGroupId {
    PowerGroupId(id.into())
}
#[test]
fn fitting_reconfiguration_preserves_every_continuation_and_refuses_buffer_or_link_changes() {
    let mut power = PowerSystem::default();
    tick(&mut power, 10);
    let before = power.capture_continuation();
    let mut hardware = power.config().clone();
    hardware.reactor.ramp_per_sec *= 1.1;
    hardware.reactor.reserve.charge_heat_per_unit *= 0.9;
    power.reconfigure_energy(hardware).unwrap();
    assert_eq!(before, power.capture_continuation());
    let preserved = power.clone();
    let mut illegal = power.config().clone();
    illegal.reactor.reserve.capacity += 1.0;
    assert!(power.reconfigure_energy(illegal).is_err());
    assert_eq!(power, preserved);
    let mut illegal = power.config().clone();
    illegal.reactor.reserve.charge_heat_per_unit = f32::NAN;
    assert!(power.reconfigure_energy(illegal).is_err());
    assert_eq!(power, preserved);
}
#[test]
fn battery_only_reconfiguration_keeps_charge_and_rejects_invalid_generation_without_writes() {
    let mut power = PowerSystem::default();
    tick(&mut power, 10);
    let before = power.capture_continuation();
    let mut hardware = power.config().clone();
    hardware.reactor.sustainable_output_per_sec = 0.0;
    power.reconfigure_energy(hardware).unwrap();
    assert_eq!(before, power.capture_continuation());
    let preserved = power.clone();
    for output in [-1.0, f32::NAN, f32::INFINITY] {
        let mut invalid = power.config().clone();
        invalid.reactor.sustainable_output_per_sec = output;
        assert!(power.reconfigure_energy(invalid).is_err());
        assert_eq!(power, preserved);
    }
    for ramp in [0.0, -1.0, f32::NAN, f32::INFINITY] {
        let mut invalid = power.config().clone();
        invalid.reactor.ramp_per_sec = ramp;
        assert!(power.reconfigure_energy(invalid).is_err());
        assert_eq!(power, preserved);
    }
}
#[test]
fn fractional_coolant_credit_survives_json_continuation_bit_for_bit() {
    let mut state = PowerState::default();
    // Observed in a real seeded duel. The non-roundtrip JSON parser changes
    // the final binary bits even though its decimal output appears similar.
    for value in [
        0.9667950463481249_f64,
        0.1_f64,
        f64::from_bits(0x3fefffffffffffff),
    ] {
        state.coolant_credit.insert("fixture-coolant".into(), value);
        let restored: PowerState =
            serde_json::from_str(&serde_json::to_string(&state).unwrap()).unwrap();
        assert_eq!(
            restored.coolant_credit["fixture-coolant"].to_bits(),
            value.to_bits()
        );
        assert_eq!(restored, state);
    }
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
    // This helper drives complete intervals. Integration itself also serves
    // callers which have already paid Input work in the current interval.
    power.begin_work_interval();
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
fn work_interval_preserves_input_receipts_through_integration_and_expires_them_explicitly() {
    let mut power = PowerSystem::default();
    let owner = power.config().consumers[0].system.clone();
    let capacity = power.config().consumers[0].reserve.capacity;
    power
        .network_mut()
        .consumers
        .get_mut(&owner)
        .unwrap()
        .energy = capacity;
    power.begin_work_interval();
    assert!(power.try_work_discrete_amount(&owner, 0.25).unwrap());
    let bindings = bindings(&power);
    let stocks = BTreeMap::from([("fixture-coolant".into(), 10)]);
    let mut equivalent = power.clone();
    // Removing derived receipts cannot change any canonical energy outcome.
    equivalent.begin_work_interval();
    let actual = power
        .tick_energy(0.25, &bindings, &stocks, &mut |_| 0)
        .unwrap();
    let expected = equivalent
        .tick_energy(0.25, &bindings, &stocks, &mut |_| 0)
        .unwrap();
    assert_eq!(actual, expected);
    assert_eq!(
        power.capture_continuation(),
        equivalent.capture_continuation()
    );
    assert_eq!(power.emitted_work(&owner), 0.25);
    assert!(power.try_work_discrete_amount(&owner, 0.25).unwrap());
    assert_eq!(power.emitted_work(&owner), 0.5);
    let canonical = power.capture_continuation();
    power.begin_work_interval();
    assert_eq!(power.emitted_work(&owner), 0.0);
    assert_eq!(power.capture_continuation(), canonical);
    power
        .tick_energy(0.25, &bindings, &stocks, &mut |_| 0)
        .unwrap();
    assert_eq!(
        power.emitted_work(&owner),
        0.0,
        "intake is not paid machinery work"
    );
}

#[test]
fn restoring_energy_never_restores_or_preserves_paid_work_receipts() {
    let mut power = PowerSystem::default();
    let consumer = power.config().consumers[0].clone();
    power
        .network_mut()
        .consumers
        .get_mut(&consumer.system)
        .unwrap()
        .energy = consumer.reserve.capacity;
    assert!(power
        .try_work_discrete_amount(&consumer.system, 0.25)
        .unwrap());
    let saved = power.capture_continuation();
    assert!(power.emitted_work(&consumer.system) > 0.0);
    let json = serde_json::to_string(&saved).unwrap();
    let decoded: PowerState = serde_json::from_str(&json).unwrap();
    power.restore_continuation(&decoded).unwrap();
    assert_eq!(power.emitted_work(&consumer.system), 0.0);
    assert_eq!(power.capture_continuation(), saved);
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

#[test]
fn recovered_charge_shares_the_damaged_intake_budget_with_actual_prior_delivery() {
    let mut power = PowerSystem::default();
    let id = power.config().consumers[0].system.clone();
    let maximum = power.config().consumers[0].reserve.max_intake_per_sec;
    power.network_mut().consumers.get_mut(&id).unwrap().energy = 0.0;
    power.last_intake_per_sec.insert(id.clone(), maximum * 0.25);
    let accepted = power
        .accept_recovered_energy_with_intake_factor(&id, 1.0, maximum, 0.5)
        .unwrap();
    assert!((accepted - maximum * 0.25).abs() < 0.0001);
    assert_eq!(
        power
            .accept_recovered_energy_with_intake_factor(&id, 1.0, maximum, 0.5)
            .unwrap(),
        0.0
    );
    assert_eq!(
        power
            .accept_recovered_energy_with_intake_factor(&id, 1.0, maximum, 0.0)
            .unwrap(),
        0.0
    );
    let before = power.capture_continuation();
    assert!(power
        .accept_recovered_energy_with_intake_factor(&id, 1.0, maximum, f32::NAN)
        .is_err());
    assert_eq!(power.capture_continuation(), before);
}

#[test]
fn compiled_definition_is_replaced_by_refit_and_rederived_before_saved_work_resumes() {
    let mut power = PowerSystem::default();
    tick(&mut power, 10);
    let before = power.capture_continuation();
    let previous = power.clone();
    assert!(std::sync::Arc::ptr_eq(
        &power.definition,
        &previous.definition
    ));
    let mut hardware = power.config().clone();
    hardware.reactor.sustainable_output_per_sec *= 0.5;
    hardware.reactor.reserve.charge_heat_per_unit += 0.1;
    for consumer in &mut hardware.consumers {
        consumer.continuous_cost_per_sec *= 0.5;
    }
    power.reconfigure_energy(hardware.clone()).unwrap();
    assert!(!std::sync::Arc::ptr_eq(
        &power.definition,
        &previous.definition
    ));
    assert_ne!(power.config(), previous.config());
    assert_eq!(power.capture_continuation(), before);
    let saved: PowerState = serde_json::from_str(&serde_json::to_string(&before).unwrap()).unwrap();
    let mut fresh = PowerSystem::from_energy(hardware, power.ship_config().clone()).unwrap();
    assert!(!std::sync::Arc::ptr_eq(
        &power.definition,
        &fresh.definition
    ));
    fresh.restore_continuation(&saved).unwrap();
    for step in 0..60 {
        assert_eq!(tick(&mut power, 10), tick(&mut fresh, 10), "result {step}");
        assert_eq!(
            power.capture_continuation(),
            fresh.capture_continuation(),
            "state {step}"
        );
    }
    let original = power.clone();
    let removed = power.config().consumers[0].system.clone();
    power.network_mut().consumers.remove(&removed);
    let malformed = power.clone();
    assert!(power
        .tick_energy(
            0.125,
            &bindings(&power),
            &BTreeMap::from([("fixture-coolant".into(), 10)]),
            &mut |_| 0
        )
        .is_err());
    assert_eq!(
        power, malformed,
        "public Power transaction must retain mutable-key refusal"
    );
    assert_ne!(
        power.capture_continuation(),
        original.capture_continuation()
    );
}
