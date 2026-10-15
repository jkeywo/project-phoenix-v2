use super::*;
use crate::core::messages::SystemControlPayload as Control;
use crate::core::messages::SystemId;
use crate::resources::logistics_server::tests::{fixture_with, send};
use crate::ship::control_source::ControlSource;

const VISITOR: &str = "00000000-0000-0000-0000-000000000002";
const PROVIDER: &str = "00000000-0000-0000-0000-000000000003";

fn id(value: &str) -> SystemId {
    SystemId(value.into())
}

fn rig(policy: PortServicePolicy) -> (App, Entity, Entity) {
    let (mut app, ship, provider) = rig_with(policy, |_| {});
    // These replenishment cases start after finite ordinary parts use. Preserve
    // authored ceilings; custom full-bay cases use rig_with directly instead.
    let mut inventory = app
        .world_mut()
        .get_mut::<crate::infrastructure::InfrastructureCondition>(ship)
        .unwrap();
    let mut stock = inventory
        .0
        .capacity_stock("hold-machinery-parts")
        .unwrap()
        .clone();
    stock.reserve("fixture-completed-parts-use", 20).unwrap();
    assert_eq!(stock.consume("fixture-completed-parts-use"), Some(20));
    inventory
        .0
        .replace_capacity_stock("hold-machinery-parts", stock)
        .unwrap();
    drop(inventory);
    (app, ship, provider)
}
fn rig_with(
    policy: PortServicePolicy,
    edit: impl FnOnce(&mut crate::entities::config::EntityConfig),
) -> (App, Entity, Entity) {
    let hull = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    let provider =
        crate::entities::include_resolve::load_entity_config("assets/entities/station_axiom.toml")
            .unwrap();
    let (mut app, ship) = fixture_with(ControlSource::Human, edit);
    app.add_plugins(crate::umbilical::UmbilicalPlugin);
    register(&mut app);
    app.init_resource::<crate::world::server::WorldContentRuntime>()
        .init_resource::<crate::effect_queue::EffectQueue<crate::resources::server::ResourceTransfer>>()
        .init_resource::<crate::effect_queue::EffectQueue<crate::resources::server::ResourceTransferReceipt>>()
        .init_resource::<crate::effect_queue::EffectQueue<crate::infrastructure::CapacityAdjustment>>()
        .init_resource::<crate::effect_queue::EffectQueue<crate::infrastructure::ConditionAdjustment>>()
        .add_systems(FixedUpdate, crate::infrastructure::tick_infrastructure_condition.in_set(crate::sim_sets::SimSet::Modifiers))
        .add_systems(FixedUpdate, crate::resources::server::tick_resource_transfers.in_set(crate::sim_sets::SimSet::Modifiers).after(crate::umbilical::tick_umbilical).before(crate::resources::logistics_server::tick));
    let faction = uuid::Uuid::parse_str("aaaaaaaa-1111-4111-8111-aaaaaaaaaaaa").unwrap();
    let mut dock = crate::dock::DockControl::from_system(
        hull.ship_config
            .as_ref()
            .unwrap()
            .system(&id("dock"))
            .unwrap(),
    );
    dock.set_docking_target(Some(PROVIDER.into()));
    dock.set_engaged(true);
    dock.docked = true;
    let umbilical_system = hull
        .ship_config
        .as_ref()
        .unwrap()
        .system(&id("umbilical"))
        .unwrap();
    app.world_mut().entity_mut(ship).insert((
        EntityUuid(VISITOR.into()),
        FactionComponent(faction),
        dock,
        TransferUmbilical::new(
            hull.umbilical.unwrap(),
            umbilical_system.power_group.clone().unwrap(),
        ),
    ));
    {
        let mut sources = app
            .world_mut()
            .get_mut::<crate::ship_plugin::ShipSystemControlSources>(ship)
            .unwrap();
        sources.0.set(id("umbilical"), ControlSource::Human);
        sources.0.set(id("logistics"), ControlSource::Offline);
        sources.0.set(id("fabrication"), ControlSource::Offline);
    }
    app.world_mut()
        .get_mut::<ShipLogistics>(ship)
        .unwrap()
        .0
        .enabled = false;
    let profile = PortServiceConfig {
        policy,
        eligible_factions: vec![faction],
        offers: vec![phoenix_model::ports::PortOfferConfig {
            id: "parts".into(),
            resource: "machinery_parts".into(),
            quota: 4,
            unit_amount: 5,
            costs: if policy == PortServicePolicy::ManualOffer {
                vec![phoenix_model::ports::PortCost {
                    resource: "alloy".into(),
                    amount: 1,
                }]
            } else {
                Vec::new()
            },
        }],
    };
    let mut infrastructure = provider.infrastructure.unwrap();
    infrastructure
        .capacities
        .iter_mut()
        .find(|capacity| capacity.id == "alloy")
        .unwrap()
        .amount = 0;
    let facility = app
        .world_mut()
        .spawn((
            EntityUuid(PROVIDER.into()),
            FactionComponent(faction),
            CompiledPortService(profile),
            PortProviderState::default(),
            ResourceServices(provider.resource_services),
            crate::infrastructure::InfrastructureCondition(
                crate::infrastructure::InfrastructureState::from_config(&infrastructure),
            ),
        ))
        .id();
    app.update();
    (app, ship, facility)
}

