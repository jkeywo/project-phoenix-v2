use super::*;
use crate::infrastructure::CapacityAdjustment;
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
        receipt: None,
    }
}
fn quantity(world: &mut World, uuid: &str, id: &str) -> i64 {
    world
        .query::<(&EntityUuid, &InfrastructureCondition)>()
        .iter(world)
        .find(|(key, _)| key.0 == uuid)
        .unwrap()
        .1
         .0
        .capacity_stock(id)
        .unwrap()
        .level
}
fn run(requests: Vec<ResourceTransfer>) -> World {
    let mut world = settlement_world(requests);
    world.spawn((EntityUuid("depot".into()), capacity(7, 7)));
    world.spawn((EntityUuid("a".into()), capacity(0, 6)));
    world.spawn((EntityUuid("b".into()), capacity(0, 6)));
    publish_capacities(&mut world);
    world.run_system_once(tick_resource_transfers).unwrap();
    world
}
#[test]
fn simultaneous_links_reserve_one_supply_in_stable_order() {
    let mut forward = run(vec![
        request("a", "depot", "a", 6),
        request("b", "depot", "b", 6),
    ]);
    let mut reverse = run(vec![
        request("b", "depot", "b", 6),
        request("a", "depot", "a", 6),
    ]);
    for world in [&mut forward, &mut reverse] {
        assert_eq!(quantity(world, "depot", "torpedoes"), 0);
        assert_eq!(quantity(world, "a", "torpedoes"), 6);
        assert_eq!(quantity(world, "b", "torpedoes"), 1);
        assert!(world
            .resource::<EffectQueue<CapacityAdjustment>>()
            .0
            .is_empty());
        assert!(world
            .resource::<EffectQueue<ResourceTransferReceipt>>()
            .0
            .is_empty());
    }
}
#[test]
fn missing_destination_takes_nothing_and_prior_stock_effects_are_honoured() {
    let mut world = run(vec![request("a", "depot", "missing", 6)]);
    assert_eq!(quantity(&mut world, "depot", "torpedoes"), 7);
    assert_eq!(quantity(&mut world, "a", "torpedoes"), 0);
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
    publish_capacities(&mut world);
    world.run_system_once(tick_resource_transfers).unwrap();
    assert_eq!(quantity(&mut world, "depot", "torpedoes"), 0);
    assert_eq!(quantity(&mut world, "a", "torpedoes"), 2);
    assert!(world
        .resource::<EffectQueue<CapacityAdjustment>>()
        .0
        .is_empty());
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
    let mut flow = request("crew", "depot", "crew", 7);
    flow.destination_endpoint = ResourceEndpoint::TorpedoMagazine {};
    let mut world = settlement_world(vec![flow]);
    world.spawn((EntityUuid("depot".into()), capacity(7, 7)));
    let crew = world
        .spawn((EntityUuid("crew".into()), TorpedoSystemResource(system)))
        .id();
    publish_capacities(&mut world);
    world.run_system_once(tick_resource_transfers).unwrap();
    let system = &world.get::<TorpedoSystemResource>(crew).unwrap().0;
    assert_eq!(system.torpedoes_remaining(), 3);
    assert_eq!(system.rounds_aboard(), 4);
    assert_eq!(system.magazine_headroom(), 0);
    system.validate_magazine().unwrap();
    assert_eq!(quantity(&mut world, "depot", "torpedoes"), 4);
    assert!(world
        .resource::<EffectQueue<CapacityAdjustment>>()
        .0
        .is_empty());
}

fn receipt_request(
    operator: &str,
    source: &str,
    destination: &str,
    amount: i64,
) -> ResourceTransfer {
    let mut flow = request(operator, source, destination, amount);
    flow.receipt = Some(TransferReceiptKey {
        system: SystemId("recovery".into()),
        tick: 17,
    });
    flow
}

