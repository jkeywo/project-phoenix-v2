use super::*;
use crate::core::messages::AdmittedCommand;
use crate::ship::control_source::{ControlSource, ControlSourceResolver};
use bevy::ecs::system::RunSystemOnce;
use phoenix_model::helm_control::{HelmControlMode, SpeedDemand};

fn fixture() -> (App, Entity) {
    let authored = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    let profile = authored.helm_control.clone().unwrap();
    let tuning = authored.helm_console.as_ref().unwrap();
    let physics_config = crate::ship::physics::ShipPhysicsConfig {
        max_speed: tuning.max_speed,
        max_reverse_speed: tuning.max_reverse_speed,
        acceleration: tuning.acceleration,
        deceleration: tuning.deceleration,
        max_yaw_rate: tuning.max_yaw_rate,
        low_speed_turn_boost: tuning.low_speed_turn_boost,
        ..Default::default()
    };
    let mut sources = ControlSourceResolver::default();
    for system in &authored.ship_config.as_ref().unwrap().systems {
        sources.set(system.id.clone(), ControlSource::Human);
    }
    let damage = crate::ship::compartment_damage::ShipDamage::from_config(&authored)
        .unwrap()
        .unwrap();
    let power = crate::ship::power::ShipPowerSystem(
        crate::modifiers::power_system::PowerSystem::from_energy(
            authored.energy.clone().unwrap(),
            authored.ship_config.clone().unwrap(),
        )
        .unwrap(),
    );
    let mut app = App::new();
    let mut time: Time = Time::default();
    time.advance_by(std::time::Duration::from_secs_f32(1.0 / 60.0));
    app.insert_resource(time)
        .insert_resource(crate::sim_tick::SimTick(1));
    #[cfg(debug_assertions)]
    app.insert_resource(crate::ship::helm::HelmPhysicsFrame(1));
    let entity = app
        .world_mut()
        .spawn((
            EntityUuid("movement-owner".into()),
            crate::server_app::Ship,
            crate::ai::server::AiHighFidelity,
            CompiledHelmControl(profile.clone()),
            HelmControlState::new(&profile),
            ShipConfigComponent(authored.ship_config.unwrap()),
            ShipPhysicsConfigResource(physics_config),
            ShipSystemControlSources(sources),
            ShipPhysics {
                forward_speed: 10.0,
                ..Default::default()
            },
            AdmittedCommands::default(),
            ThrustInput(0.0),
            SteeringInput(0.0),
            LateralThrustInput(0.0),
            crate::ship::helm::VerticalThrustInput(0.0),
            (power, damage),
        ))
        .id();
    (app, entity)
}

fn command(app: &mut App, entity: Entity, owner: SystemId, payload: SystemControlPayload) {
    app.world_mut()
        .get_mut::<AdmittedCommands>(entity)
        .unwrap()
        .0
        .push(AdmittedCommand {
            target: owner,
            payload,
            response_token: None,
            feedback_correlation: None,
        });
}

fn step(app: &mut App, entity: Entity) {
    app.world_mut().run_system_once(apply).unwrap();
    app.world_mut()
        .run_system_once(crate::ship_plugin::integrate_ship_physics)
        .unwrap();
    app.world_mut()
        .get_mut::<AdmittedCommands>(entity)
        .unwrap()
        .0
        .clear();
    app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 += 1;
    #[cfg(debug_assertions)]
    {
        app.world_mut()
            .resource_mut::<crate::ship::helm::HelmPhysicsFrame>()
            .0 += 1;
    }
}

