use super::*;
use crate::core::messages::{ClientMessage, StationId, SystemId};
use crate::lobby::{InboundMessage, LobbyPlugin};
use crate::ship::control_source::ControlSource;
use bevy::ecs::system::RunSystemOnce;

#[path = "logistics_activity_tests.rs"]
mod activities;

#[test]
fn an_unmanaged_magazine_returns_a_correlated_refusal() {
    let mut world = World::new();
    world.insert_resource(Messages::<crate::lobby::OutboundMessage>::default());
    let correlation = crate::core::messages::ActionCorrelationId::new("unsupported-stock").unwrap();
    world.spawn(AdmittedCommands(vec![
        crate::core::messages::AdmittedCommand {
            target: SystemId("torpedo-magazine".into()),
            payload: Control::SetStockTarget {
                store: "magazine".into(),
                amount: 4,
            },
            response_token: Some("gunner".into()),
            feedback_correlation: Some(correlation.clone()),
        },
    ]));
    world
        .run_system_once(refuse_unmanaged_stock_controls)
        .unwrap();
    let messages = world.resource::<Messages<crate::lobby::OutboundMessage>>();
    let mut cursor = messages.get_cursor();
    let replies: Vec<_> = cursor.read(messages).collect();
    assert_eq!(replies.len(), 1);
    assert!(
        matches!(&replies[0].msg, crate::core::messages::ServerMessage::ActionFeedback {
        correlation: actual, outcome: crate::core::messages::ActionFeedbackOutcome::Refused
    } if actual == &correlation)
    );
    assert_eq!(
        replies[0].target,
        crate::lobby::Target::Token("gunner".into())
    );
}

pub(crate) fn fixture(source: ControlSource) -> (App, Entity) {
    fixture_with(source, |_| {})
}
pub(crate) fn fixture_with(
    source: ControlSource,
    edit: impl FnOnce(&mut crate::entities::config::EntityConfig),
) -> (App, Entity) {
    let mut content = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    // Keep this authority/route fixture's two-unit batch bound explicit while
    // shipped local-store policy evolves independently.
    content.logistics.as_mut().unwrap().handling[0].max_batch = 2;
    content.logistics.as_mut().unwrap().handling[0].reorder_fraction = 1.0;
    edit(&mut content);
    let profile = crate::resources::logistics_profile::compile_entity_logistics(&content)
        .unwrap()
        .unwrap();
    let state = LogisticsState::new(&profile);
    let production =
        crate::resources::production_profile::compile_entity_production(&content).unwrap();
    let ship = content.ship_config.clone().unwrap();
    let (programs, trees) = crate::ai::state_tree_runtime::compile_ship_trees_with_energy(
        &ship,
        content.energy.as_ref(),
    )
    .unwrap();
    let mut sources = ShipSystemControlSources::default();
    if let Some(profile) = &production {
        sources.0.set(profile.config().system.clone(), source);
    }
    sources.0.set(state.owner.clone(), source);
    sources
        .0
        .set(SystemId("torpedo-magazine".into()), ControlSource::Human);
    let mut app = App::new();
    app.add_plugins((
        LobbyPlugin,
        bevy::time::TimePlugin,
        crate::server_app::AdmissionPlugin,
    ))
    .configure_sets(
        FixedUpdate,
        (
            crate::sim_sets::SimSet::Input,
            crate::sim_sets::SimSet::Physics,
            crate::sim_sets::SimSet::Modifiers,
            crate::sim_sets::SimSet::Publish,
            crate::sim_sets::SimSet::Broadcast,
        )
            .chain(),
    );
    crate::sim_tick::register_sim_tick(&mut app);
    crate::ai::cadence::register_ai_cadence(&mut app);
    register(&mut app);
    crate::resources::production_server::register(&mut app);
    app.add_systems(
        FixedUpdate,
        crate::console::weapons::blackboard::publish_torpedo_magazine_blackboard
            .in_set(crate::sim_sets::SimSet::Publish),
    );
    crate::ship::test_support::drive_one_fixed_step_per_update(
        &mut app,
        std::time::Duration::from_millis(200),
    );
    for (token, station) in [("engineer", "engineering"), ("gunner", "tactical")] {
        let mut sessions = app.world_mut().resource_mut::<crate::lobby::Sessions>();
        sessions.0.register(token.into(), token.into()).unwrap();
        sessions
            .0
            .set_station(token, Some(StationId(station.into())));
    }
    let torpedoes = content.torpedoes.as_ref().unwrap();
    let entity = app
        .world_mut()
        .spawn((
            crate::server_app::Ship,
            crate::server_app::LocalShip,
            crate::entities::spawner::EntityUuid("logistics-fixture".into()),
            CompiledShipLogistics(profile),
            ShipLogistics(state),
            ShipConfigComponent(ship.clone()),
            sources,
            programs,
            trees,
            AdmittedCommands::default(),
            crate::server_app::ShipSystemBlackboards::default(),
            InfrastructureCondition(crate::infrastructure::InfrastructureState::from_config(
                content.infrastructure.as_ref().unwrap(),
            )),
            TorpedoSystemResource(crate::weapons::torpedo::TorpedoSystem::from_configs(
                &torpedoes.tubes,
                torpedoes.to_runtime(),
            )),
            crate::ship::power::ShipPowerSystem(
                crate::modifiers::power_system::PowerSystem::from_energy(
                    content.energy.unwrap(),
                    ship,
                )
                .unwrap(),
            ),
        ))
        .id();
    if let Some(profile) = production {
        let state = crate::resources::production::ProductionState::new(&profile);
        app.world_mut().entity_mut(entity).insert((
            crate::resources::production_server::CompiledShipProduction(profile),
            crate::resources::production_server::ShipProduction(state),
        ));
    }
    (app, entity)
}
pub(crate) fn send(app: &mut App, token: &str, target: &str, payload: Control) {
    app.world_mut()
        .resource_mut::<Messages<InboundMessage>>()
        .write(InboundMessage {
            token: token.into(),
            msg: ClientMessage::ControlSystem {
                target: SystemId(target.into()),
                payload,
            },
        });
}
fn quantity(app: &App, entity: Entity) -> (i64, i64, i64) {
    let hold = app
        .world()
        .get::<InfrastructureCondition>(entity)
        .unwrap()
        .0
        .capacity_stock("hold-ammunition")
        .unwrap()
        .level;
    let magazine = app
        .world()
        .get::<TorpedoSystemResource>(entity)
        .unwrap()
        .0
        .magazine
        .level;
    let transit = app
        .world()
        .get::<ShipLogistics>(entity)
        .unwrap()
        .0
        .batches
        .values()
        .map(|batch| batch.amount)
        .sum();
    (hold, magazine, transit)
}

