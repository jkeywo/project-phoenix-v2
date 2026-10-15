//! The ordinary Power System tree sees permitted observations and emits only
//! admitted machinery commands. Physical routing is covered by ship/power tests.
use super::*;
use crate::ai::state_tree_runtime::{
    compile_ship_trees_with_energy, ShipAiTreePrograms, ShipAiTreeRuntime,
};
use crate::core::messages::{SystemBlackboard, SystemControlPayload, SystemId};
use crate::entities::config::EntityConfig;
use crate::modifiers::power_system::{PowerState, PowerSystem};
use crate::ship::power::ShipPowerSystem;
use bevy::ecs::system::RunSystemOnce;

fn clear_admitted_each_tick(mut queues: Query<&mut AdmittedCommands>) {
    for mut queue in &mut queues {
        queue.0.clear();
    }
}
fn advance_fixture_tick(mut tick: ResMut<crate::sim_tick::SimTick>) {
    tick.0 += 1;
}

fn content() -> EntityConfig {
    crate::entities::include_resolve::load_entity_config("assets/entities/alliance_cruiser.toml")
        .unwrap()
}
fn reactor() -> SystemId {
    crate::ship::system_registry::power_reactor_system_id()
}
fn app_for(mut content: EntityConfig, memory_tree: bool) -> (App, Entity) {
    let hardware = content.energy.clone().unwrap();
    let ship = content.ship_config.as_mut().unwrap();
    if memory_tree {
        let definition: phoenix_sim_gameplay::ai::state_tree::AiTreeConfig = serde_json::from_value(serde_json::json!({
            "root":"power", "tree":[{"id":"power", "initial":"balance", "memory":{"rationing":false}, "state":[{
                "id":"balance", "task":[
                    {"id":"normal", "action":{"kind":"write_number", "channel":"reactor-output", "value":hardware.reactor.sustainable_output_per_sec}},
                    {"id":"notice", "when":[{"value":{"source":"number_compare", "name":"reactor_reserve_fraction", "comparison":"less", "value":0.5}, "equals":true}],
                     "action":{"kind":"remember", "key":"rationing", "value":{"source":"literal", "value":true}}},
                    {"id":"ration", "when":[{"value":{"source":"memory", "name":"rationing"}, "equals":true}],
                     "action":{"kind":"write_number", "channel":"reactor-output", "value":hardware.reactor.sustainable_output_per_sec * 0.5}}
                ]
            }]}]
        })).unwrap();
        ship.systems
            .iter_mut()
            .find(|system| system.id == reactor())
            .unwrap()
            .config
            .as_mut()
            .unwrap()
            .as_table_mut()
            .unwrap()
            .insert("ai".into(), toml::Value::try_from(definition).unwrap());
    }
    let ship = content.ship_config.unwrap();
    let mut power = PowerSystem::from_energy(hardware.clone(), ship.clone()).unwrap();
    if !memory_tree {
        // The stock tree's allocation selection is already settled for tests
        // isolating later flow and knowledge changes. Explicit mode crossings
        // are exercised below from both canonical starting modes.
        power.set_mode(phoenix_model::energy::EnergyAllocationMode::Minimal);
    }
    let (programs, runtimes) = compile_ship_trees_with_energy(&ship, Some(&hardware)).unwrap();
    let mut sources = ShipSystemControlSources::default();
    sources.0.set(reactor(), ControlSource::Ai);
    let mut app = App::new();
    crate::ai::host::register_ai_host_env(&mut app);
    app.insert_resource(crate::sim_tick::SimTick::default())
        .add_systems(
            Update,
            (
                clear_admitted_each_tick,
                ai_power_allocation,
                crate::ship::power::handle_power_messages,
                crate::ship::power::publish_power_blackboard,
            )
                .chain(),
        )
        .add_systems(Last, advance_fixture_tick);
    let entity = app
        .world_mut()
        .spawn((
            crate::server_app::Ship,
            crate::entities::spawner::EntityUuid("power-ai-fixture".into()),
            ShipPowerSystem(power),
            ShipConfigComponent(ship),
            sources,
            AdmittedCommands::default(),
            crate::server_app::ShipSystemBlackboards::default(),
            programs,
            runtimes,
            crate::resources::server::ResourceServices(content.resource_services),
            crate::infrastructure::InfrastructureCondition(
                crate::infrastructure::InfrastructureState::from_config(
                    content.infrastructure.as_ref().unwrap(),
                ),
            ),
        ))
        .id();
    // Seed one real publication; decisions on the following tick read the same
    // frozen view that a human Console receives. Publishers run after the applier.
    app.world_mut()
        .run_system_once(crate::ship::power::publish_power_blackboard)
        .unwrap();
    (app, entity)
}
fn fixture() -> (App, Entity) {
    app_for(content(), false)
}
fn content_with_allocation(priority: Option<bool>) -> EntityConfig {
    let mut content = content();
    let config = content
        .ship_config
        .as_mut()
        .unwrap()
        .systems
        .iter_mut()
        .find(|system| system.id == reactor())
        .unwrap()
        .config
        .as_mut()
        .unwrap();
    let tasks = config
        .get_mut("ai")
        .unwrap()
        .get_mut("tree")
        .unwrap()
        .as_array_mut()
        .unwrap()[0]
        .get_mut("state")
        .unwrap()
        .as_array_mut()
        .unwrap()[0]
        .get_mut("task")
        .unwrap()
        .as_array_mut()
        .unwrap();
    tasks.retain(|task| task.get("id").and_then(toml::Value::as_str) != Some("allocation"));
    if let Some(priority) = priority {
        tasks.insert(0, toml::Value::try_from(serde_json::json!({
            "id":"allocation", "action":{"kind":"write", "channel":"priority-allocation", "value":{"source":"literal", "value":priority}}
        })).unwrap());
    }
    content
}

