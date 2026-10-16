use super::*;
use crate::core::messages::{ConsoleId, SystemId};
use crate::resources::logistics_server::tests::{fixture_with, send};
use crate::ship::control_source::ControlSource;

fn id(value: &str) -> SystemId {
    SystemId(value.into())
}

/// Retain the real authored stocks, buffers and tree. Only unrelated delivery
/// execution is suspended through its ordinary controller/standing-order state.
fn factory(
    source: ControlSource,
    target: i64,
    edit: impl FnOnce(&mut crate::entities::config::EntityConfig),
) -> (App, Entity) {
    let (mut app, entity) = fixture_with(source, |config| {
        config.production.as_mut().unwrap().initial_target = target;
        edit(config);
    });
    app.world_mut()
        .get_mut::<ShipLogistics>(entity)
        .unwrap()
        .0
        .enabled = false;
    app.world_mut()
        .get_mut::<ShipSystemControlSources>(entity)
        .unwrap()
        .0
        .set(id("logistics"), ControlSource::Offline);
    (app, entity)
}

fn stock(app: &App, entity: Entity, capacity: &str) -> crate::resources::Stock {
    app.world()
        .get::<InfrastructureCondition>(entity)
        .unwrap()
        .0
        .capacity_stock(capacity)
        .unwrap()
        .clone()
}

fn energy(app: &App, entity: Entity) -> f32 {
    app.world()
        .get::<crate::ship::power::ShipPowerSystem>(entity)
        .unwrap()
        .0
        .network()
        .consumers[&id("fabrication")]
        .energy
}

fn owner(app: &App, entity: Entity) -> &ProductionState {
    &app.world().get::<ShipProduction>(entity).unwrap().0
}

fn stocks(
    app: &App,
    entity: Entity,
) -> (
    crate::resources::Stock,
    crate::resources::Stock,
    crate::resources::Stock,
) {
    (
        stock(app, entity, "hold-ammunition"),
        stock(app, entity, "alloy"),
        stock(app, entity, "hold-machinery-parts"),
    )
}

fn assert_quantity(app: &App, entity: Entity, ammunition: i64, alloy: i64, machinery: i64) {
    let (ammo, material, parts) = stocks(app, entity);
    assert_eq!(
        (ammo.level, material.level, parts.level),
        (ammunition, alloy, machinery)
    );
    let weapons = &app.world().get::<TorpedoSystemResource>(entity).unwrap().0;
    assert!(weapons.in_flight.is_empty());
    assert_eq!(
        i64::from(weapons.rounds_aboard()) + ammo.level,
        ammunition + 2
    );
    assert!(app
        .world()
        .get::<ShipLogistics>(entity)
        .unwrap()
        .0
        .batches
        .is_empty());
}

fn advance_to_active(app: &mut App, entity: Entity) {
    for _ in 0..20 {
        app.update();
        if owner(app, entity)
            .active
            .as_ref()
            .is_some_and(|batch| batch.progress > 0.0)
        {
            return;
        }
    }
    panic!("ordinary controller must start actual paid fabrication");
}

fn advance_to_goal(app: &mut App, entity: Entity, goal: i64) {
    let work = app
        .world()
        .get::<CompiledShipProduction>(entity)
        .unwrap()
        .0
        .config()
        .recipes[0]
        .work_secs;
    // Two existing ten-second recipes, five fixed steps per second, plus
    // publication/ordinary AI cadence slack. No hardware/timing is modified.
    let limit = (2.0 * work / 0.2).ceil() as usize + 30;
    for _ in 0..limit {
        app.update();
        if stock(app, entity, "hold-ammunition").level == goal
            && owner(app, entity).active.is_none()
        {
            return;
        }
    }
    panic!("actual paid work must finish the standing stock goal");
}

#[test]
fn human_standing_goal_consumes_real_materials_and_local_energy_once() {
    let (mut app, entity) = factory(ControlSource::Human, 0, |_| {});
    let before_energy = energy(&app, entity);
    assert_quantity(&app, entity, 4, 12, 102);
    send(
        &mut app,
        "engineer",
        "fabrication",
        Control::SetProductionTarget { amount: 6 },
    );
    advance_to_active(&mut app, entity);
    assert_quantity(&app, entity, 4, 10, 101);
    assert_eq!(stock(&app, entity, "hold-ammunition").incoming_total(), 1);
    advance_to_goal(&mut app, entity, 6);
    assert_quantity(&app, entity, 6, 8, 100); // Six starting rounds become eight only through two real recipes.
    assert!(stock(&app, entity, "hold-ammunition").incoming.is_empty());
    assert!((before_energy - energy(&app, entity) - 20.0).abs() < 0.001);
    assert_eq!(owner(&app, entity).target, 6); // A physical stock goal, never a decrementing batch counter.
    let finished = (stocks(&app, entity), energy(&app, entity));
    for _ in 0..30 {
        app.update();
    }
    assert_eq!((stocks(&app, entity), energy(&app, entity)), finished);
}

