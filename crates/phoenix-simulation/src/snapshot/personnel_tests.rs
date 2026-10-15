use super::*;
use phoenix_sim_gameplay::personnel::PersonnelState;

#[path = "activity_snapshot_tests.rs"]
mod activities;

// These owner fixtures bypass App plugins. Install the normal run resources
// whose absence is deliberately distinct in the canonical digest; whole restore
// always installs their captured values. No personnel outcome is manufactured.
fn install_fixture_run_scope(world: &mut World) {
    world.init_resource::<crate::gm::actions::action::SimulationPaused>();
    world.init_resource::<crate::gm::actions::action::GmActionJournal>();
    world.init_resource::<crate::gm::operations::puppet::StationPuppets>();
    world.init_resource::<crate::gm::operations::puppet::PendingGmStationCommands>();
    world.init_resource::<crate::core::report::MissionReport>();
    // Restore runs the actual Tactical/AI read-model publishers, whose Query
    // initialization registers AsteroidUuid even in a rock-free World. The
    // digest deliberately distinguishes an unregistered asteroid namespace
    // from a registered empty one; ordinary hosts always register this type.
    world.register_component::<crate::server_app::AsteroidUuid>();
}
fn publish_fixture_observations(world: &mut World) {
    use bevy::ecs::system::RunSystemOnce;
    // Actual observation read-model owners, matching whole restore's rebuild.
    // They buy no scan/receiver work and mutate no authoritative observations.
    world
        .run_system_once(crate::science::server::publish_scan_blackboard)
        .unwrap();
    world
        .run_system_once(crate::science::passive::publish)
        .unwrap();
    world
        .run_system_once(crate::ship::sensors::publish_sensor_radar_blackboard)
        .unwrap();
}

fn fixture() -> (World, Entity, PhoenixSnapshot) {
    let config = crate::entities::include_resolve::load_entity_config(
        "assets/entities/personnel_cruiser.toml",
    )
    .unwrap();
    let mut world = World::new();
    install_fixture_run_scope(&mut world);
    crate::console::repair::personnel::register_hooks(&mut world);
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
        "personnel-ship".into(),
        None,
    );
    world.flush();
    let profile = world.get::<CompiledPersonnel>(entity).unwrap().clone();
    let jobs = world
        .get::<crate::console::repair::server::ShipRepairTeams>(entity)
        .unwrap()
        .0
        .save_state();
    world
        .resource_mut::<PersonnelRoster>()
        .state_mut("personnel-ship")
        .unwrap()
        .post(
            &profile,
            &jobs,
            config.interior.as_ref().unwrap(),
            &phoenix_model::personnel::PersonRef::new("personnel-ship", "repair-chief"),
            Some(0),
            18,
        )
        .unwrap();
    publish_fixture_observations(&mut world);
    let saved = capture(&world);
    assert_eq!(saved.personnel.origins.len(), 1);
    assert!(saved.entities[0].repair.as_ref().unwrap().jobs.is_some());
    (world, entity, saved)
}

#[test]
fn actual_spawn_and_json_application_preserve_one_custody_owner_and_team_frontier() {
    let (mut world, entity, saved) = fixture();
    let decoded: PhoenixSnapshot =
        serde_json::from_slice(&serde_json::to_vec(&saved).unwrap()).unwrap();
    validate_roster(&world, &decoded).unwrap();
    let jobs = world
        .get::<crate::console::repair::server::ShipRepairTeams>(entity)
        .unwrap()
        .0
        .save_state();
    // A single machinery row cannot rewind global custody; the full candidate
    // owns that transaction and preserves the same actual job frontier.
    world
        .resource_mut::<PersonnelRoster>()
        .origins
        .get_mut("personnel-ship")
        .unwrap()
        .state = PersonnelState::new(
        &world.get::<CompiledPersonnel>(entity).unwrap().clone(),
        "personnel-ship",
    );
    let current = world.resource::<PersonnelRoster>().clone();
    super::super::application::apply_entity_state(
        &mut world,
        entity,
        &decoded.entities[0],
        decoded.tick,
    );
    assert_eq!(world.resource::<PersonnelRoster>(), &current);
    let report = restore(&mut world, &decoded);
    assert!(report.is_complete(), "{report:?}");
    assert_eq!(world.resource::<PersonnelRoster>(), &decoded.personnel);
    assert_eq!(
        world
            .get::<crate::console::repair::server::ShipRepairTeams>(entity)
            .unwrap()
            .0
            .save_state(),
        jobs
    );
    assert_eq!(capture(&world).personnel, decoded.personnel);
}