#[test]
fn admitted_direct_force_uses_real_reserve_and_neutral_or_unavailable_coasts() {
    let (mut app, entity) = fixture();
    let profile = app
        .world()
        .get::<CompiledHelmControl>(entity)
        .unwrap()
        .0
        .clone();
    let energy = |app: &App| {
        app.world()
            .get::<crate::ship::power::ShipPowerSystem>(entity)
            .unwrap()
            .0
            .network()
            .consumers[&profile.thrust]
            .energy
    };
    let before = energy(&app);
    command(
        &mut app,
        entity,
        profile.thrust.clone(),
        SystemControlPayload::SetHelmMode {
            mode: HelmControlMode::Direct,
        },
    );
    command(
        &mut app,
        entity,
        profile.thrust.clone(),
        SystemControlPayload::SetThrust { value: 1.0 },
    );
    step(&mut app, entity);
    let paid = *app.world().get::<ShipPhysics>(entity).unwrap();
    assert!(paid.forward_speed > 10.0);
    assert!(energy(&app) < before);
    command(
        &mut app,
        entity,
        profile.thrust.clone(),
        SystemControlPayload::SetThrust { value: 0.0 },
    );
    let before = energy(&app);
    step(&mut app, entity);
    assert_eq!(
        app.world()
            .get::<ShipPhysics>(entity)
            .unwrap()
            .forward_speed,
        paid.forward_speed
    );
    assert_eq!(energy(&app), before);
    // Real unavailable source gates the old force before work, retaining motion.
    command(
        &mut app,
        entity,
        profile.thrust.clone(),
        SystemControlPayload::SetThrust { value: -1.0 },
    );
    app.world_mut()
        .get_mut::<ShipSystemControlSources>(entity)
        .unwrap()
        .0
        .set_offline(profile.thrust.clone(), true);
    step(&mut app, entity);
    assert_eq!(
        app.world()
            .get::<ShipPhysics>(entity)
            .unwrap()
            .forward_speed,
        paid.forward_speed
    );
    assert_eq!(energy(&app), before);
    app.world_mut()
        .get_mut::<ShipSystemControlSources>(entity)
        .unwrap()
        .0
        .set_offline(profile.thrust.clone(), false);
    step(&mut app, entity);
    assert_eq!(
        app.world()
            .get::<ShipPhysics>(entity)
            .unwrap()
            .forward_speed,
        paid.forward_speed,
        "an already admitted command cannot revive after an Offline interruption"
    );
    assert_eq!(energy(&app), before);
    assert_eq!(app.world().get::<SteeringInput>(entity).unwrap().0, 0.0);
}

#[test]
fn ordinary_assisted_request_after_delegation_never_interprets_speed_as_force() {
    let (mut app, entity) = fixture();
    let profile = app
        .world()
        .get::<CompiledHelmControl>(entity)
        .unwrap()
        .0
        .clone();
    command(
        &mut app,
        entity,
        profile.thrust.clone(),
        SystemControlPayload::SetHelmMode {
            mode: HelmControlMode::Direct,
        },
    );
    command(
        &mut app,
        entity,
        profile.thrust.clone(),
        SystemControlPayload::SetHelmMode {
            mode: HelmControlMode::Assisted,
        },
    );
    command(
        &mut app,
        entity,
        profile.thrust.clone(),
        SystemControlPayload::SetThrust { value: 0.25 },
    );
    app.world_mut().run_system_once(apply).unwrap();
    assert_eq!(
        app.world().get::<HelmControlState>(entity).unwrap().axes[&profile.thrust],
        AxisIntent::Thrust(ThrustIntent::Assisted {
            demand: SpeedDemand::Fraction { value: 0.25 }
        })
    );
    assert_eq!(app.world().get::<ThrustInput>(entity).unwrap().0, 0.25);
}

