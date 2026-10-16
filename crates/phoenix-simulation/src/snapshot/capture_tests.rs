use super::*;

fn sparse_capture_world(missing: Option<usize>) -> World {
    let mut world = World::new();
    if missing != Some(0) {
        world.register_component::<ShipPhysics>();
    }
    if missing != Some(1) {
        world.register_component::<EntitySystemHull>();
    }
    if missing != Some(2) {
        world.register_component::<ShipRedAlert>();
    }
    if missing != Some(3) {
        world.register_component::<crate::console::command::server::ShipStationStances>();
    }
    if missing != Some(4) {
        world.register_component::<crate::ship_plugin::ShipSystemControlSources>();
    }
    world
}

#[test]
fn sparse_capture_keeps_registration_gate_and_serialized_presence() {
    for missing in 0..5 {
        let mut world = sparse_capture_world(Some(missing));
        world.spawn(EntityUuid("empty".into()));
        assert!(
            capture_entities(&world).is_empty(),
            "missing registration {missing}"
        );
    }
    for order in [["c", "b", "a"], ["a", "c", "b"]] {
        let mut world = sparse_capture_world(None);
        for id in order {
            let mut entity = world.spawn(EntityUuid(id.into()));
            if id != "b" {
                entity.insert(ShipRedAlert(id == "a"));
            }
        }
        assert_eq!(
            serde_json::to_value(capture_entities(&world)).unwrap(),
            serde_json::json!([
                {"uuid":"a", "red_alert":true}, {"uuid":"b"}, {"uuid":"c", "red_alert":false}
            ])
        );
    }
}

#[test]
fn capture_preserves_group_registration_and_explicit_default_continuations() {
    let mut world = sparse_capture_world(None);
    let id = world
        .spawn((
            EntityUuid("ship".into()),
            ShipImpulse::default(),
            crate::console::navigation::NavigationWaypoint::default(),
            crate::ship_plugin::PendingArcBearingRequest::default(),
        ))
        .id();
    let first = capture_entities(&world).remove(0);
    assert!(first.drive.is_none());
    assert!(first.navigation_waypoint.is_none());
    assert!(first.arc_request.is_none());
    world.register_component::<ShipBoost>();
    world.register_component::<crate::ship_plugin::HelmWaypointClearance>();
    world.register_component::<crate::console::weapons::WeaponsArcRequestState>();
    let row = capture_entities(&world).remove(0);
    assert_eq!(
        row.drive,
        Some(DriveState {
            impulse_phase: 0,
            impulse_charge_progress: 0.0,
            impulse_previous_hull_hp: None,
            boost_active: false,
            boost_battery: 0.0
        })
    );
    assert_eq!(
        row.navigation_waypoint,
        Some(NavigationWaypointState {
            charted: None,
            formation: None,
            accepted_requests: Default::default(),
            position: None,
            source_uuid: None,
            generation: 0
        })
    );
    assert!(row.navigation_clearance_issue.is_some());
    assert!(row.helm_waypoint_clearance.is_none());
    assert_eq!(
        row.arc_request,
        Some(ArcRequestState {
            last: None,
            pending_target: None,
            pending_arcs: vec![]
        })
    );
    world.entity_mut(id).remove::<ShipImpulse>();
    assert!(capture_entities(&world)[0].drive.is_none());
}

#[test]
fn capture_projects_ship_fidelity_and_resource_motion_on_the_fixed_clock() {
    use crate::ship::helm_planner::{
        DesiredMotion, HazardAssessment, HelmMotionPlan, ShipMotionPlan,
    };
    let mut world = sparse_capture_world(None);
    let ship = world
        .spawn((
            EntityUuid("ship".into()),
            crate::server_app::Ship,
            crate::ai::server::AiHighFidelity,
            crate::ai::server::LodTransitionTimer {
                last_state_change_secs: 2.25,
            },
        ))
        .id();
    world.spawn((
        EntityUuid("point".into()),
        crate::ai::server::AiHighFidelity,
    ));
    let mut clock = Time::<bevy::time::Fixed>::default();
    clock.advance_by(std::time::Duration::from_secs(3));
    world.insert_resource(clock);
    let mut plan = HelmMotionPlan::default();
    plan.ships.insert(
        ship,
        ShipMotionPlan {
            motion: DesiredMotion {
                desired_velocity_local: Vec3::new(1.0, 2.0, 3.0),
                desired_facing_local: Vec3::Z,
            },
            hazard: HazardAssessment::default(),
            docking_active: true,
            navigation_generation: None,
        },
    );
    world.insert_resource(plan);
    let rows = capture_entities(&world);
    assert!(rows[0].ai_fidelity.is_none());
    assert!(rows[0].motion_plan.is_none());
    assert_eq!(
        rows[1].ai_fidelity,
        Some(AiFidelityState {
            high_fidelity: true,
            last_transition_secs: Some(2.25)
        })
    );
    assert_eq!(
        rows[1].motion_plan,
        Some(MotionPlanState {
            desired_velocity_local: [1.0, 2.0, 3.0],
            desired_facing_local: [0.0, 0.0, 1.0],
            hazard_forces: [0.0; 3],
            docking_active: true,
            navigation_generation: None,
            ..Default::default()
        })
    );
}