fn quantity(app: &App, entity: Entity, capacity: &str) -> i64 {
    app.world()
        .get::<crate::infrastructure::InfrastructureCondition>(entity)
        .unwrap()
        .0
        .capacity_stock(capacity)
        .unwrap()
        .level
}

fn order(app: &mut App, ship: Entity, lots: i64) {
    let visit = app
        .world()
        .get::<TransferUmbilical>(ship)
        .unwrap()
        .port_view
        .as_ref()
        .unwrap()
        .visit;
    send(
        app,
        "engineer",
        "umbilical",
        Control::StartPortOrder {
            partner_uuid: PROVIDER.into(),
            visit,
            lines: vec![phoenix_model::ports::PortOrderLine {
                offer: "parts".into(),
                store: "hold-machinery-parts".into(),
                lots,
            }],
        },
    );
}

fn request(app: &mut App, ship: Entity, facility: Entity, store: &str, lots: i64) -> PortTransfer {
    let profile = app
        .world()
        .get::<CompiledPortService>(facility)
        .unwrap()
        .0
        .clone();
    let visit = app
        .world()
        .get::<PortProviderState>(facility)
        .unwrap()
        .0
        .visitors[VISITOR]
        .visit;
    let offer = &profile.offers[0];
    let destination = actual_store(app.world(), ship, store, &offer.resource).unwrap();
    let mut approved = PortOrderState::default();
    approved
        .approve(
            &profile,
            PROVIDER,
            visit,
            vec![phoenix_model::ports::PortOrderLine {
                offer: offer.id.clone(),
                store: store.into(),
                lots,
            }],
        )
        .unwrap();
    app.world_mut()
        .get_mut::<TransferUmbilical>(ship)
        .unwrap()
        .port_order = approved;
    PortTransfer {
        operator: VISITOR.into(),
        provider: PROVIDER.into(),
        visit,
        offer: offer.id.clone(),
        destination,
        store: Some(store.into()),
        lots,
        tick: app.world().resource::<crate::sim_tick::SimTick>().0,
        ordered: profile.policy == PortServicePolicy::ManualOffer,
        inbound: false,
        maintain_goal: false,
    }
}

#[test]
fn full_same_endpoint_exchange_uses_original_payment_and_final_net_quantity() {
    let (mut app, ship, facility) = rig(PortServicePolicy::ManualOffer);
    {
        let mut profile = app
            .world_mut()
            .get_mut::<CompiledPortService>(facility)
            .unwrap();
        profile.0.offers[0].resource = "alloy".into();
        profile.0.offers[0].costs[0].amount = 5;
    }
    app.world_mut()
        .get_mut::<crate::infrastructure::InfrastructureCondition>(facility)
        .unwrap()
        .0
        .adjust_capacity("alloy", 20);
    let ceiling = app
        .world()
        .get::<crate::infrastructure::InfrastructureCondition>(ship)
        .unwrap()
        .0
        .capacity_stock("alloy")
        .unwrap()
        .ceiling;
    app.world_mut()
        .get_mut::<crate::infrastructure::InfrastructureCondition>(ship)
        .unwrap()
        .0
        .adjust_capacity("alloy", ceiling);
    let transfer = request(&mut app, ship, facility, "hold-alloy", 2);
    assert_eq!(
        crate::resources::server::preview_port_exchange(app.world_mut(), &transfer).unwrap(),
        2
    );
    assert_eq!(
        crate::resources::server::settle_port_exchange(app.world_mut(), &transfer).unwrap(),
        2
    );
    assert_eq!(quantity(&app, ship, "alloy"), ceiling);
    assert_eq!(quantity(&app, facility, "alloy"), 20);
    assert_eq!(
        app.world()
            .get::<PortProviderState>(facility)
            .unwrap()
            .0
            .visitors[VISITOR]
            .delivered["parts"],
        2
    );
    // A net-zero quote still cannot finance its payment from incoming goods.
    let current = quantity(&app, ship, "alloy");
    app.world_mut()
        .get_mut::<ShipLogistics>(ship)
        .unwrap()
        .0
        .floors
        .insert("hold-alloy".into(), current);
    let second = request(&mut app, ship, facility, "hold-alloy", 1);
    assert_eq!(
        crate::resources::server::settle_port_exchange(app.world_mut(), &second).unwrap(),
        0
    );
    assert_eq!(quantity(&app, ship, "alloy"), ceiling);
}