fn inventory_fixture() -> (
    crate::entities::config::EntityConfig,
    CompiledShipLogistics,
    ShipLogistics,
    InfrastructureCondition,
) {
    let content = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    let profile = crate::resources::logistics_profile::compile_entity_logistics(&content)
        .unwrap()
        .unwrap();
    let state = crate::resources::logistics::LogisticsState::new(&profile);
    let inventory =
        InfrastructureCondition(crate::infrastructure::InfrastructureState::from_config(
            content.infrastructure.as_ref().unwrap(),
        ));
    (
        content,
        CompiledShipLogistics(profile),
        ShipLogistics(state),
        inventory,
    )
}

fn settlement_world(requests: Vec<ResourceTransfer>) -> World {
    let mut world = World::new();
    world.insert_resource(EffectQueue(requests));
    world.insert_resource(EffectQueue::<CapacityAdjustment>::default());
    world.insert_resource(EffectQueue::<ResourceTransferReceipt>::default());
    world.insert_resource(crate::world::server::WorldContentRuntime::default());
    world.insert_resource(EffectQueue::<crate::infrastructure::ConditionAdjustment>::default());
    world
}

fn magazines(content: &crate::entities::config::EntityConfig) -> TorpedoSystemResource {
    let config = content.torpedoes.as_ref().unwrap();
    TorpedoSystemResource(crate::weapons::torpedo::TorpedoSystem::from_configs(
        &config.tubes,
        config.to_runtime(),
    ))
}

fn publish_capacities(world: &mut World) {
    world
        .run_system_once(crate::infrastructure::server::tick_infrastructure_condition)
        .unwrap();
    assert!(world
        .resource::<EffectQueue<CapacityAdjustment>>()
        .0
        .is_empty());
}

#[test]
fn main_floors_use_live_policy_and_prior_effects_across_competing_requests() {
    fn settle(reverse: bool) -> (Vec<ResourceTransferReceipt>, Vec<i64>) {
        let (content, profile, mut state, mut stock) = inventory_fixture();
        // Authored reserve is zero. The operator's current floor is three.
        assert_eq!(state.0.floors["hold-ammunition"], 0);
        state.0.floors.insert("hold-ammunition".into(), 3);
        let mut held = stock.0.capacity_stock("hold-ammunition").unwrap().clone();
        held.reserve("existing-job", 1).unwrap();
        stock
            .0
            .replace_capacity_stock("hold-ammunition", held)
            .unwrap();
        let mut requests: Vec<_> = ["a", "b", "c"]
            .into_iter()
            .map(|operator| {
                let mut flow = receipt_request(operator, "hold", operator, 2);
                flow.source_endpoint = ResourceEndpoint::Capacity {
                    id: "hold-ammunition".into(),
                };
                flow
            })
            .collect();
        if reverse {
            requests.reverse();
        }
        let mut world = settlement_world(requests);
        let source = world
            .spawn((
                EntityUuid("hold".into()),
                stock,
                profile,
                state,
                magazines(&content),
            ))
            .id();
        let receivers: Vec<_> = ["a", "b", "c"]
            .into_iter()
            .map(|id| world.spawn((EntityUuid(id.into()), capacity(0, 6))).id())
            .collect();
        // Earlier effects replenish four to six before settlement. A source
        // clone taken before the effect or a per-request floor check is wrong.
        world
            .resource_mut::<EffectQueue<CapacityAdjustment>>()
            .0
            .push(CapacityAdjustment {
                uuid: "hold".into(),
                capacity: "hold-ammunition".into(),
                delta: 2,
            });
        publish_capacities(&mut world);
        world.run_system_once(tick_resource_transfers).unwrap();
        let receipts = std::mem::take(
            &mut world
                .resource_mut::<EffectQueue<ResourceTransferReceipt>>()
                .0,
        );
        assert_eq!(
            receipts.iter().map(|r| r.moved).collect::<Vec<_>>(),
            vec![2, 1, 0]
        );
        assert!(receipts
            .iter()
            .all(|r| r.requested == 2 && r.key.tick == 17));
        publish_capacities(&mut world);
        let source = world
            .get::<InfrastructureCondition>(source)
            .unwrap()
            .0
            .capacity_stock("hold-ammunition")
            .unwrap();
        assert_eq!(source.level, 3);
        assert_eq!(source.reserved["existing-job"], 1);
        let levels: Vec<_> = receivers
            .into_iter()
            .map(|entity| {
                world
                    .get::<InfrastructureCondition>(entity)
                    .unwrap()
                    .0
                    .capacity_stock("torpedoes")
                    .unwrap()
                    .level
            })
            .collect();
        assert_eq!(source.level + levels.iter().sum::<i64>(), 6);
        assert!(world
            .resource::<EffectQueue<ResourceTransferReceipt>>()
            .0
            .is_empty());
        (receipts, levels)
    }
    assert_eq!(settle(false), settle(true));
}

