use super::*;
use crate::{
    core::messages::{
        AdmittedCommand, AdmittedCommands, SystemBlackboard, SystemControlPayload as Control,
    },
    ship::{power::ShipPowerSystem, state::ShipPhysics},
    ship_plugin::ShipSystemControlSources,
};
use bevy::ecs::{entity_disabling::Disabled, system::RunSystemOnce};
use phoenix_model::job::JobOutcome;
use phoenix_sim_gameplay::ship::craft::BayOperation;

fn owner() -> SystemId {
    SystemId("probe-bay".into())
}
fn fixture(stored: bool) -> (App, Entity, Entity) {
    let parent =
        super::super::include_resolve::load_entity_config("assets/entities/probe_carrier.toml")
            .unwrap();
    fixture_with(stored, parent)
}
fn fixture_with(stored: bool, parent: EntityConfig) -> (App, Entity, Entity) {
    let mut app = App::new();
    register(&mut app);
    app.insert_resource(crate::sim_tick::SimTick(1));
    app.insert_resource(Time::<Fixed>::from_seconds(1.0 / 60.0));
    app.init_resource::<Time>();
    app.init_resource::<crate::world::server::WorldContentRuntime>();
    app.add_message::<crate::ai::server::AiEntityDestroyed>();
    let probe =
        super::super::include_resolve::load_entity_config("assets/entities/physical_probe.toml")
            .unwrap();
    let parent = super::super::spawner::spawn_entity(
        &mut app.world_mut().commands(),
        &parent,
        Vec3::ZERO,
        "actual-carrier".into(),
        None,
    );
    let child = super::super::spawner::spawn_entity(
        &mut app.world_mut().commands(),
        &probe,
        Vec3::new(0.0, 0.0, -18.0),
        "actual-probe".into(),
        None,
    );
    if stored {
        stage_initial(
            &mut app.world_mut().commands(),
            child,
            Some(CraftLocation::Carried {
                parent_uuid: "actual-carrier".into(),
                bay: owner(),
            }),
        );
    }
    app.world_mut().flush();
    app.world_mut()
        .get_mut::<ShipSystemControlSources>(parent)
        .unwrap()
        .0
        .set(owner(), crate::ship::control_source::ControlSource::Human);
    assert!(app.world().get::<crate::server_app::Ship>(child).is_none());
    assert!(app
        .world()
        .get::<crate::ship_plugin::ShipPhysicsConfigResource>(child)
        .is_none());
    (app, parent, child)
}
fn energy(app: &App, parent: Entity) -> f32 {
    app.world()
        .get::<ShipPowerSystem>(parent)
        .unwrap()
        .0
        .network()
        .consumers[&owner()]
        .energy
}
fn command(app: &mut App, parent: Entity, payload: Control) {
    app.world_mut()
        .get_mut::<AdmittedCommands>(parent)
        .unwrap()
        .0
        .push(AdmittedCommand {
            target: owner(),
            payload,
            response_token: None,
            feedback_correlation: None,
        });
    runtime::handle(app.world_mut());
    app.world_mut()
        .get_mut::<AdmittedCommands>(parent)
        .unwrap()
        .0
        .clear();
}
fn next(app: &mut App) -> u64 {
    let mut tick = app.world_mut().resource_mut::<crate::sim_tick::SimTick>();
    tick.0 += 1;
    tick.0
}
fn ready(app: &mut App, parent: Entity) {
    for _ in 0..240 {
        next(app);
        runtime::tick(app.world_mut());
        if app.world().get::<ShipBayJobs>(parent).unwrap().0[&owner()]
            .job
            .as_ref()
            .unwrap()
            .activity
            .ready()
        {
            return;
        }
    }
    panic!("actual finite paid handling must become ready");
}
fn scan(app: &mut App, parent: Entity) {
    let dt = app
        .world()
        .resource::<Time<Fixed>>()
        .timestep()
        .as_secs_f32();
    app.world_mut()
        .get_mut::<ShipPowerSystem>(parent)
        .unwrap()
        .0
        .work_continuous(&SystemId("tactical-radar".into()), dt, 1.0)
        .unwrap();
    app.world_mut()
        .run_system_once(crate::science::broad::tick)
        .unwrap();
    app.world_mut()
        .run_system_once(crate::console::weapons::blackboard::publish_tactical_radar_blackboard)
        .unwrap();
}