#[test]
fn ordinary_installed_ai_maintains_the_authored_goal_with_the_same_paid_work() {
    let (mut app, entity) = factory(ControlSource::Ai, 6, |_| {});
    let before_energy = energy(&app, entity);
    advance_to_active(&mut app, entity);
    advance_to_goal(&mut app, entity, 6);
    assert_quantity(&app, entity, 6, 8, 100);
    assert!((before_energy - energy(&app, entity) - 20.0).abs() < 0.001);
    assert!(owner(&app, entity)
        .last_outcome
        .as_ref()
        .is_some_and(|outcome| !outcome.discarded));
}

#[test]
fn zero_goal_keeps_human_and_ai_fabrication_idle_without_spending() {
    for source in [ControlSource::Human, ControlSource::Ai] {
        let (mut app, entity) = factory(source, 0, |_| {});
        let initial = (stocks(&app, entity), energy(&app, entity));
        for _ in 0..30 {
            app.update();
        }
        assert_eq!((stocks(&app, entity), energy(&app, entity)), initial);
        assert!(owner(&app, entity).active.is_none());
        assert_eq!(owner(&app, entity).next_id, 1);
    }
}

#[test]
fn disabled_then_offline_work_preserves_consumed_wip_claims_and_energy_until_resumed() {
    let (mut app, entity) = factory(ControlSource::Human, 6, |_| {});
    advance_to_active(&mut app, entity);
    send(
        &mut app,
        "engineer",
        "fabrication",
        Control::SetProductionEnabled { enabled: false },
    );
    app.update();
    assert!(!owner(&app, entity).enabled);
    let paused = (
        owner(&app, entity).active.clone(),
        stocks(&app, entity),
        energy(&app, entity),
    );
    for _ in 0..20 {
        app.update();
    }
    assert_eq!(
        (
            owner(&app, entity).active.clone(),
            stocks(&app, entity),
            energy(&app, entity)
        ),
        paused
    );
    send(
        &mut app,
        "engineer",
        "fabrication",
        Control::SetProductionEnabled { enabled: true },
    );
    app.update();
    assert!(
        owner(&app, entity).active.as_ref().unwrap().progress > paused.0.as_ref().unwrap().progress
    );
    app.world_mut()
        .get_mut::<ShipSystemControlSources>(entity)
        .unwrap()
        .0
        .set(id("fabrication"), ControlSource::Offline);
    let offline = (
        owner(&app, entity).active.clone(),
        stocks(&app, entity),
        energy(&app, entity),
    );
    for _ in 0..20 {
        app.update();
    }
    assert_eq!(
        (
            owner(&app, entity).active.clone(),
            stocks(&app, entity),
            energy(&app, entity)
        ),
        offline
    );
    app.world_mut()
        .get_mut::<ShipSystemControlSources>(entity)
        .unwrap()
        .0
        .set(id("fabrication"), ControlSource::Human);
    advance_to_goal(&mut app, entity, 6);
    assert_quantity(&app, entity, 6, 8, 100);
}

fn set_inventory_floor(app: &mut App, entity: Entity, amount: i64) {
    app.world_mut()
        .get_mut::<ShipSystemControlSources>(entity)
        .unwrap()
        .0
        .set(id("logistics"), ControlSource::Human);
    send(
        app,
        "engineer",
        "logistics",
        Control::SetStockReserve {
            store: "hold-machinery-parts".into(),
            amount,
        },
    );
    app.update();
    assert_eq!(
        app.world().get::<ShipLogistics>(entity).unwrap().0.floors["hold-machinery-parts"],
        amount
    );
    app.world_mut()
        .get_mut::<ShipSystemControlSources>(entity)
        .unwrap()
        .0
        .set(id("logistics"), ControlSource::Offline);
}