#[test]
fn maintained_flow_rechecks_actual_goal_after_another_same_tick_fill() {
    let (mut app, ship, facility) = rig(PortServicePolicy::FreeAlliedReplenishment);
    let before = quantity(&app, ship, "hold-machinery-parts");
    app.world_mut()
        .get_mut::<ShipLogistics>(ship)
        .unwrap()
        .0
        .goals
        .insert("hold-machinery-parts".into(), before + 5);
    let mut transfer = request(&mut app, ship, facility, "hold-machinery-parts", 1);
    transfer.maintain_goal = true;
    app.world_mut()
        .get_mut::<crate::infrastructure::InfrastructureCondition>(ship)
        .unwrap()
        .0
        .adjust_capacity("hold-machinery-parts", 5);
    let provider_before = quantity(&app, facility, "machinery_parts");
    assert_eq!(
        crate::resources::server::settle_port_exchange(app.world_mut(), &transfer).unwrap(),
        0
    );
    assert_eq!(quantity(&app, ship, "hold-machinery-parts"), before + 5);
    assert_eq!(quantity(&app, facility, "machinery_parts"), provider_before);
    assert_eq!(
        app.world()
            .get::<PortProviderState>(facility)
            .unwrap()
            .0
            .visitors[VISITOR]
            .delivered["parts"],
        0
    );
}

#[test]
fn blocked_high_priority_goal_does_not_starve_feasible_lower_goal() {
    let (mut app, ship, facility) = rig(PortServicePolicy::FreeAlliedReplenishment);
    app.world_mut()
        .get_mut::<CompiledPortService>(facility)
        .unwrap()
        .0
        .offers
        .push(phoenix_model::ports::PortOfferConfig {
            id: "alloy".into(),
            resource: "alloy".into(),
            quota: 20,
            unit_amount: 1,
            costs: Vec::new(),
        });
    app.world_mut()
        .get_mut::<PortProviderState>(facility)
        .unwrap()
        .0 = Default::default();
    let (alloy, parts) = (
        quantity(&app, ship, "alloy"),
        quantity(&app, ship, "hold-machinery-parts"),
    );
    {
        let mut state = app.world_mut().get_mut::<ShipLogistics>(ship).unwrap();
        state.0.goals.insert("hold-alloy".into(), alloy + 1);
        state
            .0
            .goals
            .insert("hold-machinery-parts".into(), parts + 5);
        state.0.priorities.insert("hold-alloy".into(), 0);
        state.0.priorities.insert("hold-machinery-parts".into(), 1);
    }
    send(
        &mut app,
        "engineer",
        "umbilical",
        Control::SetPortMaintenance { enabled: true },
    );
    for _ in 0..12 {
        app.update();
    }
    assert_eq!(
        quantity(&app, ship, "alloy"),
        alloy,
        "unavailable top priority remains pending"
    );
    assert_eq!(
        quantity(&app, ship, "hold-machinery-parts"),
        parts + 5,
        "next feasible observed goal receives one finite lot"
    );
}

#[test]
fn full_output_bay_uses_its_own_cost_stock_before_other_main_stores() {
    let (mut app, ship, facility) = rig_with(PortServicePolicy::ManualOffer, |content| {
        let logistics = content.logistics.as_mut().unwrap();
        let mut remote = logistics
            .stores
            .iter()
            .find(|store| store.id == "hold-alloy")
            .unwrap()
            .clone();
        remote.id = "aaa-remote-alloy".into();
        remote.endpoint = ResourceEndpoint::Capacity {
            id: "remote-alloy".into(),
        };
        remote.compartment = "engineering".into();
        remote.target = 0;
        logistics.stores.push(remote);
        let bay = logistics
            .storage
            .iter_mut()
            .find(|bay| bay.compartment == "engineering")
            .unwrap();
        bay.unit_space.insert("alloy".into(), 1);
        bay.space += 2;
        content.infrastructure.as_mut().unwrap().capacities.push(
            crate::infrastructure::CapacityConfig {
                id: "remote-alloy".into(),
                amount: 2,
                ceiling: Some(2),
                label: None,
            },
        );
        let profile = crate::resources::logistics_profile::compile_entity_logistics(content)
            .unwrap()
            .unwrap();
        let inventory = crate::infrastructure::InfrastructureCondition(
            crate::infrastructure::InfrastructureState::from_config(
                content.infrastructure.as_ref().unwrap(),
            ),
        );
        let torpedoes = content.torpedoes.as_ref().unwrap();
        let torpedoes = crate::console::weapons::torpedo::TorpedoSystemResource(
            crate::weapons::torpedo::TorpedoSystem::from_configs(
                &torpedoes.tubes,
                torpedoes.to_runtime(),
            ),
        );
        let stocks = crate::resources::logistics_server::endpoint_view(
            &profile,
            Some(&inventory),
            Some(&torpedoes),
        )
        .unwrap();
        let occupied = profile.occupied_space(&stocks, "support").unwrap();
        content
            .logistics
            .as_mut()
            .unwrap()
            .storage
            .iter_mut()
            .find(|bay| bay.compartment == "support")
            .unwrap()
            .space = occupied;
    });
    {
        let mut terms = app
            .world_mut()
            .get_mut::<CompiledPortService>(facility)
            .unwrap();
        terms.0.offers[0].unit_amount = 1;
        terms.0.offers[0].costs[0].amount = 2;
    }
    let before = (
        quantity(&app, ship, "hold-machinery-parts"),
        quantity(&app, ship, "alloy"),
    );
    let transfer = request(&mut app, ship, facility, "hold-machinery-parts", 3);
    assert_eq!(
        crate::resources::server::preview_port_exchange(app.world_mut(), &transfer).unwrap(),
        3
    );
    assert_eq!(
        crate::resources::server::settle_port_exchange(app.world_mut(), &transfer).unwrap(),
        3
    );
    assert_eq!(quantity(&app, ship, "hold-machinery-parts"), before.0 + 3);
    assert_eq!(quantity(&app, ship, "alloy"), before.1 - 6);
    assert_eq!(
        quantity(&app, ship, "remote-alloy"),
        2,
        "costs that free the actual receiving compartment are spent first"
    );
}

