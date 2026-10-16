//! Narrow physical-parent restore preconditions, before the full host tracer.
use super::*;

fn fixture() -> (World, Entity, EntityState) {
    let config = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    let damage = crate::ship::compartment_damage::ShipDamage::from_config(&config)
        .unwrap()
        .unwrap();
    let mut world = World::new();
    let row = EntityState {
        uuid: crate::world_id::WorldId::new(crate::world_id::IdNamespace::Entity, 0, 0).render(),
        physics: Some([0.0; 8]),
        physical_damage: Some(damage.state.clone()),
        ..Default::default()
    };
    let entity = world
        .spawn((
            EntityUuid(row.uuid.clone()),
            crate::ship::state::ShipPhysics::default(),
            crate::entities::spawner::ColliderSection(config.collider.unwrap()),
            damage,
        ))
        .id();
    (world, entity, row)
}

#[test]
fn actual_and_complete_candidate_parent_guards_exclude_all_carry_capable_equipment() {
    let (mut world, entity, row) = fixture();
    parent(&world, &row.uuid, None).unwrap();
    parent(&world, &row.uuid, Some(std::slice::from_ref(&row))).unwrap();
    // The accepted derived profile alone already makes a body carry-capable.
    world
        .entity_mut(entity)
        .insert(crate::entities::craft::PhysicalProbe(
            phoenix_model::craft::ProbeProfile {
                space: 1,
                passenger_berths: vec![],
            },
        ));
    assert!(parent(&world, &row.uuid, None).is_err());
    assert!(parent(&world, &row.uuid, Some(std::slice::from_ref(&row))).is_err());
    world
        .entity_mut(entity)
        .remove::<crate::entities::craft::PhysicalProbe>();
    // Current deployed custody is just as unsupported as stored custody.
    world
        .entity_mut(entity)
        .insert(crate::entities::craft::ProbeCustody::default());
    assert!(parent(&world, &row.uuid, None).is_err());
    assert!(parent(&world, &row.uuid, Some(std::slice::from_ref(&row))).is_err());
    world
        .entity_mut(entity)
        .remove::<crate::entities::craft::ProbeCustody>();
    let mut candidate = row.clone();
    candidate.probe_custody = Some(crate::entities::craft::ProbeCustody::default());
    assert!(parent(&world, &row.uuid, Some(&[candidate])).is_err());
    // Candidate accepted recipe takes precedence over the old live hull, even
    // when a forged candidate has omitted its required custody state.
    let mut candidate = row.clone();
    candidate.spawn = Some(crate::world::spawn_origin::SpawnOrigin {
        template_path: "assets/entities/physical_probe.toml".into(),
        ..Default::default()
    });
    assert!(parent(&world, &row.uuid, Some(&[candidate])).is_err());
}

#[test]
fn complete_candidate_uses_its_saved_parent_pose_instead_of_stale_live_pose() {
    let (world, _, mut row) = fixture();
    row.physics = Some([12.0, 2.0, -8.0, 0.7, 3.0, 0.5, 1.0, 0.0]);
    assert_eq!(
        values(
            parent(&world, &row.uuid, Some(std::slice::from_ref(&row)))
                .unwrap()
                .0
        ),
        row.physics.unwrap()
    );
    assert_eq!(values(parent(&world, &row.uuid, None).unwrap().0), [0.0; 8]);
}