#[test]
fn current_inventory_protected_floor_blocks_material_intake_then_releases_the_same_goal() {
    let (mut app, entity) = factory(ControlSource::Human, 0, |_| {});
    set_inventory_floor(&mut app, entity, 102);
    send(
        &mut app,
        "engineer",
        "fabrication",
        Control::SetProductionTarget { amount: 6 },
    );
    let initial = (stocks(&app, entity), energy(&app, entity));
    for _ in 0..20 {
        app.update();
    }
    assert_eq!((stocks(&app, entity), energy(&app, entity)), initial);
    assert!(owner(&app, entity).active.is_none());
    set_inventory_floor(&mut app, entity, 20);
    advance_to_goal(&mut app, entity, 6);
    assert_quantity(&app, entity, 6, 8, 100);
}

#[test]
fn explicit_recipe_change_discards_only_consumed_wip_and_releases_its_receiving_claim() {
    let (mut app, entity) = factory(ControlSource::Human, 6, |config| {
        let profile = config.production.as_mut().unwrap();
        let mut alternate = profile.recipes[0].clone();
        alternate.id = "alternate-round".into();
        profile.recipes.push(alternate);
    });
    advance_to_active(&mut app, entity);
    let batch = owner(&app, entity).active.clone().unwrap();
    assert_quantity(&app, entity, 4, 10, 101);
    let consumed = (
        stock(&app, entity, "alloy"),
        stock(&app, entity, "hold-machinery-parts"),
        energy(&app, entity),
    );
    send(
        &mut app,
        "engineer",
        "fabrication",
        Control::SelectProductionRecipe {
            recipe: Some("alternate-round".into()),
            target: 0,
        },
    );
    app.update();
    assert!(owner(&app, entity).active.is_none());
    let outcome = owner(&app, entity).last_outcome.as_ref().unwrap();
    assert!(outcome.discarded);
    assert_eq!(outcome.id, batch.id);
    assert!(stock(&app, entity, "hold-ammunition").incoming.is_empty());
    assert_eq!(
        (
            stock(&app, entity, "alloy"),
            stock(&app, entity, "hold-machinery-parts"),
            energy(&app, entity)
        ),
        consumed
    );
    assert_quantity(&app, entity, 4, 10, 101);
    send(
        &mut app,
        "engineer",
        "fabrication",
        Control::SetProductionTarget { amount: 5 },
    );
    advance_to_goal(&mut app, entity, 5);
    assert_quantity(&app, entity, 5, 8, 100);
}

#[test]
fn cohosted_consoles_do_not_share_logistics_knowledge_for_human_or_ordinary_ai_starts() {
    for source in [ControlSource::Human, ControlSource::Ai] {
        let (mut app, entity) = factory(source, 6, |config| {
            let ship = config.ship_config.as_mut().unwrap();
            let mut console = ship
                .consoles
                .iter()
                .find(|console| console.id.0 == "engineering")
                .unwrap()
                .clone();
            console.id = ConsoleId("factory-console".into());
            ship.consoles.push(console);
            ship.systems
                .iter_mut()
                .find(|system| system.id == id("fabrication"))
                .unwrap()
                .console = Some(ConsoleId("factory-console".into()));
        });
        let original = {
            let mut config = app
                .world_mut()
                .get_mut::<ShipConfigComponent>(entity)
                .unwrap();
            let console = config
                .0
                .consoles
                .iter_mut()
                .find(|console| console.id.0 == "factory-console")
                .unwrap();
            let feeds = console.feeds.clone();
            console.feeds.retain(|feed| feed.source != id("logistics"));
            feeds
        };
        let initial = (stocks(&app, entity), energy(&app, entity));
        for _ in 0..20 {
            app.update();
        }
        assert!(owner(&app, entity).active.is_none());
        assert_eq!((stocks(&app, entity), energy(&app, entity)), initial);
        let config = &app.world().get::<ShipConfigComponent>(entity).unwrap().0;
        let boards = &app
            .world()
            .get::<crate::server_app::ShipSystemBlackboards>(entity)
            .unwrap()
            .0;
        assert!(matches!(
            boards.get(&id("logistics")),
            Some(SystemBlackboard::Logistics(_))
        ));
        assert!(config
            .observations_for_system(&id("fabrication"), boards)
            .get(&id("logistics"))
            .is_none());
        assert_eq!(
            config
                .consoles
                .iter()
                .find(|console| console.id.0 == "factory-console")
                .unwrap()
                .home_station,
            config
                .consoles
                .iter()
                .find(|console| console.id.0 == "engineering")
                .unwrap()
                .home_station
        );
        app.world_mut()
            .get_mut::<ShipConfigComponent>(entity)
            .unwrap()
            .0
            .consoles
            .iter_mut()
            .find(|console| console.id.0 == "factory-console")
            .unwrap()
            .feeds = original;
        advance_to_goal(&mut app, entity, 6);
        assert_quantity(&app, entity, 6, 8, 100);
    }
}

