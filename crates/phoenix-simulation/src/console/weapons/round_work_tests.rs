use super::*;

#[test]
fn actual_local_rounds_and_energy_commit_per_barrel_and_shortfalls_leave_both_owners_unchanged() {
    let config = crate::entities::include_resolve::load_entity_config(
        "assets/entities/broadside_cruiser.toml",
    )
    .unwrap();
    let bank = config
        .weapons_console
        .as_ref()
        .unwrap()
        .blaster_banks
        .iter()
        .find(|bank| bank.rounds.is_some())
        .unwrap();
    let feed = bank.rounds.clone().unwrap();
    let owner = crate::ship::system_registry::blaster_bank_system_id(&bank.id).unwrap();
    let logistics = crate::resources::logistics_server::CompiledShipLogistics(
        crate::resources::logistics_profile::compile_entity_logistics(&config)
            .unwrap()
            .unwrap(),
    );
    let phoenix_model::logistics::ResourceEndpoint::Capacity { id: capacity } =
        &logistics.0.store(&feed.store).unwrap().endpoint
    else {
        panic!("rounds need a real capacity")
    };
    assert_ne!(
        &feed.store, capacity,
        "managed store identity is distinct from its capacity endpoint"
    );
    let mut inventory = InfrastructureCondition(
        crate::infrastructure::condition::InfrastructureState::from_config(
            config.infrastructure.as_ref().unwrap(),
        ),
    );
    let mut power = ShipPowerSystem(
        crate::modifiers::power_system::PowerSystem::from_energy(
            config.energy.unwrap(),
            config.ship_config.unwrap(),
        )
        .unwrap(),
    );
    power
        .0
        .network_mut()
        .consumers
        .get_mut(&owner)
        .unwrap()
        .energy = 0.0;
    let before_inventory = inventory.clone();
    let before_power = power.0.capture_continuation();
    assert!(pay_barrel(
        &owner,
        Some(&feed),
        0,
        Some(&mut power),
        Some(&mut inventory),
        Some(&logistics),
        &Default::default()
    )
    .is_none());
    assert_eq!(inventory, before_inventory);
    assert_eq!(power.0.capture_continuation(), before_power);

    let cost = power
        .0
        .config()
        .consumers
        .iter()
        .find(|row| row.system == owner)
        .unwrap()
        .discrete_cost;
    assert!(cost > 0.0);
    power
        .0
        .network_mut()
        .consumers
        .get_mut(&owner)
        .unwrap()
        .energy = cost * 2.0;
    let initial = inventory.0.capacity_stock(capacity).unwrap().level;
    let before_inventory = inventory.clone();
    let before_power = power.0.capture_continuation();
    assert!(
        pay_barrel(
            &owner,
            Some(&feed),
            0,
            Some(&mut power),
            Some(&mut inventory),
            Some(&logistics),
            &std::collections::BTreeMap::from([(capacity.clone(), initial)])
        )
        .is_none(),
        "quantity already committed to an earlier physical owner is unavailable to this barrel"
    );
    assert_eq!(inventory, before_inventory);
    assert_eq!(power.0.capture_continuation(), before_power);
    assert!(pay_barrel(
        &owner,
        Some(&feed),
        0,
        Some(&mut power),
        Some(&mut inventory),
        Some(&logistics),
        &Default::default()
    )
    .is_some());
    assert!(pay_barrel(
        &owner,
        Some(&feed),
        1,
        Some(&mut power),
        Some(&mut inventory),
        Some(&logistics),
        &Default::default()
    )
    .is_some());
    assert_eq!(
        inventory.0.capacity_stock(capacity).unwrap().level,
        initial - feed.amount * 2
    );
    assert!(inventory
        .0
        .capacity_stock(capacity)
        .unwrap()
        .reserved
        .is_empty());
    assert_eq!(power.0.network().consumers[&owner].energy, 0.0);

    let empty =
        crate::resources::Stock::new(0, inventory.0.capacity_stock(capacity).unwrap().ceiling)
            .unwrap();
    inventory.0.replace_capacity_stock(capacity, empty).unwrap();
    power
        .0
        .network_mut()
        .consumers
        .get_mut(&owner)
        .unwrap()
        .energy = cost * 2.0;
    let before_power = power.0.capture_continuation();
    let before_inventory = inventory.clone();
    assert!(pay_barrel(
        &owner,
        Some(&feed),
        0,
        Some(&mut power),
        Some(&mut inventory),
        Some(&logistics),
        &Default::default()
    )
    .is_none());
    assert_eq!(power.0.capture_continuation(), before_power);
    assert_eq!(inventory, before_inventory);
}
