//! Actual optional recipe acceptance; the integrator registers this owner module.
use phoenix_model::identity::SystemId;

#[test]
fn comms_disruptor_preserves_base_hardware_and_uses_existing_physical_emp_bank() {
    let load = |path| crate::test_fixtures::load_entity_config(path).unwrap();
    let base = load("assets/entities/alliance_cruiser.toml");
    let configured = load("assets/entities/disruptor_cruiser.toml");
    let ship = configured.ship_config.as_ref().unwrap();
    let baseline = base.ship_config.as_ref().unwrap();
    assert_eq!(ship.stations, baseline.stations);
    for system in &baseline.systems {
        assert_eq!(ship.system(&system.id), Some(system));
    }
    let owner = SystemId("blaster-disruptor".into());
    let console = ship.console_for_system(&owner).unwrap();
    assert_eq!(console.home_station.as_ref().unwrap().0, "comms");
    assert_eq!(
        console.screen.as_deref(),
        Some("gui/destroyer/tactical.html")
    );
    let energy = configured.energy.as_ref().unwrap();
    assert_eq!(energy.reactor, base.energy.as_ref().unwrap().reactor);
    for consumer in &base.energy.as_ref().unwrap().consumers {
        assert_eq!(
            energy
                .consumers
                .iter()
                .find(|row| row.system == consumer.system),
            Some(consumer)
        );
    }
    let bank = configured
        .weapons_console
        .as_ref()
        .unwrap()
        .blaster_banks
        .iter()
        .find(|bank| bank.id == "disruptor")
        .unwrap();
    assert_eq!(bank.impact.emp.as_ref().unwrap().duration_secs, 3.0);
    assert!(bank.projectile_speed > 0.0 && bank.damage > 0);
    assert!(energy
        .consumers
        .iter()
        .any(|row| row.system == owner && row.initial_energy > 0.0 && row.discrete_cost > 0.0));
    assert_eq!(
        configured.interior.as_ref().unwrap().placements[&owner],
        "weapons"
    );
}