#[test]
fn forged_custody_refuses_before_any_global_or_entity_write() {
    let (mut world, _, saved) = fixture();
    let before = capture(&world);
    let before_digest = crate::sim_digest::world_digest(&world);
    for fault in 0..7 {
        let mut candidate = saved.clone();
        candidate.tick += 10;
        let row = &mut candidate.entities[0];
        match fault {
            0 => {
                candidate.personnel.origins.remove("personnel-ship");
            }
            1 => {
                candidate
                    .personnel
                    .origins
                    .get_mut("personnel-ship")
                    .unwrap()
                    .state
                    .officers
                    .get_mut("repair-chief")
                    .unwrap()
                    .changed_tick = candidate.tick + 1
            }
            2 => {
                candidate
                    .personnel
                    .origins
                    .get_mut("personnel-ship")
                    .unwrap()
                    .state
                    .officers
                    .get_mut("repair-chief")
                    .unwrap()
                    .location = phoenix_model::personnel::OfficerLocation::WithRepairTeam {
                    actor: "personnel-ship".into(),
                    team_idx: 99,
                }
            }
            3 => row.repair.as_mut().unwrap().jobs = None,
            4 => {
                candidate
                    .personnel
                    .origins
                    .get_mut("personnel-ship")
                    .unwrap()
                    .state
                    .officers
                    .clear();
            }
            5 => {
                candidate
                    .personnel
                    .origins
                    .get_mut("personnel-ship")
                    .unwrap()
                    .state
                    .officers
                    .get_mut("repair-chief")
                    .unwrap()
                    .location = OfficerLocation::WithRepairTeam {
                    actor: "another-holder".into(),
                    team_idx: 0,
                }
            }
            _ => {
                candidate.entities[0]
                    .repair
                    .as_mut()
                    .unwrap()
                    .personnel_destinations
                    .insert(
                        phoenix_model::personnel::PersonRef::new("another-origin", "repair-chief"),
                        phoenix_model::personnel::PersonnelDestinationRequest {
                            destination: phoenix_model::personnel::PersonnelDestination::Home,
                            requested_tick: 20,
                        },
                    );
            }
        }
        let report = restore(&mut world, &candidate);
        assert!(!report.is_complete());
        assert!(
            format!("{:?}", report.gaps).contains("personnel")
                || format!("{:?}", report.gaps).contains("officer")
        );
        assert_eq!(capture(&world), before);
        assert_eq!(crate::sim_digest::world_digest(&world), before_digest);
    }
}

#[path = "planning_snapshot_tests.rs"]
mod planning;

