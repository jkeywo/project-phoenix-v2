//! Actual capture/restore owner and candidate-only imported plan validation.
use super::*;
use bevy::ecs::system::RunSystemOnce;
fn prepared() -> (World, Entity, PhoenixSnapshot) {
    let (mut world, actor, _) = fixture();
    let mut base = crate::world::activities::declared_actor_fixture(
        "personnel-ship",
        "assets/entities/personnel_cruiser.toml",
    );
    let mut planning = crate::world::config::parse_world(
        &crate::content_fs::read_to_string("tests/fixtures/maintenance-htn-world.toml").unwrap(),
    )
    .unwrap()
    .planning
    .unwrap();
    planning.arguments.insert(
        "actor".into(),
        phoenix_sim_world::world::planning::Value::Actor("personnel-ship".into()),
    );
    let (compiled, _) =
        phoenix_sim_world::world::planning::CompiledPlanning::load(planning.clone(), |path| {
            crate::content_fs::read_to_string(path).ok()
        })
        .unwrap();
    base.planning = Some(planning);
    base.compiled_planning = Some(compiled);
    let config = world
        .get::<crate::ship_plugin::ShipConfigComponent>(actor)
        .unwrap()
        .0
        .clone();
    let damage = world
        .get::<crate::ship::compartment_damage::ShipDamage>(actor)
        .unwrap()
        .clone();
    let teams = world
        .get::<crate::console::repair::server::ShipRepairTeams>(actor)
        .unwrap();
    let owner = teams.0.job_state().owner.clone();
    let count = teams.0.slots().len();
    let logistics = world
        .get::<crate::resources::logistics_server::CompiledShipLogistics>(actor)
        .unwrap()
        .0
        .clone();
    let mut runtime = WorldContentRuntime::default();
    runtime.mission_clock = Some(crate::world::server::MissionClock::new(
        std::time::Duration::ZERO,
    ));
    crate::world::planning::advance_for_actor(
        &mut runtime,
        &base,
        "personnel-ship",
        None,
        20,
        |request| match request {
            phoenix_sim_world::world::activities::ActivityRequest::Communication { .. } => false,
            phoenix_sim_world::world::activities::ActivityRequest::Repair { .. } => {
                crate::world::activities::repair_binding_supported(
                    request, &config, &damage, &owner, count,
                )
            }
            phoenix_sim_world::world::activities::ActivityRequest::Delivery { .. } => {
                crate::world::activities::delivery_binding_supported(request, &logistics)
            }
        },
    )
    .unwrap();
    world.insert_resource(base);
    world.insert_resource(runtime);
    world
        .run_system_once(crate::console::repair::server::publish_repair_blackboard)
        .unwrap();
    let saved = capture(&world);
    crate::snapshot::activities::validate(&world, &saved).unwrap();
    (world, actor, saved)
}
#[test]
fn actual_plan_choice_capture_json_restore_and_forged_frontiers_have_no_partial_writes() {
    let (mut world, _actor, saved) = prepared();
    let decoded: PhoenixSnapshot =
        serde_json::from_slice(&serde_json::to_vec(&saved).unwrap()).unwrap();
    let choice = decoded
        .scenario
        .as_ref()
        .unwrap()
        .plan_choice
        .as_ref()
        .unwrap();
    assert_eq!(choice.released.len(), 1);
    let mut exact = decoded.clone();
    exact.entities[0].spawn = Some(crate::world::spawn_origin::SpawnOrigin {
        template_path: "assets/entities/personnel_cruiser.toml".into(),
        name: "personnel-ship".into(),
        ..Default::default()
    });
    crate::snapshot::activities::validate(&world, &exact).unwrap();
    let before = capture(&world);
    let digest = crate::sim_digest::world_digest(&world);
    for fault in 0..14 {
        let mut forged = decoded.clone();
        let scenario = forged.scenario.as_mut().unwrap();
        match fault {
            0 => scenario.plan_choice = None,
            1 => scenario.plan_choice.as_mut().unwrap().actor_uuid = "foreign".into(),
            2 => {
                scenario
                    .plan_choice
                    .as_mut()
                    .unwrap()
                    .methods
                    .get_mut("root")
                    .unwrap()
                    .method = "unknown".into()
            }
            3 => {
                *scenario
                    .plan_choice
                    .as_mut()
                    .unwrap()
                    .released
                    .values_mut()
                    .next()
                    .unwrap() = saved.tick + 1
            }
            4 => scenario.situations.instances.clear(),
            5 => {
                scenario
                    .plan_choice
                    .as_mut()
                    .unwrap()
                    .released
                    .insert("root/unknown".into(), 20);
            }
            6..=8 => {
                let board = forged
                    .entities
                    .iter_mut()
                    .flat_map(|row| &mut row.blackboards)
                    .find_map(|(_, board)| match board {
                        crate::core::messages::SystemBlackboard::Repair(board)
                            if !board.planned_requests.is_empty() =>
                        {
                            Some(board)
                        }
                        _ => None,
                    })
                    .expect("actual publisher emitted the instruction");
                match fault {
                    6 => board.planned_requests[0].goal = phoenix_model::activity::RepairGoal::Full,
                    7 => board.planned_requests[0].released_tick += 1,
                    _ => board
                        .planned_requests
                        .push(board.planned_requests[0].clone()),
                }
            }
            12 => scenario
                .plan_choice
                .as_mut()
                .unwrap()
                .refused_methods
                .push("invented current damage".into()),
            13 => scenario
                .plan_choice
                .as_mut()
                .unwrap()
                .refused_methods
                .push("x".repeat(513)),
            _ => {
                forged.entities[0].spawn = Some(crate::world::spawn_origin::SpawnOrigin {
                    template_path: if fault == 9 {
                        "assets/entities/missing-activity-origin.toml"
                    } else if fault == 10 {
                        "assets/entities/personnel_cruiser.toml"
                    } else {
                        "assets/entities/alliance_destroyer.toml"
                    }
                    .into(),
                    name: "personnel-ship".into(),
                    overrides: (fault == 10)
                        .then(|| toml::from_str("[repair]\nrepair_team_count=-1\n").unwrap()),
                    ..Default::default()
                });
            }
        }
        assert!(
            crate::snapshot::activities::validate(&world, &forged).is_err(),
            "case{fault}"
        );
        assert_eq!(capture(&world), before);
        assert_eq!(crate::sim_digest::world_digest(&world), digest);
    }
    world.resource_mut::<WorldContentRuntime>().plan_choice = None;
    world.resource_mut::<WorldContentRuntime>().situations = Default::default();
    let mut report = RestoreReport::default();
    crate::snapshot::application::restore_scenario(&mut world, &decoded, &mut report);
    assert!(report.is_complete());
    assert_eq!(capture(&world), before);
    assert_eq!(crate::sim_digest::world_digest(&world), digest);
}
#[test]
fn released_historical_intent_survives_declared_actor_removal_without_rearming_or_settlement() {
    let (mut world, actor, _) = prepared();
    assert!(world.despawn(actor));
    let saved = capture(&world);
    assert!(saved.entities.is_empty());
    crate::snapshot::activities::validate(&world, &saved).unwrap();
    let scenario = saved.scenario.as_ref().unwrap();
    assert!(scenario
        .situations
        .instances
        .values()
        .all(|row| row.accepted.is_none() && row.settled.is_none()));
    assert_eq!(scenario.plan_choice.as_ref().unwrap().released.len(), 1);
    let mut forged = saved.clone();
    forged
        .scenario
        .as_mut()
        .unwrap()
        .situations
        .instances
        .clear();
    assert!(crate::snapshot::activities::validate(&world, &forged).is_err());
}

