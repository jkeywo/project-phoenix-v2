//! Actual configured machinery/finite endpoint owner tests, not host journeys.
use super::*;
use crate::ship::compartment_damage::ShipDamage;
use crate::ship::power::ShipPowerSystem;
use bevy::ecs::system::RunSystemOnce;

fn equipped(policy: PortServicePolicy) -> (App, Entity, Entity) {
    let (mut app, visitor, old_provider) = rig(policy);
    // The adjacent Logistics/port rig deliberately constructs only its stock,
    // controls and power owners. Equipment-bound service also requires the
    // actual physical condition and Repair owners. Build those through the
    // normal accepted hull spawner, retaining the rig's existing finite stock,
    // energy, local authority and completed parts-use frontier unchanged.
    let hull = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    let machinery = crate::entities::spawner::spawn_entity(
        &mut app.world_mut().commands(),
        &hull,
        Vec3::ZERO,
        "facility-fixture-machinery".into(),
        None,
    );
    app.world_mut().flush();
    let damage = app
        .world_mut()
        .entity_mut(machinery)
        .take::<ShipDamage>()
        .unwrap();
    let repair = app
        .world_mut()
        .entity_mut(machinery)
        .take::<crate::console::repair::server::ShipRepairTeams>()
        .unwrap();
    let services = app
        .world_mut()
        .entity_mut(machinery)
        .take::<ResourceServices>()
        .expect("accepted physical Repair equipment binds actual part services");
    app.world_mut()
        .entity_mut(visitor)
        .insert((damage, repair, services));
    assert!(app.world_mut().despawn(machinery));
    assert_eq!(
        app.world().get::<ShipDamage>(visitor).unwrap().factor(
            &id("umbilical"),
            phoenix_model::damage::DamageBinding::Operation
        ),
        1.0
    );
    let mut config = crate::entities::include_resolve::load_entity_config(
        if policy == PortServicePolicy::ManualOffer {
            "assets/entities/manual_trade_port.toml"
        } else {
            "assets/entities/station_axiom.toml"
        },
    )
    .unwrap();
    let mut terms = app
        .world()
        .get::<CompiledPortService>(old_provider)
        .unwrap()
        .0
        .clone();
    terms.system = Some(id("dock"));
    config.port_service = Some(terms);
    config
        .infrastructure
        .as_mut()
        .unwrap()
        .capacities
        .iter_mut()
        .find(|row| row.id == "alloy")
        .unwrap()
        .amount = 0;
    phoenix_sim_gameplay::resources::ports_profile::validate_entity_port(&config).unwrap();
    assert!(app.world_mut().despawn(old_provider));
    let provider = crate::entities::spawner::spawn_entity(
        &mut app.world_mut().commands(),
        &config,
        Vec3::ZERO,
        PROVIDER.into(),
        None,
    );
    app.world_mut().flush();
    app.world_mut()
        .get_mut::<crate::ship_plugin::ShipSystemControlSources>(provider)
        .unwrap()
        .0
        .set(id("dock"), ControlSource::Ai);
    app.update();
    (app, visitor, provider)
}
fn work(app: &mut App, visitor: Entity, provider: Entity, dt: f32) -> PortTransfer {
    app.world_mut()
        .resource_mut::<Time>()
        .advance_by(std::time::Duration::from_secs_f32(dt));
    app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 += 1;
    app.world_mut()
        .get_mut::<TransferUmbilical>(visitor)
        .unwrap()
        .running = true;
    request(app, visitor, provider, "hold-machinery-parts", 4)
}
fn energy(app: &App, entity: Entity, system: &str) -> f32 {
    app.world()
        .get::<ShipPowerSystem>(entity)
        .unwrap()
        .0
        .capture_continuation()
        .network
        .consumers[&id(system)]
        .energy
}
fn actual_state(app: &App, visitor: Entity, provider: Entity) -> serde_json::Value {
    serde_json::to_value((
        app.world()
            .get::<ShipPowerSystem>(visitor)
            .unwrap()
            .0
            .capture_continuation(),
        app.world()
            .get::<ShipPowerSystem>(provider)
            .unwrap()
            .0
            .capture_continuation(),
        &app.world()
            .get::<crate::infrastructure::InfrastructureCondition>(visitor)
            .unwrap()
            .0,
        &app.world()
            .get::<crate::infrastructure::InfrastructureCondition>(provider)
            .unwrap()
            .0,
        app.world()
            .get::<TransferUmbilical>(visitor)
            .unwrap()
            .save_state(),
        &app.world().get::<PortProviderState>(provider).unwrap().0,
    ))
    .unwrap()
}

