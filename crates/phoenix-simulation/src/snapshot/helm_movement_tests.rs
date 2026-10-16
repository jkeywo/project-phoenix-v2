use super::*;
use phoenix_sim_gameplay::ship::helm_control::HelmControlState;

fn fixture() -> (World, Entity, PhoenixSnapshot) {
    let config = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
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
        "movement-ship".into(),
        None,
    );
    world.flush();
    let profile = config.helm_control.unwrap();
    world
        .get_mut::<HelmControlState>(entity)
        .unwrap()
        .match_speed(&profile.thrust, "issued-public-handle", 15.0, 17, 20)
        .unwrap();
    let saved = capture(&world);
    assert!(saved.entities[0].helm_movement.is_some());
    (world, entity, saved)
}

#[test]
fn actual_installed_movement_json_and_single_row_application_keep_one_intent_owner() {
    let (mut world, entity, saved) = fixture();
    let saved: PhoenixSnapshot =
        serde_json::from_slice(&serde_json::to_vec(&saved).unwrap()).unwrap();
    validate_roster(&world, &saved).unwrap();
    let profile = world.get::<CompiledHelmControl>(entity).unwrap().0.clone();
    world
        .entity_mut(entity)
        .insert(HelmControlState::new(&profile));
    super::super::application::apply_entity_state(
        &mut world,
        entity,
        &saved.entities[0],
        saved.tick,
    );
    assert_eq!(
        world.get::<HelmControlState>(entity),
        saved.entities[0].helm_movement.as_ref()
    );
    assert_eq!(
        capture(&world).entities[0].helm_movement,
        saved.entities[0].helm_movement
    );
}

#[test]
fn malformed_movement_candidates_fail_before_clock_or_actor_writes() {
    let (mut world, entity, saved) = fixture();
    let profile = world.get::<CompiledHelmControl>(entity).unwrap().0.clone();
    let before = capture(&world);
    let digest = crate::sim_digest::world_digest(&world);
    for fault in 0..5 {
        let mut candidate = saved.clone();
        candidate.tick += 10;
        let row = &mut candidate.entities[0];
        match fault {
            0 => row.helm_movement = None,
            1 => {
                row.helm_movement
                    .as_mut()
                    .unwrap()
                    .axes
                    .remove(&profile.steering);
            }
            2 => {
                row.helm_movement.as_mut().unwrap().axes.insert(SystemId("foreign".into()),
                phoenix_sim_gameplay::ship::helm_control::AxisIntent::Thrust(Default::default()));
            }
            3 => row
                .helm_movement
                .as_mut()
                .unwrap()
                .match_speed(
                    &profile.thrust,
                    "issued-public-handle",
                    15.0,
                    candidate.tick + 1,
                    candidate.tick + 1,
                )
                .unwrap(),
            _ => {
                row.control = Some(ControlState {
                    thrust: 1.0,
                    ..row.control.clone().unwrap_or_default()
                });
            }
        }
        assert!(validate_roster(&world, &candidate).is_err());
        assert!(!restore(&mut world, &candidate).is_complete());
        assert_eq!(capture(&world), before);
        assert_eq!(crate::sim_digest::world_digest(&world), digest);
    }
}

#[test]
fn forged_future_navigation_axis_or_held_plan_refuses_whole_candidate_without_writes() {
    let (mut world, entity, saved) = fixture();
    let profile = world.get::<CompiledHelmControl>(entity).unwrap().0.clone();
    let before = capture(&world);
    let digest = crate::sim_digest::world_digest(&world);
    for held_plan in [false, true] {
        let mut candidate = saved.clone();
        let row = candidate
            .entities
            .iter_mut()
            .find(|row| row.uuid == "movement-ship")
            .unwrap();
        if held_plan {
            row.motion_plan = Some(MotionPlanState {
                navigation_generation: Some(1),
                ..Default::default()
            });
        } else {
            row.helm_movement
                .as_mut()
                .unwrap()
                .follow_navigation(
                    &profile.thrust,
                    Some(1),
                    crate::ship::physics::ShipPhysicsState {
                        x: 0.0,
                        y: 0.0,
                        z: 0.0,
                        yaw: 0.0,
                        forward_speed: 0.0,
                        lateral_speed: 0.0,
                        vertical_speed: 0.0,
                    },
                )
                .unwrap();
        }
        assert!(validate_roster(&world, &candidate).is_err());
        assert!(!restore(&mut world, &candidate).is_complete());
        assert_eq!(capture(&world), before);
        assert_eq!(crate::sim_digest::world_digest(&world), digest);
    }
}
