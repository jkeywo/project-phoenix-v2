use super::torpedo::build_torpedo_target_snapshot;
use super::{shared::TorpedoTargetSnapshot, TorpedoSystemResource};
use crate::{
    entities::spawner::{EntityUuid, RegionShapeSection},
    lobby::WorldResource,
    sim_tick::SimTick,
    weapons::torpedo::{TorpedoConfig, TorpedoSystem},
};
use bevy::{ecs::system::RunSystemOnce, prelude::*};
use std::collections::HashMap;

fn host() -> (App, Entity, Entity) {
    let mut app = App::new();
    app.insert_resource(SimTick(42));
    app.insert_resource(Time::<Fixed>::default());
    app.init_resource::<TorpedoTargetSnapshot>();
    app.insert_resource(WorldResource(crate::core::messages::WorldData {
        entities: vec![crate::core::messages::EntitySnapshot {
            uuid: "subject".into(),
            position: Some([0.0, 0.0, -20.0]),
            radius: Some(1.0),
            ..Default::default()
        }],
        ..Default::default()
    }));
    let mut owner = TorpedoSystem::new(TorpedoConfig {
        load_time: 0.1,
        seeker: Some(phoenix_model::guided_guidance::SeekerProfile {
            range: 40.0,
            fire_arc_deg: 90.0,
            minimum_transmission: 0.2,
        }),
        ..Default::default()
    });
    let tube = owner.tubes[0].id.clone();
    assert!(owner.start_load(&tube));
    owner.tick(0.1, &HashMap::new(), &mut || unreachable!());
    assert!(matches!(
        owner.launch(
            &tube,
            "flight".into(),
            0.0,
            0.0,
            0.0,
            0.0,
            Some("subject".into()),
            Some("launcher".into())
        ),
        crate::weapons::torpedo::LaunchResult::Launched { .. }
    ));
    let launcher = app
        .world_mut()
        .spawn((EntityUuid("launcher".into()), TorpedoSystemResource(owner)))
        .id();
    let target = app
        .world_mut()
        .spawn((
            EntityUuid("subject".into()),
            Transform::from_xyz(0.0, 0.0, -20.0),
        ))
        .id();
    (app, launcher, target)
}

#[test]
fn actual_seeker_samples_once_then_loss_cannot_refresh_from_collision_or_cached_truth() {
    let (mut app, launcher, target) = host();
    app.world_mut()
        .run_system_once(build_torpedo_target_snapshot)
        .unwrap();
    let measured = app
        .world()
        .resource::<TorpedoTargetSnapshot>()
        .guidance
        .clone();
    assert_eq!(measured["flight"].position, [0.0, 0.0, -20.0]);
    assert_eq!(measured["flight"].sampled_tick, 42);
    app.world_mut()
        .get_mut::<TorpedoSystemResource>(launcher)
        .unwrap()
        .0
        .tick_with_work(
            0.1,
            &measured,
            &mut |_| panic!("emitted flight requires no launcher work"),
            &mut || unreachable!(),
        );
    let held = app
        .world()
        .get::<TorpedoSystemResource>(launcher)
        .unwrap()
        .0
        .in_flight[0]
        .last_guidance
        .clone();
    app.world_mut()
        .get_mut::<Transform>(target)
        .unwrap()
        .translation = Vec3::new(80.0, 30.0, -20.0);
    app.world_mut().resource_mut::<SimTick>().0 = 43;
    app.world_mut()
        .run_system_once(build_torpedo_target_snapshot)
        .unwrap();
    assert!(app
        .world()
        .resource::<TorpedoTargetSnapshot>()
        .guidance
        .is_empty());
    assert!(app
        .world()
        .resource::<TorpedoTargetSnapshot>()
        .targets
        .iter()
        .any(|(id, x, y, _, _)| id == "subject" && *x == 80.0 && *y == 30.0));
    app.world_mut()
        .get_mut::<TorpedoSystemResource>(launcher)
        .unwrap()
        .0
        .tick_with_work(
            0.1,
            &HashMap::new(),
            &mut |_| unreachable!(),
            &mut || unreachable!(),
        );
    assert_eq!(
        app.world()
            .get::<TorpedoSystemResource>(launcher)
            .unwrap()
            .0
            .in_flight[0]
            .last_guidance,
        held
    );
    app.world_mut().despawn(target);
    app.world_mut()
        .run_system_once(build_torpedo_target_snapshot)
        .unwrap();
    assert!(
        app.world()
            .resource::<TorpedoTargetSnapshot>()
            .guidance
            .is_empty(),
        "cached world row cannot reacquire a missing subject"
    );
    assert!(
        app.world()
            .resource::<TorpedoTargetSnapshot>()
            .targets
            .iter()
            .any(|row| row.0 == "subject"),
        "physical coarse collision roster is independently retained"
    );
}

