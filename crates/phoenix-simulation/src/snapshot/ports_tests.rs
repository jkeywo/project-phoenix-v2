//! Actual shipped owners, finite barter and complete-roster restore refusal.
use super::*;
use crate::resources::ports::{PortOrderLine, PortOrderState};
use crate::resources::ports_server::{CompiledPortService, PortProviderState, PortTransfer};
use crate::umbilical::TransferUmbilical;

const VISITOR: &str = "11111111-1111-4111-8111-111111111111";
const PROVIDER: &str = "22222222-2222-4222-8222-222222222222";

fn bare_world() -> World {
    let mut world = World::new();
    world.insert_resource(SimTick(3));
    world.insert_resource(Time::<Fixed>::default());
    world.insert_resource(crate::world::server::WorldContentRuntime::default());
    // Match the ordinary host's run-scope owners. Their absence is a different
    // digest fact from the empty canonical state restored by a snapshot.
    world.init_resource::<crate::gm::actions::action::SimulationPaused>();
    world.init_resource::<crate::gm::actions::action::GmActionJournal>();
    world.init_resource::<crate::gm::operations::puppet::StationPuppets>();
    world.init_resource::<crate::gm::operations::puppet::PendingGmStationCommands>();
    world.init_resource::<crate::core::report::MissionReport>();
    world.register_component::<ShipPhysics>();
    world.register_component::<AsteroidUuid>();
    world.register_component::<EntitySystemHull>();
    world.register_component::<ShipRedAlert>();
    world.register_component::<crate::console::command::server::ShipStationStances>();
    world.register_component::<crate::ship_plugin::ShipSystemControlSources>();
    world.register_component::<ActiveBeam>();
    world.register_component::<PhaserCooldown>();
    world.register_component::<TorpedoSystemResource>();
    world.register_component::<EntityShipArcHull>();
    world.register_component::<ShipRepairTeams>();
    world.register_component::<RepairRequestQueue>();
    world.register_component::<RepairHumanAlerted>();
    world.register_component::<crate::server_app::ShipSystemBlackboards>();
    world.register_component::<crate::ai::server::ObjectiveCursors>();
    world.register_component::<crate::console::weapons::blaster::BlasterSystemResource>();
    world.register_component::<crate::ship::shields::ShipShields>();
    world
}

fn spawn(world: &mut World, uuid: &str, template: &str) -> Entity {
    let origin = crate::world::spawn_origin::SpawnOrigin {
        template_path: template.into(),
        name: uuid.into(),
        ..Default::default()
    };
    super::spawn_from_origin(
        world,
        &EntityState {
            uuid: uuid.into(),
            spawn: Some(origin.clone()),
            ..Default::default()
        },
        &origin,
    )
    .expect("actual port/visitor content must rebuild")
}

fn advance(world: &mut World, entity: Entity, tick: u64, dt: f32) -> i64 {
    world.resource_mut::<SimTick>().0 = tick;
    let provider = world
        .query_filtered::<Entity, With<CompiledPortService>>()
        .single(world)
        .unwrap();
    let profile = world
        .get::<CompiledPortService>(provider)
        .unwrap()
        .0
        .clone();
    world
        .get_mut::<PortProviderState>(provider)
        .unwrap()
        .0
        .refresh(
            &profile,
            tick,
            &std::collections::BTreeSet::from([VISITOR.into()]),
        )
        .unwrap();
    let mut tool = world.get::<TransferUmbilical>(entity).unwrap().clone();
    let mut power = world
        .get::<crate::ship::power::ShipPowerSystem>(entity)
        .unwrap()
        .clone();
    let offered = tool
        .port_order
        .advance(
            &profile,
            &crate::ship::system_registry::umbilical_system_id(),
            tick,
            dt,
            tool.config.rate,
            2,
            |system, seconds| {
                power
                    .0
                    .work_continuous(system, seconds, 1.0)
                    .map(|grant| grant.fraction)
            },
        )
        .unwrap();
    world.entity_mut(entity).insert((tool, power));
    offered
}