#[test]
fn actual_medical_candidate_rejects_forged_site_terms_condition_and_exclusive_team_before_writes() {
    let config = crate::entities::include_resolve::load_entity_config(
        "assets/entities/medical_cruiser.toml",
    )
    .unwrap();
    let mut world = World::new();
    install_fixture_run_scope(&mut world);
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
        "medical-ship".into(),
        None,
    );
    world.flush();
    let profile = world.get::<CompiledPersonnel>(entity).unwrap().clone();
    let personnel = world
        .resource::<PersonnelRoster>()
        .state("medical-ship")
        .unwrap()
        .clone();
    world
        .get_mut::<crate::console::repair::server::ShipRepairTeams>(entity)
        .unwrap()
        .0
        .claim_treatment(
            &profile,
            &personnel,
            &phoenix_model::personnel::PersonRef::new("medical-ship", "repair-chief"),
            1,
        )
        .unwrap();
    publish_fixture_observations(&mut world);
    let saved: PhoenixSnapshot =
        serde_json::from_slice(&serde_json::to_vec(&capture(&world)).unwrap()).unwrap();
    validate_roster(&world, &saved).unwrap();
    let before = capture(&world);
    let digest = crate::sim_digest::world_digest(&world);
    for mode in 0..10 {
        let mut candidate = saved.clone();
        candidate.tick += 10;
        let row = candidate
            .entities
            .iter_mut()
            .find(|row| row.uuid == "medical-ship")
            .unwrap();
        match mode {
            0 => {
                candidate
                    .personnel
                    .origins
                    .get_mut("medical-ship")
                    .unwrap()
                    .state
                    .officers
                    .get_mut("repair-chief")
                    .unwrap()
                    .condition = phoenix_model::personnel::PersonnelCondition::Fit
            }
            1 => {
                candidate
                    .personnel
                    .origins
                    .get_mut("medical-ship")
                    .unwrap()
                    .state
                    .officers
                    .get_mut("repair-chief")
                    .unwrap()
                    .location = phoenix_model::personnel::OfficerLocation::AtCompartment {
                    actor: "medical-ship".into(),
                    compartment: "bridge".into(),
                }
            }
            2 => {
                candidate.personnel.origins.remove("medical-ship");
            }
            _ => {
                let jobs = row.repair.as_mut().unwrap().jobs.as_mut().unwrap();
                if mode == 7 {
                    let mut duplicate = jobs.teams[1].clone().unwrap();
                    duplicate.id.0 += 1;
                    jobs.next_id = duplicate.id.0 + 1;
                    jobs.teams[0] = Some(duplicate);
                } else {
                    let crate::jobs::repair::RepairActivity::Treatment { phase, work, .. } =
                        &mut jobs.teams[1].as_mut().unwrap().activity
                    else {
                        panic!()
                    };
                    match mode {
                        3 => work.work_secs = 0.001,
                        4 => {
                            work.patient = phoenix_model::personnel::PersonRef::new(
                                "medical-ship",
                                "imaginary",
                            )
                        }
                        5 => work.progress = 1.0,
                        8 => work.patient.origin = "another-origin".into(),
                        9 => {
                            candidate
                                .personnel
                                .origins
                                .get_mut("medical-ship")
                                .unwrap()
                                .state
                                .officers
                                .get_mut("repair-chief")
                                .unwrap()
                                .location = OfficerLocation::AtCompartment {
                                actor: "another-holder".into(),
                                compartment: profile.care_compartment.clone().unwrap(),
                            }
                        }
                        _ => {
                            *phase = crate::core::messages::TeamSlot::Repairing {
                                system_id: Some(crate::ship::system_registry::repair_system_id()),
                                display_name: None,
                                priority: None,
                                priority_system_id: None,
                            }
                        }
                    }
                }
            }
        }
        let report = restore(&mut world, &candidate);
        assert!(
            !report.is_complete(),
            "forgery {mode} must fail: {report:?}"
        );
        assert_eq!(capture(&world), before);
        assert_eq!(crate::sim_digest::world_digest(&world), digest);
    }
    let report = restore(&mut world, &saved);
    assert!(report.is_complete(), "{report:?}");
    assert_eq!(capture(&world), before);
}

