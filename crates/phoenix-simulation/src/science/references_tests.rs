use super::*;
use crate::core::messages::{
    DossierBlackboard, DossierEvidenceSnapshot, DossierSnapshot, HelmBlackboard,
    SensorRadarBlackboard, SystemId, TacticalRadarBlackboard,
};
use phoenix_model::{
    console::{ConsoleFeed, ConsoleObservationKind},
    observation::{
        ObservationMetadata, ObservationProvenance, ObservationSource, ObservationSubject,
        ObservationUncertainty,
    },
    sensing::BroadConfig,
    sensing_references::KnownReferenceMetadata,
};
use phoenix_sim_gameplay::science::tracks::{BroadMeasurement, BroadTrackState, TrackIdentity};

fn dossiers() -> DossierBlackboard {
    let evidence = |id: &str, unusual, tick| DossierEvidenceSnapshot {
        reference: Some(KnownReferenceMetadata {
            id: id.into(),
            unusual,
            reading_ticks: 2,
        }),
        text: format!("text-{id}"),
        provenance: "held-records".into(),
        gathered_at_tick: tick,
        observation: None,
    };
    let mut duplicate = evidence("ambiguous", false, 4);
    duplicate.text = "conflicting-text".into();
    DossierBlackboard {
        subjects: vec![
            DossierSnapshot {
                uuid: "known-one".into(),
                name: "unique".into(),
                evidence: vec![
                    evidence("routine", false, 20),
                    evidence("ambiguous", false, 4),
                    duplicate,
                ],
                ..Default::default()
            },
            DossierSnapshot {
                uuid: "known-two".into(),
                name: "duplicate".into(),
                evidence: vec![evidence("hypothesis", true, 12)],
                ..Default::default()
            },
        ],
    }
}

fn state(owner: &str) -> (BroadTrackState, Vec<SensorTrackSnapshot>) {
    let profile = BroadConfig {
        system: SystemId(owner.into()),
        reading_ticks: 2,
        sample_interval_secs: 1.0,
        identity_secs: 0.0,
        classification_secs: 1.0,
        assessment_secs: 2.0,
        position_error: 1.0,
        drift_error_per_sec: 1.0,
        track_retention_secs: 100.0,
    };
    let physical = |source: &str, x| BroadMeasurement {
        elevation: None,
        source: source.into(),
        observation: None,
        selectable: true,
        position: [x, 0.0],
        support: 1.0,
        identity: Some(TrackIdentity {
            name: "duplicate".into(),
            faction: None,
            references: vec![],
        }),
        classification: None,
        assessment: None,
    };
    let report = BroadMeasurement {
        elevation: None,
        source: "private-reported-source".into(),
        observation: Some(
            ObservationMetadata::at(
                10,
                ObservationSubject::Entity {
                    uuid: "private-reported-source".into(),
                },
                ObservationProvenance {
                    kind: ObservationSource::SensorsReport,
                    system: Some(profile.system.clone()),
                    label: "held-report".into(),
                },
                ObservationUncertainty::Quantized {
                    field: "position".into(),
                    step: 1.0,
                },
            )
            .received(24),
        ),
        selectable: true,
        position: [100.0, 0.0],
        support: 1.0,
        identity: Some(TrackIdentity {
            name: "unique".into(),
            faction: None,
            references: vec![],
        }),
        classification: None,
        assessment: None,
    };
    let mut state = BroadTrackState::new(&profile);
    state
        .advance(
            &profile,
            30,
            1.0,
            1.0,
            &[physical("one", 10.0), physical("two", 30.0), report],
        )
        .unwrap();
    let deliberate = state.current["one"].clone();
    phoenix_sim_gameplay::science::references::attach(
        state.tracks.get_mut(&deliberate).unwrap(),
        &KnownReferenceMetadata {
            id: "hypothesis".into(),
            unusual: true,
            reading_ticks: 2,
        },
        35,
    )
    .unwrap();
    state.validate(&profile, 40, 1.0).unwrap();
    let mut rows = state.projection(&profile, 40, 1.0).unwrap();
    let mut orphan = rows[0].clone();
    orphan.contact_id = "unretained-public-row".into();
    rows.push(orphan);
    (state, rows)
}

fn fixture() -> (App, Entity) {
    let mut config: crate::ship::config::ShipConfig = toml::from_str(
        "[[console]]\nid='allowed'\n[[console]]\nid='restricted'\n[[system]]\nid='sensor'\nkind='sensor-radar'\nconsole='allowed'\n[[system]]\nid='tactical'\nkind='tactical-radar'\nconsole='restricted'\n[[system]]\nid='helm'\nkind='helm-radar'\nconsole='allowed'",
    ).unwrap();
    config.consoles[0].feeds.push(ConsoleFeed {
        source: crate::dossier::dossier_blackboard_key(),
        kind: ConsoleObservationKind::Dossiers,
    });
    let mut states = BTreeMap::new();
    let mut boards = ShipSystemBlackboards::default();
    for owner in ["sensor", "tactical", "helm"] {
        let (state, rows) = state(owner);
        let board = match owner {
            "sensor" => SystemBlackboard::SensorRadar(SensorRadarBlackboard {
                selected_target: Some(rows[0].contact_id.clone()),
                tracks: Some(rows),
                ..Default::default()
            }),
            "tactical" => SystemBlackboard::TacticalRadar(TacticalRadarBlackboard {
                selected_target: Some(rows[0].contact_id.clone()),
                tracks: Some(rows),
                ..Default::default()
            }),
            _ => SystemBlackboard::Helm(HelmBlackboard {
                tracks: Some(rows),
                x: 42.0,
                ..Default::default()
            }),
        };
        states.insert(SystemId(owner.into()), state);
        boards.0.insert(SystemId(owner.into()), board);
    }
    boards.0.insert(
        crate::dossier::dossier_blackboard_key(),
        SystemBlackboard::Dossiers(dossiers()),
    );
    let mut app = App::new();
    app.add_systems(Update, publish);
    let entity = app
        .world_mut()
        .spawn((ShipConfigComponent(config), BroadTracks(states), boards))
        .id();
    (app, entity)
}