#[test]
fn allocation_mode_is_optional_and_uses_the_same_admitted_power_owner() {
    use phoenix_model::energy::EnergyAllocationMode::{Manual, Minimal};
    for (output, initial, expected) in [
        (Some(true), Manual, Minimal),
        (Some(false), Minimal, Manual),
        (None, Minimal, Minimal),
    ] {
        let (mut app, entity) = app_for(content_with_allocation(output), false);
        app.world_mut()
            .get_mut::<ShipPowerSystem>(entity)
            .unwrap()
            .0
            .set_mode(initial);
        app.world_mut()
            .run_system_once(crate::ship::power::publish_power_blackboard)
            .unwrap();
        app.update();
        assert_eq!(power(&app, entity).network().mode, expected);
        let mode_commands: Vec<_> = commands(&app, entity)
            .into_iter()
            .filter(|command| matches!(command, SystemControlPayload::SetEnergyMode { .. }))
            .collect();
        assert_eq!(
            mode_commands,
            if initial == expected {
                vec![]
            } else {
                vec![SystemControlPayload::SetEnergyMode { mode: expected }]
            }
        );
        app.update();
        assert!(
            commands(&app, entity).is_empty(),
            "unchanged allocation is not reissued"
        );
    }
}
fn power(app: &App, entity: Entity) -> &PowerSystem {
    &app.world().get::<ShipPowerSystem>(entity).unwrap().0
}
fn energy_mut(
    app: &mut App,
    entity: Entity,
    update: impl FnOnce(&mut phoenix_model::energy::PowerEnergyBlackboard),
) {
    let mut boards = app
        .world_mut()
        .get_mut::<crate::server_app::ShipSystemBlackboards>(entity)
        .unwrap();
    let SystemBlackboard::Power(board) = boards.0.get_mut(&SystemId("power".into())).unwrap()
    else {
        panic!("Power publication")
    };
    update(board.energy.as_mut().unwrap());
}
fn commands(app: &App, entity: Entity) -> Vec<SystemControlPayload> {
    app.world()
        .get::<AdmittedCommands>(entity)
        .unwrap()
        .0
        .iter()
        .map(|command| {
            assert_eq!(command.target, reactor());
            assert_eq!(
                command.response_token.as_deref(),
                Some(
                    crate::command_admission::ai_emit::ai_token_for(
                        app.world()
                            .get::<crate::entities::spawner::EntityUuid>(entity)
                    )
                    .as_str()
                )
            );
            assert!(command.feedback_correlation.is_none());
            command.payload.clone()
        })
        .collect()
}
fn runtime(app: &App, entity: Entity) -> &ShipAiTreeRuntime {
    app.world().get::<ShipAiTreeRuntime>(entity).unwrap()
}
fn fine_consumer(app: &App, entity: Entity) -> (SystemId, crate::core::messages::PowerGroupId) {
    let config = power(app, entity);
    let consumer = config.config().consumers.first().unwrap().system.clone();
    let group = config
        .ship_config()
        .system(&consumer)
        .unwrap()
        .power_group
        .clone()
        .unwrap();
    (consumer, group)
}
fn has_flow(
    commands: &[SystemControlPayload],
    group: &crate::core::messages::PowerGroupId,
    flow: f32,
) -> bool {
    commands.iter().any(|command| matches!(command, SystemControlPayload::SetEnergyGroup { group: id, flow_per_sec, .. } if id == group && *flow_per_sec == flow))
}