#[test]
fn typed_stock_feed_grants_information_but_local_targets_keep_their_operator() {
    let (mut app, entity) = fixture(ControlSource::Human);
    send(
        &mut app,
        "engineer",
        "logistics",
        Control::SetLogisticsEnabled { enabled: false },
    );
    send(
        &mut app,
        "engineer",
        "logistics",
        Control::SetStockTarget {
            store: "hold-ammunition".into(),
            amount: 3,
        },
    );
    send(
        &mut app,
        "engineer",
        "torpedo-magazine",
        Control::SetStockTarget {
            store: "magazine".into(),
            amount: 4,
        },
    );
    app.update();
    let state = &app.world().get::<ShipLogistics>(entity).unwrap().0;
    assert_eq!(state.goals["hold-ammunition"], 3);
    assert_eq!(state.goals["magazine"], 6);
    send(
        &mut app,
        "gunner",
        "torpedo-magazine",
        Control::SetStockTarget {
            store: "magazine".into(),
            amount: 4,
        },
    );
    app.update();
    assert_eq!(
        app.world().get::<ShipLogistics>(entity).unwrap().0.goals["magazine"],
        4
    );
    assert_eq!(quantity(&app, entity), (4, 2, 0));
}

#[test]
fn a_recalled_delivery_keeps_real_stock_until_its_return_job_finishes() {
    let (mut app, entity) = fixture(ControlSource::Human);
    send(
        &mut app,
        "engineer",
        "logistics",
        Control::DispatchDelivery {
            source: "hold-ammunition".into(),
            destination: "magazine".into(),
            amount: 2,
        },
    );
    app.update();
    assert_eq!(quantity(&app, entity), (2, 2, 2));
    let id = app
        .world()
        .get::<ShipLogistics>(entity)
        .unwrap()
        .0
        .batches
        .keys()
        .next()
        .unwrap()
        .0;
    // Lower the standing demand as well as recalling this individual batch;
    // otherwise maintained stock correctly starts another independent delivery.
    send(
        &mut app,
        "gunner",
        "torpedo-magazine",
        Control::SetStockTarget {
            store: "magazine".into(),
            amount: 2,
        },
    );
    send(
        &mut app,
        "engineer",
        "logistics",
        Control::ReturnDelivery { job: id },
    );
    app.update();
    assert_eq!(
        quantity(&app, entity),
        (2, 2, 2),
        "recall cannot teleport its cargo"
    );
    for _ in 0..100 {
        app.update();
        let q = quantity(&app, entity);
        assert_eq!(
            q.0 + q.1 + q.2,
            6,
            "each paid route step conserves all actual ammunition"
        );
        if q.2 == 0 {
            break;
        }
    }
    assert_eq!(quantity(&app, entity), (4, 2, 0));
}