#[test]
fn both_actual_provider_profiles_pay_common_work_and_move_only_real_stock_once() {
    for policy in [
        PortServicePolicy::FreeAlliedReplenishment,
        PortServicePolicy::ManualOffer,
    ] {
        let (mut app, visitor, provider) = equipped(policy);
        let request = work(&mut app, visitor, provider, 0.25);
        let before_provider = quantity(&app, provider, "machinery_parts");
        let before_visitor = quantity(&app, visitor, "hold-machinery-parts");
        let before_power = [
            energy(&app, visitor, "umbilical"),
            energy(&app, provider, "dock"),
        ];
        assert_eq!(
            crate::resources::ports_equipment::settle(app.world_mut(), &request),
            Ok(1)
        );
        assert_eq!(
            quantity(&app, provider, "machinery_parts"),
            before_provider - 5
        );
        assert_eq!(
            quantity(&app, visitor, "hold-machinery-parts"),
            before_visitor + 5
        );
        assert!(energy(&app, visitor, "umbilical") < before_power[0]);
        assert!(energy(&app, provider, "dock") < before_power[1]);
        let after = actual_state(&app, visitor, provider);
        assert_eq!(
            crate::resources::ports_equipment::settle(app.world_mut(), &request),
            Ok(0)
        );
        assert_eq!(
            actual_state(&app, visitor, provider),
            after,
            "duplicate work cannot charge either endpoint"
        );
    }
}

#[test]
fn unavailable_or_rejected_final_stock_has_no_payment_or_work_frontier_write() {
    for refusal in 0..4 {
        let (mut app, visitor, provider) = equipped(PortServicePolicy::ManualOffer);
        let request = work(&mut app, visitor, provider, 0.25);
        match refusal {
            0 => app
                .world_mut()
                .get_mut::<crate::ship_plugin::ShipSystemControlSources>(provider)
                .unwrap()
                .0
                .set(id("dock"), ControlSource::Offline),
            1 => {
                let mut damage = app.world_mut().get_mut::<ShipDamage>(provider).unwrap();
                let component = damage
                    .state
                    .components
                    .get_mut(&id("dock"))
                    .unwrap()
                    .get_mut("operation")
                    .unwrap();
                component.condition = phoenix_model::damage::ComponentCondition::Destroyed;
                component.last_source = Some("actual-test-incident".into());
            }
            2 => {
                app.world_mut()
                    .get_mut::<ShipPowerSystem>(provider)
                    .unwrap()
                    .0
                    .spend_continuous(&id("dock"), f32::MAX)
                    .unwrap();
            }
            _ => {
                let mut inventory = app
                    .world_mut()
                    .get_mut::<crate::infrastructure::InfrastructureCondition>(provider)
                    .unwrap();
                let mut stock = inventory
                    .0
                    .capacity_stock("machinery_parts")
                    .unwrap()
                    .clone();
                stock
                    .reserve("completed-prior-withdrawal", stock.level)
                    .unwrap();
                stock.consume("completed-prior-withdrawal").unwrap();
                inventory
                    .0
                    .replace_capacity_stock("machinery_parts", stock)
                    .unwrap();
            }
        }
        let before = actual_state(&app, visitor, provider);
        assert!(
            crate::resources::ports_equipment::settle(app.world_mut(), &request).is_err(),
            "case {refusal}"
        );
        assert_eq!(
            actual_state(&app, visitor, provider),
            before,
            "case {refusal}"
        );
    }
}