// These bare owner Worlds omit the App's Publish schedule. Whole restore
// rebuilds these four actual read models; capture the same normal publications
// before saving rather than expecting an empty bootstrap blackboard to survive.
fn publish_fixture_observations(world: &mut World, visitor: Entity) {
    use bevy::ecs::system::RunSystemOnce;
    world
        .run_system_once(crate::science::server::publish_scan_blackboard)
        .unwrap();
    world
        .run_system_once(crate::science::passive::publish)
        .unwrap();
    world
        .run_system_once(crate::ship::sensors::publish_sensor_radar_blackboard)
        .unwrap();
    world
        .run_system_once(crate::console::command::server::publish_command_blackboard)
        .unwrap();
    let boards = &world
        .get::<crate::server_app::ShipSystemBlackboards>(visitor)
        .unwrap()
        .0;
    for key in [
        "scan/7:sensors",
        "passive-receiver",
        "sensor-radar",
        "command",
    ] {
        assert!(
            boards.contains_key(&crate::core::messages::SystemId(key.into())),
            "actual ordinary publication {key}"
        );
    }
}

fn fixture() -> (World, Entity, Entity) {
    let mut world = bare_world();
    let visitor = spawn(&mut world, VISITOR, "assets/entities/alliance_cruiser.toml");
    // Receiving room comes from finite canonical use, not a larger ceiling.
    let mut stock = world
        .get::<crate::infrastructure::InfrastructureCondition>(visitor)
        .unwrap()
        .0
        .capacity_stock("hold-machinery-parts")
        .unwrap()
        .clone();
    let original_level = stock.level;
    assert_eq!(stock.headroom(), 0);
    stock.reserve("fixture-parts-use", 5).unwrap();
    assert_eq!(stock.consume("fixture-parts-use"), Some(5));
    assert_eq!(stock.level, original_level - 5);
    world
        .get_mut::<crate::infrastructure::InfrastructureCondition>(visitor)
        .unwrap()
        .0
        .replace_capacity_stock("hold-machinery-parts", stock)
        .unwrap();
    let provider = spawn(
        &mut world,
        PROVIDER,
        "assets/entities/manual_trade_port.toml",
    );
    {
        let mut dock = world.get_mut::<crate::dock::DockControl>(visitor).unwrap();
        dock.set_engaged(true);
        dock.set_docking_target(Some(PROVIDER.into()));
        dock.docked = true;
    }
    let profile = world
        .get::<CompiledPortService>(provider)
        .unwrap()
        .0
        .clone();
    let mut ledger = world.get::<PortProviderState>(provider).unwrap().clone();
    let visitors = std::collections::BTreeSet::from([VISITOR.into()]);
    ledger.0.refresh(&profile, 1, &visitors).unwrap();
    world.entity_mut(provider).insert(ledger);
    {
        let mut tool = world.get_mut::<TransferUmbilical>(visitor).unwrap();
        tool.selected_resource = "machinery_parts".into();
        tool.selected_store = Some("hold-machinery-parts".into());
        tool.port_maintenance = true;
        tool.running = true;
        tool.port_order
            .approve(
                &profile,
                PROVIDER,
                1,
                vec![PortOrderLine {
                    offer: "machinery-for-alloy".into(),
                    store: "hold-machinery-parts".into(),
                    lots: 2,
                }],
            )
            .unwrap();
    }
    assert_eq!(advance(&mut world, visitor, 1, 0.125), 0);
    assert_eq!(advance(&mut world, visitor, 2, 0.125), 1);
    let destination = world
        .get::<crate::resources::logistics_server::CompiledShipLogistics>(visitor)
        .unwrap()
        .0
        .store("hold-machinery-parts")
        .unwrap()
        .endpoint
        .clone();
    let moved = crate::resources::server::settle_port_exchange(
        &mut world,
        &PortTransfer {
            operator: VISITOR.into(),
            provider: PROVIDER.into(),
            visit: 1,
            offer: "machinery-for-alloy".into(),
            destination,
            store: Some("hold-machinery-parts".into()),
            lots: 1,
            tick: 2,
            ordered: true,
            inbound: false,
            maintain_goal: false,
        },
    )
    .unwrap();
    assert_eq!(
        moved, 1,
        "fixture must perform actual finite goods/cost settlement"
    );
    world
        .get_mut::<TransferUmbilical>(visitor)
        .unwrap()
        .port_order
        .record_receipt(2, moved)
        .unwrap();
    assert_eq!(advance(&mut world, visitor, 3, 0.0625), 0);
    assert_eq!(
        world
            .get::<TransferUmbilical>(visitor)
            .unwrap()
            .port_order
            .fraction,
        0.25
    );
    assert_eq!(
        world.get::<PortProviderState>(provider).unwrap().0.visitors[VISITOR].delivered
            ["machinery-for-alloy"],
        1
    );
    publish_fixture_observations(&mut world, visitor);
    (world, visitor, provider)
}

