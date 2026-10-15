use super::*;
use bevy::ecs::system::RunSystemOnce;
use phoenix_model::transit::*;
use phoenix_sim_gameplay::ship::transit::*;

fn owner() -> SystemId {
    SystemId("local-jump".into())
}
fn fixture() -> (World, Entity) {
    let config = crate::entities::include_resolve::load_entity_config(
        "assets/entities/local_jump_cruiser.toml",
    )
    .unwrap();
    let mut world = World::new();
    world.register_component::<crate::entities::spawner::EntitySystemHull>();
    world.register_component::<crate::entities::spawner::EntityShipArcHull>();
    world.register_component::<crate::console::repair::server::RepairRequestQueue>();
    world.register_component::<crate::ship_plugin::RepairHumanAlerted>();
    world.register_component::<crate::ai::server::ObjectiveCursors>();
    world.register_component::<crate::console::weapons::blaster::BlasterSystemResource>();
    world.register_component::<AsteroidUuid>();
    world.insert_resource(SimTick(10));
    world.insert_resource(Time::<Fixed>::from_duration(
        std::time::Duration::from_secs(1),
    ));
    world.insert_resource(crate::world::config::WorldConfig::default());
    let entity = crate::entities::spawner::spawn_entity(
        &mut world.commands(),
        &config,
        Vec3::ZERO,
        "jump-saved".into(),
        None,
    );
    world.flush();
    world
        .get_mut::<crate::ship_plugin::ShipSystemControlSources>(entity)
        .unwrap()
        .0
        .set(owner(), crate::ship::control_source::ControlSource::Human);
    let system = config
        .ship_config
        .as_ref()
        .unwrap()
        .system(&owner())
        .unwrap();
    let binding = crate::ship::composition::transit_composition(&system.components).unwrap();
    let consumer = config
        .energy
        .as_ref()
        .unwrap()
        .consumers
        .iter()
        .find(|row| row.system == owner())
        .unwrap();
    let order = TransitOrder {
        destination: TransitDestination::Local {
            point: [100.0, 0.0, 30.0],
            course_generation: 7,
        },
        origin: [0.0; 3],
        profile: AcceptedLocalJumpProfile {
            config: binding.config.clone(),
            preparation_energy_per_sec: consumer.continuous_cost_per_sec,
            commit_energy: consumer.discrete_cost,
        },
        accepted_tick: 10,
        request: Some(TransitRequestKey {
            objective_id: "ordered-jump".into(),
            revision: 3,
        }),
    };
    world
        .get_mut::<ShipTransitDrives>(entity)
        .unwrap()
        .0
        .get_mut(&owner())
        .unwrap()
        .begin(order, 10)
        .unwrap();
    world
        .run_system_once(crate::console::helm::transit::tick)
        .unwrap();
    world.resource_mut::<SimTick>().0 = 11;
    (world, entity)
}
fn full_capture(world: &World) -> PhoenixSnapshot {
    super::super::capture(world)
}

#[test]
fn actual_paid_preparation_json_single_actor_restore_keeps_exact_360_owner_continuations() {
    let (mut original, actor) = fixture();
    let saved = full_capture(&original);
    assert_eq!(saved.entities.len(), 1);
    assert!(
        matches!(saved.entities[0].transit.as_ref().unwrap().0[&owner()].phase,TransitPhase::Preparing{worked_secs,..} if worked_secs>0.0)
    );
    let decoded: PhoenixSnapshot =
        serde_json::from_slice(&serde_json::to_vec(&saved).unwrap()).unwrap();
    let (mut resumed, restored) = fixture();
    validate_roster(&resumed, &decoded).unwrap();
    super::super::application::apply_entity_state(
        &mut resumed,
        restored,
        &decoded.entities[0],
        decoded.tick,
    );
    assert_eq!(full_capture(&resumed), saved);
    for _ in 0..360 {
        original
            .run_system_once(crate::console::helm::transit::tick)
            .unwrap();
        resumed
            .run_system_once(crate::console::helm::transit::tick)
            .unwrap();
        assert_eq!(full_capture(&original), full_capture(&resumed));
        assert_eq!(
            crate::sim_digest::world_digest(&original),
            crate::sim_digest::world_digest(&resumed)
        );
        original.resource_mut::<SimTick>().0 += 1;
        resumed.resource_mut::<SimTick>().0 += 1;
    }
    assert!(matches!(
        original.get::<ShipTransitDrives>(actor).unwrap().0[&owner()].phase,
        TransitPhase::Arrived { .. }
    ));
    assert_eq!(
        original
            .get::<crate::infrastructure::InfrastructureCondition>(actor)
            .unwrap()
            .0
            .capacity("drive-transit-fuel"),
        Some(4)
    );
    let state = &resumed.get::<ShipTransitDrives>(restored).unwrap().0[&owner()];
    assert!(!state.request_is_fresh(&TransitRequestKey {
        objective_id: "ordered-jump".into(),
        revision: 3
    }));
}