#[test]
fn receipts_report_shared_bay_and_incoming_limited_actual_settlement() {
    let (mut content, _, _, mut inventory) = inventory_fixture();
    let support = content
        .logistics
        .as_ref()
        .unwrap()
        .stores
        .iter()
        .filter(|store| store.compartment == "support")
        .map(|store| match &store.endpoint {
            ResourceEndpoint::Capacity { id } => inventory.0.capacity_stock(id).unwrap().level,
            _ => panic!("support stores are actual capacities"),
        })
        .sum::<i64>();
    content
        .logistics
        .as_mut()
        .unwrap()
        .storage
        .iter_mut()
        .find(|bay| bay.compartment == "support")
        .unwrap()
        .space = support + 3;
    let profile = crate::resources::logistics_profile::compile_entity_logistics(&content)
        .unwrap()
        .unwrap();
    let state = crate::resources::logistics::LogisticsState::new(&profile);
    let mut alloy = inventory.0.capacity_stock("alloy").unwrap().clone();
    alloy.reserve_incoming("active-delivery", 1).unwrap();
    inventory.0.replace_capacity_stock("alloy", alloy).unwrap();
    let mut requests: Vec<_> = ["a", "b"]
        .into_iter()
        .map(|operator| {
            let mut flow = receipt_request(operator, "depot", "crew", 4);
            flow.destination_endpoint = ResourceEndpoint::Capacity { id: "alloy".into() };
            flow
        })
        .collect();
    requests.reverse();
    let mut world = settlement_world(requests);
    let depot = world
        .spawn((EntityUuid("depot".into()), capacity(7, 7)))
        .id();
    let crew = world
        .spawn((
            EntityUuid("crew".into()),
            inventory,
            CompiledShipLogistics(profile),
            ShipLogistics(state),
            magazines(&content),
        ))
        .id();
    publish_capacities(&mut world);
    world.run_system_once(tick_resource_transfers).unwrap();
    let receipts = &world.resource::<EffectQueue<ResourceTransferReceipt>>().0;
    assert_eq!(
        receipts
            .iter()
            .map(|r| (r.operator.as_str(), r.requested, r.moved))
            .collect::<Vec<_>>(),
        vec![("a", 4, 2), ("b", 4, 0)]
    );
    assert_eq!(
        world
            .resource::<EffectQueue<CapacityAdjustment>>()
            .0
            .iter()
            .map(|d| d.delta)
            .sum::<i64>(),
        0
    );
    publish_capacities(&mut world);
    let actual = world
        .get::<InfrastructureCondition>(crew)
        .unwrap()
        .0
        .capacity_stock("alloy")
        .unwrap();
    assert_eq!(actual.level, 14);
    assert_eq!(actual.incoming["active-delivery"], 1);
    assert!(
        actual.headroom() > 0,
        "shared room, not the per-resource ceiling, stopped settlement"
    );
    assert_eq!(
        world
            .get::<InfrastructureCondition>(depot)
            .unwrap()
            .0
            .capacity_stock("torpedoes")
            .unwrap()
            .level,
        5
    );
}