#[test]
fn body_only_restore_cannot_rewind_the_job_beneath_an_accepted_world_frontier() {
    let (mut world, actor, before_dispatch) = prepared();
    let request = before_dispatch
        .scenario
        .as_ref()
        .unwrap()
        .situations
        .instances
        .values()
        .next()
        .unwrap()
        .request
        .clone();
    let phoenix_sim_world::world::activities::ActivityRequest::Repair {
        owner,
        team,
        request,
        ..
    } = request
    else {
        panic!("released Repair leaf")
    };
    {
        let mut power = world
            .get_mut::<crate::ship::power::ShipPowerSystem>(actor)
            .unwrap();
        let capacity = power
            .0
            .config()
            .consumers
            .iter()
            .find(|row| row.system == owner)
            .unwrap()
            .reserve
            .capacity;
        power
            .0
            .network_mut()
            .consumers
            .get_mut(&owner)
            .unwrap()
            .energy = capacity;
    }
    world
        .get_mut::<crate::ship::compartment_damage::ShipDamage>(actor)
        .unwrap()
        .state
        .compartments
        .get_mut("engineering")
        .unwrap()
        .hp -= 1.0;
    world
        .get_mut::<crate::core::messages::AdmittedCommands>(actor)
        .unwrap()
        .0
        .push(crate::core::messages::AdmittedCommand {
            target: owner,
            payload: crate::core::messages::SystemControlPayload::DispatchRepairTeam {
                team_idx: team as u8,
                target: request,
            },
            response_token: None,
            feedback_correlation: None,
        });
    world
        .run_system_once(crate::console::repair::dispatch::handle_dispatch_repair_team)
        .unwrap();
    world
        .get_mut::<crate::core::messages::AdmittedCommands>(actor)
        .unwrap()
        .0
        .clear();
    assert!(world
        .resource::<WorldContentRuntime>()
        .situations
        .instances
        .values()
        .next()
        .unwrap()
        .accepted
        .is_some());
    let accepted = capture(&world);
    let digest = crate::sim_digest::world_digest(&world);
    assert!(
        crate::snapshot::validate_activity_single_row(&world, &before_dispatch.entities[0])
            .is_err()
    );
    crate::snapshot::apply_entity_state(&mut world, actor, &before_dispatch.entities[0], 20);
    assert_eq!(capture(&world), accepted);
    assert_eq!(crate::sim_digest::world_digest(&world), digest);
    // A full candidate carries its matching older World choice too; this
    // guard deliberately belongs only to the body-only application path.
    crate::snapshot::activities::validate(&world, &before_dispatch).unwrap();
    assert!(crate::snapshot::validate_activity_single_row(&world, &accepted.entities[0]).is_ok());
    assert!(world.despawn(actor));
    assert!(crate::snapshot::validate_activity_single_row(&world, &accepted.entities[0]).is_err());
}