#[test]
fn actual_paid_launch_waits_for_next_backend_boundary_and_blocked_space_never_recharges() {
    let (mut app, parent, child) = fixture(true);
    let reserve = energy(&app, parent);
    command(
        &mut app,
        parent,
        Control::LaunchCraft {
            craft: "actual-probe".into(),
        },
    );
    ready(&mut app, parent);
    assert!(reserve > energy(&app, parent));
    let paid = energy(&app, parent);
    runtime::commit(app.world_mut());
    assert!(
        app.world().get::<Disabled>(child).is_some(),
        "same-boundary ready work does not hand off early"
    );
    let obstacle = app
        .world_mut()
        .spawn((
            Transform::from_xyz(0.0, 0.0, -18.0),
            bevy_rapier3d::prelude::Collider::cuboid(1.0, 1.0, 1.0),
        ))
        .id();
    next(&mut app);
    runtime::commit(app.world_mut());
    runtime::tick(app.world_mut());
    assert_eq!(energy(&app, parent), paid);
    assert!(app.world().get::<Disabled>(child).is_some());
    assert!(app.world().get::<ShipBayJobs>(parent).unwrap().0[&owner()]
        .job
        .as_ref()
        .unwrap()
        .outcome
        .is_none());
    app.world_mut().despawn(obstacle);
    next(&mut app);
    runtime::commit(app.world_mut());
    assert_eq!(
        app.world().get::<ProbeCustody>(child).unwrap().0,
        CraftLocation::Deployed {}
    );
    assert_eq!(
        app.world().get::<EntityUuid>(child).unwrap().0,
        "actual-probe"
    );
    assert_eq!(app.world().get::<ShipPhysics>(child).unwrap().z, -18.0);
    assert!(app.world().get::<Disabled>(child).is_none());
    assert_eq!(energy(&app, parent), paid);
    assert_eq!(
        app.world().get::<ShipBayJobs>(parent).unwrap().0[&owner()]
            .job
            .as_ref()
            .unwrap()
            .outcome,
        Some(JobOutcome::Completed)
    );
    runtime::commit(app.world_mut());
    assert_eq!(energy(&app, parent), paid);
}

#[test]
fn interruption_preserves_paid_job_and_terminal_cancel_releases_it_without_moving_craft() {
    let (mut app, parent, child) = fixture(true);
    command(
        &mut app,
        parent,
        Control::LaunchCraft {
            craft: "actual-probe".into(),
        },
    );
    next(&mut app);
    runtime::tick(app.world_mut());
    let partial = app.world().get::<ShipBayJobs>(parent).unwrap().0[&owner()]
        .job
        .clone()
        .unwrap();
    assert!(partial.activity.worked_secs > 0.0);
    let paid = energy(&app, parent);
    app.world_mut()
        .get_mut::<ShipSystemControlSources>(parent)
        .unwrap()
        .0
        .set(owner(), crate::ship::control_source::ControlSource::Offline);
    next(&mut app);
    runtime::tick(app.world_mut());
    assert_eq!(energy(&app, parent), paid);
    let paused = app.world().get::<ShipBayJobs>(parent).unwrap().0[&owner()]
        .job
        .as_ref()
        .unwrap();
    assert_eq!(paused.id, partial.id);
    assert_eq!(paused.activity, partial.activity);
    assert!(paused.paused);
    app.world_mut()
        .get_mut::<ShipSystemControlSources>(parent)
        .unwrap()
        .0
        .set(owner(), crate::ship::control_source::ControlSource::Human);
    app.world_mut()
        .get_mut::<ShipPhysics>(parent)
        .unwrap()
        .forward_speed = 1.0;
    next(&mut app);
    runtime::tick(app.world_mut());
    assert_eq!(energy(&app, parent), paid);
    app.world_mut()
        .get_mut::<ShipPhysics>(parent)
        .unwrap()
        .forward_speed = 0.0;
    next(&mut app);
    runtime::tick(app.world_mut());
    assert!(energy(&app, parent) < paid);
    command(&mut app, parent, Control::CancelCraftHandling);
    let cancelled = app.world().get::<ShipBayJobs>(parent).unwrap().0[&owner()]
        .job
        .clone()
        .unwrap();
    assert_eq!(cancelled.outcome, Some(JobOutcome::Cancelled));
    assert!(cancelled.activity.worked_secs > 0.0);
    assert!(matches!(
        app.world().get::<ProbeCustody>(child).unwrap().0,
        CraftLocation::Carried { .. }
    ));
    command(
        &mut app,
        parent,
        Control::LaunchCraft {
            craft: "actual-probe".into(),
        },
    );
    let new = app.world().get::<ShipBayJobs>(parent).unwrap().0[&owner()]
        .job
        .as_ref()
        .unwrap();
    assert!(new.id.0 > cancelled.id.0);
    assert_eq!(new.activity.worked_secs, 0.0);
}

