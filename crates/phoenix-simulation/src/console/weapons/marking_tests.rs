//! Isolated actual physical-owner proofs; no full-host or paid-report claim.
use super::*;
use bevy::ecs::system::SystemState;
use phoenix_model::marking::{
    FrozenMarkingPayload, MarkingAttachment, MarkingLocation, MarkingPayloadConfig,
};

fn entity_id(seq: u64) -> String {
    crate::world_id::WorldId::new(crate::world_id::IdNamespace::Entity, 0, seq).render()
}
fn attachment(subject: &str) -> ShipMarkingAttachment {
    ShipMarkingAttachment(MarkingAttachment {
        round: crate::world_id::WorldId::new(crate::world_id::IdNamespace::Projectile, 2, 0)
            .render(),
        source: entity_id(4),
        payload: FrozenMarkingPayload {
            recipe: MarkingPayloadConfig {
                template: "assets/entities/marking_beacon.toml".into(),
            },
            faction: uuid::Uuid::from_u128(7),
        },
        contact_tick: 3,
        location: MarkingLocation::Attached {
            subject: subject.into(),
            offset: [1.0, 0.0, 0.0],
        },
        report: None,
    })
}
fn hull(world: &mut World, id: &str) -> Entity {
    let config = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    let collider = config.collider.as_ref().unwrap();
    world
        .spawn((
            EntityUuid(id.into()),
            ShipPhysics::default(),
            crate::entities::spawner::physical_collider(collider),
            ColliderScale::Absolute(Vec3::ONE),
            ColliderSection(collider.clone()),
            crate::ship::compartment_damage::ShipDamage::from_config(&config)
                .unwrap()
                .unwrap(),
        ))
        .id()
}

#[test]
fn actual_contact_query_excludes_each_carry_capability_even_while_deployed() {
    let mut world = World::new();
    let independent = hull(&mut world, &entity_id(0));
    let profiled = hull(&mut world, &entity_id(1));
    let custodied = hull(&mut world, &entity_id(2));
    world
        .entity_mut(profiled)
        .insert(crate::entities::craft::PhysicalProbe(
            phoenix_model::craft::ProbeProfile {
                space: 1,
                passenger_berths: vec![],
            },
        ));
    world
        .entity_mut(custodied)
        .insert(crate::entities::craft::ProbeCustody::default());
    let mut query = SystemState::<MarkingContacts>::new(&mut world);
    let contacts = query.get(&world);
    let rows: Vec<_> = contacts.hulls.iter().map(|row| row.0 .0.clone()).collect();
    assert_eq!(
        rows,
        vec![world.get::<EntityUuid>(independent).unwrap().0.clone()]
    );
}

#[test]
fn mechanical_pair_exclusion_is_exact_symmetric_and_ends_on_detach() {
    let mut world = World::new();
    let parent = world.spawn(EntityUuid(entity_id(0))).id();
    let child = world
        .spawn((EntityUuid(entity_id(1)), attachment(&entity_id(0))))
        .id();
    let other = world.spawn(EntityUuid(entity_id(2))).id();
    let mut query = SystemState::<MechanicalPairs>::new(&mut world);
    let pairs = query.get(&world);
    assert!(pairs.connected(parent, child));
    assert!(pairs.connected(child, parent));
    assert!(!pairs.connected(child, other));
    assert!(!pairs.connected(parent, other));
    world
        .get_mut::<ShipMarkingAttachment>(child)
        .unwrap()
        .0
        .location = MarkingLocation::Detached { tick: 4 };
    assert!(!query.get(&world).connected(parent, child));
}

#[test]
fn actual_removal_transaction_detaches_before_immediate_capture_without_projection() {
    let mut app = App::new();
    crate::entities::craft::register(&mut app);
    app.insert_resource(crate::sim_tick::SimTick(8));
    // Bare physical-owner rig: register capture's optional schema without
    // installing another hull, controller or work owner on either actor.
    app.world_mut().register_component::<ShipPhysics>();
    app.world_mut()
        .register_component::<crate::entities::spawner::EntitySystemHull>();
    app.world_mut()
        .register_component::<crate::ship::state::ShipRedAlert>();
    app.world_mut()
        .register_component::<crate::console::command::server::ShipStationStances>();
    app.world_mut()
        .register_component::<crate::ship_plugin::ShipSystemControlSources>();
    let parent = app.world_mut().spawn(EntityUuid(entity_id(0))).id();
    let last = ShipPhysics {
        x: 12.0,
        y: 4.0,
        z: -3.0,
        yaw: 0.3,
        roll: 0.5,
        forward_speed: 2.0,
        lateral_speed: 3.0,
        vertical_speed: 1.0,
    };
    app.world_mut()
        .spawn((EntityUuid(entity_id(1)), last, attachment(&entity_id(0))));
    app.world_mut().despawn(parent);
    app.world_mut().flush();
    let captured = crate::snapshot::capture(app.world());
    assert!(captured.entities.iter().all(|row| row.uuid != entity_id(0)));
    let row = captured
        .entities
        .iter()
        .find(|row| row.uuid == entity_id(1))
        .unwrap();
    assert_eq!(
        row.marking_attachment.as_ref().unwrap().location,
        MarkingLocation::Detached { tick: 8 }
    );
    assert_eq!(
        row.physics,
        Some([last.x, last.y, last.z, last.yaw, 0.0, last.roll, 0.0, 0.0])
    );
}