#[test]
fn match_requires_exact_current_permitted_dated_velocity_and_loss_keeps_the_sample() {
    use crate::core::messages::{HelmBlackboard, SystemBlackboard};
    use crate::science::broad::{BroadMeasurement, BroadProfile, BroadTrackState};
    use std::collections::BTreeMap;
    let (mut app, entity) = fixture();
    let authored = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    let owner = app
        .world()
        .get::<CompiledHelmControl>(entity)
        .unwrap()
        .0
        .thrust
        .clone();
    let profiles = BroadProfile::from_config(&authored).unwrap().unwrap();
    let instrument = SystemId("helm-radar".into());
    let profile = &profiles.0[&instrument].profile;
    let mut history = BroadTrackState::new(profile);
    // Work/cadence accrues once per actual logical step; jumping the supplied
    // tick alone must not fabricate a second paid sample or velocity.
    for tick in 0..=30 {
        let x = 20.0 + tick as f32 / 6.0;
        history
            .advance(
                profile,
                tick,
                1.0 / 60.0,
                1.0,
                &[BroadMeasurement {
                    source: "private-target".into(),
                    selectable: true,
                    observation: None,
                    position: [x, 0.0],
                    elevation: None,
                    support: 1.0,
                    identity: None,
                    classification: None,
                    assessment: None,
                }],
            )
            .unwrap();
    }
    history.validate(profile, 30, 1.0 / 60.0).unwrap();
    let records = history.projection(profile, 30, 1.0 / 60.0).unwrap();
    let record = records.iter().find(|record| !record.predicted).unwrap();
    let velocity = record.velocity.unwrap();
    let expected = (velocity[0] as f64).hypot(velocity[1] as f64) as f32;
    let contact = record.contact_id.clone();
    let observed = record.observed_tick;
    let mut boards = crate::server_app::ShipSystemBlackboards::default();
    boards.0.insert(
        instrument.clone(),
        SystemBlackboard::Helm(HelmBlackboard {
            movement: None,
            tracks: Some(records),
            ..Default::default()
        }),
    );
    app.world_mut()
        .entity_mut(entity)
        .insert((boards, BroadTracks(BTreeMap::from([(instrument, history)]))));
    app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 = 30;
    command(
        &mut app,
        entity,
        owner.clone(),
        SystemControlPayload::SetMatchSpeed {
            contact: Some("private-target".into()),
        },
    );
    app.world_mut().run_system_once(apply).unwrap();
    assert!(matches!(
        &app.world().get::<HelmControlState>(entity).unwrap().axes[&owner],
        AxisIntent::Thrust(ThrustIntent::Assisted {
            demand: SpeedDemand::Fraction { value: 0.0 }
        })
    ));
    app.world_mut()
        .get_mut::<AdmittedCommands>(entity)
        .unwrap()
        .0
        .clear();
    command(
        &mut app,
        entity,
        owner.clone(),
        SystemControlPayload::SetMatchSpeed {
            contact: Some(contact.clone()),
        },
    );
    app.world_mut().run_system_once(apply).unwrap();
    let held = app.world().get::<HelmControlState>(entity).unwrap().clone();
    assert_eq!(
        held.axes[&owner],
        AxisIntent::Thrust(ThrustIntent::Assisted {
            demand: SpeedDemand::Match {
                contact,
                speed: expected,
                observed_tick: observed
            }
        })
    );
    app.world_mut()
        .get_mut::<AdmittedCommands>(entity)
        .unwrap()
        .0
        .clear();
    app.world_mut()
        .get_mut::<crate::server_app::ShipSystemBlackboards>(entity)
        .unwrap()
        .0
        .clear();
    app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 = 90;
    app.world_mut().run_system_once(apply).unwrap();
    assert_eq!(
        app.world().get::<HelmControlState>(entity),
        Some(&held),
        "observation loss must neither erase the chosen demand nor refresh its clock"
    );
}

#[test]
fn successful_impulse_start_and_scalar_requests_keep_their_admitted_order() {
    for start_first in [false, true] {
        let (mut app, entity) = fixture();
        let profile = app
            .world()
            .get::<CompiledHelmControl>(entity)
            .unwrap()
            .0
            .clone();
        app.world_mut()
            .entity_mut(entity)
            .insert(crate::ship::helm::ImpulseCommand::default());
        for start in [start_first, !start_first] {
            if start {
                command(
                    &mut app,
                    entity,
                    crate::ship::system_registry::helm_impulse_system_id(),
                    SystemControlPayload::StartImpulseCharge,
                );
            } else {
                command(
                    &mut app,
                    entity,
                    profile.thrust.clone(),
                    SystemControlPayload::SetThrust { value: 0.75 },
                );
            }
        }
        app.world_mut()
            .run_system_once(crate::ship::helm_admission::process_helm_inputs)
            .unwrap();
        app.world_mut().run_system_once(apply).unwrap();
        assert_eq!(
            app.world().get::<ThrustInput>(entity).unwrap().0,
            if start_first { 0.75 } else { 0.0 }
        );
    }
}