#[test]
fn port_json_restore_preserves_actual_barter_quota_and_paid_lot_work() {
    let (world, _, _) = fixture();
    let saved: PhoenixSnapshot =
        serde_json::from_slice(&serde_json::to_vec(&capture(&world)).unwrap()).unwrap();
    let (mut fresh, visitor, _) = fixture();
    let tool = fresh
        .get_mut::<TransferUmbilical>(visitor)
        .unwrap()
        .into_inner();
    tool.port_order = PortOrderState::default();
    tool.port_maintenance = false;
    let result = restore(&mut fresh, &saved);
    assert!(result.is_complete(), "{:?}", result.gaps);
    assert_eq!(capture(&fresh), saved);
    assert_eq!(
        crate::sim_digest::world_digest(&fresh),
        crate::sim_digest::world_digest(&world),
        "first divergent scope: {:?}",
        crate::sim_digest::first_divergent_scope(&fresh, &crate::sim_digest::digest_stages(&world))
    );
}

#[test]
fn invalid_port_visits_owners_terms_or_receipts_refuse_before_any_write() {
    let (world, _, _) = fixture();
    let saved = capture(&world);
    let mut valid = saved.clone();
    valid.tick += 50;
    valid
        .entities
        .iter_mut()
        .find(|row| row.uuid == VISITOR)
        .unwrap()
        .red_alert = Some(true);
    let (mut positive, _, _) = fixture();
    let positive_report = restore(&mut positive, &valid);
    assert!(
        positive_report.is_complete(),
        "changed global/entity fields must be admissible before corrupting port state: {:?}",
        positive_report.gaps
    );
    for mode in 0..10 {
        let mut bad = saved.clone();
        bad.tick += 50;
        let provider = bad
            .entities
            .iter_mut()
            .find(|row| row.uuid == PROVIDER)
            .unwrap();
        match mode {
            0 => provider.port_provider = None,
            1 => {
                provider.port_provider.as_mut().unwrap().visitors.clear();
            }
            2 => {
                provider
                    .port_provider
                    .as_mut()
                    .unwrap()
                    .visitors
                    .get_mut(VISITOR)
                    .unwrap()
                    .visit = 2
            }
            3 => {
                provider
                    .port_provider
                    .as_mut()
                    .unwrap()
                    .visitors
                    .get_mut(VISITOR)
                    .unwrap()
                    .delivered
                    .insert("machinery-for-alloy".into(), 0);
            }
            _ => {}
        }
        let visitor = bad
            .entities
            .iter_mut()
            .find(|row| row.uuid == VISITOR)
            .unwrap();
        visitor.red_alert = Some(true);
        match mode {
            4 => visitor.umbilical = None,
            5 => {
                visitor
                    .umbilical
                    .as_mut()
                    .unwrap()
                    .port_order
                    .last_outcome
                    .as_mut()
                    .unwrap()
                    .settled = false
            }
            6 => {
                visitor.umbilical.as_mut().unwrap().port_order.lines[0].store =
                    "repair-machinery".into()
            }
            7 => {
                visitor.umbilical.as_mut().unwrap().port_order.lines[0].offer =
                    "unapproved-price".into()
            }
            8 => visitor.umbilical.as_mut().unwrap().selected_store = Some("absent".into()),
            9 => visitor.dock.as_mut().unwrap().docked = false,
            _ => {}
        }
        let (mut fresh, _, _) = fixture();
        let before = capture(&fresh);
        let count = fresh.entities().len();
        let result = restore(&mut fresh, &bad);
        assert!(!result.is_complete(), "mode {mode}");
        assert_eq!(result.entities_restored, 0);
        assert_eq!(result.entities_spawned, 0);
        assert_eq!(fresh.entities().len(), count);
        assert_eq!(
            capture(&fresh),
            before,
            "mode {mode} changed global/entity/stock state"
        );
    }
}

