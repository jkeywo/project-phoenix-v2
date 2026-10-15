//! Actual AI emission, accepted assignment and committed physical result seam.
use super::*;
use crate::core::messages::{AdmittedCommands, RepairTarget, SystemControlPayload};
use bevy::ecs::system::RunSystemOnce;
use phoenix_model::damage::{ComponentCondition, RepairWorkTarget};
use phoenix_sim_world::world::activities::*;

#[test]
fn ai_group_work_binds_accepted_job_but_only_the_named_component_credits_the_situation() {
    let config = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    let initial = crate::ship::compartment_damage::ShipDamage::from_config(&config)
        .unwrap()
        .unwrap();
    let owner = repair_system_id();
    let ship_config = config.ship_config.as_ref().unwrap();
    let (system, parts) = initial
        .definition
        .components
        .iter()
        .find(|(system, parts)| {
            *system != &owner
                // A group visits its ordinary router's actual first equipment
                // room. Engineering owns machinery in multiple compartments;
                // a team home alone does not establish this request's scope.
                && crate::console::repair::dispatch::resolve_physical_repair_target(
                    &repair_group(system, ship_config).target(), ship_config, &initial,
                ).is_some_and(|selected| initial.definition.interior.placements.get(&selected)
                    == initial.definition.interior.placements.get(*system))
                && parts.len() >= 2
                && parts.values().all(|part| part.field_replaceable)
        })
        .unwrap();
    let first = parts.keys().next().unwrap().clone();
    let last = parts.keys().next_back().unwrap().clone();
    assert_ne!(first, last);
    let system = system.clone();
    let target = RepairWorkTarget::Component {
        system: system.clone(),
        component: last.clone(),
    };
    let group = repair_group(&system, config.ship_config.as_ref().unwrap());
    let mut app = App::new();
    crate::ai::host::register_ai_host_env(&mut app);
    app.insert_resource(crate::world::activities::declared_actor_fixture(
        "activity-repair-actor",
        "assets/entities/alliance_cruiser.toml",
    ));
    app.insert_resource(crate::sim_tick::SimTick(1));
    let mut time = Time::<()>::default();
    time.advance_by(std::time::Duration::from_millis(100));
    app.insert_resource(time);
    app.add_message::<DeliveredCoordination>()
        .add_message::<OrderedCoordinationPopup>();
    let actor = crate::entities::spawner::spawn_entity(
        &mut app.world_mut().commands(),
        &config,
        Vec3::ZERO,
        "activity-repair-actor".into(),
        None,
    );
    app.world_mut().flush();
    {
        let mut damage = app
            .world_mut()
            .get_mut::<crate::ship::compartment_damage::ShipDamage>(actor)
            .unwrap();
        for id in [&first, &last] {
            let part = damage
                .state
                .components
                .get_mut(&system)
                .unwrap()
                .get_mut(id)
                .unwrap();
            part.condition = ComponentCondition::Damaged;
            part.last_source = Some("activity-test-impact".into());
        }
        damage.state.validate(&damage.definition).unwrap();
    }
    app.world_mut()
        .get_mut::<ShipSystemControlSources>(actor)
        .unwrap()
        .0
        .set(
            owner.clone(),
            crate::ship::control_source::ControlSource::Ai,
        );
    {
        let mut power = app
            .world_mut()
            .get_mut::<crate::ship::power::ShipPowerSystem>(actor)
            .unwrap();
        let capacity = power
            .0
            .config()
            .consumers
            .iter()
            .find(|profile| profile.system == owner)
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
    let definition = SituationDefinition {
        id: "component-online".into(),
        initial: "useful".into(),
        state: vec![SituationState {
            id: "useful".into(),
            when: SituationCondition::Achieved,
            call: None,
            next: None,
        }],
    };
    let binding = SituationConfig {
        id: "restore-required-component".into(),
        definition: definition.id.clone(),
        request: ActivityRequest::Repair {
            actor: "activity-repair-actor".into(),
            owner: owner.clone(),
            team: 0,
            request: group.target(),
            target: target.clone(),
            goal: RepairGoal::Full,
        },
    };
    {
        let mut world = app
            .world_mut()
            .resource_mut::<crate::world::server::WorldContentRuntime>();
        world.situation_definitions = vec![definition];
        world.situation_configs = vec![binding];
    }
    let ship = &app.world().get::<ShipConfigComponent>(actor).unwrap().0;
    let address = crate::ship::coordination::address_for_system(ship, &owner).unwrap();
    app.world_mut()
        .resource_mut::<Messages<DeliveredCoordination>>()
        .write(DeliveredCoordination {
            source_entity: actor,
            address,
            payload: CoordinationPayload::RepairRequest {
                observation: None,
                system_id: system.clone(),
                group: group.clone(),
                station_label: "reported damage".into(),
                tier: DamageTier::Disabled,
                deficit: Some(10.0),
            },
            presentation: crate::core::messages::CoordinationPresentation::titled(
                "coordination.repair.title",
            ),
            delivery: CoordinationDelivery::Ai,
        });
    app.world_mut()
        .run_system_once(receive_repair_coordination)
        .unwrap();
    app.world_mut().run_system_once(operate_repair_ai).unwrap();
    assert!(app.world().get::<crate::core::messages::AdmittedCommands>(actor).unwrap().0.iter().any(|command| matches!(&command.payload,crate::core::messages::SystemControlPayload::DispatchRepairTeam {team_idx:0,target} if target == &group.target())));
    app.world_mut()
        .run_system_once(crate::console::repair::dispatch::handle_dispatch_repair_team)
        .unwrap();
    app.world_mut()
        .get_mut::<crate::core::messages::AdmittedCommands>(actor)
        .unwrap()
        .0
        .clear();
    let accepted = app
        .world()
        .resource::<crate::world::server::WorldContentRuntime>()
        .situations
        .instances["restore-required-component"]
        .accepted
        .clone()
        .unwrap();
    assert_eq!(
        app.world()
            .get::<ShipRepairTeams>(actor)
            .unwrap()
            .0
            .job_state()
            .teams[0]
            .as_ref()
            .unwrap()
            .id,
        accepted.physical_job().unwrap()
    );
    let mut other_completed = false;
    let mut goal_completed = false;
    for tick in 2..=2400 {
        app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 = tick;
        app.world_mut().run_system_once(tick_repair_teams).unwrap();
        let damage = app
            .world()
            .get::<crate::ship::compartment_damage::ShipDamage>(actor)
            .unwrap();
        let first_done =
            damage.state.components[&system][&first].condition == ComponentCondition::Healthy;
        let last_done =
            damage.state.components[&system][&last].condition == ComponentCondition::Healthy;
        let runtime = app
            .world()
            .resource::<crate::world::server::WorldContentRuntime>();
        if first_done && !last_done {
            other_completed = true;
            assert!(runtime
                .pending_world_events
                .iter()
                .all(|event| !matches!(event, crate::world::content::WorldEvent::Activity(_))));
            assert!(runtime.situations.instances["restore-required-component"]
                .achieved
                .is_none());
        }
        if last_done {
            goal_completed = true;
            assert!(runtime.pending_world_events.iter().any(|event| matches!(event,crate::world::content::WorldEvent::Activity(fact) if fact.scope == accepted && matches!(&fact.outcome, phoenix_model::activity::ActivityOutcome::RepairAchieved { target:actual,..} if actual == &target))));
            break;
        }
    }
    assert!(
        other_completed,
        "ordinary group sweep first completed a different component"
    );
    assert!(
        goal_completed,
        "real finite parts/tool work reaches the named full remedy"
    );
    let mut runtime = app
        .world_mut()
        .resource_mut::<crate::world::server::WorldContentRuntime>();
    crate::world::activities::acknowledge_pending(&mut runtime, 2400);
    assert!(runtime.situations.instances["restore-required-component"]
        .achieved
        .is_some());
    assert_eq!(
        runtime
            .situations
            .ready(&runtime.situation_definitions)
            .len(),
        1
    );
}

#[test]
fn future_intent_holds_exact_generic_visit_footprint_for_any_team_but_leaves_other_work_available()
{
    let recipe = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    let damage = crate::ship::compartment_damage::ShipDamage::from_config(&recipe)
        .unwrap()
        .unwrap();
    let config = recipe.ship_config.as_ref().unwrap();
    let groups = config
        .systems
        .iter()
        .map(|system| repair_group(&system.id, config))
        .collect::<std::collections::BTreeSet<_>>();
    let visits = groups
        .iter()
        .filter_map(|group| {
            let system = crate::console::repair::dispatch::resolve_physical_repair_target(
                &group.target(),
                config,
                &damage,
            )?;
            Some((
                group.clone(),
                damage.definition.interior.placements.get(&system)?.clone(),
            ))
        })
        .collect::<Vec<_>>();
    let (held, room) = visits.first().unwrap();
    let (unrelated, other_room) = visits
        .iter()
        .find(|(_, candidate)| candidate != room)
        .expect("shipped hull has independent visit footprints");
    assert_ne!(room, other_room);
    let target = RepairWorkTarget::Compartment {
        compartment: room.clone(),
    };
    let intent = phoenix_model::activity::PlannedRepairIntent {
        id: "htn/intent/future".into(),
        chosen_tick: 10,
        released_tick: None,
        team: 0,
        request: RepairTarget::Work(target.clone()),
        target,
        goal: phoenix_model::activity::RepairGoal::Structure,
        prerequisites: vec![phoenix_model::activity::PlannedPrerequisite {
            id: "htn/intent/supply".into(),
            condition: phoenix_model::activity::PlannedPrerequisiteKind::Received,
            satisfied: false,
        }],
        disposition: None,
    };
    assert!(generic_repair_preempts_intent(
        held,
        std::slice::from_ref(&intent),
        config,
        &damage
    ));
    assert!(!generic_repair_preempts_intent(
        unrelated,
        std::slice::from_ref(&intent),
        config,
        &damage
    ));
    let mut changed = intent.clone();
    changed.disposition = Some(phoenix_model::activity::PlannedRepairDisposition {
        kind: phoenix_model::activity::PlannedRepairDispositionKind::RestoredExternally,
        tick: 11,
        consequence_applied: None,
    });
    assert!(
        !generic_repair_preempts_intent(held, &[changed], config, &damage),
        "closed work does not suppress later ordinary repairs"
    );
}

#[test]
fn ordinary_ai_starts_unrelated_reported_work_while_the_future_target_waits() {
    let recipe = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    let damage = crate::ship::compartment_damage::ShipDamage::from_config(&recipe)
        .unwrap()
        .unwrap();
    let config = recipe.ship_config.as_ref().unwrap();
    let visits = config
        .systems
        .iter()
        .map(|system| repair_group(&system.id, config))
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .filter_map(|group| {
            let system = crate::console::repair::dispatch::resolve_physical_repair_target(
                &group.target(),
                config,
                &damage,
            )?;
            let room = damage.definition.interior.placements.get(&system)?.clone();
            Some((group, system, room))
        })
        .collect::<Vec<_>>();
    let (held, held_system, held_room) = visits
        .iter()
        .find(|(group, _, _)| !matches!(group, RepairGroup::Core))
        .unwrap()
        .clone();
    let (unrelated, unrelated_system, unrelated_room) = visits
        .iter()
        .find(|(group, _, room)| !matches!(group, RepairGroup::Core) && room != &held_room)
        .unwrap()
        .clone();
    let owner = repair_system_id();
    let mut app = App::new();
    crate::ai::host::register_ai_host_env(&mut app);
    app.insert_resource(crate::sim_tick::SimTick(10));
    app.add_message::<DeliveredCoordination>()
        .add_message::<OrderedCoordinationPopup>();
    let actor = crate::entities::spawner::spawn_entity(
        &mut app.world_mut().commands(),
        &recipe,
        Vec3::ZERO,
        "intent-actor".into(),
        None,
    );
    app.world_mut().flush();
    app.world_mut()
        .get_mut::<ShipSystemControlSources>(actor)
        .unwrap()
        .0
        .set(
            owner.clone(),
            crate::ship::control_source::ControlSource::Ai,
        );
    // Owner-fixture damage establishes two real routing candidates. This test
    // exercises ordinary selection/admission, not World discovery provenance.
    {
        let mut state = app
            .world_mut()
            .get_mut::<crate::ship::compartment_damage::ShipDamage>(actor)
            .unwrap();
        state.state.compartments.get_mut(&held_room).unwrap().hp -= 1.0;
        state
            .state
            .compartments
            .get_mut(&unrelated_room)
            .unwrap()
            .hp -= 1.0;
    }
    let target = RepairWorkTarget::Compartment {
        compartment: held_room,
    };
    let intent = phoenix_model::activity::PlannedRepairIntent {
        id: "htn/intent/future".into(),
        chosen_tick: 10,
        released_tick: None,
        team: 0,
        request: RepairTarget::Work(target.clone()),
        target,
        goal: RepairGoal::Structure,
        prerequisites: vec![phoenix_model::activity::PlannedPrerequisite {
            id: "htn/intent/delivery".into(),
            condition: phoenix_model::activity::PlannedPrerequisiteKind::Received,
            satisfied: false,
        }],
        disposition: None,
    };
    app.world_mut()
        .get_mut::<crate::server_app::ShipSystemBlackboards>(actor)
        .unwrap()
        .0
        .insert(
            owner.clone(),
            SystemBlackboard::Repair(RepairBlackboard {
                planned_intents: vec![intent],
                ..Default::default()
            }),
        );
    let address = crate::ship::coordination::address_for_system(config, &owner).unwrap();
    for (group, system) in [(&held, &held_system), (&unrelated, &unrelated_system)] {
        app.world_mut()
            .resource_mut::<Messages<DeliveredCoordination>>()
            .write(DeliveredCoordination {
                source_entity: actor,
                address: address.clone(),
                payload: CoordinationPayload::RepairRequest {
                    observation: None,
                    system_id: system.clone(),
                    group: group.clone(),
                    station_label: "reported damage".into(),
                    tier: DamageTier::Damaged,
                    deficit: Some(1.0),
                },
                presentation: crate::core::messages::CoordinationPresentation::titled(
                    "coordination.repair.title",
                ),
                delivery: CoordinationDelivery::Ai,
            });
    }
    app.world_mut()
        .run_system_once(receive_repair_coordination)
        .unwrap();
    app.world_mut().run_system_once(operate_repair_ai).unwrap();
    let orders = &app.world().get::<AdmittedCommands>(actor).unwrap().0;
    assert!(orders.iter().any(|order|matches!(&order.payload,SystemControlPayload::DispatchRepairTeam{target,..} if target==&unrelated.target())));
    assert!(orders.iter().all(|order|!matches!(&order.payload,SystemControlPayload::DispatchRepairTeam{target,..} if target==&held.target())),"all free teams respect exact future footprint");
    app.world_mut()
        .run_system_once(crate::console::repair::dispatch::handle_dispatch_repair_team)
        .unwrap();
    let jobs = app
        .world()
        .get::<ShipRepairTeams>(actor)
        .unwrap()
        .0
        .work_list();
    assert!(
        jobs.iter()
            .any(|job| job.target_system.as_ref().is_some_and(|system| damage
                .definition
                .interior
                .placements
                .get(system)
                == Some(&unrelated_room))),
        "the ordinary router actually starts independent work"
    );
}