#[test]
fn provider_operation_and_partial_funding_limit_only_actual_work_with_dated_fraction() {
    for partial_power in [false, true] {
        let (mut app, visitor, provider) = equipped(PortServicePolicy::ManualOffer);
        let request = work(&mut app, visitor, provider, 0.25);
        if partial_power {
            let amount = energy(&app, provider, "dock") - 0.0125;
            app.world_mut()
                .get_mut::<ShipPowerSystem>(provider)
                .unwrap()
                .0
                .spend_continuous(&id("dock"), amount)
                .unwrap();
        } else {
            let mut damage = app.world_mut().get_mut::<ShipDamage>(provider).unwrap();
            let component = damage
                .state
                .components
                .get_mut(&id("dock"))
                .unwrap()
                .get_mut("operation")
                .unwrap();
            component.condition = phoenix_model::damage::ComponentCondition::Damaged;
            component.last_source = Some("actual-test-incident".into());
        }
        let before = quantity(&app, provider, "machinery_parts");
        let visitor_before = energy(&app, visitor, "umbilical");
        assert_eq!(
            crate::resources::ports_equipment::settle(app.world_mut(), &request),
            Ok(0)
        );
        let plan = &app
            .world()
            .get::<TransferUmbilical>(visitor)
            .unwrap()
            .port_order;
        assert!(plan.fraction > 0.0 && plan.fraction < 1.0);
        assert_eq!(plan.last_tick, request.tick);
        assert!(plan.last_outcome.as_ref().unwrap().settled);
        assert_eq!(plan.lines[0].delivered_lots, 0);
        assert_eq!(quantity(&app, provider, "machinery_parts"), before);
        assert!(energy(&app, visitor, "umbilical") < visitor_before);
    }
}

fn repair_dock(
    mut visitors: Query<(
        &mut crate::console::repair::server::ShipRepairTeams,
        &mut ShipPowerSystem,
        &mut crate::infrastructure::InfrastructureCondition,
        &ResourceServices,
    )>,
    mut targets: Query<(&EntityUuid, &mut ShipDamage)>,
) {
    let (_, mut target) = targets
        .iter_mut()
        .find(|(uuid, _)| uuid.0 == PROVIDER)
        .unwrap();
    let ShipDamage {
        definition, state, ..
    } = &mut *target;
    let (mut teams, mut power, mut inventory, services) = visitors.single_mut().unwrap();
    teams.0.tick_external_physical(
        0.25,
        PROVIDER,
        definition,
        state,
        &BTreeSet::new(),
        |action, claim, recipe| {
            if action == crate::jobs::repair::RepairStockAction::Release {
                inventory.0.release_repair_claim(claim);
                return true;
            }
            inventory
                .0
                .repair_parts(action, claim, recipe.unwrap(), &services.0)
                .is_ok()
        },
        |system, seconds| {
            power
                .0
                .work_continuous(system, seconds, 1.0)
                .map_or(0.0, |grant| grant.fraction)
        },
    );
}

#[test]
fn real_repair_owner_consumes_visitor_parts_and_restores_the_same_facility_service() {
    let (mut app, visitor, provider) = equipped(PortServicePolicy::ManualOffer);
    let request = work(&mut app, visitor, provider, 0.25);
    {
        let mut damage = app.world_mut().get_mut::<ShipDamage>(provider).unwrap();
        let component = damage
            .state
            .components
            .get_mut(&id("dock"))
            .unwrap()
            .get_mut("operation")
            .unwrap();
        component.condition = phoenix_model::damage::ComponentCondition::Destroyed;
        component.last_source = Some("actual-test-incident".into());
    }
    assert!(crate::resources::ports_equipment::settle(app.world_mut(), &request).is_err());
    assert!(app
        .world_mut()
        .get_mut::<crate::console::repair::server::ShipRepairTeams>(visitor)
        .unwrap()
        .0
        .claim_external(0, PROVIDER.into()));
    let parts = quantity(&app, visitor, "machinery_parts");
    let provider_stock = quantity(&app, provider, "machinery_parts");
    for _ in 0..120 {
        app.world_mut().run_system_once(repair_dock).unwrap();
        if app
            .world()
            .get::<ShipDamage>(provider)
            .unwrap()
            .factor(&id("dock"), phoenix_model::damage::DamageBinding::Operation)
            == 1.0
        {
            break;
        }
    }
    assert_eq!(
        app.world()
            .get::<ShipDamage>(provider)
            .unwrap()
            .factor(&id("dock"), phoenix_model::damage::DamageBinding::Operation),
        1.0
    );
    assert!(quantity(&app, visitor, "machinery_parts") < parts);
    assert_eq!(
        quantity(&app, provider, "machinery_parts"),
        provider_stock,
        "repair never refills provider supplies"
    );
    app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 += 1;
    let mut resumed = request;
    resumed.tick += 1;
    assert_eq!(
        crate::resources::ports_equipment::settle(app.world_mut(), &resumed),
        Ok(1)
    );
    assert_eq!(
        quantity(&app, provider, "machinery_parts"),
        provider_stock - 5
    );
}