/// The former complete-board clone/stage algorithm, retained only as a packet
/// oracle. It intentionally rebuilds the permitted catalogue for each track.
fn legacy_projection(
    config: &ShipConfigComponent,
    states: &BroadTracks,
    boards: &ShipSystemBlackboards,
) -> ShipSystemBlackboards {
    let mut expected = boards.clone();
    for (owner, state) in &states.0 {
        let Some(board) = expected.0.get_mut(owner) else {
            continue;
        };
        let rows = match board {
            SystemBlackboard::SensorRadar(board) => board.tracks.as_mut(),
            SystemBlackboard::TacticalRadar(board) => board.tracks.as_mut(),
            SystemBlackboard::Helm(board) => board.tracks.as_mut(),
            _ => None,
        };
        let Some(rows) = rows else {
            continue;
        };
        let names = phoenix_sim_gameplay::science::references::routine_names(rows);
        let view = config.0.observations_for_system(owner, &boards.0);
        for row in rows {
            row.references = match (
                state.tracks.get(&row.contact_id),
                view.get(&crate::dossier::dossier_blackboard_key()),
            ) {
                (Some(track), Some(SystemBlackboard::Dossiers(dossiers))) => project(
                    track,
                    dossiers,
                    row.identity
                        .as_ref()
                        .is_some_and(|identity| names.contains(&identity.name)),
                ),
                _ => Vec::new(),
            };
        }
    }
    expected
}

#[test]
fn reference_publication_matches_full_board_projection_through_feed_and_dossier_changes() {
    let (mut app, entity) = fixture();
    for step in 0..8 {
        let before = app.world().get::<BroadTracks>(entity).unwrap().0.clone();
        let expected = legacy_projection(
            app.world().get::<ShipConfigComponent>(entity).unwrap(),
            app.world().get::<BroadTracks>(entity).unwrap(),
            app.world().get::<ShipSystemBlackboards>(entity).unwrap(),
        );
        app.update();
        let actual = app.world().get::<ShipSystemBlackboards>(entity).unwrap();
        assert_eq!(
            actual.0, expected.0,
            "complete publication parity at {step}"
        );
        assert_eq!(
            app.world().get::<BroadTracks>(entity).unwrap().0,
            before,
            "reference publication must not mutate evidence"
        );
        let restricted =
            super::super::contact_routing::observations(&actual.0[&SystemId("tactical".into())])
                .unwrap();
        assert!(restricted.iter().all(|row| row.references.is_empty()));
        let rows =
            super::super::contact_routing::observations(&actual.0[&SystemId("sensor".into())])
                .unwrap();
        if matches!(step, 0 | 1 | 3 | 6) {
            let refs: Vec<_> = rows.iter().flat_map(|row| &row.references).collect();
            assert_eq!(refs.len(), 2);
            assert_eq!(
                refs.iter()
                    .find(|reference| reference.reference.id == "routine")
                    .unwrap()
                    .attached_at_tick,
                if step < 2 { 24 } else { 29 }
            );
            assert_eq!(
                refs.iter()
                    .find(|reference| reference.reference.id == "hypothesis")
                    .unwrap()
                    .attached_at_tick,
                35
            );
            assert!(refs
                .iter()
                .all(|reference| reference.reference.id != "ambiguous"));
        } else {
            assert!(
                rows.iter().all(|row| row.references.is_empty()),
                "missing/wrong typed feed clears old annotations"
            );
        }
        match step {
            1 => {
                app.world_mut()
                    .get_mut::<ShipSystemBlackboards>(entity)
                    .unwrap()
                    .0
                    .remove(&crate::dossier::dossier_blackboard_key());
            }
            2 => {
                let mut changed = dossiers();
                changed.subjects[0].evidence[0].gathered_at_tick = 29;
                app.world_mut()
                    .get_mut::<ShipSystemBlackboards>(entity)
                    .unwrap()
                    .0
                    .insert(
                        crate::dossier::dossier_blackboard_key(),
                        SystemBlackboard::Dossiers(changed),
                    );
            }
            3 => {
                app.world_mut()
                    .get_mut::<ShipConfigComponent>(entity)
                    .unwrap()
                    .0
                    .consoles[0]
                    .feeds
                    .clear();
            }
            4 => {
                app.world_mut()
                    .get_mut::<ShipConfigComponent>(entity)
                    .unwrap()
                    .0
                    .consoles[0]
                    .feeds
                    .push(ConsoleFeed {
                        source: crate::dossier::dossier_blackboard_key(),
                        kind: ConsoleObservationKind::Helm,
                    });
            }
            5 => {
                app.world_mut()
                    .get_mut::<ShipConfigComponent>(entity)
                    .unwrap()
                    .0
                    .consoles[0]
                    .feeds[0]
                    .kind = ConsoleObservationKind::Dossiers;
            }
            6 => {
                app.world_mut()
                    .get_mut::<ShipConfigComponent>(entity)
                    .unwrap()
                    .0
                    .consoles[0]
                    .feeds[0]
                    .kind = ConsoleObservationKind::Helm;
            }
            _ => {}
        }
    }
}