#[test]
fn recovery_needs_current_permitted_alias_then_paid_handling_withdraws_support_and_keeps_history() {
    let (mut app, parent, child) = fixture(false);
    let before = energy(&app, parent);
    command(
        &mut app,
        parent,
        Control::RecoverCraft {
            contact: "actual-probe".into(),
        },
    );
    assert!(app.world().get::<ShipBayJobs>(parent).unwrap().0[&owner()]
        .job
        .is_none());
    assert_eq!(energy(&app, parent), before);
    let contact = (0..120)
        .find_map(|_| {
            next(&mut app);
            scan(&mut app, parent);
            runtime::alias(app.world(), parent, &owner(), "actual-probe")
        })
        .expect("actual paid radar must produce permitted current contact");
    assert_ne!(contact, "actual-probe");
    command(&mut app, parent, Control::RecoverCraft { contact });
    assert_eq!(
        app.world().get::<ShipBayJobs>(parent).unwrap().0[&owner()]
            .job
            .as_ref()
            .unwrap()
            .activity
            .operation,
        BayOperation::Recover
    );
    for _ in 0..240 {
        next(&mut app);
        scan(&mut app, parent);
        runtime::tick(app.world_mut());
        if app.world().get::<ShipBayJobs>(parent).unwrap().0[&owner()]
            .job
            .as_ref()
            .unwrap()
            .activity
            .ready()
        {
            break;
        }
    }
    let tracks_before = app
        .world()
        .get::<crate::science::broad::BroadTracks>(parent)
        .unwrap()
        .0[&SystemId("tactical-radar".into())]
        .tracks
        .clone();
    let child_power = app
        .world()
        .get::<ShipPowerSystem>(child)
        .unwrap()
        .0
        .capture_state();
    next(&mut app);
    runtime::commit(app.world_mut());
    assert!(app.world().get::<Disabled>(child).is_some());
    let after = &app
        .world()
        .get::<crate::science::broad::BroadTracks>(parent)
        .unwrap()
        .0[&SystemId("tactical-radar".into())];
    assert!(after.resolve_current("actual-probe").is_none());
    assert!(after.current.get("actual-probe").is_none());
    assert_eq!(after.tracks, tracks_before);
    assert_eq!(
        app.world()
            .get::<ShipPowerSystem>(child)
            .unwrap()
            .0
            .capture_state(),
        child_power
    );
    view::publish(app.world_mut());
    let SystemBlackboard::Bay(view) = &app
        .world()
        .get::<crate::server_app::ShipSystemBlackboards>(parent)
        .unwrap()
        .0[&owner()]
    else {
        panic!()
    };
    assert_eq!(view.stored[0].id, "actual-probe");
    assert!(view
        .contacts
        .iter()
        .all(|row| row.contact != "actual-probe"));
}

#[test]
fn parent_removal_loses_only_carried_actor_once_and_bulk_restore_does_not_cascade() {
    let (mut app, parent, child) = fixture(true);
    let mut cursor = app
        .world()
        .resource::<Messages<crate::ai::server::AiEntityDestroyed>>()
        .get_cursor();
    app.world_mut().despawn(parent);
    app.world_mut().flush();
    assert!(app.world().get_entity(child).is_err());
    let lost: Vec<_> = cursor
        .read(
            app.world()
                .resource::<Messages<crate::ai::server::AiEntityDestroyed>>(),
        )
        .map(|event| event.entity_uuid.as_str())
        .collect();
    assert_eq!(
        lost,
        vec!["actual-probe"],
        "hook never duplicates the primary parent's event"
    );
    let (mut app, parent, child) = fixture(false);
    app.world_mut().despawn(parent);
    app.world_mut().flush();
    assert!(app.world().get_entity(child).is_ok());
    let (mut app, parent, child) = fixture(true);
    let previous = restore_guard(app.world_mut(), true);
    app.world_mut().despawn(parent);
    app.world_mut().flush();
    restore_guard(app.world_mut(), previous);
    assert!(
        app.world().get_entity(child).is_ok(),
        "exclusive validated bulk restore is not gameplay loss"
    );
}

#[test]
fn finite_release_uses_the_same_full_parent_pose_and_refuses_unrepresentable_result() {
    let rotation = Quat::from_euler(EulerRot::YXZ, -0.7, 0.0, 0.2);
    let offset = [1.0, 2.0, -3.0];
    let point = runtime::release_point([5.0, 6.0, 7.0], rotation, &offset).unwrap();
    let expected = Vec3::new(5.0, 6.0, 7.0) + rotation * Vec3::from_array(offset);
    assert!((Vec3::from_array(point) - expected).length() < 0.00001);
    assert!(
        runtime::release_point([f32::MAX, 0.0, 0.0], Quat::IDENTITY, &[f32::MAX, 0.0, 0.0])
            .is_none()
    );
}