#[test]
fn json_paid_fraction_resumes_existing_energy_stock_quota_and_order_owners_for_360_steps() {
    type Saved = (
        phoenix_sim_gameplay::modifiers::power_system::PowerState,
        phoenix_sim_gameplay::modifiers::power_system::PowerState,
        crate::infrastructure::InfrastructureState,
        crate::infrastructure::InfrastructureState,
        crate::umbilical::UmbilicalSaveState,
        PortVisitState,
    );
    let (mut live, visitor, provider) = equipped(PortServicePolicy::ManualOffer);
    let request = work(&mut live, visitor, provider, 1.0 / 256.0);
    assert_eq!(
        crate::resources::ports_equipment::settle(live.world_mut(), &request),
        Ok(0)
    );
    let fraction = live
        .world()
        .get::<TransferUmbilical>(visitor)
        .unwrap()
        .port_order
        .fraction;
    assert!(fraction > 0.0 && fraction < 1.0);
    let saved = serde_json::to_vec(&actual_state(&live, visitor, provider)).unwrap();
    let (left_power, right_power, left_stock, right_stock, order, ledger): Saved =
        serde_json::from_slice(&saved).unwrap();
    let (mut resumed, second_visitor, second_provider) = equipped(PortServicePolicy::ManualOffer);
    resumed
        .world_mut()
        .get_mut::<ShipPowerSystem>(second_visitor)
        .unwrap()
        .0
        .restore_continuation(&left_power)
        .unwrap();
    resumed
        .world_mut()
        .get_mut::<ShipPowerSystem>(second_provider)
        .unwrap()
        .0
        .restore_continuation(&right_power)
        .unwrap();
    resumed
        .world_mut()
        .get_mut::<crate::infrastructure::InfrastructureCondition>(second_visitor)
        .unwrap()
        .0 = left_stock;
    resumed
        .world_mut()
        .get_mut::<crate::infrastructure::InfrastructureCondition>(second_provider)
        .unwrap()
        .0 = right_stock;
    let profile = resumed
        .world()
        .get::<CompiledPortService>(second_provider)
        .unwrap()
        .0
        .clone();
    order.port_order.validate(&profile, request.tick).unwrap();
    resumed
        .world_mut()
        .get_mut::<TransferUmbilical>(second_visitor)
        .unwrap()
        .restore(&order);
    resumed
        .world_mut()
        .get_mut::<PortProviderState>(second_provider)
        .unwrap()
        .0 = ledger;
    assert_eq!(
        actual_state(&live, visitor, provider),
        actual_state(&resumed, second_visitor, second_provider)
    );
    for offset in 1..=360 {
        let mut next = request.clone();
        next.tick += offset;
        for app in [&mut live, &mut resumed] {
            app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 = next.tick;
            app.world_mut()
                .resource_mut::<Time>()
                .advance_by(std::time::Duration::from_secs_f32(1.0 / 256.0));
        }
        assert_eq!(
            crate::resources::ports_equipment::settle(live.world_mut(), &next),
            crate::resources::ports_equipment::settle(resumed.world_mut(), &next),
            "step {offset}"
        );
        assert_eq!(
            actual_state(&live, visitor, provider),
            actual_state(&resumed, second_visitor, second_provider),
            "step {offset}"
        );
    }
    assert_eq!(
        live.world()
            .get::<TransferUmbilical>(visitor)
            .unwrap()
            .port_order
            .lines[0]
            .delivered_lots,
        4
    );
}