#[test]
fn power_tree_operates_without_high_fidelity_or_a_ship_tag_and_suppresses_no_ops() {
    let (mut app, entity) = fixture();
    app.world_mut()
        .entity_mut(entity)
        .remove::<crate::server_app::Ship>();
    assert!(!app.world().entity(entity).contains::<AiHighFidelity>());
    app.update();
    assert!(commands(&app, entity).is_empty());
    assert_eq!(runtime(&app, entity).0[&reactor()].last_tick, Some(0));
    let (_, group) = fine_consumer(&app, entity);
    app.world_mut()
        .get_mut::<ShipPowerSystem>(entity)
        .unwrap()
        .0
        .set_flow(&group, 0.0)
        .unwrap();
    app.world_mut()
        .run_system_once(crate::ship::power::publish_power_blackboard)
        .unwrap();
    app.update();
    let desired = power(&app, entity)
        .config()
        .groups
        .iter()
        .find(|row| row.id == group)
        .unwrap()
        .default_flow_per_sec;
    assert!(has_flow(&commands(&app, entity), &group, desired));
    assert_eq!(power(&app, entity).flow_for(&group), Some(desired));
    app.update();
    assert!(
        commands(&app, entity).is_empty(),
        "unchanged settings are not emitted every tick"
    );
}

#[test]
fn human_and_offline_reactors_do_not_evaluate_or_emit_until_ordinary_ai_resumes() {
    for offline in [false, true] {
        let (mut app, entity) = fixture();
        let (_, group) = fine_consumer(&app, entity);
        energy_mut(&mut app, entity, |view| {
            for consumer in &mut view.consumers {
                consumer.energy = 0.0;
            }
        });
        {
            let mut sources = app
                .world_mut()
                .get_mut::<ShipSystemControlSources>(entity)
                .unwrap();
            if offline {
                sources.0.set_offline(reactor(), true);
            } else {
                sources.0.set(reactor(), ControlSource::Human);
            }
        }
        let before = runtime(&app, entity).clone();
        app.update();
        assert!(commands(&app, entity).is_empty());
        assert_eq!(runtime(&app, entity), &before);
        {
            let mut sources = app
                .world_mut()
                .get_mut::<ShipSystemControlSources>(entity)
                .unwrap();
            sources.0.set_offline(reactor(), false);
            sources.0.set(reactor(), ControlSource::Ai);
        }
        energy_mut(&mut app, entity, |view| {
            for consumer in &mut view.consumers {
                consumer.energy = 0.0;
            }
        });
        app.update();
        let maximum = power(&app, entity)
            .config()
            .groups
            .iter()
            .find(|row| row.id == group)
            .unwrap()
            .max_flow_per_sec;
        assert!(has_flow(&commands(&app, entity), &group, maximum));
        assert!(runtime(&app, entity).0[&reactor()].last_tick.is_some());
    }
}

#[test]
fn missing_program_or_private_runtime_does_not_activate_a_fallback_controller() {
    for missing_program in [true, false] {
        let (mut app, entity) = fixture();
        energy_mut(&mut app, entity, |view| {
            for consumer in &mut view.consumers {
                consumer.energy = 0.0;
            }
        });
        if missing_program {
            app.world_mut()
                .get_mut::<ShipAiTreePrograms>(entity)
                .unwrap()
                .0
                .remove(&reactor());
        } else {
            app.world_mut()
                .get_mut::<ShipAiTreeRuntime>(entity)
                .unwrap()
                .0
                .remove(&reactor());
        }
        let before = runtime(&app, entity).clone();
        app.update();
        assert!(commands(&app, entity).is_empty());
        assert_eq!(runtime(&app, entity), &before);
    }
}

#[test]
fn tree_reads_the_permitted_publication_instead_of_hidden_live_reserve_state() {
    let (mut app, entity) = fixture();
    let (consumer, group) = fine_consumer(&app, entity);
    app.world_mut()
        .get_mut::<ShipPowerSystem>(entity)
        .unwrap()
        .0
        .network_mut()
        .consumers
        .get_mut(&consumer)
        .unwrap()
        .energy = 0.0;
    // The published view still reports its previous healthy reserve.
    app.update();
    assert!(commands(&app, entity).is_empty());
    // The ordinary publisher has now exposed the deficit. No live-state read
    // can explain why the first decision held and the second responds.
    app.update();
    let maximum = power(&app, entity)
        .config()
        .groups
        .iter()
        .find(|row| row.id == group)
        .unwrap()
        .max_flow_per_sec;
    assert!(has_flow(&commands(&app, entity), &group, maximum));
}

