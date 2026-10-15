use super::*;
use crate::server_app::ShipBoost;
use crate::ship::components::BoostConfigResource;
use phoenix_sim_gameplay::ship::impulse_boost_systems::BoostActuation;

fn boosted_fixture() -> (App, Entity, SystemId) {
    let (mut app, entity) = fixture();
    let authored = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    let boost = authored.helm_console.unwrap().boost.unwrap();
    let owner = SystemId("helm-boost".into());
    app.world_mut().entity_mut(entity).insert((
        ShipBoost::default(),
        crate::ship::helm::BoostCommand::default(),
        BoostConfigResource {
            enabled: true,
            multiplier: boost.multiplier,
            steering_multiplier: boost.steering_multiplier,
            active_duration: boost.active_duration,
            recharge_duration: boost.recharge_duration,
        },
    ));
    let thrust = app
        .world()
        .get::<CompiledHelmControl>(entity)
        .unwrap()
        .0
        .thrust
        .clone();
    command(
        &mut app,
        entity,
        thrust.clone(),
        SystemControlPayload::SetHelmMode {
            mode: HelmControlMode::Direct,
        },
    );
    command(
        &mut app,
        entity,
        thrust,
        SystemControlPayload::SetThrust { value: 1.0 },
    );
    command(
        &mut app,
        entity,
        owner.clone(),
        SystemControlPayload::SetBoost { active: true },
    );
    (app, entity, owner)
}
fn boosted_step(app: &mut App, entity: Entity) {
    app.world_mut()
        .run_system_once(crate::ship::helm_admission::process_helm_inputs)
        .unwrap();
    app.world_mut()
        .run_system_once(crate::ship_plugin::apply_helm_commands)
        .unwrap();
    step(app, entity);
}
fn reserve(app: &App, entity: Entity, owner: &SystemId) -> f32 {
    app.world()
        .get::<crate::ship::power::ShipPowerSystem>(entity)
        .unwrap()
        .0
        .network()
        .consumers[owner]
        .energy
}
#[test]
fn sustained_boost_outlasts_legacy_duration_and_partial_reserve_reengages() {
    let (mut app, entity, owner) = boosted_fixture();
    let before = reserve(&app, entity, &owner);
    for _ in 0..240 {
        boosted_step(&mut app, entity);
    }
    assert!(app.world().get::<ShipBoost>(entity).unwrap().0.active);
    assert_eq!(app.world().get::<BoostActuation>(entity).unwrap().0, 1.0);
    assert!(reserve(&app, entity, &owner) < before);
    assert!(reserve(&app, entity, &owner) > 0.0);
    assert!(
        app.world()
            .get::<ShipPhysics>(entity)
            .unwrap()
            .forward_speed
            > 10.0
    );
    command(
        &mut app,
        entity,
        owner.clone(),
        SystemControlPayload::SetBoost { active: false },
    );
    boosted_step(&mut app, entity);
    assert!(!app.world().get::<ShipBoost>(entity).unwrap().0.active);
    assert_eq!(app.world().get::<BoostActuation>(entity).unwrap().0, 0.0);
    let partial = reserve(&app, entity, &owner);
    command(
        &mut app,
        entity,
        owner.clone(),
        SystemControlPayload::SetBoost { active: true },
    );
    boosted_step(&mut app, entity);
    assert!(app.world().get::<ShipBoost>(entity).unwrap().0.active);
    assert!(reserve(&app, entity, &owner) < partial);
    assert_eq!(app.world().get::<BoostActuation>(entity).unwrap().0, 1.0);
}
#[test]
fn unavailable_exact_boost_withholds_payment_and_multiplier_but_is_not_another_intent_writer() {
    for reason in ["offline", "missing", "empty", "thermal", "destroyed"] {
        let (mut app, entity, owner) = boosted_fixture();
        app.world_mut()
            .run_system_once(crate::ship::helm_admission::process_helm_inputs)
            .unwrap();
        app.world_mut()
            .run_system_once(crate::ship_plugin::apply_helm_commands)
            .unwrap();
        app.world_mut().run_system_once(apply).unwrap();
        if reason == "offline" {
            app.world_mut()
                .get_mut::<ShipSystemControlSources>(entity)
                .unwrap()
                .0
                .set(owner.clone(), ControlSource::Offline);
        }
        if reason == "missing" {
            let mut sources = ControlSourceResolver::default();
            for system in &app
                .world()
                .get::<ShipConfigComponent>(entity)
                .unwrap()
                .0
                .systems
            {
                if system.id != owner {
                    sources.set(system.id.clone(), ControlSource::Human);
                }
            }
            app.world_mut()
                .get_mut::<ShipSystemControlSources>(entity)
                .unwrap()
                .0 = sources;
        }
        {
            let mut power = app
                .world_mut()
                .get_mut::<crate::ship::power::ShipPowerSystem>(entity)
                .unwrap();
            let energy = power.0.network_mut().consumers.get_mut(&owner).unwrap();
            if reason == "empty" {
                energy.energy = 0.0;
            }
            if reason == "thermal" {
                energy.heat = 50.0;
                energy.thermal_stopped = true;
            }
        }
        if reason == "destroyed" {
            let mut damage = app
                .world_mut()
                .get_mut::<crate::ship::compartment_damage::ShipDamage>(entity)
                .unwrap();
            for component in damage
                .state
                .components
                .get_mut(&owner)
                .unwrap()
                .values_mut()
            {
                component.condition = phoenix_model::damage::ComponentCondition::Destroyed;
            }
        }
        let before = reserve(&app, entity, &owner);
        let speed = app
            .world()
            .get::<ShipPhysics>(entity)
            .unwrap()
            .forward_speed;
        let acceleration = app
            .world()
            .get::<ShipPhysicsConfigResource>(entity)
            .unwrap()
            .0
            .acceleration;
        app.world_mut()
            .run_system_once(crate::ship_plugin::integrate_ship_physics)
            .unwrap();
        assert_eq!(reserve(&app, entity, &owner), before, "{reason}");
        assert_eq!(
            app.world().get::<BoostActuation>(entity).unwrap().0,
            0.0,
            "{reason}"
        );
        assert!(app.world().get::<ShipBoost>(entity).unwrap().0.active);
        assert!(
            (app.world()
                .get::<ShipPhysics>(entity)
                .unwrap()
                .forward_speed
                - speed
                - acceleration / 60.0)
                .abs()
                < 0.0001,
            "{reason}"
        );
    }
}