#[test]
fn explicit_manual_order_moves_real_goods_and_costs_with_one_visit_quota() {
    let (mut app, ship, facility) = rig(PortServicePolicy::ManualOffer);
    let before = (
        quantity(&app, ship, "hold-machinery-parts"),
        quantity(&app, ship, "alloy"),
        quantity(&app, facility, "machinery_parts"),
    );
    order(&mut app, ship, 2);
    for _ in 0..12 {
        app.update();
    }
    assert_eq!(quantity(&app, ship, "hold-machinery-parts"), before.0 + 10);
    assert_eq!(quantity(&app, ship, "alloy"), before.1 - 2);
    assert_eq!(quantity(&app, facility, "machinery_parts"), before.2 - 10);
    assert_eq!(quantity(&app, facility, "alloy"), 2);
    let ledger = &app.world().get::<PortProviderState>(facility).unwrap().0;
    assert_eq!(ledger.visitors[VISITOR].delivered["parts"], 2);
    let tool = app.world().get::<TransferUmbilical>(ship).unwrap();
    assert!(!tool.port_order.active);
    assert_eq!(tool.port_order.lines[0].delivered_lots, 2);
}

#[test]
fn ordinary_start_never_purchases_and_protected_costs_never_pay() {
    let (mut app, ship, facility) = rig(PortServicePolicy::ManualOffer);
    let before = quantity(&app, facility, "machinery_parts");
    send(&mut app, "engineer", "umbilical", Control::StartTransfer);
    for _ in 0..3 {
        app.update();
    }
    assert_eq!(quantity(&app, facility, "machinery_parts"), before);
    send(&mut app, "engineer", "umbilical", Control::StopTransfer);
    app.update();
    let alloy = quantity(&app, ship, "alloy");
    app.world_mut()
        .get_mut::<ShipLogistics>(ship)
        .unwrap()
        .0
        .floors
        .insert("hold-alloy".into(), alloy);
    let energy = app
        .world()
        .get::<crate::ship::power::ShipPowerSystem>(ship)
        .unwrap()
        .0
        .network()
        .clone();
    order(&mut app, ship, 1);
    for _ in 0..5 {
        app.update();
    }
    assert_eq!(quantity(&app, facility, "machinery_parts"), before);
    assert_eq!(quantity(&app, ship, "alloy"), alloy);
    assert_eq!(
        *app.world()
            .get::<crate::ship::power::ShipPowerSystem>(ship)
            .unwrap()
            .0
            .network(),
        energy
    );
}

#[test]
fn ended_visit_revokes_order_without_refunding_goods_or_costs() {
    let (mut app, ship, facility) = rig(PortServicePolicy::ManualOffer);
    order(&mut app, ship, 1);
    for _ in 0..8 {
        app.update();
    }
    let actual = (
        quantity(&app, ship, "hold-machinery-parts"),
        quantity(&app, ship, "alloy"),
    );
    app.world_mut()
        .get_mut::<crate::dock::DockControl>(ship)
        .unwrap()
        .docked = false;
    app.update();
    assert!(app
        .world()
        .get::<TransferUmbilical>(ship)
        .unwrap()
        .port_order
        .partner
        .is_none());
    assert!(app
        .world()
        .get::<PortProviderState>(facility)
        .unwrap()
        .0
        .visitors
        .is_empty());
    assert_eq!(
        (
            quantity(&app, ship, "hold-machinery-parts"),
            quantity(&app, ship, "alloy")
        ),
        actual
    );
    app.world_mut()
        .get_mut::<crate::dock::DockControl>(ship)
        .unwrap()
        .docked = true;
    app.update();
    assert_eq!(
        app.world()
            .get::<PortProviderState>(facility)
            .unwrap()
            .0
            .visitors[VISITOR]
            .delivered["parts"],
        0
    );
    for _ in 0..3 {
        app.update();
    }
    assert_eq!(
        (
            quantity(&app, ship, "hold-machinery-parts"),
            quantity(&app, ship, "alloy")
        ),
        actual
    );
}

