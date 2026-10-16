use super::*;

fn fixture() -> (World, Entity, PhoenixSnapshot) {
    fixture_for("assets/entities/personnel_cruiser.toml")
}
fn fixture_for(path: &str) -> (World, Entity, PhoenixSnapshot) {
    let config = crate::entities::include_resolve::load_entity_config(path).unwrap();
    let mut world = World::new();
    world.register_component::<EntitySystemHull>();
    world.register_component::<EntityShipArcHull>();
    world.register_component::<crate::console::repair::server::RepairRequestQueue>();
    world.register_component::<crate::ship_plugin::RepairHumanAlerted>();
    world.register_component::<crate::ai::server::ObjectiveCursors>();
    world.register_component::<crate::console::weapons::blaster::BlasterSystemResource>();
    world.insert_resource(SimTick(20));
    world.insert_resource(Time::<Fixed>::default());
    let entity = crate::entities::spawner::spawn_entity(
        &mut world.commands(),
        &config,
        Vec3::ZERO,
        "security-ship".into(),
        None,
    );
    world.flush();
    world.spawn((
        EntityUuid("target".into()),
        Transform::default(),
        crate::security::SecurityTargetActions(crate::security::SecurityTargetConfig {
            actions: vec![crate::security::SecurityActionConfig {
                acquisition: None,
                action: crate::security::SecurityAction::Board,
                duration_secs: 4.0,
                risk: 0.5,
                priority: crate::security::SecurityPriority::Optional,
                outcome_flag: None,
                outcome_value: 1,
                warning: None,
            }],
        }),
    ));
    let local = config.security.as_ref().unwrap().local.is_some();
    let crossing = if local {
        0.0
    } else {
        config.security.as_ref().unwrap().deploy_duration_secs
    };
    assert!(world
        .get_mut::<crate::security::ShipSecurityTeams>(entity)
        .unwrap()
        .teams
        .deploy(
            0,
            if local {
                "compartment:engineering".into()
            } else {
                "target".into()
            },
            if local {
                crate::security::SecurityAction::SecureContain
            } else {
                crate::security::SecurityAction::Board
            },
            if local { 0.0 } else { 0.5 },
            crossing
        ));
    let saved = capture(&world);
    (world, entity, saved)
}

#[test]
fn json_restore_preserves_security_job_identity_clock_and_consumed_mint() {
    let (mut world, entity, saved) = fixture();
    let decoded: PhoenixSnapshot =
        serde_json::from_slice(&serde_json::to_vec(&saved).unwrap()).unwrap();
    validate_roster(&world, &decoded).unwrap();
    let actual_config = world
        .get::<crate::security::ShipSecurityTeams>(entity)
        .unwrap()
        .config
        .clone();
    world
        .entity_mut(entity)
        .insert(crate::security::ShipSecurityTeams::new(actual_config));
    super::super::application::apply_entity_state(
        &mut world,
        entity,
        &decoded.entities[0],
        decoded.tick,
    );
    assert_eq!(
        capture(&world).entities[0].security,
        decoded.entities[0].security
    );
    let mut idle = decoded.entities[0].clone();
    idle.security = None;
    super::super::application::apply_entity_state(&mut world, entity, &idle, decoded.tick);
    assert!(
        capture(&world).entities[0].security.is_none(),
        "restoring an idle candidate clears later live history"
    );
}

#[test]
fn forged_security_commitments_refuse_before_global_or_entity_mutation() {
    let (mut world, _, saved) = fixture();
    let before = capture(&world);
    let digest = crate::sim_digest::world_digest(&world);
    for fault in 0..10 {
        let mut candidate = saved.clone();
        candidate.tick += 10;
        let state = candidate.entities[0].security.as_mut().unwrap();
        match fault {
            0 => state.jobs.next_id = 1,
            1 => state.jobs.owner = crate::core::messages::SystemId("repair".into()),
            2 => state.jobs.teams[0].as_mut().unwrap().activity.team.elapsed = -1.0,
            3 => {
                state.jobs.teams[0]
                    .as_mut()
                    .unwrap()
                    .activity
                    .returning_outcome = Some(phoenix_model::job::JobOutcome::Completed)
            }
            4 => state.jobs.teams[0].as_mut().unwrap().execution_tick = candidate.tick + 1,
            5 => state.jobs.teams[1] = state.jobs.teams[0].clone(),
            6 => {
                state.jobs.teams[0]
                    .as_mut()
                    .unwrap()
                    .activity
                    .team
                    .phase_duration = 0.001
            }
            7 => state.jobs.teams[0]
                .as_mut()
                .unwrap()
                .activity
                .team
                .begin_work(0.001),
            8 => {
                state.jobs.teams[0].as_mut().unwrap().activity.team.action =
                    Some(crate::security::SecurityAction::SecureContain)
            }
            _ => state.jobs.teams[0].as_mut().unwrap().activity.team.risk = 0.0,
        }
        let result = restore(&mut world, &candidate);
        assert!(!result.is_complete());
        assert!(format!("{:?}", result.gaps).contains("Security"));
        assert_eq!(capture(&world), before);
        assert_eq!(crate::sim_digest::world_digest(&world), digest);
    }
}

#[test]
fn local_physical_route_forgery_refuses_without_any_restore_write() {
    let (mut world, _, saved) = fixture_for("assets/entities/security_cruiser.toml");
    validate_roster(&world, &saved).unwrap();
    let before = capture(&world);
    let digest = crate::sim_digest::world_digest(&world);
    for fault in 0..7 {
        let mut candidate = saved.clone();
        candidate.tick += 10;
        let state = candidate.entities[0].security.as_mut().unwrap();
        match fault {
            0 => state.navigation = None,
            1 => {
                state.navigation.as_mut().unwrap().pop();
            }
            2 => state.get_mut(0).unwrap().begin_work(6.0),
            3 => {
                state.navigation.as_mut().unwrap()[0].location =
                    phoenix_model::interior::RouteLocation::External {
                        target: "target".into(),
                    }
            }
            4 => {
                state.navigation.as_mut().unwrap()[0].location =
                    phoenix_model::interior::RouteLocation::Crossing {
                        connection: "fake".into(),
                        from: "bridge".into(),
                        to: "engineering".into(),
                        elapsed: 1.0,
                    }
            }
            5 => state.get_mut(0).unwrap().target = Some("compartment:unknown".into()),
            _ => {
                state.home(0);
                state.navigation.as_mut().unwrap()[0].location =
                    phoenix_model::interior::RouteLocation::Compartment {
                        compartment: "engineering".into(),
                    };
            }
        }
        let result = restore(&mut world, &candidate);
        assert!(!result.is_complete());
        assert_eq!(capture(&world), before);
        assert_eq!(crate::sim_digest::world_digest(&world), digest);
    }
}
