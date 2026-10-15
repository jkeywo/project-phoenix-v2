use super::*;

fn fixture() -> (World, Entity, PhoenixSnapshot) {
    let config = crate::entities::include_resolve::load_entity_config(
        "assets/entities/personnel_cruiser.toml",
    )
    .unwrap();
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
    let crossing = config.security.as_ref().unwrap().deploy_duration_secs;
    assert!(world
        .get_mut::<crate::security::ShipSecurityTeams>(entity)
        .unwrap()
        .teams
        .deploy(
            0,
            "target".into(),
            crate::security::SecurityAction::Board,
            0.5,
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