#[test]
fn ordinary_removal_retains_terminal_identity_and_full_restore_does_not_reroll_it() {
    let (mut world, entity, _) = fixture();
    world.resource_mut::<SimTick>().0 = 25;
    world.despawn(entity);
    world.flush();
    assert!(world.get_entity(entity).is_err());
    let saved = capture(&world);
    assert!(saved
        .entities
        .iter()
        .all(|row| row.uuid != "personnel-ship"));
    let origin = &saved.personnel.origins["personnel-ship"];
    assert_eq!(origin.removed_tick, Some(25));
    assert_eq!(origin.state.officers.len(), 1);
    assert_eq!(
        origin.state.officers["repair-chief"].location,
        OfficerLocation::Lost {
            actor: "personnel-ship".into()
        }
    );
    let (mut fresh, original, _) = fixture();
    let decoded: PhoenixSnapshot =
        serde_json::from_slice(&serde_json::to_vec(&saved).unwrap()).unwrap();
    let report = restore(&mut fresh, &decoded);
    assert!(report.is_complete(), "{report:?}");
    assert!(fresh.get_entity(original).is_err());
    assert_eq!(capture(&fresh), decoded);
    let original_stages = crate::sim_digest::digest_stages(&world);
    assert_eq!(
        crate::sim_digest::first_divergent_scope(&fresh, &original_stages),
        None,
        "matching complete state must agree at every canonical fold stage; live={original_stages:?}, restored={:?}",
        crate::sim_digest::digest_stages(&fresh)
    );
    assert_eq!(
        crate::sim_digest::world_digest(&fresh),
        crate::sim_digest::world_digest(&world)
    );
    assert!(fresh
        .resource::<PersonnelRoster>()
        .state("personnel-ship")
        .is_none());
}

#[test]
fn removed_origin_population_provenance_and_terminal_closure_are_preflighted() {
    let (mut world, entity, _) = fixture();
    world.resource_mut::<SimTick>().0 = 25;
    world.despawn(entity);
    world.flush();
    let saved = capture(&world);
    let (mut fresh, _, _) = fixture();
    let before = capture(&fresh);
    let digest = crate::sim_digest::world_digest(&fresh);
    for fault in 0..9 {
        let mut bad = saved.clone();
        match fault {
            0 => {
                bad.personnel.origins.clear();
            }
            1 => {
                bad.personnel
                    .origins
                    .get_mut("personnel-ship")
                    .unwrap()
                    .removed_tick = None;
            }
            2 => {
                bad.personnel
                    .origins
                    .get_mut("personnel-ship")
                    .unwrap()
                    .removed_tick = Some(26);
            }
            3 => {
                bad.personnel
                    .origins
                    .get_mut("personnel-ship")
                    .unwrap()
                    .state
                    .officers
                    .clear();
            }
            4 => {
                bad.personnel
                    .origins
                    .get_mut("personnel-ship")
                    .unwrap()
                    .definition
                    .config
                    .officers[0]
                    .repair_work_multiplier = 99.0;
            }
            5 => {
                bad.personnel
                    .origins
                    .get_mut("personnel-ship")
                    .unwrap()
                    .state
                    .officers
                    .get_mut("repair-chief")
                    .unwrap()
                    .location = OfficerLocation::AtCompartment {
                    actor: "personnel-ship".into(),
                    compartment: "engineering".into(),
                };
            }
            6 => {
                bad.personnel
                    .origins
                    .get_mut("personnel-ship")
                    .unwrap()
                    .state
                    .officers
                    .get_mut("repair-chief")
                    .unwrap()
                    .changed_tick = 24;
            }
            7 => {
                let origin = bad.personnel.origins["personnel-ship"].clone();
                bad.personnel
                    .origins
                    .insert("invented-origin".into(), origin);
            }
            _ => {
                bad.personnel
                    .origins
                    .get_mut("personnel-ship")
                    .unwrap()
                    .accepted_tick = 1;
            }
        }
        let report = restore(&mut fresh, &bad);
        assert!(!report.is_complete(), "forgery {fault}: {report:?}");
        assert_eq!(capture(&fresh), before);
        assert_eq!(crate::sim_digest::world_digest(&fresh), digest);
    }
    let mut old = serde_json::to_value(&saved).unwrap();
    old.as_object_mut().unwrap().remove("personnel");
    assert!(
        serde_json::from_value::<PhoenixSnapshot>(old).is_err(),
        "required global owner must not silently default"
    );
}

