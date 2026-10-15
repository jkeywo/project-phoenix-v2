use super::*;
#[test]
fn renamed_lateral_publication_uses_actual_owner_condition_source_and_exact_typed_feed() {
    use crate::ship::control_source::{ControlSource, ControlSourceResolver};
    use phoenix_model::console::{ConsoleFeed, ConsoleObservationKind};
    let source = crate::entities::include_resolve::resolve_from_disk(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap()
    .toml
    .replace("helm-lateral-thrust", "actual-lateral");
    let authored = crate::entities::config::EntityConfig::from_toml(&source).unwrap();
    let owner = SystemId("actual-lateral".into());
    let mut config = authored.ship_config.clone().unwrap();
    let mut sources = ControlSourceResolver::default();
    for system in &config.systems {
        sources.set(system.id.clone(), ControlSource::Human);
    }
    sources.set(owner.clone(), ControlSource::Offline);
    let mut damage = crate::ship::compartment_damage::ShipDamage::from_config(&authored)
        .unwrap()
        .unwrap();
    for component in damage
        .state
        .components
        .get_mut(&owner)
        .unwrap()
        .values_mut()
    {
        component.condition = phoenix_model::damage::ComponentCondition::Destroyed;
    }
    let mut app = base_app();
    let entity = app
        .world_mut()
        .query_filtered::<Entity, With<crate::server_app::LocalShip>>()
        .single(app.world())
        .unwrap();
    app.world_mut().entity_mut(entity).insert((
        ShipConfigComponent(config.clone()),
        phoenix_sim_gameplay::ship::helm_control::CompiledHelmControl(
            authored.helm_control.unwrap(),
        ),
        ShipSystemControlSources(sources),
        damage,
    ));
    app.update();
    let boards = &app.world().get::<ShipSystemBlackboards>(entity).unwrap().0;
    assert!(!boards.contains_key(&lateral_thrust_system_id()));
    let SystemBlackboard::HelmLateralThrust(board) = &boards[&owner] else {
        panic!("actual lateral publication")
    };
    assert!(!board.is_online);
    assert!(!board.auto);
    assert!(config
        .observations_for_system(&owner, boards)
        .get(&owner)
        .is_some());
    let recipient = crate::ship::system_registry::navigation_system_id();
    let console = config.system(&recipient).unwrap().console.clone().unwrap();
    config
        .consoles
        .iter_mut()
        .find(|row| row.id == console)
        .unwrap()
        .feeds
        .retain(|feed| feed.source != owner);
    assert!(config
        .observations_for_system(&recipient, boards)
        .get(&owner)
        .is_none());
    config
        .consoles
        .iter_mut()
        .find(|row| row.id == console)
        .unwrap()
        .feeds
        .push(ConsoleFeed {
            source: owner.clone(),
            kind: ConsoleObservationKind::HelmLateralThrust,
        });
    phoenix_sim_gameplay::ship::console_ownership::validate_consoles(&config).unwrap();
    assert!(config
        .observations_for_system(&recipient, boards)
        .get(&owner)
        .is_some());
    config
        .consoles
        .iter_mut()
        .find(|row| row.id == console)
        .unwrap()
        .feeds
        .last_mut()
        .unwrap()
        .kind = ConsoleObservationKind::Helm;
    assert!(config
        .observations_for_system(&recipient, boards)
        .get(&owner)
        .is_none());
    // Restoring source alone cannot disguise a destroyed actual mechanism.
    app.world_mut()
        .get_mut::<ShipSystemControlSources>(entity)
        .unwrap()
        .0
        .set(owner.clone(), ControlSource::Human);
    app.update();
    let SystemBlackboard::HelmLateralThrust(board) =
        &app.world().get::<ShipSystemBlackboards>(entity).unwrap().0[&owner]
    else {
        unreachable!()
    };
    assert!(!board.is_online);
}