#[test]
fn offline_tool_preserves_order_and_cannot_spend_then_resumes() {
    let (mut app, ship, _) = rig(PortServicePolicy::ManualOffer);
    order(&mut app, ship, 2);
    app.update();
    app.world_mut()
        .get_mut::<crate::ship_plugin::ShipSystemControlSources>(ship)
        .unwrap()
        .0
        .set(id("umbilical"), ControlSource::Offline);
    let before = quantity(&app, ship, "hold-machinery-parts");
    let progress = app
        .world()
        .get::<TransferUmbilical>(ship)
        .unwrap()
        .port_order
        .clone();
    for _ in 0..5 {
        app.update();
    }
    assert_eq!(quantity(&app, ship, "hold-machinery-parts"), before);
    assert_eq!(
        app.world()
            .get::<TransferUmbilical>(ship)
            .unwrap()
            .port_order,
        progress
    );
    app.world_mut()
        .get_mut::<crate::ship_plugin::ShipSystemControlSources>(ship)
        .unwrap()
        .0
        .set(id("umbilical"), ControlSource::Human);
    for _ in 0..12 {
        app.update();
    }
    assert!(
        !app.world()
            .get::<TransferUmbilical>(ship)
            .unwrap()
            .port_order
            .active
    );
    assert_eq!(
        app.world()
            .get::<TransferUmbilical>(ship)
            .unwrap()
            .port_order
            .lines[0]
            .delivered_lots,
        2
    );
}

#[test]
fn partial_operation_reduces_yield_for_the_same_paid_work_and_zero_pauses() {
    let (mut normal, ship, _) = rig(PortServicePolicy::ManualOffer);
    let (mut impaired, impaired_ship, _) = rig(PortServicePolicy::ManualOffer);
    let authored = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    let mut damage = crate::ship::compartment_damage::ShipDamage::from_config(&authored)
        .unwrap()
        .unwrap();
    let system = id("umbilical");
    let component = damage.definition.components[&system]
        .iter()
        .find(|(_, definition)| {
            definition.binding == phoenix_model::damage::DamageBinding::Operation
        })
        .unwrap()
        .0
        .clone();
    damage
        .state
        .components
        .get_mut(&system)
        .unwrap()
        .get_mut(&component)
        .unwrap()
        .condition = phoenix_model::damage::ComponentCondition::Patched { factor: 0.5 };
    impaired
        .world_mut()
        .entity_mut(impaired_ship)
        .insert(damage);
    let before = normal
        .world()
        .get::<crate::ship::power::ShipPowerSystem>(ship)
        .unwrap()
        .0
        .capture_continuation();
    order(&mut normal, ship, 1);
    order(&mut impaired, impaired_ship, 1);
    normal.update();
    impaired.update();
    let full = normal
        .world()
        .get::<TransferUmbilical>(ship)
        .unwrap()
        .port_order
        .fraction;
    let half = impaired
        .world()
        .get::<TransferUmbilical>(impaired_ship)
        .unwrap()
        .port_order
        .fraction;
    assert!(full > 0.0 && full < 1.0);
    assert_eq!(half, full * 0.5);
    let paid = normal
        .world()
        .get::<crate::ship::power::ShipPowerSystem>(ship)
        .unwrap()
        .0
        .capture_continuation();
    let impaired_paid = impaired
        .world()
        .get::<crate::ship::power::ShipPowerSystem>(impaired_ship)
        .unwrap()
        .0
        .capture_continuation();
    assert_ne!(
        paid.network.consumers[&system].energy,
        before.network.consumers[&system].energy
    );
    assert_eq!(
        paid, impaired_paid,
        "damage changes useful output, not the energy charged for that work"
    );
    impaired
        .world_mut()
        .get_mut::<crate::ship::compartment_damage::ShipDamage>(impaired_ship)
        .unwrap()
        .state
        .components
        .get_mut(&system)
        .unwrap()
        .get_mut(&component)
        .unwrap()
        .condition = phoenix_model::damage::ComponentCondition::Destroyed;
    let intent = impaired
        .world()
        .get::<TransferUmbilical>(impaired_ship)
        .unwrap()
        .port_order
        .clone();
    impaired.update();
    assert_eq!(
        impaired
            .world()
            .get::<TransferUmbilical>(impaired_ship)
            .unwrap()
            .port_order,
        intent
    );
    assert_eq!(
        impaired
            .world()
            .get::<crate::ship::power::ShipPowerSystem>(impaired_ship)
            .unwrap()
            .0
            .capture_continuation(),
        impaired_paid
    );
}