#[test]
fn immutable_recipe_missing_capability_cannot_borrow_live_replacement_equipment() {
    let (mut world, actor, mut saved) = prepared();
    // Keep the real live Cruiser machinery present while the accepted recipe
    // supplies no Repair capability. Per-field fallback would wrongly accept it.
    assert!(world
        .get::<crate::console::repair::server::ShipRepairTeams>(actor)
        .is_some());
    world
        .resource_mut::<crate::world::config::WorldConfig>()
        .entities[0]
        .template_path = "assets/entities/star_sun.toml".into();
    saved
        .entities
        .iter_mut()
        .find(|row| row.uuid == "personnel-ship")
        .unwrap()
        .spawn = None;
    let before = capture(&world);
    let digest = crate::sim_digest::world_digest(&world);
    let error = crate::snapshot::activities::validate(&world, &saved).unwrap_err();
    assert!(error.contains("no compatible method"), "{error}");
    assert_eq!(capture(&world), before);
    assert_eq!(crate::sim_digest::world_digest(&world), digest);
}

#[test]
fn accepted_root_promise_and_its_original_clocks_preflight_before_any_writes() {
    let (mut world, _, _) = prepared();
    let mut base = world
        .resource::<crate::world::config::WorldConfig>()
        .clone();
    let mut source = base.planning.take().unwrap();
    source.required_commitment = Some("assistance".into());
    let (compiled, _) =
        phoenix_sim_world::world::planning::CompiledPlanning::load(source.clone(), |path| {
            crate::content_fs::read_to_string(path).ok()
        })
        .unwrap();
    base.planning = Some(source);
    base.compiled_planning = Some(compiled);
    world.insert_resource(base);
    let mut runtime = world.resource_mut::<WorldContentRuntime>();
    let chosen = runtime.plan_choice.as_ref().unwrap().chosen_tick;
    runtime.plan_choice.as_mut().unwrap().required_commitment = Some("assistance".into());
    runtime
        .commitments
        .record("assistance", "party", "terms", "resolution", chosen - 1)
        .unwrap();
    drop(runtime);
    let saved = capture(&world);
    crate::snapshot::activities::validate(&world, &saved).unwrap();
    let before = capture(&world);
    let digest = crate::sim_digest::world_digest(&world);
    for fault in 0..6 {
        let mut forged = saved.clone();
        let state = forged.scenario.as_mut().unwrap();
        match fault {
            0 => state.commitments.records.clear(),
            1 => {
                state
                    .commitments
                    .records
                    .push(state.commitments.records[0].clone());
            }
            2 => state.commitments.records[0].made_at_tick = chosen + 1,
            3 => {
                state.commitments.records[0].state =
                    crate::world::commitments::CommitmentState::Kept;
                state.commitments.records[0].resolved_at_tick = Some(chosen - 1);
            }
            4 => state.plan_choice.as_mut().unwrap().required_commitment = None,
            _ => {
                state.plan_choice = None;
                state.situations.instances.clear();
            }
        }
        assert!(
            crate::snapshot::activities::validate(&world, &forged).is_err(),
            "forged promise binding {fault}"
        );
        assert!(!restore(&mut world, &forged).is_complete());
        assert_eq!(capture(&world), before);
        assert_eq!(crate::sim_digest::world_digest(&world), digest);
    }
}

