use super::*;
use phoenix_sim_gameplay::ai::{state_tree::TreeRuntime, text_reading::TextReading};
use phoenix_sim_gameplay::science::tracks::{BroadMeasurement, BroadTrackState};

fn candidate() -> (World, PhoenixSnapshot) {
    let config = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    let profile = &config.sensing.as_ref().unwrap().broad[0];
    let mut state = BroadTrackState::new(profile);
    state
        .advance(
            profile,
            10,
            1.0,
            1.0,
            &[BroadMeasurement {
                elevation: None,
                source: "physical-subject".into(),
                observation: None,
                selectable: true,
                position: [5.0, 0.0],
                support: 1.0,
                identity: None,
                classification: None,
                assessment: None,
            }],
        )
        .unwrap();
    state
        .tracks
        .values_mut()
        .next()
        .unwrap()
        .attached_references
        .insert("held-record".into(), 12);
    let mut evidence = phoenix_sim_world::dossier::evidence::EvidenceLog::default();
    evidence
        .append_reference(
            "original-subject",
            "held.text",
            phoenix_sim_world::dossier::evidence::EvidenceProvenance::Records,
            8,
            Some(phoenix_model::sensing_references::KnownReferenceMetadata {
                id: "held-record".into(),
                unusual: true,
                reading_ticks: 2,
            }),
        )
        .unwrap();
    let mut world = World::new();
    world.insert_resource(crate::sim_tick::SimTick(7));
    world.insert_resource(Time::<Fixed>::default());
    world.spawn((
        EntityUuid("surplus-must-survive".into()),
        ShipRedAlert(false),
    ));
    let snapshot = PhoenixSnapshot {
        tick: 20,
        scenario: Some(ScenarioState {
            evidence,
            ..Default::default()
        }),
        entities: vec![EntityState {
            uuid: "candidate-ship".into(),
            broad: Some(std::collections::BTreeMap::from([(
                profile.system.clone(),
                state,
            )])),
            ai_trees: Some(crate::ai::state_tree_runtime::ShipAiTreeRuntime(
                std::collections::BTreeMap::from([(
                    profile.system.clone(),
                    TreeRuntime {
                        // Reading has its own cadence, independent of evaluation.
                        last_tick: Some(1),
                        reading: Some(TextReading {
                            key: "held semantic text".into(),
                            required_ticks: 2,
                            completed_ticks: 1,
                            last_tick: Some(20),
                        }),
                        ..Default::default()
                    },
                )]),
            )),
            ..Default::default()
        }],
        ..Default::default()
    };
    (world, snapshot)
}

#[test]
fn candidate_findings_and_reading_have_independent_valid_continuation_clocks() {
    let (world, snapshot) = candidate();
    let decoded = serde_json::from_slice(&serde_json::to_vec(&snapshot).unwrap()).unwrap();
    assert!(validate_roster(&world, &decoded).is_ok());
}

#[test]
fn reference_attachment_resolves_candidate_evidence_and_original_acquisition_time() {
    for mutation in 0..3 {
        let (world, mut snapshot) = candidate();
        let evidence = &mut snapshot.scenario.as_mut().unwrap().evidence;
        match mutation {
            0 => evidence.entries.clear(),
            1 => evidence.entries[0].gathered_at_tick = 13,
            _ => evidence.entries.push(evidence.entries[0].clone()),
        }
        assert!(validate_roster(&world, &snapshot).is_err());
    }
}

#[test]
fn future_reading_refuses_whole_restore_before_clock_or_roster_changes() {
    let (mut world, mut snapshot) = candidate();
    snapshot.entities[0]
        .ai_trees
        .as_mut()
        .unwrap()
        .0
        .values_mut()
        .next()
        .unwrap()
        .reading
        .as_mut()
        .unwrap()
        .last_tick = Some(21);
    let before = capture(&world);
    let report = restore(&mut world, &snapshot);
    assert!(!report.is_complete());
    assert!(
        matches!(report.gaps.as_slice(), [RestoreGap::ResourceStateInvalid { uuid, reason }]
        if uuid == "candidate-ship" && reason == "invalid AI semantic reading frontier")
    );
    assert_eq!(report.entities_spawned, 0);
    assert_eq!(report.despawned, 0);
    assert_eq!(world.resource::<crate::sim_tick::SimTick>().0, 7);
    assert!(world
        .query::<&EntityUuid>()
        .iter(&world)
        .any(|uuid| uuid.0 == "surplus-must-survive"));
    assert_eq!(capture(&world), before);
}
