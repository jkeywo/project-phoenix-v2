use super::*;
use crate::ship::control_source::ControlSource;
use crate::ship::power::ShipPowerSystem;
use crate::ship::shields::ShipShields;
use crate::ship_plugin::{ShipConfigComponent, ShipSystemControlSources};
use bevy::ecs::system::RunSystemOnce;

fn fixture() -> (App, Entity, SystemId, SystemId) {
    let content = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    let hardware = content.energy.unwrap();
    let mut ship = content.ship_config.unwrap();
    let donor = crate::ship::system_registry::shield_arc_system_id("aft").unwrap();
    let target = crate::ship::system_registry::shield_arc_system_id("fore").unwrap();
    let scope = phoenix_model::identity::ConsoleId("private-shield-test".into());
    let mut private_console = ship.console_for_system(&donor).unwrap().clone();
    private_console.id = scope.clone();
    private_console.feeds = vec![phoenix_model::console::ConsoleFeed {
        source: SystemId("shields".into()),
        kind: phoenix_model::console::ConsoleObservationKind::Shields,
    }];
    ship.systems
        .iter_mut()
        .find(|system| system.id == donor)
        .unwrap()
        .console = Some(scope.clone());
    ship.consoles.push(private_console);
    let (programs, runtimes) =
        crate::ai::state_tree_runtime::compile_ship_trees_with_energy(&ship, Some(&hardware))
            .unwrap();
    let mut power =
        crate::modifiers::power_system::PowerSystem::from_energy(hardware, ship.clone()).unwrap();
    power
        .network_mut()
        .consumers
        .get_mut(&target)
        .unwrap()
        .energy = 0.0;
    let base = content.shields_console.unwrap().base.unwrap().to_runtime();
    let arcs: Vec<_> = content
        .shield_arcs
        .iter()
        .map(|arc| arc.to_runtime())
        .collect();
    let mut shields = crate::weapons::shield::ShieldSystem::from_arcs(&arcs, &base);
    shields
        .facings
        .iter_mut()
        .find(|facing| facing.id == "fore")
        .unwrap()
        .hp = 10;
    let mut sources = ShipSystemControlSources::default();
    sources.0.set(donor.clone(), ControlSource::Ai);
    let mut app = App::new();
    crate::ai::host::register_ai_host_env(&mut app);
    app.insert_resource(crate::sim_tick::SimTick::default());
    // No Ship marker, global target state or second controller is required.
    let entity = app
        .world_mut()
        .spawn((
            crate::entities::spawner::EntityUuid("shield-tree-fixture".into()),
            ShipConfigComponent(ship),
            sources,
            ShipPowerSystem(power),
            ShipShields(shields, 0.5),
            crate::server_app::ShipSystemBlackboards::default(),
            crate::core::messages::AdmittedCommands::default(),
            programs,
            runtimes,
        ))
        .id();
    publish(&mut app, entity);
    (app, entity, donor, target)
}

fn publish(app: &mut App, entity: Entity) {
    app.world_mut()
        .run_system_once(crate::ship::shields::publish_shields_blackboard)
        .unwrap();
    let mut boards = app
        .world_mut()
        .get_mut::<crate::server_app::ShipSystemBlackboards>(entity)
        .unwrap();
    let SystemBlackboard::Shields(reading) = boards.0.get_mut(&SystemId("shields".into())).unwrap()
    else {
        panic!("aggregate")
    };
    reading.threat_bearing = Some(0.0);
}

fn decide(app: &mut App, entity: Entity) -> Vec<crate::core::messages::AdmittedCommand> {
    app.world_mut()
        .get_mut::<crate::core::messages::AdmittedCommands>(entity)
        .unwrap()
        .0
        .clear();
    app.world_mut().run_system_once(ai_shields).unwrap();
    app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 += 1;
    app.world()
        .get::<crate::core::messages::AdmittedCommands>(entity)
        .unwrap()
        .0
        .clone()
}

