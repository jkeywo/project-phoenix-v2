//! Actual AI emission, accepted assignment and committed physical result seam.
use super::*;
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
