use super::*;

fn app_local() -> (App, Entity) {
    let (mut app, operator) = app_with(None);
    let content = crate::entities::include_resolve::load_entity_config(
        "assets/entities/security_cruiser.toml",
    )
    .unwrap();
    let local = phoenix_sim_gameplay::security::local::compile_entity(&content)
        .unwrap()
        .unwrap();
    let mut teams = ShipSecurityTeams::new(content.security.clone().unwrap());
    teams.teams.navigation = Some(local.initial_navigation());
    teams.local = Some(local);
    let ship = content.ship_config.unwrap();
    app.world_mut().entity_mut(operator).insert((
        teams,
        crate::ship::access::InteriorAccess::from_ship(&ship),
        crate::ship_plugin::ShipConfigComponent(ship.clone()),
        crate::ship::power::ShipPowerSystem(
            crate::modifiers::power_system::PowerSystem::from_energy(content.energy.unwrap(), ship)
                .unwrap(),
        ),
    ));
    (app, operator)
}
fn reserve(app: &App, operator: Entity) -> f32 {
    app.world()
        .get::<crate::ship::power::ShipPowerSystem>(operator)
        .unwrap()
        .0
        .network()
        .consumers[&security_system_id()]
        .energy
}

#[test]
fn actual_local_route_waits_for_closed_boundary_then_pays_work_and_returns_without_a_crossing_timer(
) {
    let (mut app, operator) = app_local();
    let gate = SystemId("weapons-access".into());
    app.world_mut()
        .get_mut::<crate::ship::access::InteriorAccess>(operator)
        .unwrap()
        .set(&gate, true, true);
    let initial = reserve(&app, operator);
    dispatch(
        &mut app,
        operator,
        0,
        "compartment:weapons",
        "secure_contain",
    );
    for _ in 0..12 {
        app.update();
    }
    let waiting = teams(&app, operator);
    assert_eq!(waiting.teams[0].state, SecurityTeamState::Deploying);
    assert!(waiting.teams.navigation.as_ref().unwrap()[0].blocked);
    assert_eq!(
        reserve(&app, operator),
        initial,
        "route waiting/travel cannot spend work energy"
    );
    assert!(!app
        .world()
        .resource::<WorldContentRuntime>()
        .flags
        .get_flag("local_weapons_secured"));
    let saved = waiting.save_state();
    let decoded = serde_json::from_slice(&serde_json::to_vec(&saved).unwrap()).unwrap();
    app.world_mut()
        .get_mut::<ShipSecurityTeams>(operator)
        .unwrap()
        .restore_at(&decoded, 100)
        .unwrap();
    app.world_mut()
        .get_mut::<crate::ship::access::InteriorAccess>(operator)
        .unwrap()
        .set(&gate, false, true);
    for _ in 0..30 {
        app.update();
    }
    let returned = teams(&app, operator);
    assert!(returned.teams[0].is_available());
    assert_eq!(
        returned.teams.work_list()[0].outcome,
        Some(phoenix_model::job::JobOutcome::Completed)
    );
    assert_eq!(
        initial - reserve(&app, operator),
        6.0,
        "actual Security work charges its accepted six seconds exactly once"
    );
    assert!(app
        .world()
        .resource::<WorldContentRuntime>()
        .flags
        .get_flag("local_weapons_secured"));
    assert_eq!(
        blackboard(&app, operator).navigation,
        returned.teams.navigation.clone().unwrap()
    );
}

#[test]
fn ordinary_security_ai_selects_own_local_work_through_existing_admission() {
    let (mut app, operator) = app_local();
    app.insert_resource(crate::lobby::Sessions(
        crate::lobby::session::SessionManager::new(),
    ));
    let mut sources = crate::ship::control_source::ControlSourceResolver::new();
    sources.set(
        security_system_id(),
        crate::ship::control_source::ControlSource::Ai,
    );
    app.world_mut()
        .entity_mut(operator)
        .insert(crate::ship_plugin::ShipSystemControlSources(sources));
    app.add_systems(Update, operate_security_ai.before(handle_security_commands));
    app.update();
    let active = teams(&app, operator);
    assert!(active
        .teams
        .iter()
        .any(|team| team.target.as_deref() == Some("compartment:engineering")));
    assert!(active
        .teams
        .iter()
        .any(|team| team.target.as_deref() == Some("compartment:bridge")));
    for _ in 0..40 {
        app.update();
    }
    assert!(app
        .world()
        .resource::<WorldContentRuntime>()
        .flags
        .get_flag("local_engineering_secured"));
    assert!(app
        .world()
        .resource::<WorldContentRuntime>()
        .flags
        .get_flag("local_bridge_assisted"));
    assert!(app
        .world()
        .resource::<WorldContentRuntime>()
        .flags
        .get_flag("local_weapons_secured"));
    assert!(teams(&app, operator)
        .teams
        .iter()
        .all(|team| team.is_available()));
}