#[test]
fn actual_seeker_respects_sensor_opaque_path_and_fixed_intent_equipment_absence() {
    let (mut app, launcher, _) = host();
    let occluder = app
        .world_mut()
        .spawn((
            EntityUuid("opaque-region".into()),
            Transform::from_xyz(0.0, 0.0, -10.0),
            RegionShapeSection(crate::regions::shape::RegionShape::Sphere { radius: 2.0 }),
            phoenix_model::field::SignalMaterialConfig {
                sensor_absorption_per_unit: 0.0,
                comms_absorption_per_unit: 0.0,
                sensor_opaque: true,
                comms_opaque: false,
            },
        ))
        .id();
    app.world_mut()
        .run_system_once(build_torpedo_target_snapshot)
        .unwrap();
    assert!(app
        .world()
        .resource::<TorpedoTargetSnapshot>()
        .guidance
        .is_empty());
    app.world_mut().despawn(occluder);
    app.world_mut()
        .run_system_once(build_torpedo_target_snapshot)
        .unwrap();
    assert!(app
        .world()
        .resource::<TorpedoTargetSnapshot>()
        .guidance
        .contains_key("flight"));
    // Equipment absence is independent of an existing physical target/route.
    app.world_mut()
        .get_mut::<TorpedoSystemResource>(launcher)
        .unwrap()
        .0
        .in_flight[0]
        .profile
        .seeker = None;
    app.world_mut()
        .run_system_once(build_torpedo_target_snapshot)
        .unwrap();
    assert!(app
        .world()
        .resource::<TorpedoTargetSnapshot>()
        .guidance
        .is_empty());
}

#[test]
fn actual_canonical_ship_pose_overrides_display_pose_for_seeker_range_and_sample() {
    let (mut app, _, target) = host();
    app.world_mut()
        .entity_mut(target)
        .insert(crate::ship::state::ShipPhysics {
            x: 0.0,
            y: 3.0,
            z: -22.0,
            ..Default::default()
        });
    app.world_mut()
        .get_mut::<Transform>(target)
        .unwrap()
        .translation = Vec3::new(900.0, 0.0, 900.0);
    app.world_mut()
        .run_system_once(build_torpedo_target_snapshot)
        .unwrap();
    assert_eq!(
        app.world().resource::<TorpedoTargetSnapshot>().guidance["flight"].position,
        [0.0, 3.0, -22.0]
    );
    app.world_mut()
        .get_mut::<crate::ship::state::ShipPhysics>(target)
        .unwrap()
        .z = -90.0;
    app.world_mut()
        .get_mut::<Transform>(target)
        .unwrap()
        .translation = Vec3::new(0.0, 0.0, -20.0);
    app.world_mut()
        .run_system_once(build_torpedo_target_snapshot)
        .unwrap();
    assert!(
        app.world()
            .resource::<TorpedoTargetSnapshot>()
            .guidance
            .is_empty(),
        "an in-range display pose cannot override out-of-range canonical physics"
    );
}

#[test]
fn either_round_or_source_concealment_withholds_new_seeker_evidence_only() {
    let (mut app, launcher, _) = host();
    app.world_mut()
        .run_system_once(build_torpedo_target_snapshot)
        .unwrap();
    let initial = app.world().resource::<TorpedoTargetSnapshot>().guidance["flight"].clone();
    assert!(app
        .world_mut()
        .get_mut::<TorpedoSystemResource>(launcher)
        .unwrap()
        .0
        .in_flight[0]
        .accept_guidance(&initial));
    for observer in ["launcher", "flight"] {
        let mut runtime = crate::world::server::WorldContentRuntime::default();
        crate::gm::operations::contact::set(
            &mut runtime.contact_overrides,
            observer,
            "subject",
            crate::gm::operations::contact::ContactMode::Conceal,
        );
        app.insert_resource(runtime);
        app.world_mut().resource_mut::<SimTick>().0 = 43;
        app.world_mut()
            .run_system_once(build_torpedo_target_snapshot)
            .unwrap();
        let snapshot = app.world().resource::<TorpedoTargetSnapshot>();
        assert!(snapshot.guidance.is_empty(), "observer {observer}");
        assert!(
            snapshot.targets.iter().any(|row| row.0 == "subject"),
            "concealment never grants physical immunity"
        );
        assert_eq!(
            app.world()
                .get::<TorpedoSystemResource>(launcher)
                .unwrap()
                .0
                .in_flight[0]
                .last_guidance
                .as_ref(),
            Some(&initial)
        );
    }
}
