//! Real accepted recipes, ordinary report/AI/admission and committed work.
use super::*;
use crate::{
    console::repair::server::{
        operate_repair_ai, publish_repair_blackboard, receive_repair_coordination,
        tick_repair_teams, ShipRepairTeams,
    },
    core::messages::*,
    ship_plugin::{
        CoordinationDelivery, DeliveredCoordination, OrderedCoordinationPopup, ShipConfigComponent,
        ShipSystemControlSources,
    },
};
use bevy::{ecs::system::RunSystemOnce, prelude::*};
const ROOT: &str = "tests/fixtures/maintenance-htn-world.toml";
#[path = "planning_temporal_tests.rs"]
mod temporal;
fn setup(count: u32) -> (App, Entity) {
    let reader = crate::world::load::OverlayFsReader;
    let mut base = crate::world::load::load(crate::world::load::LoadRequest::new(
        ROOT,
        &reader,
        &crate::world::script::load::NoSiblingScripts,
        crate::world::load::LoadPolicy::Merge,
    ))
    .unwrap()
    .config;
    base.name_to_uuid
        .insert("maintenance-ship".into(), "actual-maintenance".into());
    let mut config = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    config.repair.as_mut().unwrap().repair_team_count = count;
    config
        .interior
        .as_mut()
        .unwrap()
        .team_homes
        .truncate(count as usize);
    phoenix_sim_gameplay::ship::interior::InteriorGraph::compile(
        config.interior.as_ref().unwrap(),
        config.ship_config.as_ref().unwrap(),
        count as usize,
    )
    .unwrap();
    let mut app = App::new();
    crate::ai::host::register_ai_host_env(&mut app);
    app.insert_resource(base)
        .insert_resource(WorldContentRuntime::default())
        .insert_resource(crate::sim_tick::SimTick(20));
    let mut time = Time::<()>::default();
    time.advance_by(std::time::Duration::from_millis(100));
    app.insert_resource(time);
    app.add_message::<DeliveredCoordination>()
        .add_message::<OrderedCoordinationPopup>();
    let actor = crate::entities::spawner::spawn_entity(
        &mut app.world_mut().commands(),
        &config,
        Vec3::ZERO,
        "actual-maintenance".into(),
        None,
    );
    app.world_mut().flush();
    app.world_mut()
        .get_mut::<ShipSystemControlSources>(actor)
        .unwrap()
        .0
        .set(
            SystemId("repair".into()),
            crate::ship::control_source::ControlSource::Ai,
        );
    (app, actor)
}
fn advance(app: &mut App, actor: Entity) {
    let base = app.world().resource::<WorldConfig>().clone();
    let tick = app.world().resource::<crate::sim_tick::SimTick>().0;
    let config = app
        .world()
        .get::<ShipConfigComponent>(actor)
        .unwrap()
        .0
        .clone();
    let damage = app
        .world()
        .get::<crate::ship::compartment_damage::ShipDamage>(actor)
        .unwrap()
        .clone();
    let teams = app.world().get::<ShipRepairTeams>(actor).unwrap();
    let owner = teams.0.job_state().owner.clone();
    let count = teams.0.slots().len();
    let logistics = app
        .world()
        .get::<crate::resources::logistics_server::CompiledShipLogistics>(actor)
        .unwrap()
        .0
        .clone();
    let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
    advance_for_actor(
        &mut runtime,
        &base,
        "actual-maintenance",
        None,
        tick,
        |request| match request {
            ActivityRequest::Repair { .. } => crate::world::activities::repair_binding_supported(
                request, &config, &damage, &owner, count,
            ),
            ActivityRequest::Communication { .. } => false,
            ActivityRequest::Delivery { .. } => {
                crate::world::activities::delivery_binding_supported(request, &logistics)
            }
        },
    )
    .unwrap();
}
#[test]
fn actual_hull_alternatives_and_owner_publications_preserve_physical_state_and_scope() {
    for (count, method, team) in [(2, "spare-team", 1), (1, "primary-team", 0)] {
        let (mut app, actor) = setup(count);
        let before = app
            .world()
            .get::<ShipRepairTeams>(actor)
            .unwrap()
            .0
            .save_state();
        let stock = app
            .world()
            .get::<crate::infrastructure::InfrastructureCondition>(actor)
            .unwrap()
            .clone();
        advance(&mut app, actor);
        assert_eq!(
            app.world()
                .get::<ShipRepairTeams>(actor)
                .unwrap()
                .0
                .save_state(),
            before
        );
        assert_eq!(
            app.world()
                .get::<crate::infrastructure::InfrastructureCondition>(actor),
            Some(&stock)
        );
        let runtime = app.world().resource::<WorldContentRuntime>();
        let choice = runtime.plan_choice.as_ref().unwrap();
        assert_eq!(choice.methods["root"].method, method);
        assert_eq!(choice.actor_uuid, "actual-maintenance");
        assert_eq!(choice.released.len(), 1);
        let base = app.world().resource::<WorldConfig>();
        let rows = repair_requests(
            Some(runtime),
            Some(base),
            "actual-maintenance",
            &SystemId("repair".into()),
            20,
        );
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].team, team);
        assert_eq!(rows[0].released_tick, 20);
        assert!(repair_requests(
            Some(runtime),
            Some(base),
            "foreign",
            &SystemId("repair".into()),
            20
        )
        .is_empty());
        assert!(delivery_requests(
            Some(runtime),
            Some(base),
            "actual-maintenance",
            &SystemId("logistics".into()),
            20
        )
        .is_empty());
        app.world_mut()
            .run_system_once(publish_repair_blackboard)
            .unwrap();
        let boards = &app
            .world()
            .get::<crate::server_app::ShipSystemBlackboards>(actor)
            .unwrap()
            .0;
        let SystemBlackboard::Repair(board) = boards.get(&SystemId("repair".into())).unwrap()
        else {
            panic!("actual Repair publication")
        };
        assert_eq!(board.planned_requests, rows);
        let choice = app
            .world()
            .resource::<WorldContentRuntime>()
            .plan_choice
            .clone();
        let original = app
            .world()
            .resource::<WorldContentRuntime>()
            .situations
            .clone();
        let decoded = serde_json::from_slice(
            &serde_json::to_vec(&(choice.clone(), original.clone())).unwrap(),
        )
        .unwrap();
        let (restored_choice, restored_instances): (
            Option<phoenix_sim_world::world::planning::PlanChoice>,
            phoenix_sim_world::world::activities::SituationInstances,
        ) = decoded;
        let compiled = app
            .world()
            .resource::<WorldConfig>()
            .compiled_planning
            .as_ref()
            .unwrap();
        for tick in 20..380 {
            compiled
                .validate_history(restored_choice.as_ref().unwrap(), &restored_instances, tick)
                .unwrap();
        }
        assert_eq!(restored_choice, choice);
        assert_eq!(restored_instances, original);
    }
}
#[test]
fn instruction_does_not_diagnose_damage_and_only_actual_acknowledged_work_releases_delivery() {
    let (mut app, actor) = setup(2);
    advance(&mut app, actor);
    app.world_mut()
        .run_system_once(publish_repair_blackboard)
        .unwrap();
    app.world_mut().run_system_once(operate_repair_ai).unwrap();
    assert!(
        app.world()
            .get::<AdmittedCommands>(actor)
            .unwrap()
            .0
            .is_empty(),
        "mission text supplies no damage report"
    );
    let owner = SystemId("repair".into());
    let room = "engineering";
    app.world_mut()
        .get_mut::<crate::ship::compartment_damage::ShipDamage>(actor)
        .unwrap()
        .state
        .compartments
        .get_mut(room)
        .unwrap()
        .hp -= 1.0;
    let (address, visit, group) = {
        let config = &app.world().get::<ShipConfigComponent>(actor).unwrap().0;
        let damage = app
            .world()
            .get::<crate::ship::compartment_damage::ShipDamage>(actor)
            .unwrap();
        let visit = crate::jobs::activity_bindings::resolve_repair_visit(
            &RepairTarget::Work(phoenix_model::damage::RepairWorkTarget::Compartment {
                compartment: room.into(),
            }),
            config,
            &damage.definition,
        )
        .unwrap();
        (
            crate::ship::coordination::address_for_system(config, &owner).unwrap(),
            visit.clone(),
            crate::modifiers::repair_teams::repair_group(&visit, config),
        )
    };
    app.world_mut()
        .resource_mut::<Messages<DeliveredCoordination>>()
        .write(DeliveredCoordination {
            source_entity: actor,
            address,
            payload: CoordinationPayload::RepairRequest {
                observation: None,
                system_id: visit,
                group,
                station_label: "actual damage report".into(),
                tier: crate::ship::damage::DamageTier::Damaged,
                deficit: Some(1.0),
            },
            presentation: CoordinationPresentation::titled("coordination.repair.title"),
            delivery: CoordinationDelivery::Ai,
        });
    app.world_mut()
        .run_system_once(receive_repair_coordination)
        .unwrap();
    app.world_mut().run_system_once(operate_repair_ai).unwrap();
    assert!(app
        .world()
        .get::<AdmittedCommands>(actor)
        .unwrap()
        .0
        .iter()
        .any(|row| matches!(
            row.payload,
            SystemControlPayload::DispatchRepairTeam { team_idx: 1, .. }
        )));
    let initial_parts = app
        .world()
        .get::<crate::infrastructure::InfrastructureCondition>(actor)
        .unwrap()
        .clone();
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
    app.world_mut()
        .run_system_once(crate::console::repair::dispatch::handle_dispatch_repair_team)
        .unwrap();
    app.world_mut()
        .get_mut::<AdmittedCommands>(actor)
        .unwrap()
        .0
        .clear();
    let runtime = app.world().resource::<WorldContentRuntime>();
    let id = runtime.situations.instances.keys().next().unwrap().clone();
    assert!(runtime.situations.instances[&id].accepted.is_some());
    advance(&mut app, actor);
    assert_eq!(
        app.world()
            .resource::<WorldContentRuntime>()
            .situations
            .instances
            .len(),
        1,
        "acceptance is not useful work"
    );
    let mut achieved = None;
    for tick in 21..=2400 {
        app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 = tick;
        app.world_mut().run_system_once(tick_repair_teams).unwrap();
        if app.world().resource::<WorldContentRuntime>().pending_world_events.iter().any(|event|matches!(event,crate::world::content::WorldEvent::Activity(fact) if matches!(fact.outcome,phoenix_model::activity::ActivityOutcome::RepairAchieved{..}))) {achieved=Some(tick);break;}
    }
    let tick = achieved.expect("real finite parts/tool work must commit");
    advance(&mut app, actor);
    assert_eq!(
        app.world()
            .resource::<WorldContentRuntime>()
            .situations
            .instances
            .len(),
        1,
        "unacknowledged result is not a dependency"
    );
    crate::world::activities::acknowledge_pending(
        &mut app.world_mut().resource_mut::<WorldContentRuntime>(),
        tick,
    );
    advance(&mut app, actor);
    let runtime = app.world().resource::<WorldContentRuntime>();
    assert_eq!(runtime.situations.instances.len(), 2);
    let base = app.world().resource::<WorldConfig>();
    let requests = delivery_requests(
        Some(runtime),
        Some(base),
        "actual-maintenance",
        &SystemId("logistics".into()),
        tick,
    );
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].amount, 4);
    assert_eq!(requests[0].released_tick, tick);
    assert_ne!(
        app.world()
            .get::<crate::infrastructure::InfrastructureCondition>(actor),
        Some(&initial_parts),
        "real remedy consumed finite parts"
    );
}
