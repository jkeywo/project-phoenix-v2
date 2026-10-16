use super::physics_systems::*;
use crate::entities::spawner::EntityUuid;
use crate::weapons::marking::ShipMarkingAttachment;
use bevy::prelude::*;
use phoenix_model::marking::*;

fn attachment(subject: &str) -> ShipMarkingAttachment {
    ShipMarkingAttachment(MarkingAttachment {
        round: crate::world_id::WorldId::new(crate::world_id::IdNamespace::Projectile, 3, 0)
            .render(),
        source: crate::world_id::WorldId::new(crate::world_id::IdNamespace::Entity, 0, 1).render(),
        payload: FrozenMarkingPayload {
            recipe: MarkingPayloadConfig {
                template: "assets/entities/marking_beacon.toml".into(),
            },
            faction: uuid::Uuid::from_u128(7),
        },
        contact_tick: 4,
        location: MarkingLocation::Attached {
            subject: subject.into(),
            offset: [1.0, 2.0, 3.0],
        },
        report: None,
    })
}
#[test]
fn actual_subject_pose_and_removal_project_without_a_second_motion_integrator() {
    let mut app = App::new();
    app.insert_resource(crate::sim_tick::SimTick(8));
    app.add_systems(Update, project_marking_attachments);
    let id = crate::world_id::WorldId::new(crate::world_id::IdNamespace::Entity, 0, 0).render();
    let parent_pose = ShipPhysics {
        x: 10.0,
        y: 3.0,
        z: 20.0,
        yaw: 0.7,
        roll: 0.5,
        forward_speed: 4.0,
        lateral_speed: 2.0,
        vertical_speed: 1.0,
    };
    let parent = app
        .world_mut()
        .spawn((EntityUuid(id.clone()), parent_pose))
        .id();
    let marker = app
        .world_mut()
        .spawn((
            ShipPhysics::default(),
            Transform::default().with_scale(Vec3::splat(100.0)),
            attachment(&id),
        ))
        .id();
    app.update();
    let first = *app.world().get::<ShipPhysics>(marker).unwrap();
    assert_eq!(first, marking_attached_pose(&parent_pose, [1.0, 2.0, 3.0]));
    let transform = app.world().get::<Transform>(marker).unwrap();
    assert_eq!(transform.rotation, physical_rotation(&parent_pose));
    assert_eq!(transform.scale, Vec3::splat(100.0));
    let next_pose = ShipPhysics {
        x: -5.0,
        yaw: -0.3,
        roll: -0.8,
        ..parent_pose
    };
    *app.world_mut().get_mut::<ShipPhysics>(parent).unwrap() = next_pose;
    // Disabled is not disappearance, destruction or an observation timeout.
    app.world_mut()
        .entity_mut(parent)
        .insert(bevy::ecs::entity_disabling::Disabled);
    app.update();
    let last = *app.world().get::<ShipPhysics>(marker).unwrap();
    assert_eq!(last, marking_attached_pose(&next_pose, [1.0, 2.0, 3.0]));
    assert!(matches!(
        app.world()
            .get::<ShipMarkingAttachment>(marker)
            .unwrap()
            .0
            .location,
        MarkingLocation::Attached { .. }
    ));
    app.world_mut().despawn(parent);
    app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 = 9;
    app.update();
    let stopped = *app.world().get::<ShipPhysics>(marker).unwrap();
    assert_eq!(
        [stopped.x, stopped.y, stopped.z, stopped.yaw, stopped.roll],
        [last.x, last.y, last.z, last.yaw, last.roll]
    );
    assert_eq!(
        [
            stopped.forward_speed,
            stopped.lateral_speed,
            stopped.vertical_speed
        ],
        [0.0; 3]
    );
    assert_eq!(
        app.world()
            .get::<ShipMarkingAttachment>(marker)
            .unwrap()
            .0
            .location,
        MarkingLocation::Detached { tick: 9 }
    );
}