#[test]
fn runtime_origin_recipe_is_retained_after_body_loss_and_validated_on_fresh_restore() {
    use crate::world_id::{IdNamespace, WorldId, WorldIdMint, WorldIdMintState};
    let (mut world, entity, _) = fixture();
    let actor = WorldId::new(IdNamespace::Entity, 20, 0).render();
    world.get_mut::<EntityUuid>(entity).unwrap().0 = actor.clone();
    let mut record = world
        .resource_mut::<PersonnelRoster>()
        .origins
        .remove("personnel-ship")
        .unwrap();
    record
        .state
        .officers
        .get_mut("repair-chief")
        .unwrap()
        .location = OfficerLocation::WithRepairTeam {
        actor: actor.clone(),
        team_idx: 0,
    };
    world
        .resource_mut::<PersonnelRoster>()
        .origins
        .insert(actor.clone(), record);
    world.insert_resource(WorldIdMint::from_state(WorldIdMintState {
        tick: 20,
        next_seq: [1, 0, 0, 0],
    }));
    world
        .entity_mut(entity)
        .insert(crate::entities::spawner::EntitySpawnOrigin(
            crate::world::spawn_origin::SpawnOrigin {
                template_path: "assets/entities/personnel_cruiser.toml".into(),
                ..Default::default()
            },
        ));
    assert_eq!(
        world.resource::<PersonnelRoster>().origins[&actor].accepted_tick,
        20
    );
    assert!(world.resource::<PersonnelRoster>().origins[&actor]
        .source
        .is_some());
    world.resource_mut::<SimTick>().0 = 25;
    world.despawn(entity);
    let saved = capture(&world);
    let mut fresh = World::new();
    fresh.insert_resource(SimTick(0));
    validate_roster(&fresh, &saved).unwrap();
    let before = capture(&fresh);
    let digest = crate::sim_digest::world_digest(&fresh);
    for fault in 0..7 {
        let mut wrong = saved.clone();
        match fault {
            0 => {
                wrong
                    .personnel
                    .origins
                    .get_mut(&actor)
                    .unwrap()
                    .source
                    .as_mut()
                    .unwrap()
                    .template_path = "assets/entities/physical_probe.toml".into()
            }
            1 => {
                wrong
                    .personnel
                    .origins
                    .get_mut(&actor)
                    .unwrap()
                    .definition
                    .config
                    .officers[0]
                    .repair_work_multiplier = 99.0
            }
            2 | 3 => {
                let other = WorldId::new(IdNamespace::Entity, 20, 1).render();
                let mut duplicate = wrong.personnel.origins[&actor].clone();
                duplicate
                    .state
                    .officers
                    .get_mut("repair-chief")
                    .unwrap()
                    .location = OfficerLocation::Lost {
                    actor: other.clone(),
                };
                if fault == 3 {
                    wrong.personnel.origins.remove(&actor);
                }
                wrong.personnel.origins.insert(other, duplicate);
            }
            4 => {
                wrong
                    .personnel
                    .origins
                    .get_mut(&actor)
                    .unwrap()
                    .source
                    .as_mut()
                    .unwrap()
                    .birth_tick = 19
            }
            5 => wrong.mint.as_mut().unwrap().next_seq[0] = 0,
            _ => wrong.mint = None,
        }
        let report = restore(&mut fresh, &wrong);
        assert!(
            !report.is_complete(),
            "runtime receipt forgery {fault}: {report:?}"
        );
        assert_eq!(capture(&fresh), before);
        assert_eq!(crate::sim_digest::world_digest(&fresh), digest);
    }
}