#[test]
fn recovery_requires_a_permitted_fine_feed_and_never_grants_recipient_control() {
    let (mut app, entity, donor, target) = fixture();
    assert!(!decide(&mut app, entity).iter().any(|command| matches!(
        &command.payload,
        Control::SetShieldRecovery { target: Some(_) }
    )));
    {
        let mut config = app
            .world_mut()
            .get_mut::<ShipConfigComponent>(entity)
            .unwrap();
        config
            .0
            .consoles
            .iter_mut()
            .find(|console| console.id.0 == "private-shield-test")
            .unwrap()
            .feeds
            .push(phoenix_model::console::ConsoleFeed {
                source: target.clone(),
                kind: phoenix_model::console::ConsoleObservationKind::ShieldArc,
            });
    }
    let commands = decide(&mut app, entity);
    assert!(commands.iter().all(|command| command.target == donor));
    assert!(commands.iter().any(|command| command.payload
        == Control::SetShieldRecovery {
            target: Some(target.clone())
        }));
    assert_eq!(
        app.world()
            .get::<ShipSystemControlSources>(entity)
            .unwrap()
            .0
            .source_for(&target),
        ControlSource::Human
    );
}

#[test]
fn individual_tree_raises_static_equipment_and_suspends_memory_when_human_operated() {
    let (mut app, entity, donor, _) = fixture();
    app.world_mut()
        .get_mut::<ShipShields>(entity)
        .unwrap()
        .0
        .facings
        .iter_mut()
        .find(|facing| facing.id == "aft")
        .unwrap()
        .raised = false;
    publish(&mut app, entity);
    let commands = decide(&mut app, entity);
    assert!(commands.iter().any(|command| command.target == donor
        && command.payload == Control::SetShieldRaised { raised: true }));
    app.world_mut()
        .run_system_once(crate::ship::shields::handle_shields_messages)
        .unwrap();
    assert!(
        app.world()
            .get::<ShipShields>(entity)
            .unwrap()
            .0
            .facings
            .iter()
            .find(|facing| facing.id == "aft")
            .unwrap()
            .raised
    );
    let before = serde_json::to_value(
        &app.world()
            .get::<crate::ai::state_tree_runtime::ShipAiTreeRuntime>(entity)
            .unwrap()
            .0,
    )
    .unwrap();
    app.world_mut()
        .get_mut::<ShipSystemControlSources>(entity)
        .unwrap()
        .0
        .set(donor, ControlSource::Human);
    assert!(decide(&mut app, entity).is_empty());
    let after = serde_json::to_value(
        &app.world()
            .get::<crate::ai::state_tree_runtime::ShipAiTreeRuntime>(entity)
            .unwrap()
            .0,
    )
    .unwrap();
    assert_eq!(before, after);
}

#[test]
fn focus_eligibility_does_not_block_recovery_to_an_urgent_nonfocus_arc() {
    let (mut app, entity, donor, target) = fixture();
    app.world_mut()
        .get_mut::<ShipShields>(entity)
        .unwrap()
        .0
        .facings
        .iter_mut()
        .find(|arc| arc.id == "fore")
        .unwrap()
        .focus_available = false;
    app.world_mut()
        .get_mut::<ShipConfigComponent>(entity)
        .unwrap()
        .0
        .consoles
        .iter_mut()
        .find(|console| console.id.0 == "private-shield-test")
        .unwrap()
        .feeds
        .push(phoenix_model::console::ConsoleFeed {
            source: target.clone(),
            kind: phoenix_model::console::ConsoleObservationKind::ShieldArc,
        });
    publish(&mut app, entity);
    let commands = decide(&mut app, entity);
    assert!(commands.iter().all(|command| command.target == donor));
    assert!(commands
        .iter()
        .any(|command| command.payload == Control::SetShieldArcFocus { focused: true }));
    assert!(commands.iter().any(|command| command.payload
        == Control::SetShieldRecovery {
            target: Some(target.clone())
        }));
}