#[test]
fn renamed_actual_owner_runs_the_existing_compiled_ai_and_normal_admission() {
    use crate::ship::helm_ai::{
        AiPolicyTickClock, FineSystemAiPolicies, HelmAiShipFrame, HelmAiSurfacesFrame,
        HelmEnginesAiPolicyState, HelmSteeringAiPolicyState,
    };
    // Re-parse accepted authored source, not the runtime DTO: generated shield
    // equipment and other compile-only fields intentionally skip serialization.
    let source = crate::entities::include_resolve::resolve_from_disk(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap()
    .toml
    .replace("helm-thrust", "actual-drive")
    .replace("helm-steering", "actual-rudder")
    .replace("helm-lateral-thrust", "actual-lateral");
    let authored = crate::entities::config::EntityConfig::from_toml(&source).unwrap();
    assert!(
        !authored.shield_arcs.is_empty(),
        "keep the real resolved hull equipment"
    );
    let profile = authored.helm_control.clone().unwrap();
    let mut app = App::new();
    crate::ai::host::register_ai_host_env(&mut app);
    let entity = crate::entities::spawner::spawn_entity(
        &mut app.world_mut().commands(),
        &authored,
        Vec3::ZERO,
        "custom-movement".into(),
        None,
    );
    app.world_mut().flush();
    let mut sources = ControlSourceResolver::default();
    for system in &authored.ship_config.as_ref().unwrap().systems {
        sources.set(system.id.clone(), ControlSource::Human);
    }
    sources.set(profile.thrust.clone(), ControlSource::Ai);
    sources.set(profile.steering.clone(), ControlSource::Ai);
    app.world_mut().entity_mut(entity).insert((
        crate::ai::server::AiHighFidelity,
        ShipSystemControlSources(sources),
        AdmittedCommands::default(),
    ));
    let actual = crate::ship::physics::ShipPhysicsState {
        x: 0.0,
        y: 0.0,
        z: 0.0,
        yaw: 0.0,
        forward_speed: 0.0,
        lateral_speed: 0.0,
        vertical_speed: 0.0,
    };
    let tuning = app
        .world()
        .get::<ShipPhysicsConfigResource>(entity)
        .unwrap()
        .0;
    app.world_mut()
        .get_mut::<HelmControlState>(entity)
        .unwrap()
        .set_mode(&profile.thrust, HelmControlMode::Direct, actual, &tuning)
        .unwrap();
    let policies = &app.world().get::<FineSystemAiPolicies>(entity).unwrap().0;
    assert!(policies.contains_key(&profile.thrust));
    assert!(!policies.contains_key(&crate::ship::system_registry::helm_thrust_system_id()));
    let engines = crate::ai::policy::AiPolicyRuntimeState::reset(&policies[&profile.thrust], 0.0);
    let steering =
        crate::ai::policy::AiPolicyRuntimeState::reset(&policies[&profile.steering], 0.0);
    let pass = phoenix_sim_gameplay::ship::helm_ai::build_pass_surface_with_authority(
        &policies[&profile.thrust],
        &policies[&profile.steering],
        &steering,
        true,
        &Default::default(),
        0.0,
        0.0,
        &[],
    );
    assert!(
        pass.active,
        "custom bound travel authority preserves authored pass capability"
    );
    app.world_mut().entity_mut(entity).insert((
        HelmEnginesAiPolicyState(engines),
        HelmSteeringAiPolicyState(steering),
    ));
    let mut frame = HelmAiSurfacesFrame::default();
    frame.ships.insert(
        entity,
        HelmAiShipFrame {
            has_objective: true,
            ..Default::default()
        },
    );
    let mut plan = crate::ship::helm_planner::HelmMotionPlan::default();
    plan.ships.insert(
        entity,
        crate::ship::helm_planner::ShipMotionPlan {
            motion: crate::ship::helm_planner::DesiredMotion {
                desired_velocity_local: Vec3::new(0.0, 0.0, -0.25),
                desired_facing_local: Vec3::NEG_Z,
            },
            ..Default::default()
        },
    );
    app.insert_resource(frame)
        .insert_resource(plan)
        .insert_resource(AiPolicyTickClock::default());
    app.world_mut()
        .run_system_once(crate::ship::helm_ai::ai_helm_thrust)
        .unwrap();
    let commands = &app.world().get::<AdmittedCommands>(entity).unwrap().0;
    assert_eq!(commands.len(), 2);
    assert!(commands
        .iter()
        .all(|command| command.target == profile.thrust));
    assert!(matches!(
        commands[0].payload,
        SystemControlPayload::SetHelmMode {
            mode: HelmControlMode::Assisted
        }
    ));
    assert!(matches!(
        commands[1].payload,
        SystemControlPayload::SetThrust { value: 0.25 }
    ));
    app.world_mut().run_system_once(apply).unwrap();
    assert_eq!(
        app.world().get::<HelmControlState>(entity).unwrap().axes[&profile.thrust],
        AxisIntent::Thrust(ThrustIntent::Assisted {
            demand: SpeedDemand::Fraction { value: 0.25 }
        })
    );
}