#[test]
fn ordinary_logistics_ai_needs_the_explicit_foreign_stock_feed() {
    for source in [ControlSource::Ai, ControlSource::Human] {
        let (mut app, entity) = fixture(source);
        let magazine_feed = {
            let mut config = app
                .world_mut()
                .get_mut::<ShipConfigComponent>(entity)
                .unwrap();
            let console = config
                .0
                .consoles
                .iter_mut()
                .find(|console| console.id.0 == "engineering")
                .unwrap();
            let index = console
                .feeds
                .iter()
                .position(|feed| feed.source.0 == "stock/torpedo-magazine")
                .unwrap();
            console.feeds.remove(index)
        };
        for _ in 0..5 {
            app.update();
        }
        assert_eq!(
            quantity(&app, entity),
            (4, 2, 0),
            "co-hosting cannot supply the absent stock reading"
        );
        app.world_mut()
            .get_mut::<ShipConfigComponent>(entity)
            .unwrap()
            .0
            .consoles
            .iter_mut()
            .find(|console| console.id.0 == "engineering")
            .unwrap()
            .feeds
            .push(magazine_feed);
        for _ in 0..5 {
            app.update();
        }
        assert!(
            quantity(&app, entity).2 > 0,
            "an authorised controller needs the explicit feed to maintain stock"
        );
    }
}

#[test]
fn human_targets_maintain_several_batches_without_dispatch_presses() {
    let (mut app, entity) = fixture(ControlSource::Human);
    let mut jobs = std::collections::BTreeSet::new();
    for _ in 0..250 {
        app.update();
        let state = &app.world().get::<ShipLogistics>(entity).unwrap().0;
        jobs.extend(state.batches.keys().copied());
        let (hold, local, cargo) = quantity(&app, entity);
        assert_eq!(hold + local + cargo, 6);
        if hold == 0 && local == 6 && cargo == 0 {
            break;
        }
    }
    assert_eq!(quantity(&app, entity), (0, 6, 0));
    assert_eq!(
        jobs.len(),
        2,
        "one standing policy executes two bounded batches"
    );
}

#[test]
fn offline_pauses_existing_cargo_and_restoring_control_resumes_it() {
    let (mut app, entity) = fixture(ControlSource::Human);
    for _ in 0..5 {
        app.update();
    }
    assert!(quantity(&app, entity).2 > 0);
    let before = app
        .world()
        .get::<ShipLogistics>(entity)
        .unwrap()
        .0
        .batches
        .clone();
    let stock = quantity(&app, entity);
    app.world_mut()
        .get_mut::<ShipSystemControlSources>(entity)
        .unwrap()
        .0
        .set(SystemId("logistics".into()), ControlSource::Offline);
    for _ in 0..10 {
        app.update();
    }
    assert_eq!(quantity(&app, entity), stock);
    assert_eq!(
        serde_json::to_value(&app.world().get::<ShipLogistics>(entity).unwrap().0.batches).unwrap(),
        serde_json::to_value(&before).unwrap(),
        "offline machinery cannot advance paid work or release claims"
    );
    app.world_mut()
        .get_mut::<ShipSystemControlSources>(entity)
        .unwrap()
        .0
        .set(SystemId("logistics".into()), ControlSource::Human);
    for _ in 0..250 {
        app.update();
    }
    assert_eq!(quantity(&app, entity), (0, 6, 0));
}

