use super::*;
use bevy::ecs::system::RunSystemOnce;

#[test]
fn actual_noncanonical_command_publishes_without_a_directed_station() {
    let config = crate::entities::include_resolve::load_entity_config(
        "assets/entities/emission_profile_cruiser.toml",
    )
    .unwrap();
    let mut ship = config.ship_config.unwrap();
    let (old, profile) = profile(&ship).unwrap().unwrap();
    let owner = SystemId("quiet-orders".into());
    ship.systems
        .iter_mut()
        .find(|system| system.id == old)
        .unwrap()
        .id = owner.clone();
    let console = ship.console_for_system(&owner).unwrap().id.clone();
    ship.consoles
        .iter_mut()
        .find(|row| row.id == console)
        .unwrap()
        .command_target = None;
    let mut sources = ShipSystemControlSources::default();
    sources.0.set(
        owner.clone(),
        crate::ship::control_source::ControlSource::Human,
    );
    let mut world = World::new();
    let entity = world
        .spawn((
            crate::server_app::Ship,
            ShipConfigComponent(ship),
            sources,
            crate::ship::state::ShipRedAlert(false),
            crate::server_app::ShipSystemBlackboards::default(),
            ShipEmissionIntent(EmissionIntentState::new(owner.clone(), &profile)),
        ))
        .id();
    world
        .run_system_once(super::super::server::publish_command_blackboard)
        .unwrap();
    let boards = world
        .get::<crate::server_app::ShipSystemBlackboards>(entity)
        .unwrap();
    let SystemBlackboard::Command(board) = &boards.0[&owner] else {
        panic!("actual Command publication")
    };
    assert_eq!(board.command_system_id, owner);
    assert!(board.directed_station.0.is_empty() && board.stances.is_empty());
    let emission = board.emission.as_ref().unwrap();
    assert_eq!(emission.owner, owner);
    assert_eq!(emission.intent, profile.initial);
    assert!(emission.controllable);
    assert!(
        !boards.0.contains_key(&old),
        "no reserved-ID alias grants knowledge or control"
    );
}