#[test]
fn missing_provider_is_materialized_only_after_all_port_grants_validate() {
    let (world, _, _) = fixture();
    let saved = capture(&world);
    let mut bad = saved.clone();
    bad.entities
        .iter_mut()
        .find(|row| row.uuid == PROVIDER)
        .unwrap()
        .port_provider
        .as_mut()
        .unwrap()
        .visitors
        .get_mut(VISITOR)
        .unwrap()
        .visit = 2;
    let (mut fresh, _, provider) = fixture();
    fresh.despawn(provider);
    let before = capture(&fresh);
    let count = fresh.entities().len();
    let result = restore(&mut fresh, &bad);
    assert!(!result.is_complete());
    assert_eq!(fresh.entities().len(), count);
    assert_eq!(capture(&fresh), before);
    let result = restore(&mut fresh, &saved);
    assert!(result.is_complete(), "{:?}", result.gaps);
    assert_eq!(result.entities_spawned, 1);
    assert_eq!(capture(&fresh), saved);
}

#[test]
fn free_receiving_meter_cannot_claim_more_goods_than_provider_settled() {
    let mut world = bare_world();
    let visitor = spawn(&mut world, VISITOR, "assets/entities/alliance_cruiser.toml");
    let provider = spawn(&mut world, PROVIDER, "assets/entities/station_axiom.toml");
    {
        let mut dock = world.get_mut::<crate::dock::DockControl>(visitor).unwrap();
        dock.set_engaged(true);
        dock.set_docking_target(Some(PROVIDER.into()));
        dock.docked = true;
    }
    let profile = world
        .get::<CompiledPortService>(provider)
        .unwrap()
        .0
        .clone();
    world
        .get_mut::<PortProviderState>(provider)
        .unwrap()
        .0
        .refresh(
            &profile,
            1,
            &std::collections::BTreeSet::from([VISITOR.into()]),
        )
        .unwrap();
    world
        .get_mut::<PortProviderState>(provider)
        .unwrap()
        .0
        .refresh(
            &profile,
            3,
            &std::collections::BTreeSet::from([VISITOR.into()]),
        )
        .unwrap();
    let mut meter = PortOrderState::default();
    meter
        .approve(
            &profile,
            PROVIDER,
            1,
            vec![PortOrderLine {
                offer: "machinery-parts".into(),
                store: "hold-machinery-parts".into(),
                lots: 2,
            }],
        )
        .unwrap();
    meter
        .advance(
            &profile,
            &crate::ship::system_registry::umbilical_system_id(),
            3,
            0.0,
            20.0,
            0,
            |_, _| panic!("zero work cannot draw energy"),
        )
        .unwrap();
    {
        let mut tool = world.get_mut::<TransferUmbilical>(visitor).unwrap();
        tool.selected_direction = crate::umbilical::UmbilicalDirection::Collect;
        tool.selected_resource = "machinery_parts".into();
        tool.selected_store = Some("hold-machinery-parts".into());
        tool.port_flow = Some(meter);
    }
    let valid = capture(&world);
    assert!(super::ports::validate_candidate(&world, &valid).is_ok());
    let mut forged = valid.clone();
    forged
        .entities
        .iter_mut()
        .find(|row| row.uuid == VISITOR)
        .unwrap()
        .umbilical
        .as_mut()
        .unwrap()
        .port_flow
        .as_mut()
        .unwrap()
        .lines[0]
        .delivered_lots = 1;
    let report = restore(&mut world, &forged);
    assert!(!report.is_complete());
    assert!(format!("{:?}", report.gaps).contains("settled quota"));
    assert_eq!(capture(&world), valid);
    // Reverse donations consume visitor stocks and never debit this outbound
    // grant. Their meter legitimately has no matching provider delivery.
    forged
        .entities
        .iter_mut()
        .find(|row| row.uuid == VISITOR)
        .unwrap()
        .umbilical
        .as_mut()
        .unwrap()
        .selected_direction = crate::umbilical::UmbilicalDirection::Deliver;
    assert!(super::ports::validate_candidate(&world, &forged).is_ok());
}