#[test]
fn accepted_repair_intent_projection_cannot_invent_a_target_prerequisite_or_owner() {
    let (mut world, _, saved) = prepared();
    let before = capture(&world);
    for fault in 0..5 {
        let mut forged = saved.clone();
        if fault == 4 {
            forged.scenario = None;
        } else {
            let board = forged
                .entities
                .iter_mut()
                .flat_map(|row| &mut row.blackboards)
                .find_map(|(_, board)| match board {
                    crate::core::messages::SystemBlackboard::Repair(board)
                        if !board.planned_intents.is_empty() =>
                    {
                        Some(board)
                    }
                    _ => None,
                })
                .expect("actual accepted graph publishes its own intent");
            match fault {
                0 => board.planned_intents[0].id = "foreign/node".into(),
                1 => board.planned_intents[0].chosen_tick += 1,
                2 => {
                    board.planned_intents[0].target =
                        phoenix_model::damage::RepairWorkTarget::Compartment {
                            compartment: "foreign".into(),
                        }
                }
                _ => board.planned_intents[0].prerequisites.push(
                    phoenix_model::activity::PlannedPrerequisite {
                        id: "invented prerequisite".into(),
                        condition: phoenix_model::activity::PlannedPrerequisiteKind::Received,
                        satisfied: true,
                    },
                ),
            }
        }
        assert!(crate::snapshot::activities::validate(&world, &forged).is_err());
        assert!(!restore(&mut world, &forged).is_complete());
        assert!(
            capture(&world) == before,
            "intent refusal{fault} must write nothing"
        );
    }
}