#[test]
fn last_applied_grant_is_not_recomputed_from_post_consumption_affordability() {
    let (mut app, entity, owner) = boosted_fixture();
    let requested = app
        .world()
        .get::<crate::ship::power::ShipPowerSystem>(entity)
        .unwrap()
        .0
        .config()
        .consumers
        .iter()
        .find(|consumer| consumer.system == owner)
        .unwrap()
        .continuous_cost_per_sec
        / 60.0;
    app.world_mut()
        .get_mut::<crate::ship::power::ShipPowerSystem>(entity)
        .unwrap()
        .0
        .network_mut()
        .consumers
        .get_mut(&owner)
        .unwrap()
        .energy = requested;
    boosted_step(&mut app, entity);
    assert_eq!(reserve(&app, entity, &owner), 0.0);
    assert_eq!(app.world().get::<BoostActuation>(entity).unwrap().0, 1.0);
    app.init_resource::<crate::core::messages::InterSystemQueue>();
    app.insert_resource(crate::lobby::server::ShipClientConfigResource::default());
    app.world_mut()
        .entity_mut(entity)
        .insert(crate::server_app::ShipSystemBlackboards::default());
    app.world_mut()
        .run_system_once(crate::console::helm::server::publish_helm_blackboard)
        .unwrap();
    let boards = &app
        .world()
        .get::<crate::server_app::ShipSystemBlackboards>(entity)
        .unwrap()
        .0;
    let crate::core::messages::SystemBlackboard::Helm(board) = &boards[&SystemId("helm".into())]
    else {
        panic!("Helm projection")
    };
    assert!(board.boost_active);
    assert!(!board.boost_can_operate);
    assert_eq!(board.boost_work_fraction, 1.0);
}

#[test]
fn ordinary_boost_ai_releases_the_custom_owner_through_normal_admission() {
    use crate::ship::helm_ai::{AiPolicyTickClock, FineSystemAiPolicies, HelmAiSurfacesFrame};
    let source = crate::entities::include_resolve::resolve_from_disk(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap()
    .toml
    .replace("helm-boost", "custom-overdrive");
    let authored = crate::entities::config::EntityConfig::from_toml(&source).unwrap();
    let mut app = App::new();
    crate::ai::host::register_ai_host_env(&mut app);
    let entity = crate::entities::spawner::spawn_entity(
        &mut app.world_mut().commands(),
        &authored,
        Vec3::ZERO,
        "custom-boost".into(),
        None,
    );
    app.world_mut().flush();
    let owner = SystemId("custom-overdrive".into());
    assert!(app
        .world()
        .get::<FineSystemAiPolicies>(entity)
        .unwrap()
        .0
        .contains_key(&owner));
    let mut sources = ControlSourceResolver::default();
    for system in &authored.ship_config.as_ref().unwrap().systems {
        sources.set(system.id.clone(), ControlSource::Human);
    }
    sources.set(owner.clone(), ControlSource::Ai);
    app.world_mut().entity_mut(entity).insert((
        crate::ai::server::AiHighFidelity,
        ShipSystemControlSources(sources),
        AdmittedCommands::default(),
    ));
    app.world_mut()
        .get_mut::<ShipBoost>(entity)
        .unwrap()
        .0
        .active = true;
    app.insert_resource(HelmAiSurfacesFrame::default())
        .insert_resource(crate::ship::helm_planner::HelmMotionPlan::default())
        .insert_resource(AiPolicyTickClock::default());
    app.world_mut()
        .run_system_once(crate::ship::helm_ai::ai_helm_boost)
        .unwrap();
    let commands = &app.world().get::<AdmittedCommands>(entity).unwrap().0;
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].target, owner);
    assert!(matches!(
        commands[0].payload,
        SystemControlPayload::SetBoost { active: false }
    ));
    app.world_mut()
        .run_system_once(crate::ship::helm_admission::process_helm_inputs)
        .unwrap();
    app.world_mut()
        .run_system_once(crate::ship_plugin::apply_helm_commands)
        .unwrap();
    assert!(!app.world().get::<ShipBoost>(entity).unwrap().0.active);
}