#[test]
fn another_cohosted_console_cannot_supply_the_power_ais_missing_feed() {
    let (mut app, entity) = fixture();
    energy_mut(&mut app, entity, |view| {
        for consumer in &mut view.consumers {
            consumer.energy = 0.0;
        }
    });
    {
        let mut config = app
            .world_mut()
            .get_mut::<ShipConfigComponent>(entity)
            .unwrap();
        let owner = config
            .0
            .system(&reactor())
            .unwrap()
            .console
            .clone()
            .unwrap();
        let index = config
            .0
            .consoles
            .iter()
            .position(|console| console.id == owner)
            .unwrap();
        let mut isolated = config.0.consoles[index].clone();
        isolated.id = crate::core::messages::ConsoleId("isolated-reactor".into());
        isolated
            .feeds
            .retain(|feed| feed.kind != phoenix_model::console::ConsoleObservationKind::Power);
        config
            .0
            .systems
            .iter_mut()
            .find(|system| system.id == reactor())
            .unwrap()
            .console = Some(isolated.id.clone());
        // The existing co-hosted engineering Console retains both its typed
        // feed and ownership of the `power` aggregate alias. Neither is pooled.
        config.0.consoles.push(isolated);
    }
    let before = runtime(&app, entity).clone();
    app.update();
    assert!(commands(&app, entity).is_empty());
    assert_eq!(runtime(&app, entity), &before);
}

#[test]
fn ordinary_authored_tree_recovers_fine_reserves_and_cools_hot_ancestors() {
    let (mut app, entity) = fixture();
    let (consumer, group) = fine_consumer(&app, entity);
    let mut ancestors = Vec::new();
    {
        let config = power(&app, entity).config();
        let mut cursor = Some(group.clone());
        while let Some(id) = cursor {
            ancestors.push(id.clone());
            cursor = config
                .groups
                .iter()
                .find(|row| row.id == id)
                .unwrap()
                .parent
                .clone();
        }
    }
    energy_mut(&mut app, entity, |view| {
        let own = view
            .consumers
            .iter_mut()
            .find(|row| row.system == consumer)
            .unwrap();
        own.energy = 0.0;
        own.heat = own.trip_heat * 0.75;
    });
    app.update();
    let emitted = commands(&app, entity);
    for id in ancestors {
        let config = power(&app, entity)
            .config()
            .groups
            .iter()
            .find(|row| row.id == id)
            .unwrap();
        assert!(has_flow(&emitted, &id, config.max_flow_per_sec));
        assert!(emitted.iter().any(|command| matches!(command,
            SystemControlPayload::SetEnergyGroup { group, cooling_per_sec, .. } if group == &id && *cooling_per_sec == config.max_cooling_per_sec)));
    }
}

#[test]
fn power_private_memory_survives_serialization_and_human_handoff() {
    let (mut app, entity) = app_for(content(), true);
    energy_mut(&mut app, entity, |view| view.reactor.energy = 0.0);
    app.update();
    assert_eq!(
        runtime(&app, entity).0[&reactor()].memory("rationing"),
        Some(true)
    );
    let private: ShipAiTreeRuntime =
        serde_json::from_slice(&serde_json::to_vec(runtime(&app, entity)).unwrap()).unwrap();
    let power_state: PowerState = power(&app, entity).capture_continuation();
    let (mut resumed, restored) = app_for(content(), true);
    resumed
        .world_mut()
        .entity_mut(restored)
        .insert(private.clone());
    resumed
        .world_mut()
        .get_mut::<ShipPowerSystem>(restored)
        .unwrap()
        .0
        .restore_continuation(&power_state)
        .unwrap();
    let tick = app.world().resource::<crate::sim_tick::SimTick>().0;
    resumed
        .world_mut()
        .resource_mut::<crate::sim_tick::SimTick>()
        .0 = tick;
    resumed
        .world_mut()
        .run_system_once(crate::ship::power::publish_power_blackboard)
        .unwrap();
    for _ in 0..3 {
        app.update();
        resumed.update();
        assert_eq!(commands(&app, entity), commands(&resumed, restored));
        assert_eq!(runtime(&app, entity), runtime(&resumed, restored));
        assert_eq!(
            power(&app, entity).capture_continuation(),
            power(&resumed, restored).capture_continuation()
        );
    }
    app.world_mut()
        .get_mut::<ShipSystemControlSources>(entity)
        .unwrap()
        .0
        .set(reactor(), ControlSource::Human);
    let held = runtime(&app, entity).clone();
    app.update();
    assert_eq!(runtime(&app, entity), &held);
    app.world_mut()
        .get_mut::<ShipSystemControlSources>(entity)
        .unwrap()
        .0
        .set(reactor(), ControlSource::Ai);
    app.update();
    assert_eq!(
        runtime(&app, entity).0[&reactor()].memory("rationing"),
        Some(true)
    );
}