#[test]
fn delegated_tree_can_withhold_delivery_despite_a_stock_deficit() {
    let (mut app, entity) = fixture(ControlSource::Ai);
    let definition = serde_json::from_value(serde_json::json!({"root":"logistics","tree":[{
        "id":"logistics","initial":"hold","state":[{"id":"hold","task":[{
            "id":"wait","action":{"kind":"write","channel":"dispatch","value":{"source":"literal","value":false}}
        }]}]
    }]})).unwrap();
    let program = phoenix_sim_gameplay::ai::state_tree::compile(
        &definition,
        &phoenix_sim_gameplay::resources::logistics_ai::bindings(),
    )
    .unwrap();
    app.world_mut()
        .get_mut::<crate::ai::state_tree_runtime::ShipAiTreePrograms>(entity)
        .unwrap()
        .0
        .insert(SystemId("logistics".into()), program);
    for _ in 0..20 {
        app.update();
    }
    assert_eq!(
        quantity(&app, entity),
        (4, 2, 0),
        "no maintenance controller may override the delegated tree"
    );
    let boards = &app
        .world()
        .get::<crate::server_app::ShipSystemBlackboards>(entity)
        .unwrap()
        .0;
    assert!(
        matches!(boards.get(&SystemId("logistics".into())), Some(SystemBlackboard::Logistics(board)) if board.next_delivery.is_some())
    );
}

#[test]
fn stock_publications_are_typed_feeds_and_never_confer_local_control() {
    let (mut app, entity) = fixture(ControlSource::Human);
    send(
        &mut app,
        "engineer",
        "logistics",
        Control::SetLogisticsEnabled { enabled: false },
    );
    send(
        &mut app,
        "gunner",
        "repair",
        Control::SetStockTarget {
            store: "repair-machinery".into(),
            amount: 4,
        },
    );
    send(
        &mut app,
        "gunner",
        "logistics",
        Control::SetStockPriority {
            store: "repair-machinery".into(),
            priority: 9,
        },
    );
    app.update();
    let state = &app.world().get::<ShipLogistics>(entity).unwrap().0;
    assert_eq!(state.goals["repair-machinery"], 12);
    assert_eq!(state.priorities["repair-machinery"], 1);
    let config = &app.world().get::<ShipConfigComponent>(entity).unwrap().0;
    let boards = &app
        .world()
        .get::<crate::server_app::ShipSystemBlackboards>(entity)
        .unwrap()
        .0;
    let publication = stock_publication(&SystemId("repair".into()));
    assert!(config
        .observations_for_system(&SystemId("logistics".into()), boards)
        .get(&publication)
        .is_some());
    assert!(config
        .observations_for_system(&SystemId("torpedo-magazine".into()), boards)
        .get(&publication)
        .is_none());
    send(
        &mut app,
        "engineer",
        "repair",
        Control::SetStockTarget {
            store: "repair-machinery".into(),
            amount: 4,
        },
    );
    send(
        &mut app,
        "engineer",
        "logistics",
        Control::SetStockPriority {
            store: "repair-machinery".into(),
            priority: 9,
        },
    );
    app.update();
    let state = &app.world().get::<ShipLogistics>(entity).unwrap().0;
    assert_eq!(state.goals["repair-machinery"], 4);
    assert_eq!(state.priorities["repair-machinery"], 9);
}

#[test]
fn local_capacity_demand_arrives_as_finite_routed_stock() {
    let (mut app, entity) = fixture(ControlSource::Human);
    let before = {
        let mut owner = app
            .world_mut()
            .get_mut::<InfrastructureCondition>(entity)
            .unwrap();
        let mut local = owner.0.capacity_stock("machinery_parts").unwrap().clone();
        assert_eq!(local.level, 12);
        local.level -= 4; // Actual consumer expenditure, never a fabricated target deficit.
        owner
            .0
            .replace_capacity_stock("machinery_parts", local)
            .unwrap();
        owner
            .0
            .capacity_stock("hold-machinery-parts")
            .unwrap()
            .level
            + 8
    };
    let mut saw_transit = false;
    for _ in 0..250 {
        app.update();
        let owner = &app
            .world()
            .get::<InfrastructureCondition>(entity)
            .unwrap()
            .0;
        let cargo: i64 = app
            .world()
            .get::<ShipLogistics>(entity)
            .unwrap()
            .0
            .batches
            .values()
            .filter(|batch| batch.original_destination == "repair-machinery")
            .map(|batch| batch.amount)
            .sum();
        saw_transit |= cargo > 0;
        assert_eq!(
            owner.capacity_stock("hold-machinery-parts").unwrap().level
                + owner.capacity_stock("machinery_parts").unwrap().level
                + cargo,
            before
        );
        if owner.capacity_stock("machinery_parts").unwrap().level == 12 && cargo == 0 {
            break;
        }
    }
    assert!(saw_transit);
    let owner = &app
        .world()
        .get::<InfrastructureCondition>(entity)
        .unwrap()
        .0;
    assert_eq!(owner.capacity_stock("machinery_parts").unwrap().level, 12);
    assert_eq!(
        owner.capacity_stock("hold-machinery-parts").unwrap().level,
        before - 12
    );
}

