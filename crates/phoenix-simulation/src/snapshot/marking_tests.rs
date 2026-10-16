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

#[test]
fn report_frontier_is_bounded_by_original_own_candidate_sensor_allocation_and_clock() {
    // Schema-owner proof, not an assertion that this pure input paid for work.
    // The real payment/observation/report owner pipeline has separate tests.
    use phoenix_model::{marking::*, sensing_sharing::RadarSampleOrigin};
    use phoenix_sim_gameplay::science::{
        sharing::{export_samples, origin_handle, sample_id},
        tracks::{BroadMeasurement, BroadTrackState},
    };
    let config =
        crate::entities::include_resolve::load_entity_config("assets/entities/marking_beacon.toml")
            .unwrap();
    let equipment = config.marking_beacon.as_ref().unwrap();
    let profile = &config.sensing.as_ref().unwrap().broad[0];
    let mut state = BroadTrackState::new(profile);
    state
        .advance(
            profile,
            10,
            0.5,
            1.0,
            &[BroadMeasurement {
                source: "schema-subject".into(),
                observation: None,
                selectable: true,
                position: [1.0, 2.0],
                elevation: None,
                support: 1.0,
                identity: None,
                classification: None,
                assessment: None,
            }],
        )
        .unwrap();
    let uuid = crate::world_id::WorldId::new(crate::world_id::IdNamespace::Entity, 3, 0).render();
    let sample = export_samples(&uuid, &state, profile, 10, 0.5)
        .unwrap()
        .remove(0);
    let mut row = EntityState {
        uuid,
        broad: Some([(equipment.sensor.clone(), state)].into_iter().collect()),
        marking_attachment: Some(MarkingAttachment {
            round: crate::world_id::WorldId::new(crate::world_id::IdNamespace::Projectile, 2, 0)
                .render(),
            source: crate::world_id::WorldId::new(crate::world_id::IdNamespace::Entity, 0, 4)
                .render(),
            payload: FrozenMarkingPayload {
                recipe: MarkingPayloadConfig {
                    template: "assets/entities/marking_beacon.toml".into(),
                },
                faction: uuid::Uuid::from_u128(7),
            },
            contact_tick: 3,
            location: MarkingLocation::Detached { tick: 12 },
            report: Some(MarkingReportFrontier {
                sample: sample.provenance.sample_id,
                observed_tick: 10,
                emitted_tick: 10,
            }),
        }),
        ..Default::default()
    };
    validate_report_frontier(&row, equipment).unwrap();
    for fault in 0..7 {
        let mut bad = row.clone();
        let report = bad
            .marking_attachment
            .as_mut()
            .unwrap()
            .report
            .as_mut()
            .unwrap();
        match fault {
            0 => report.emitted_tick = 11,
            1 => report.sample = report.sample.replace("radar-origin-", "other-origin-"),
            2 => {
                report.sample = sample_id(
                    &RadarSampleOrigin {
                        source: origin_handle("foreign-body"),
                        system: equipment.sensor.clone(),
                        contact: sample.provenance.origin.contact.clone(),
                    },
                    10,
                )
            }
            3 => {
                report.sample = sample_id(
                    &RadarSampleOrigin {
                        source: origin_handle(&bad.uuid),
                        system: equipment.radio.clone(),
                        contact: sample.provenance.origin.contact.clone(),
                    },
                    10,
                )
            }
            4 => {
                let state = bad
                    .broad
                    .as_mut()
                    .unwrap()
                    .get_mut(&equipment.sensor)
                    .unwrap();
                state.next_contact = 1;
            }
            5 => {
                bad.broad
                    .as_mut()
                    .unwrap()
                    .get_mut(&equipment.sensor)
                    .unwrap()
                    .last_sample_tick = Some(9)
            }
            _ => bad.broad = None,
        }
        assert!(
            validate_report_frontier(&bad, equipment).is_err(),
            "report forgery {fault}"
        );
    }
    let state = row
        .broad
        .as_mut()
        .unwrap()
        .get_mut(&equipment.sensor)
        .unwrap();
    state.advance(profile, 1000, 0.5, 0.0, &[]).unwrap();
    assert!(state.tracks.is_empty());
    validate_report_frontier(&row, equipment).unwrap(); // no resurrection/freshening
}