#[test]
fn actual_bay_tree_selects_own_stored_craft_through_normal_admission_and_pays_same_job() {
    let resolved =
        super::super::include_resolve::resolve_from_disk("assets/entities/probe_carrier.toml")
            .unwrap();
    let mut value = resolved.value;
    let bay = value
        .get_mut("system")
        .unwrap()
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|row| row.get("id").and_then(toml::Value::as_str) == Some("probe-bay"))
        .unwrap();
    bay["config"]["ai"] = toml::from_str::<toml::Value>(
        r#"
root="bay"
[[tree]]
id="bay"
initial="deploy"
[[tree.state]]
id="deploy"
[[tree.state.task]]
id="launch-own"
[[tree.state.task.when]]
equals=true
[tree.state.task.when.value]
source="fact"
name="can_operate"
[[tree.state.task.when]]
equals=true
[tree.state.task.when.value]
source="fact"
name="has_stored"
[[tree.state.task.when]]
equals=false
[tree.state.task.when.value]
source="fact"
name="active"
[tree.state.task.action]
kind="write"
channel="launch"
[tree.state.task.action.value]
source="literal"
value=true
"#,
    )
    .unwrap();
    let accepted = EntityConfig::from_toml(&toml::to_string(&value).unwrap()).unwrap();
    let (mut app, parent, child) = fixture_with(true, accepted);
    crate::ai::host::register_ai_host_env(&mut app);
    app.world_mut()
        .get_mut::<ShipSystemControlSources>(parent)
        .unwrap()
        .0
        .set(owner(), crate::ship::control_source::ControlSource::Ai);
    view::publish(app.world_mut());
    app.world_mut().run_system_once(ai::operate).unwrap();
    let commands = &app.world().get::<AdmittedCommands>(parent).unwrap().0;
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].target, owner());
    assert_eq!(
        commands[0].payload,
        Control::LaunchCraft {
            craft: "actual-probe".into()
        }
    );
    let reserve = energy(&app, parent);
    runtime::handle(app.world_mut());
    app.world_mut()
        .get_mut::<AdmittedCommands>(parent)
        .unwrap()
        .0
        .clear();
    ready(&mut app, parent);
    next(&mut app);
    runtime::commit(app.world_mut());
    assert!(energy(&app, parent) < reserve);
    assert_eq!(
        app.world().get::<ProbeCustody>(child).unwrap().0,
        CraftLocation::Deployed {}
    );
    view::publish(app.world_mut());
    next(&mut app);
    app.world_mut().run_system_once(ai::operate).unwrap();
    assert!(
        app.world()
            .get::<AdmittedCommands>(parent)
            .unwrap()
            .0
            .is_empty(),
        "empty stored roster cannot mint another craft/order"
    );
}

#[test]
fn actual_physical_removal_fails_bound_job_once_without_refunding_work() {
    let (mut app, parent, child) = fixture(true);
    command(
        &mut app,
        parent,
        Control::LaunchCraft {
            craft: "actual-probe".into(),
        },
    );
    next(&mut app);
    runtime::tick(app.world_mut());
    let partial = app.world().get::<ShipBayJobs>(parent).unwrap().0[&owner()]
        .job
        .clone()
        .unwrap();
    app.world_mut().despawn(child);
    app.world_mut().flush();
    let failed = app.world().get::<ShipBayJobs>(parent).unwrap().0[&owner()]
        .job
        .clone()
        .unwrap();
    assert_eq!(failed.id, partial.id);
    assert_eq!(failed.activity, partial.activity);
    assert_eq!(failed.outcome, Some(JobOutcome::Failed));
    assert!(!app
        .world_mut()
        .get_mut::<ShipBayJobs>(parent)
        .unwrap()
        .0
        .get_mut(&owner())
        .unwrap()
        .fail_removed("actual-probe", failed.execution_tick));
}

#[test]
fn canonical_capture_keeps_stored_identity_reserves_and_whole_custody_graph() {
    let (mut app, parent, child) = fixture(true);
    app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 = 10;
    let snapshot = crate::snapshot::capture(app.world());
    let row = snapshot
        .entities
        .iter()
        .find(|row| row.uuid == "actual-probe")
        .expect("Disabled stored body remains canonical");
    assert!(matches!(
        row.probe_custody.as_ref().unwrap().0,
        CraftLocation::Carried { .. }
    ));
    assert_eq!(
        row.power.as_ref().unwrap(),
        &app.world()
            .get::<ShipPowerSystem>(child)
            .unwrap()
            .0
            .capture_continuation()
    );
    assert!(snapshot
        .entities
        .iter()
        .find(|row| row.uuid == "actual-carrier")
        .unwrap()
        .bay_jobs
        .is_some());
    let encoded = serde_json::to_string(&snapshot).unwrap();
    let decoded: crate::snapshot::PhoenixSnapshot = serde_json::from_str(&encoded).unwrap();
    assert_eq!(decoded, snapshot);
    let _ = parent;
}