#[test]
fn trusted_mission_approval_uses_actual_permission_visit_and_normal_later_work() {
    let (mut app, ship, facility) = rig(PortServicePolicy::ManualOffer);
    let visit = app
        .world()
        .get::<PortProviderState>(facility)
        .unwrap()
        .0
        .visitors[VISITOR]
        .visit;
    let line = phoenix_model::ports::PortOrderLine {
        offer: "parts".into(),
        store: "hold-machinery-parts".into(),
        lots: 1,
    };
    let before = (
        quantity(&app, ship, "hold-machinery-parts"),
        quantity(&app, ship, "alloy"),
    );
    let state = app
        .world()
        .get::<TransferUmbilical>(ship)
        .unwrap()
        .save_state();
    assert!(approve_port_order(
        app.world_mut(),
        VISITOR,
        PROVIDER,
        visit + 1,
        vec![line.clone()]
    )
    .is_err());
    assert_eq!(
        app.world()
            .get::<TransferUmbilical>(ship)
            .unwrap()
            .save_state(),
        state
    );
    let allowed = app
        .world()
        .get::<CompiledPortService>(facility)
        .unwrap()
        .0
        .eligible_factions
        .clone();
    app.world_mut()
        .get_mut::<CompiledPortService>(facility)
        .unwrap()
        .0
        .eligible_factions
        .clear();
    assert!(approve_port_order(
        app.world_mut(),
        VISITOR,
        PROVIDER,
        visit,
        vec![line.clone()]
    )
    .is_err());
    assert_eq!(
        app.world()
            .get::<TransferUmbilical>(ship)
            .unwrap()
            .save_state(),
        state
    );
    app.world_mut()
        .get_mut::<CompiledPortService>(facility)
        .unwrap()
        .0
        .eligible_factions = allowed;
    approve_port_order(app.world_mut(), VISITOR, PROVIDER, visit, vec![line]).unwrap();
    assert_eq!(
        (
            quantity(&app, ship, "hold-machinery-parts"),
            quantity(&app, ship, "alloy")
        ),
        before,
        "approval itself never moves goods or pays costs"
    );
    for _ in 0..8 {
        app.update();
    }
    assert_eq!(
        (
            quantity(&app, ship, "hold-machinery-parts"),
            quantity(&app, ship, "alloy")
        ),
        (before.0 + 5, before.1 - 1)
    );
}

#[test]
fn ineligible_visit_cannot_query_actual_provider_availability() {
    let (mut app, ship, facility) = rig(PortServicePolicy::ManualOffer);
    assert!(
        app.world()
            .get::<TransferUmbilical>(ship)
            .unwrap()
            .port_view
            .as_ref()
            .unwrap()
            .offers[0]
            .available_lots
            > 0
    );
    app.world_mut()
        .get_mut::<CompiledPortService>(facility)
        .unwrap()
        .0
        .eligible_factions
        .clear();
    app.update();
    let view = app
        .world()
        .get::<TransferUmbilical>(ship)
        .unwrap()
        .port_view
        .as_ref()
        .unwrap();
    assert!(!view.eligible);
    assert!(view.offers.iter().all(|offer| offer.available_lots == 0));
}