#[test]
fn two_actual_visitors_settle_in_public_id_order_without_charging_rejected_work() {
    const OTHER: &str = "00000000-0000-0000-0000-000000000004";
    let run = |reverse: bool| {
        let (mut app, first, provider) = equipped(PortServicePolicy::ManualOffer);
        let hull = crate::entities::include_resolve::load_entity_config(
            "assets/entities/alliance_cruiser.toml",
        )
        .unwrap();
        let second = crate::entities::spawner::spawn_entity(
            &mut app.world_mut().commands(),
            &hull,
            Vec3::ZERO,
            OTHER.into(),
            None,
        );
        app.world_mut().flush();
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
        app.world_mut().entity_mut(second).insert(dock);
        app.world_mut()
            .get_mut::<crate::ship_plugin::ShipSystemControlSources>(second)
            .unwrap()
            .0
            .set(id("umbilical"), ControlSource::Human);
        {
            let mut inventory = app
                .world_mut()
                .get_mut::<crate::infrastructure::InfrastructureCondition>(second)
                .unwrap();
            let mut stock = inventory
                .0
                .capacity_stock("hold-machinery-parts")
                .unwrap()
                .clone();
            stock.reserve("completed-parts-use", 20).unwrap();
            stock.consume("completed-parts-use").unwrap();
            inventory
                .0
                .replace_capacity_stock("hold-machinery-parts", stock)
                .unwrap();
        }
        let mut first_request = work(&mut app, first, provider, 0.25);
        first_request.lots = 1;
        let profile = app
            .world()
            .get::<CompiledPortService>(provider)
            .unwrap()
            .0
            .clone();
        let now = first_request.tick;
        app.world_mut()
            .get_mut::<PortProviderState>(provider)
            .unwrap()
            .0
            .refresh(
                &profile,
                now,
                &BTreeSet::from([VISITOR.to_owned(), OTHER.to_owned()]),
            )
            .unwrap();
        let visit = app
            .world()
            .get::<PortProviderState>(provider)
            .unwrap()
            .0
            .visitors[OTHER]
            .visit;
        let mut second_request = first_request.clone();
        second_request.operator = OTHER.into();
        second_request.visit = visit;
        let mut order = PortOrderState::default();
        order
            .approve(
                &profile,
                PROVIDER,
                visit,
                vec![phoenix_model::ports::PortOrderLine {
                    offer: "parts".into(),
                    store: "hold-machinery-parts".into(),
                    lots: 1,
                }],
            )
            .unwrap();
        {
            let mut tool = app
                .world_mut()
                .get_mut::<TransferUmbilical>(second)
                .unwrap();
            tool.running = true;
            tool.port_order = order;
        }
        {
            let mut inventory = app
                .world_mut()
                .get_mut::<crate::infrastructure::InfrastructureCondition>(provider)
                .unwrap();
            let mut stock = inventory
                .0
                .capacity_stock("machinery_parts")
                .unwrap()
                .clone();
            stock
                .reserve("prior-visitor-withdrawal", stock.level - 5)
                .unwrap();
            stock.consume("prior-visitor-withdrawal").unwrap();
            inventory
                .0
                .replace_capacity_stock("machinery_parts", stock)
                .unwrap();
        }
        let before = actual_state(&app, second, provider);
        let first_stock = quantity(&app, first, "hold-machinery-parts");
        let requests = if reverse {
            vec![second_request, first_request]
        } else {
            vec![first_request, second_request]
        };
        app.world_mut()
            .resource_mut::<crate::effect_queue::EffectQueue<PortTransfer>>()
            .0 = requests;
        super::super::settle(app.world_mut());
        assert_eq!(
            quantity(&app, first, "hold-machinery-parts"),
            first_stock + 5
        );
        assert_eq!(quantity(&app, provider, "machinery_parts"), 0);
        let after = actual_state(&app, second, provider);
        for index in [0, 2, 4] {
            assert_eq!(
                before[index], after[index],
                "rejected visitor power/stock/standing order must not change"
            );
        }
        (
            quantity(&app, first, "hold-machinery-parts"),
            quantity(&app, second, "hold-machinery-parts"),
            energy(&app, provider, "dock"),
            app.world()
                .get::<PortProviderState>(provider)
                .unwrap()
                .0
                .clone(),
        )
    };
    assert_eq!(run(false), run(true));
}

