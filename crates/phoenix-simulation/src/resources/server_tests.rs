use super::*;
use bevy::ecs::system::RunSystemOnce;

fn capacity(amount: i64, ceiling: i64) -> InfrastructureCondition {
    InfrastructureCondition(crate::infrastructure::InfrastructureState::from_config(
        &crate::infrastructure::InfrastructureConfig {
            capacities: vec![crate::infrastructure::CapacityConfig {
                id: "torpedoes".into(),
                amount,
                ceiling: Some(ceiling),
                label: None,
            }],
            ..Default::default()
        },
    ))
}
fn request(operator: &str, source: &str, destination: &str, amount: i64) -> ResourceTransfer {
    ResourceTransfer {
        operator: operator.into(),
        source: source.into(),
        destination: destination.into(),
        source_endpoint: ResourceEndpoint::Capacity {
            id: "torpedoes".into(),
        },
        destination_endpoint: ResourceEndpoint::Capacity {
            id: "torpedoes".into(),
        },
        amount,
    }
}
fn run(requests: Vec<ResourceTransfer>) -> (World, Vec<CapacityAdjustment>) {
    let mut world = World::new();
    world.insert_resource(EffectQueue(requests));
    world.insert_resource(EffectQueue::<CapacityAdjustment>::default());
    world.spawn((EntityUuid("depot".into()), capacity(7, 7)));
    world.spawn((EntityUuid("a".into()), capacity(0, 6)));
    world.spawn((EntityUuid("b".into()), capacity(0, 6)));
    world
        .run_system_once(tick_resource_transfers)
        .expect("resource transaction");
    let adjustments = world
        .resource::<EffectQueue<CapacityAdjustment>>()
        .0
        .clone();
    (world, adjustments)
}
#[test]
fn simultaneous_links_reserve_one_supply_in_stable_order() {
    let (_, forward) = run(vec![
        request("a", "depot", "a", 6),
        request("b", "depot", "b", 6),
    ]);
    let (_, reverse) = run(vec![
        request("b", "depot", "b", 6),
        request("a", "depot", "a", 6),
    ]);
    assert_eq!(forward, reverse);
    assert_eq!(forward.iter().map(|delta| delta.delta).sum::<i64>(), 0);
    assert_eq!(
        forward
            .iter()
            .filter(|delta| delta.uuid == "depot")
            .map(|delta| delta.delta)
            .sum::<i64>(),
        -7
    );
    assert_eq!(
        forward
            .iter()
            .find(|delta| delta.uuid == "a")
            .unwrap()
            .delta,
        6
    );
    assert_eq!(
        forward
            .iter()
            .find(|delta| delta.uuid == "b")
            .unwrap()
            .delta,
        1
    );
}
#[test]
fn missing_destination_takes_nothing_and_prior_stock_effects_are_honoured() {
    let (mut world, deltas) = run(vec![request("a", "depot", "missing", 6)]);
    assert!(deltas.is_empty());
    world
        .resource_mut::<EffectQueue<CapacityAdjustment>>()
        .0
        .push(CapacityAdjustment {
            uuid: "depot".into(),
            capacity: "torpedoes".into(),
            delta: -5,
        });
    world
        .resource_mut::<EffectQueue<ResourceTransfer>>()
        .0
        .push(request("a", "depot", "a", 6));
    world.run_system_once(tick_resource_transfers).unwrap();
    let deltas = &world.resource::<EffectQueue<CapacityAdjustment>>().0;
    assert_eq!(
        deltas
            .iter()
            .filter(|delta| delta.uuid == "a")
            .map(|delta| delta.delta)
            .sum::<i64>(),
        2
    );
    assert_eq!(deltas.iter().map(|delta| delta.delta).sum::<i64>(), -5);
}
#[test]
fn docked_ammunition_replenishes_without_using_tube_held_space() {
    let mut system =
        crate::weapons::torpedo::TorpedoSystem::new(crate::weapons::torpedo::TorpedoConfig {
            count: 4,
            load_time: 1.0,
            ..Default::default()
        });
    assert!(system.start_load("fore_port"));
    system.tick(1.0, &std::collections::HashMap::new(), &mut || {
        "unused".to_string()
    });
    system.magazine.adjust(-3);
    let mut world = World::new();
    let mut flow = request("crew", "depot", "crew", 7);
    flow.destination_endpoint = ResourceEndpoint::TorpedoMagazine {};
    world.insert_resource(EffectQueue(vec![flow]));
    world.insert_resource(EffectQueue::<CapacityAdjustment>::default());
    world.spawn((EntityUuid("depot".into()), capacity(7, 7)));
    let crew = world
        .spawn((EntityUuid("crew".into()), TorpedoSystemResource(system)))
        .id();
    world.run_system_once(tick_resource_transfers).unwrap();
    let system = &world.get::<TorpedoSystemResource>(crew).unwrap().0;
    assert_eq!(system.torpedoes_remaining(), 3);
    assert_eq!(system.rounds_aboard(), 4);
    assert_eq!(system.magazine_headroom(), 0);
    system.validate_magazine().unwrap();
    assert_eq!(
        world.resource::<EffectQueue<CapacityAdjustment>>().0[0].delta,
        -3
    );
}