#[test]
fn free_legacy_fuel_donation_and_collection_use_real_named_endpoints_without_main_store() {
    let (mut app, ship, facility) =
        rig_with(PortServicePolicy::FreeAlliedReplenishment, |content| {
            content.infrastructure.as_mut().unwrap().capacities.push(
                crate::infrastructure::CapacityConfig {
                    id: "reserve_fuel".into(),
                    amount: 12,
                    ceiling: Some(20),
                    label: None,
                },
            );
        });
    {
        let mut profile = app
            .world_mut()
            .get_mut::<CompiledPortService>(facility)
            .unwrap();
        profile.0.offers[0].resource = "reserve_fuel".into();
        profile.0.offers[0].unit_amount = 1;
        profile.0.offers[0].quota = 20;
    }
    app.world_mut()
        .get_mut::<PortProviderState>(facility)
        .unwrap()
        .0 = Default::default();
    assert!(!app
        .world()
        .get::<CompiledShipLogistics>(ship)
        .unwrap()
        .0
        .config()
        .stores
        .iter()
        .any(|store| store.resource == "reserve_fuel"));
    send(&mut app, "engineer", "umbilical", Control::StartTransfer);
    for _ in 0..15 {
        app.update();
    }
    assert_eq!(quantity(&app, ship, "reserve_fuel"), 0);
    assert_eq!(quantity(&app, facility, "reserve_fuel"), 12);
    assert_eq!(
        app.world()
            .get::<PortProviderState>(facility)
            .unwrap()
            .0
            .visitors[VISITOR]
            .delivered["parts"],
        0,
        "a donation consumes no outbound provider allowance"
    );
    send(&mut app, "engineer", "umbilical", Control::StopTransfer);
    app.update();
    send(
        &mut app,
        "engineer",
        "umbilical",
        Control::SetTransferDirection {
            direction: crate::umbilical::flow::UmbilicalDirection::Collect,
        },
    );
    app.update();
    send(&mut app, "engineer", "umbilical", Control::StartTransfer);
    for _ in 0..15 {
        app.update();
    }
    assert_eq!(quantity(&app, ship, "reserve_fuel"), 12);
    assert_eq!(quantity(&app, facility, "reserve_fuel"), 0);
    assert_eq!(
        app.world()
            .get::<PortProviderState>(facility)
            .unwrap()
            .0
            .visitors[VISITOR]
            .delivered["parts"],
        12
    );
    let profile = app
        .world()
        .get::<CompiledPortService>(facility)
        .unwrap()
        .0
        .clone();
    let visit = app
        .world()
        .get::<PortProviderState>(facility)
        .unwrap()
        .0
        .visitors[VISITOR]
        .visit;
    app.world_mut()
        .get_mut::<PortProviderState>(facility)
        .unwrap()
        .0
        .record_delivery(&profile, VISITOR, visit, "parts", 8)
        .unwrap();
    send(&mut app, "engineer", "umbilical", Control::StopTransfer);
    app.update();
    send(
        &mut app,
        "engineer",
        "umbilical",
        Control::SetTransferDirection {
            direction: crate::umbilical::flow::UmbilicalDirection::Deliver,
        },
    );
    app.update();
    send(&mut app, "engineer", "umbilical", Control::StartTransfer);
    for _ in 0..15 {
        app.update();
    }
    assert_eq!(quantity(&app, ship, "reserve_fuel"), 0);
    assert_eq!(quantity(&app, facility, "reserve_fuel"), 12);
    assert_eq!(
        app.world()
            .get::<PortProviderState>(facility)
            .unwrap()
            .0
            .visitors[VISITOR]
            .delivered["parts"],
        20,
        "exhausted outbound allowance neither prevents nor charges a new real donation"
    );
}

#[test]
fn zero_quota_offer_cannot_create_an_invalid_donation_meter_or_buy_work() {
    let (mut app, ship, facility) =
        rig_with(PortServicePolicy::FreeAlliedReplenishment, |content| {
            content.infrastructure.as_mut().unwrap().capacities.push(
                crate::infrastructure::CapacityConfig {
                    id: "reserve_fuel".into(),
                    amount: 12,
                    ceiling: Some(20),
                    label: None,
                },
            );
        });
    {
        let mut terms = app
            .world_mut()
            .get_mut::<CompiledPortService>(facility)
            .unwrap();
        terms.0.offers[0].resource = "reserve_fuel".into();
        terms.0.offers[0].unit_amount = 1;
        terms.0.offers[0].quota = 0;
    }
    let energy = app
        .world()
        .get::<crate::ship::power::ShipPowerSystem>(ship)
        .unwrap()
        .0
        .network()
        .clone();
    let direct = PortTransfer {
        operator: VISITOR.into(),
        provider: PROVIDER.into(),
        visit: app
            .world()
            .get::<PortProviderState>(facility)
            .unwrap()
            .0
            .visitors[VISITOR]
            .visit,
        offer: "parts".into(),
        destination: ResourceEndpoint::Capacity {
            id: "reserve_fuel".into(),
        },
        store: None,
        lots: 1,
        tick: app.world().resource::<crate::sim_tick::SimTick>().0,
        ordered: false,
        inbound: true,
        maintain_goal: false,
    };
    let ledger = app
        .world()
        .get::<PortProviderState>(facility)
        .unwrap()
        .0
        .clone();
    assert!(crate::resources::server::settle_port_exchange(app.world_mut(), &direct).is_err());
    assert_eq!(
        app.world().get::<PortProviderState>(facility).unwrap().0,
        ledger
    );
    send(&mut app, "engineer", "umbilical", Control::StartTransfer);
    for _ in 0..5 {
        app.update();
    }
    assert_eq!(quantity(&app, ship, "reserve_fuel"), 12);
    assert_eq!(quantity(&app, facility, "reserve_fuel"), 0);
    let tool = app.world().get::<TransferUmbilical>(ship).unwrap();
    assert!(tool.port_flow.is_none());
    assert_eq!(
        tool.port_view
            .as_ref()
            .unwrap()
            .order
            .as_ref()
            .unwrap()
            .blocked
            .as_deref(),
        Some("ports.blocked.quota_exhausted")
    );
    assert_eq!(
        *app.world()
            .get::<crate::ship::power::ShipPowerSystem>(ship)
            .unwrap()
            .0
            .network(),
        energy
    );
}