#[test]
fn full_candidate_frontier_placement_and_owner_faults_refuse_before_any_writes() {
    let (mut world, entity) = fixture();
    world
        .run_system_once(crate::console::helm::transit::tick)
        .unwrap();
    world.resource_mut::<SimTick>().0 += 1;
    let saved = full_capture(&world);
    assert!(matches!(
        saved.entities[0].transit.as_ref().unwrap().0[&owner()].phase,
        TransitPhase::Committed { .. }
    ));
    validate_roster(&world, &saved).unwrap();
    let digest = crate::sim_digest::world_digest(&world);
    for fault in 0..8 {
        let mut candidate = saved.clone();
        candidate.tick += 20;
        let row = &mut candidate.entities[0];
        match fault {
            0 => row.transit = None,
            1 => {
                row.transit
                    .as_mut()
                    .unwrap()
                    .0
                    .insert(SystemId("forged-drive".into()), TransitState::default());
            }
            2 => row.physics = None,
            3 => {
                row.transit
                    .as_mut()
                    .unwrap()
                    .0
                    .get_mut(&owner())
                    .unwrap()
                    .last_tick = Some(candidate.tick + 1)
            }
            4 => {
                let state = row.transit.as_mut().unwrap().0.get_mut(&owner()).unwrap();
                if let TransitPhase::Committed { arrival, .. } = &mut state.phase {
                    arrival.point[0] += 1.0;
                }
            }
            5 => {
                let state = row.transit.as_mut().unwrap().0.get_mut(&owner()).unwrap();
                if let TransitPhase::Committed {
                    order, commit_tick, ..
                } = &mut state.phase
                {
                    *commit_tick = order.accepted_tick;
                    order.profile.config.preparation_work_secs = 1000.0;
                }
            }
            6 => {
                row.transit
                    .as_mut()
                    .unwrap()
                    .0
                    .get_mut(&owner())
                    .unwrap()
                    .handled
                    .clear();
            }
            _ => {
                let state = row.transit.as_mut().unwrap().0.get_mut(&owner()).unwrap();
                if let TransitPhase::Committed {
                    order,
                    commit_tick,
                    arrival,
                } = &state.phase
                {
                    state.phase = TransitPhase::Arrived {
                        order: order.clone(),
                        commit_tick: *commit_tick,
                        arrival: arrival.clone(),
                        arrival_tick: *commit_tick,
                    };
                }
            }
        }
        assert!(
            validate_roster(&world, &candidate).is_err(),
            "fault {fault}"
        );
        assert!(
            !super::super::restore(&mut world, &candidate).is_complete(),
            "fault {fault}"
        );
        assert_eq!(full_capture(&world), saved, "fault {fault}");
        assert_eq!(
            crate::sim_digest::world_digest(&world),
            digest,
            "fault {fault}"
        );
        super::super::application::apply_entity_state(
            &mut world,
            entity,
            &candidate.entities[0],
            candidate.tick,
        );
        assert_eq!(full_capture(&world), saved, "single-row fault {fault}");
    }
}

#[test]
fn single_row_replacement_uses_its_exact_recipe_instead_of_borrowing_live_drive() {
    let (mut world, entity) = fixture();
    let before = full_capture(&world);
    let digest = crate::sim_digest::world_digest(&world);
    for fault in 0..3 {
        let mut candidate = before.entities[0].clone();
        candidate.spawn = Some(crate::world::spawn_origin::SpawnOrigin {
            template_path: if fault == 0 {
                "assets/entities/alliance_cruiser.toml"
            } else if fault == 1 {
                "assets/entities/missing-drive.toml"
            } else {
                "assets/entities/local_jump_cruiser.toml"
            }
            .into(),
            name: "jump-saved".into(),
            overrides: if fault == 2 {
                Some(toml::from_str("damage='malformed'").unwrap())
            } else {
                None
            },
            ..Default::default()
        });
        assert!(validate_live(&world, entity, &candidate, before.tick).is_err());
        super::super::application::apply_entity_state(&mut world, entity, &candidate, before.tick);
        assert_eq!(full_capture(&world), before);
        assert_eq!(crate::sim_digest::world_digest(&world), digest);
    }
}

#[test]
fn preparing_frozen_profile_and_cost_forgeries_refuse_without_touching_live_resources() {
    let (mut world, actor) = fixture();
    let saved = full_capture(&world);
    let digest = crate::sim_digest::world_digest(&world);
    for fault in 0..7 {
        let mut candidate = saved.clone();
        let state = candidate.entities[0]
            .transit
            .as_mut()
            .unwrap()
            .0
            .get_mut(&owner())
            .unwrap();
        let TransitPhase::Preparing { order, .. } = &mut state.phase else {
            panic!("actual paid preparing fixture")
        };
        match fault {
            0 => order.profile.config.gravity_response = 0.25,
            1 => order.profile.config.max_distance += 1.0,
            2 => order.profile.config.preparation_work_secs += 1.0,
            3 => order.profile.config.fuel.amount += 1,
            4 => order.profile.preparation_energy_per_sec *= 0.5,
            5 => order.profile.commit_energy *= 0.5,
            _ => order.profile.config.scatter_radius += 1.0,
        }
        assert!(
            validate_roster(&world, &candidate).is_err(),
            "profile fault {fault}"
        );
        assert!(
            !super::super::restore(&mut world, &candidate).is_complete(),
            "profile fault {fault}"
        );
        assert_eq!(full_capture(&world), saved);
        assert_eq!(crate::sim_digest::world_digest(&world), digest);
        super::super::application::apply_entity_state(
            &mut world,
            actor,
            &candidate.entities[0],
            candidate.tick,
        );
        assert_eq!(
            full_capture(&world),
            saved,
            "single-row profile fault {fault}"
        );
    }
    // A different accepted installed component cannot inherit this old frozen profile.
    let mut different = world
        .get::<crate::ship_plugin::ShipConfigComponent>(actor)
        .unwrap()
        .0
        .clone();
    let system = different
        .systems
        .iter_mut()
        .find(|system| system.id == owner())
        .unwrap();
    for component in &mut system.components {
        if let crate::ship::composition::SystemComponentConfig::TransitDrive { config, .. } =
            component
        {
            config.gravity_response = 0.5;
        }
    }
    assert!(saved.entities[0]
        .transit
        .as_ref()
        .unwrap()
        .validate(&different, saved.tick)
        .is_err());
}