#[test]
fn keyed_refusals_each_receive_zero_and_legacy_requests_do_not_get_receipts() {
    let (content, profile, state, inventory) = inventory_fixture();
    let mut floor = receipt_request("floor", "hold", "crew", 5);
    floor.source_endpoint = ResourceEndpoint::Capacity {
        id: "hold-machinery-parts".into(),
    };
    let mut world = settlement_world(vec![
        receipt_request("missing", "depot", "absent", 2),
        receipt_request("same", "depot", "depot", 2),
        receipt_request("zero", "depot", "crew", 0),
        receipt_request("negative", "depot", "crew", -1),
        floor,
        request("legacy", "depot", "crew", 1),
    ]);
    // A fitted Main store without its live policy must not export against an
    // authored default or invent a reserve floor.
    drop(state);
    world.spawn((
        EntityUuid("hold".into()),
        inventory,
        profile,
        magazines(&content),
    ));
    world.spawn((EntityUuid("depot".into()), capacity(7, 7)));
    world.spawn((EntityUuid("crew".into()), capacity(0, 6)));
    publish_capacities(&mut world);
    world.run_system_once(tick_resource_transfers).unwrap();
    let receipts = &world.resource::<EffectQueue<ResourceTransferReceipt>>().0;
    assert_eq!(receipts.len(), 5);
    assert!(receipts.iter().all(|r| r.moved == 0));
    assert_eq!(
        receipts
            .iter()
            .map(|r| r.operator.as_str())
            .collect::<Vec<_>>(),
        vec!["floor", "missing", "negative", "same", "zero"]
    );
    let deltas = &world.resource::<EffectQueue<CapacityAdjustment>>().0;
    assert!(deltas.is_empty());
    assert_eq!(quantity(&mut world, "depot", "torpedoes"), 6);
    assert_eq!(quantity(&mut world, "crew", "torpedoes"), 1);
}

#[test]
fn transfers_preserve_existing_claims_while_moving_only_unclaimed_stock() {
    let mut stock = capacity(4, 6);
    let mut actual = stock.0.capacity_stock("torpedoes").unwrap().clone();
    actual.reserve("transfer:collector", 2).unwrap();
    stock.0.replace_capacity_stock("torpedoes", actual).unwrap();
    let mut world = settlement_world(vec![receipt_request("collector", "source", "crew", 2)]);
    let source = world.spawn((EntityUuid("source".into()), stock)).id();
    world.spawn((EntityUuid("crew".into()), capacity(0, 6)));
    publish_capacities(&mut world);
    world.run_system_once(tick_resource_transfers).unwrap();
    assert_eq!(
        world.resource::<EffectQueue<ResourceTransferReceipt>>().0[0].moved,
        2
    );
    assert!(world
        .resource::<EffectQueue<CapacityAdjustment>>()
        .0
        .is_empty());
    let actual = world
        .get::<InfrastructureCondition>(source)
        .unwrap()
        .0
        .capacity_stock("torpedoes")
        .unwrap();
    assert_eq!(actual.level, 2);
    assert_eq!(actual.reserved["transfer:collector"], 2);
    assert_eq!(quantity(&mut world, "crew", "torpedoes"), 2);
}

#[test]
fn a_prior_refill_rejected_by_shared_storage_cannot_be_exported() {
    let (mut content, _, mut state, inventory) = inventory_fixture();
    let occupied: i64 = content
        .logistics
        .as_ref()
        .unwrap()
        .stores
        .iter()
        .filter(|store| store.compartment == "support")
        .map(|store| match &store.endpoint {
            ResourceEndpoint::Capacity { id } => inventory.0.capacity_stock(id).unwrap().level,
            _ => unreachable!(),
        })
        .sum();
    content
        .logistics
        .as_mut()
        .unwrap()
        .storage
        .iter_mut()
        .find(|bay| bay.compartment == "support")
        .unwrap()
        .space = occupied;
    let profile = crate::resources::logistics_profile::compile_entity_logistics(&content)
        .unwrap()
        .unwrap();
    state.0.floors.insert("hold-ammunition".into(), 4);
    let before = inventory
        .0
        .capacity_stock("hold-ammunition")
        .unwrap()
        .clone();
    let mut flow = receipt_request("collector", "hold", "crew", 2);
    flow.source_endpoint = ResourceEndpoint::Capacity {
        id: "hold-ammunition".into(),
    };
    let mut world = settlement_world(vec![flow]);
    let source = world
        .spawn((
            EntityUuid("hold".into()),
            inventory,
            CompiledShipLogistics(profile),
            state,
            magazines(&content),
        ))
        .id();
    let destination = world
        .spawn((EntityUuid("crew".into()), capacity(0, 6)))
        .id();
    world
        .resource_mut::<EffectQueue<CapacityAdjustment>>()
        .0
        .push(CapacityAdjustment {
            uuid: "hold".into(),
            capacity: "hold-ammunition".into(),
            delta: 2,
        });
    publish_capacities(&mut world);
    world.run_system_once(tick_resource_transfers).unwrap();
    assert_eq!(
        world.resource::<EffectQueue<ResourceTransferReceipt>>().0[0].moved,
        0
    );
    assert!(world
        .resource::<EffectQueue<CapacityAdjustment>>()
        .0
        .is_empty());
    publish_capacities(&mut world);
    assert_eq!(
        world
            .get::<InfrastructureCondition>(source)
            .unwrap()
            .0
            .capacity_stock("hold-ammunition")
            .unwrap(),
        &before
    );
    assert_eq!(
        world
            .get::<InfrastructureCondition>(destination)
            .unwrap()
            .0
            .capacity_stock("torpedoes")
            .unwrap()
            .level,
        0
    );
}

