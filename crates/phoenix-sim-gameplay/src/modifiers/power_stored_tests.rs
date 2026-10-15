use super::*;
fn actual_probe() -> PowerSystem {
    let config =
        crate::test_fixtures::load_entity_config("assets/entities/battery_probe.toml").unwrap();
    PowerSystem::from_energy(config.energy.unwrap(), config.ship_config.unwrap()).unwrap()
}
#[test]
fn carried_actual_reserves_age_without_generation_routing_coolant_or_free_work() {
    let mut power = actual_probe();
    let owner = SystemId("sensor-radar".into());
    power.network_mut().consumers.get_mut(&owner).unwrap().heat = 10.0;
    power.work_continuous(&owner, 0.25, 1.0).unwrap();
    assert!(power.emitted_work(&owner) > 0.0);
    let before = power.capture_continuation();
    power.tick_stored_energy(1.0).unwrap();
    let after = power.capture_continuation();
    assert!(after.network.consumers[&owner].energy < before.network.consumers[&owner].energy);
    assert!(after.network.consumers[&owner].heat < before.network.consumers[&owner].heat);
    assert_eq!(power.emitted_work(&owner), 0.0);
    assert_eq!(power.last_work_fraction(&owner), Some(0.0));
    assert_eq!(power.last_intake_per_sec(&owner), Some(0.0));
    assert_eq!(after.network.output_actual_per_sec, 0.0);
    assert_eq!(after.coolant_credit, before.coolant_credit);
    assert_eq!(
        after.network.generation_enabled,
        before.network.generation_enabled
    );
    assert_eq!(
        after.network.output_target_per_sec,
        before.network.output_target_per_sec
    );
    assert_eq!(after.network.fine_flows, before.network.fine_flows);
    for (id, group) in &after.network.groups {
        assert_eq!(
            group.desired_flow_per_sec,
            before.network.groups[id].desired_flow_per_sec
        );
        assert_eq!(group.effective_flow_per_sec, 0.0);
        assert_eq!(group.effective_cooling_per_sec, 0.0);
    }
    let mut restored = actual_probe();
    restored.restore_continuation(&after).unwrap();
    for _ in 0..360 {
        power.tick_stored_energy(1.0 / 60.0).unwrap();
        restored.tick_stored_energy(1.0 / 60.0).unwrap();
        assert_eq!(
            power.capture_continuation(),
            restored.capture_continuation()
        );
    }
}
#[test]
fn malformed_stored_interval_never_changes_the_actual_reserves_or_readings() {
    let mut power = actual_probe();
    let before = power.clone();
    for dt in [f32::NAN, f32::INFINITY, -1.0] {
        assert!(power.tick_stored_energy(dt).is_err());
        assert_eq!(power, before);
    }
}