#[test]
fn ordinary_ai_maintains_free_observed_goals_but_cannot_originate_manual_approval() {
    let (mut app, ship, facility) = rig(PortServicePolicy::FreeAlliedReplenishment);
    let before = quantity(&app, ship, "hold-machinery-parts");
    app.world_mut()
        .get_mut::<ShipLogistics>(ship)
        .unwrap()
        .0
        .goals
        .insert("hold-machinery-parts".into(), before + 5);
    app.world_mut()
        .get_mut::<crate::ship_plugin::ShipSystemControlSources>(ship)
        .unwrap()
        .0
        .set(id("umbilical"), ControlSource::Ai);
    for _ in 0..18 {
        app.update();
    }
    assert_eq!(quantity(&app, ship, "hold-machinery-parts"), before + 5);
    assert!(
        app.world()
            .get::<TransferUmbilical>(ship)
            .unwrap()
            .port_maintenance
    );
    assert_eq!(
        app.world()
            .get::<PortProviderState>(facility)
            .unwrap()
            .0
            .visitors[VISITOR]
            .delivered["parts"],
        1
    );
    let control = app
        .world()
        .get::<crate::ship_plugin::ShipSystemControlSources>(ship)
        .unwrap();
    let config = app
        .world()
        .get::<crate::ship_plugin::ShipConfigComponent>(ship)
        .unwrap();
    let payload = Control::StartPortOrder {
        partner_uuid: PROVIDER.into(),
        visit: 1,
        lines: vec![phoenix_model::ports::PortOrderLine {
            offer: "parts".into(),
            store: "hold-machinery-parts".into(),
            lots: 1,
        }],
    };
    assert!(
        !crate::command_admission::policy::command_policy(
            &config.0,
            &control.0,
            &id("umbilical"),
            &payload
        )
        .operate_ai
    );
}

#[test]
fn exchange_publication_and_quota_are_uuid_ordered_when_ecs_allocation_reverses() {
    fn run(
        reverse: bool,
    ) -> (
        crate::world::flags::FlagStore,
        String,
        i64,
        i64,
        PortVisitState,
    ) {
        let (mut app, ship, facility) = rig(PortServicePolicy::ManualOffer);
        let transfer = request(&mut app, ship, facility, "hold-machinery-parts", 1);
        let visitor = (
            app.world().get::<EntityUuid>(ship).unwrap().clone(),
            app.world().get::<FactionComponent>(ship).unwrap().clone(),
            app.world()
                .get::<crate::infrastructure::InfrastructureCondition>(ship)
                .unwrap()
                .clone(),
            app.world()
                .get::<CompiledShipLogistics>(ship)
                .unwrap()
                .clone(),
            app.world().get::<ShipLogistics>(ship).unwrap().clone(),
            app.world()
                .get::<crate::console::weapons::torpedo::TorpedoSystemResource>(ship)
                .unwrap()
                .clone(),
            app.world()
                .get::<crate::dock::DockControl>(ship)
                .unwrap()
                .clone(),
            app.world().get::<TransferUmbilical>(ship).unwrap().clone(),
        );
        let provider = (
            app.world().get::<EntityUuid>(facility).unwrap().clone(),
            app.world()
                .get::<FactionComponent>(facility)
                .unwrap()
                .clone(),
            app.world()
                .get::<crate::infrastructure::InfrastructureCondition>(facility)
                .unwrap()
                .clone(),
            app.world()
                .get::<CompiledPortService>(facility)
                .unwrap()
                .clone(),
            app.world()
                .get::<PortProviderState>(facility)
                .unwrap()
                .clone(),
            app.world()
                .get::<ResourceServices>(facility)
                .unwrap()
                .clone(),
        );
        let mut world = World::new();
        world.insert_resource(crate::world::server::WorldContentRuntime::default());
        let (visitor, provider) = if reverse {
            let provider = world.spawn(provider).id();
            (world.spawn(visitor).id(), provider)
        } else {
            let visitor = world.spawn(visitor).id();
            (visitor, world.spawn(provider).id())
        };
        assert_eq!(
            crate::resources::server::settle_port_exchange(&mut world, &transfer).unwrap(),
            1
        );
        let runtime = world.resource::<crate::world::server::WorldContentRuntime>();
        (
            runtime.flags.clone(),
            format!("{:?}", runtime.pending_world_events),
            world
                .get::<crate::infrastructure::InfrastructureCondition>(visitor)
                .unwrap()
                .0
                .capacity("hold-machinery-parts")
                .unwrap(),
            world
                .get::<crate::infrastructure::InfrastructureCondition>(provider)
                .unwrap()
                .0
                .capacity("machinery_parts")
                .unwrap(),
            world.get::<PortProviderState>(provider).unwrap().0.clone(),
        )
    }
    assert_eq!(run(false), run(true));
}