#[test]
fn mixed_magazine_and_capacity_transfers_settle_in_their_actual_order() {
    let (mut content, _, _, mut inventory) = inventory_fixture();
    let logistics = content.logistics.as_mut().unwrap();
    logistics
        .stores
        .iter_mut()
        .find(|store| store.id == "hold-ammunition")
        .unwrap()
        .compartment = "weapons".into();
    // Start at capacity2 + magazine2 with two free shared slots. The ordered
    // sequence is capacity-in2, capacity-out2, magazine-in2. Committing the
    // magazine before earlier capacities makes the first incoming effect fail.
    inventory.0.adjust_capacity("hold-ammunition", -2).unwrap();
    let profile = crate::resources::logistics_profile::compile_entity_logistics(&content)
        .unwrap()
        .unwrap();
    let state = crate::resources::logistics::LogisticsState::new(&profile);
    let cap = ResourceEndpoint::Capacity {
        id: "hold-ammunition".into(),
    };
    let mut incoming = receipt_request("a", "depot", "crew", 2);
    incoming.destination_endpoint = cap.clone();
    let mut outgoing = receipt_request("b", "crew", "receiver", 2);
    outgoing.source_endpoint = cap;
    let mut magazine = receipt_request("c", "depot", "crew", 2);
    magazine.destination_endpoint = ResourceEndpoint::TorpedoMagazine {};
    let mut world = settlement_world(vec![magazine, outgoing, incoming]);
    let crew = world
        .spawn((
            EntityUuid("crew".into()),
            inventory,
            CompiledShipLogistics(profile),
            ShipLogistics(state),
            magazines(&content),
        ))
        .id();
    let depot = world
        .spawn((EntityUuid("depot".into()), capacity(7, 7)))
        .id();
    let receiver = world
        .spawn((EntityUuid("receiver".into()), capacity(0, 6)))
        .id();
    publish_capacities(&mut world);
    world.run_system_once(tick_resource_transfers).unwrap();
    assert_eq!(
        world
            .resource::<EffectQueue<ResourceTransferReceipt>>()
            .0
            .iter()
            .map(|r| r.moved)
            .collect::<Vec<_>>(),
        vec![2, 2, 2]
    );
    publish_capacities(&mut world);
    let hold = world
        .get::<InfrastructureCondition>(crew)
        .unwrap()
        .0
        .capacity_stock("hold-ammunition")
        .unwrap()
        .level;
    let magazine = world
        .get::<TorpedoSystemResource>(crew)
        .unwrap()
        .0
        .rounds_aboard();
    let depot = world
        .get::<InfrastructureCondition>(depot)
        .unwrap()
        .0
        .capacity_stock("torpedoes")
        .unwrap()
        .level;
    let receiver = world
        .get::<InfrastructureCondition>(receiver)
        .unwrap()
        .0
        .capacity_stock("torpedoes")
        .unwrap()
        .level;
    assert_eq!((hold, magazine, depot, receiver), (2, 4, 3, 2));
    assert_eq!(hold + magazine + depot + receiver, 11);
}