#[test]
fn idle_production_advances_its_clock_without_marking_power_changed() {
    use bevy::ecs::system::RunSystemOnce;
    let (mut app, entity) = factory(ControlSource::Human, 0, |_| {});
    app.update();
    let before = app
        .world()
        .get::<crate::ship::power::ShipPowerSystem>(entity)
        .unwrap()
        .0
        .clone();
    let mut expected = owner(&app, entity).clone();
    let quantities = stocks(&app, entity);
    app.world_mut().clear_trackers();
    app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 += 1;
    let now = app.world().resource::<crate::sim_tick::SimTick>().0;
    app.world_mut().run_system_once(tick).unwrap();
    expected.last_tick = now;
    expected.blocked = None;
    assert_eq!(owner(&app, entity), &expected);
    assert_eq!(stocks(&app, entity), quantities);
    let power = app
        .world()
        .entity(entity)
        .get_ref::<crate::ship::power::ShipPowerSystem>()
        .unwrap();
    assert_eq!(power.0, before);
    assert!(
        !power.is_changed(),
        "no payment callback must leave Power untouched"
    );
}

#[test]
fn production_lazy_payment_matches_eager_power_for_funding_shortfall_and_errors() {
    let (app, entity) = factory(ControlSource::Human, 0, |_| {});
    let original = app
        .world()
        .get::<crate::ship::power::ShipPowerSystem>(entity)
        .unwrap();
    for (system, seconds, available, factor) in [
        (id("fabrication"), 0.2, true, 1.0),
        (id("fabrication"), 1_000_000.0, true, 0.5),
        (id("missing-system"), 0.2, true, 1.0),
        (id("fabrication"), 0.2, false, 1.0),
        (id("fabrication"), 0.2, true, 0.0),
    ] {
        let mut eager = original.clone();
        let mut staged = None;
        for _ in 0..2 {
            let expected = if available && factor > 0.0 {
                eager
                    .0
                    .work_continuous(&system, seconds, 1.0)
                    .map(|grant| grant.fraction * factor)
            } else {
                Ok(0.0)
            };
            let actual = pay_work(
                &system,
                seconds,
                available,
                factor,
                Some(original),
                &mut staged,
            );
            assert_eq!(actual.map(f32::to_bits), expected.map(f32::to_bits));
            assert_eq!(
                staged.as_ref().map(|power| &power.0).unwrap_or(&original.0),
                &eager.0
            );
        }
        assert_eq!(staged.is_some(), available && factor > 0.0);
    }
}

#[test]
fn refused_production_after_actual_payment_discards_staged_power_and_materials() {
    let (app, entity) = factory(ControlSource::Human, 6, |_| {});
    let profile = &app.world().get::<CompiledShipProduction>(entity).unwrap().0;
    let mut next = owner(&app, entity).clone();
    let mut stocks = endpoint_view(
        profile.logistics(),
        app.world().get::<InfrastructureCondition>(entity),
        app.world().get::<TorpedoSystemResource>(entity),
    )
    .unwrap();
    let before_stocks = stocks.clone();
    let before_state = next.clone();
    let power = app
        .world()
        .get::<crate::ship::power::ShipPowerSystem>(entity)
        .unwrap();
    let before_power = power.0.clone();
    let mut staged = None;
    // Exercise the existing invalid funded-fraction boundary AFTER real finite
    // Power expenditure, not an injected energy grant or fabricated batch.
    let refused = next.try_start(
        profile,
        1,
        0.2,
        &app.world().get::<ShipLogistics>(entity).unwrap().0.floors,
        &mut stocks,
        |system, seconds| pay_work(system, seconds, true, f32::NAN, Some(power), &mut staged),
    );
    assert!(refused.is_err());
    let paid = &staged.as_ref().unwrap().0;
    assert!(paid.emitted_work(&id("fabrication")) > before_power.emitted_work(&id("fabrication")));
    assert!(
        paid.network().consumers[&id("fabrication")].energy
            < before_power.network().consumers[&id("fabrication")].energy
    );
    assert_eq!(next, before_state);
    assert_eq!(stocks, before_stocks);
    assert_eq!(
        power.0, before_power,
        "a refused transaction cannot publish its Power candidate"
    );
}