#[test]
fn actual_medical_reference_replenishes_its_managed_hold_from_finite_paid_port_stock() {
    let hull = crate::entities::include_resolve::load_entity_config(
        "assets/entities/medical_cruiser.toml",
    )
    .unwrap();
    let (mut app, visitor, old_provider) =
        rig_with(PortServicePolicy::FreeAlliedReplenishment, |content| {
            *content = hull.clone()
        });
    let machinery = crate::entities::spawner::spawn_entity(
        &mut app.world_mut().commands(),
        &hull,
        Vec3::ZERO,
        "medical-machinery".into(),
        None,
    );
    app.world_mut().flush();
    let damage = app
        .world_mut()
        .entity_mut(machinery)
        .take::<ShipDamage>()
        .unwrap();
    let repair = app
        .world_mut()
        .entity_mut(machinery)
        .take::<crate::console::repair::server::ShipRepairTeams>()
        .unwrap();
    let services = app
        .world_mut()
        .entity_mut(machinery)
        .take::<ResourceServices>()
        .unwrap();
    app.world_mut()
        .entity_mut(visitor)
        .insert((damage, repair, services));
    assert!(app.world_mut().despawn(machinery));
    let port = crate::entities::include_resolve::load_entity_config(
        "assets/entities/medical_service_port.toml",
    )
    .unwrap();
    assert!(app.world_mut().despawn(old_provider));
    let provider = crate::entities::spawner::spawn_entity(
        &mut app.world_mut().commands(),
        &port,
        Vec3::ZERO,
        PROVIDER.into(),
        None,
    );
    app.world_mut().flush();
    app.world_mut()
        .get_mut::<crate::ship_plugin::ShipSystemControlSources>(provider)
        .unwrap()
        .0
        .set(id("dock"), ControlSource::Ai);
    app.update();
    let profile = app
        .world()
        .get::<CompiledPortService>(provider)
        .unwrap()
        .0
        .clone();
    let visit = app
        .world()
        .get::<PortProviderState>(provider)
        .unwrap()
        .0
        .visitors[VISITOR]
        .visit;
    let offer = profile
        .offers
        .iter()
        .find(|offer| offer.resource == "medical_supplies")
        .unwrap();
    let destination =
        actual_store(app.world(), visitor, "hold-medication", &offer.resource).unwrap();
    app.world_mut()
        .resource_mut::<Time>()
        .advance_by(std::time::Duration::from_secs_f32(1.0));
    app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 += 1;
    app.world_mut()
        .get_mut::<TransferUmbilical>(visitor)
        .unwrap()
        .running = true;
    let request = PortTransfer {
        operator: VISITOR.into(),
        provider: PROVIDER.into(),
        visit,
        offer: offer.id.clone(),
        destination,
        store: Some("hold-medication".into()),
        // Permit two real lots, retaining room/quota limits. At the authored
        // rate20 a single lot takes nonrepresentable f32 0.05s: conservative
        // work rounding correctly leaves it just below one whole lot. This
        // request exercises one complete lot PLUS retained paid fractional WIP.
        lots: 2,
        tick: app.world().resource::<crate::sim_tick::SimTick>().0,
        ordered: false,
        inbound: false,
        maintain_goal: false,
    };
    let before = [
        quantity(&app, visitor, "hold-medication"),
        quantity(&app, provider, "medical-supplies"),
    ];
    let before_power = [
        energy(&app, visitor, "umbilical"),
        energy(&app, provider, "dock"),
    ];
    assert_eq!(
        crate::resources::ports_equipment::settle(app.world_mut(), &request),
        Ok(1)
    );
    let flow = app
        .world()
        .get::<TransferUmbilical>(visitor)
        .unwrap()
        .port_flow
        .as_ref()
        .unwrap();
    assert_eq!(flow.lines[0].requested_lots, 2);
    assert_eq!(flow.lines[0].delivered_lots, 1);
    assert!(
        flow.fraction > 0.0 && flow.fraction < 1.0,
        "fractional second lot is paid work, not goods"
    );
    assert_eq!(quantity(&app, visitor, "hold-medication"), before[0] + 1);
    assert_eq!(quantity(&app, provider, "medical-supplies"), before[1] - 1);
    assert!(energy(&app, visitor, "umbilical") < before_power[0]);
    assert!(energy(&app, provider, "dock") < before_power[1]);
    let actual = actual_state(&app, visitor, provider);
    assert_eq!(
        crate::resources::ports_equipment::settle(app.world_mut(), &request),
        Ok(0)
    );
    assert_eq!(actual_state(&app, visitor, provider), actual);
}
