use super::*;

#[derive(Component)]
struct MovingProbe;

#[derive(Resource, Default)]
struct Observations(Vec<(f32, f32, bool)>);

fn move_probes(mut probes: Query<&mut Transform, With<MovingProbe>>) {
    for mut transform in &mut probes {
        transform.translation.x += 2.0;
    }
}

fn observe_probes(
    context: ReadRapierContext,
    probes: Query<(Entity, &Transform, &GlobalTransform), With<MovingProbe>>,
    mut observations: ResMut<Observations>,
) {
    let context = context.single().unwrap();
    for (entity, local, global) in &probes {
        observations.0.push((
            local.translation.x,
            global.translation().x,
            context
                .contact_pairs_with(entity)
                .any(|pair| pair.has_any_active_contact()),
        ));
    }
}

#[test]
fn physics_propagates_roots_with_visual_children_on_every_fixed_step() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, bevy::transform::TransformPlugin));
    app.add_plugins(bevy::asset::AssetPlugin::default())
        .init_asset::<Mesh>()
        .init_resource::<bevy::scene::SceneSpawner>();
    app.insert_resource(bevy::transform::systems::StaticTransformOptimizations::enabled());
    register_physics(&mut app);
    app.init_resource::<Observations>()
        .add_systems(
            FixedUpdate,
            move_probes.in_set(crate::sim_sets::SimSet::Physics),
        )
        .add_systems(FixedUpdate, observe_probes.after(PhysicsSet::Writeback));
    for (y, child) in [(0.0, false), (10.0, true)] {
        let root = app
            .world_mut()
            .spawn((
                MovingProbe,
                Transform::from_xyz(0.0, y, 0.0),
                RigidBody::KinematicPositionBased,
                Collider::ball(1.0),
                ActiveCollisionTypes::KINEMATIC_STATIC,
            ))
            .id();
        if child {
            app.world_mut().spawn((Transform::default(), ChildOf(root)));
        }
        app.world_mut().spawn((
            Transform::from_xyz(10.0, y, 0.0),
            RigidBody::Fixed,
            Collider::ball(1.0),
        ));
    }
    let period = std::time::Duration::from_millis(10);
    app.world_mut()
        .resource_mut::<Time<Fixed>>()
        .set_timestep(period);
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
        std::time::Duration::ZERO,
    ));
    app.update();
    // Multiple complete fixed steps between rendered frames expose stale
    // tree markers. A visual child must not change collision authority.
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(period * 6));
    app.update();
    let observations = &app.world().resource::<Observations>().0;
    assert_eq!(observations.len(), 12);
    for pair in observations.chunks_exact(2) {
        for &(local, global, _) in pair {
            assert_eq!(local, global, "Rapier must receive this fixed step's pose");
        }
        assert_eq!(
            pair[0].2, pair[1].2,
            "a visual child cannot change contacts"
        );
    }
    assert!(
        observations.iter().any(|row| row.2),
        "the probe must actually collide"
    );
}