#[test]
fn equal_peers_emit_identical_admitted_commands_with_independent_tree_memory() {
    let (mut a, ea) = app_for(content(), true);
    let (mut b, eb) = app_for(content(), true);
    energy_mut(&mut a, ea, |view| view.reactor.energy = 0.0);
    energy_mut(&mut b, eb, |view| view.reactor.energy = 0.0);
    a.update();
    b.update();
    assert_eq!(commands(&a, ea), commands(&b, eb));
    assert_eq!(runtime(&a, ea), runtime(&b, eb));
    let (mut healthy, eh) = app_for(content(), true);
    healthy.update();
    assert_eq!(
        runtime(&healthy, eh).0[&reactor()].memory("rationing"),
        Some(false)
    );
    assert_eq!(
        runtime(&a, ea).0[&reactor()].memory("rationing"),
        Some(true)
    );
    assert_ne!(
        power(&a, ea).network().output_target_per_sec,
        power(&healthy, eh).network().output_target_per_sec
    );
    // Coexisting ships use the same SystemId/program but own separate frames.
    let peer = a
        .world_mut()
        .spawn((
            crate::server_app::Ship,
            crate::entities::spawner::EntityUuid("second-power-owner".into()),
            healthy.world().get::<ShipPowerSystem>(eh).unwrap().clone(),
            healthy
                .world()
                .get::<ShipConfigComponent>(eh)
                .unwrap()
                .clone(),
            healthy
                .world()
                .get::<ShipSystemControlSources>(eh)
                .unwrap()
                .clone(),
            healthy
                .world()
                .get::<crate::server_app::ShipSystemBlackboards>(eh)
                .unwrap()
                .clone(),
            healthy
                .world()
                .get::<ShipAiTreePrograms>(eh)
                .unwrap()
                .clone(),
            healthy
                .world()
                .get::<ShipAiTreeRuntime>(eh)
                .unwrap()
                .clone(),
            crate::resources::server::ResourceServices(
                healthy
                    .world()
                    .get::<crate::resources::server::ResourceServices>(eh)
                    .unwrap()
                    .0
                    .clone(),
            ),
            crate::infrastructure::InfrastructureCondition(
                healthy
                    .world()
                    .get::<crate::infrastructure::InfrastructureCondition>(eh)
                    .unwrap()
                    .0
                    .clone(),
            ),
            AdmittedCommands::default(),
        ))
        .id();
    a.update();
    assert_eq!(
        runtime(&a, ea).0[&reactor()].memory("rationing"),
        Some(true)
    );
    assert_eq!(
        runtime(&a, peer).0[&reactor()].memory("rationing"),
        Some(false)
    );
}

#[test]
fn dynasty_generation_cycle_reads_permitted_strike_observation_and_uses_ordinary_admission() {
    let content = crate::entities::include_resolve::load_entity_config(
        "assets/entities/dynasty_player_cruiser.toml",
    )
    .unwrap();
    let (mut app, entity) = app_for(content, false);
    assert!(power(&app, entity).network().generation_enabled);
    {
        let mut boards = app
            .world_mut()
            .get_mut::<crate::server_app::ShipSystemBlackboards>(entity)
            .unwrap();
        let SystemBlackboard::Power(board) = boards.0.get_mut(&SystemId("power".into())).unwrap()
        else {
            panic!("permitted power view")
        };
        board.strike_reserve = Some(crate::core::messages::StrikeReserveBlackboard {
            charge: 60.0,
            capacity: 95.1,
            charging: false,
            enabled: true,
            depleted: false,
        });
    }
    app.update();
    assert!(!power(&app, entity).network().generation_enabled);
    assert!(commands(&app, entity)
        .contains(&SystemControlPayload::SetEnergyGeneration { enabled: false }));
    // The next ordinary publication has no enabled strike. No live weapon or
    // global state is borrowed to retain the previous Console observation.
    app.update();
    assert!(power(&app, entity).network().generation_enabled);
    assert!(commands(&app, entity)
        .contains(&SystemControlPayload::SetEnergyGeneration { enabled: true }));
    app.update();
    assert!(!commands(&app, entity)
        .iter()
        .any(|command| matches!(command, SystemControlPayload::SetEnergyGeneration { .. })));
}