#[test]
fn a_full_high_priority_bay_does_not_starve_a_feasible_other_bay() {
    let (mut app, entity) = fixture_with(ControlSource::Human, |config| {
        let capacities = &mut config.infrastructure.as_mut().unwrap().capacities;
        capacities
            .iter_mut()
            .find(|c| c.id == "machinery_parts")
            .unwrap()
            .amount = 0;
        let coolant = capacities.iter_mut().find(|c| c.id == "coolant").unwrap();
        coolant.amount = 168;
        coolant.ceiling = Some(168);
    });
    for _ in 0..8 {
        app.update();
    }
    assert!(
        quantity(&app, entity).2 > 0,
        "the free Weapons bay must get its lower-priority ammunition delivery"
    );
    let boards = &app
        .world()
        .get::<crate::server_app::ShipSystemBlackboards>(entity)
        .unwrap()
        .0;
    let SystemBlackboard::Stock(repair) = &boards[&stock_publication(&SystemId("repair".into()))]
    else {
        panic!("typed stock report")
    };
    assert_eq!(
        repair
            .stores
            .iter()
            .find(|s| s.id == "repair-machinery")
            .unwrap()
            .headroom,
        0
    );
    assert!(app
        .world()
        .get::<ShipLogistics>(entity)
        .unwrap()
        .0
        .batches
        .values()
        .all(|batch| batch.destination == "magazine"));
}

#[test]
fn strict_stock_feeds_require_an_actual_local_store_producer() {
    let mut config = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    assert!(
        crate::resources::logistics_profile::compile_entity_logistics(&config)
            .unwrap()
            .is_some()
    );
    config.ship_config.as_mut().unwrap().consoles[0].feeds.push(
        phoenix_model::console::ConsoleFeed {
            source: stock_publication(&SystemId("sensors".into())),
            kind: phoenix_model::console::ConsoleObservationKind::Stock,
        },
    );
    assert!(
        crate::resources::logistics_profile::compile_entity_logistics(&config)
            .unwrap_err()
            .contains("configured managed local store owner")
    );
}

#[test]
fn damaged_local_tools_keep_stock_orders_but_explicit_offline_does_not() {
    let (mut app, entity) = fixture(ControlSource::Human);
    let repair = SystemId("repair".into());
    {
        let mut sources = app
            .world_mut()
            .get_mut::<ShipSystemControlSources>(entity)
            .unwrap();
        sources.0.set(repair.clone(), ControlSource::Human);
        sources.0.set_offline(repair.clone(), true);
    }
    send(
        &mut app,
        "engineer",
        "repair",
        Control::SetStockTarget {
            store: "repair-machinery".into(),
            amount: 4,
        },
    );
    app.update();
    assert_eq!(
        app.world().get::<ShipLogistics>(entity).unwrap().0.goals["repair-machinery"],
        4
    );
    app.world_mut()
        .get_mut::<ShipSystemControlSources>(entity)
        .unwrap()
        .0
        .set(repair, ControlSource::Offline);
    send(
        &mut app,
        "engineer",
        "repair",
        Control::SetStockTarget {
            store: "repair-machinery".into(),
            amount: 6,
        },
    );
    app.update();
    assert_eq!(
        app.world().get::<ShipLogistics>(entity).unwrap().0.goals["repair-machinery"],
        4
    );
}

#[path = "logistics_planning_tests.rs"]
mod planning;
