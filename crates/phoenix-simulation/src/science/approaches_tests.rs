use super::*;
use crate::core::messages::{HelmBlackboard, SystemId, TacticalRadarBlackboard};
use bevy::ecs::system::RunSystemOnce;

#[test]
fn actual_helm_estimates_only_explicitly_permitted_tracks_and_rebuilds_after_feed_loss() {
    let config = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    let mut app = App::new();
    let entity = crate::entities::spawner::spawn_entity(
        &mut app.world_mut().commands(),
        &config,
        Vec3::ZERO,
        "approach-observer".into(),
        None,
    );
    app.world_mut().flush();
    let record = serde_json::from_value(serde_json::json!({
        "contact_id":"opaque-torpedo", "observed_tick":2, "position":[100,20],
        "velocity":[-10,0], "position_error":1, "selectable":false,
        "stage":"classified", "identity":null, "identity_tick":null,
        "classification":"torpedo", "classification_tick":2,
        "assessment":null, "assessment_tick":null, "predicted":false,
        "samples":[{"origin":{"source":"public-origin", "system":"tactical-radar", "contact":"opaque-torpedo"},
            "sample_id":"sample-2", "observed_tick":2, "observer_position":[0,0],
            "velocity_error":0.5,"drift_error_per_sec":0.1,"received_tick":null}]
    })).unwrap();
    let owner = SystemId("helm-radar".into());
    let mut boards = app
        .world_mut()
        .get_mut::<ShipSystemBlackboards>(entity)
        .unwrap();
    boards.0.insert(
        owner.clone(),
        SystemBlackboard::Helm(HelmBlackboard {
            movement: None,
            tracks: Some(Vec::new()),
            ..Default::default()
        }),
    );
    boards.0.insert(
        SystemId("tactical-radar".into()),
        SystemBlackboard::TacticalRadar(TacticalRadarBlackboard {
            tracks: Some(vec![record]),
            ..Default::default()
        }),
    );
    drop(boards);
    app.world_mut().run_system_once(publish).unwrap();
    let result = |app: &App| {
        let SystemBlackboard::Helm(board) =
            &app.world().get::<ShipSystemBlackboards>(entity).unwrap().0[&owner]
        else {
            panic!("actual Helm publication");
        };
        board.approaches.clone()
    };
    assert_eq!(result(&app).len(), 1);
    assert_eq!(result(&app)[0].seconds, 10.0);
    let before = *app.world().get::<ShipPhysics>(entity).unwrap();
    let mut config = app
        .world_mut()
        .get_mut::<ShipConfigComponent>(entity)
        .unwrap();
    let console = config.0.system(&owner).unwrap().console.clone().unwrap();
    config
        .0
        .consoles
        .iter_mut()
        .find(|row| row.id == console)
        .unwrap()
        .feeds
        .retain(|feed| feed.source.0 != "tactical-radar");
    drop(config);
    app.world_mut().run_system_once(publish).unwrap();
    assert!(
        result(&app).is_empty(),
        "co-hosted/available foreign board grants no implicit feed"
    );
    assert_eq!(
        *app.world().get::<ShipPhysics>(entity).unwrap(),
        before,
        "an evidence estimate never issues avoidance or changes motion"
    );
}

// Frozen 1bf59f86 actual producer; keep record copies before the horizon gate.
fn cloning_publish(
    mut ships: Query<(
        &ShipConfigComponent,
        &ShipPhysics,
        &crate::entities::spawner::HelmConsoleSection,
        &mut ShipSystemBlackboards,
    )>,
) {
    for (config, physics, helm, mut boards) in &mut ships {
        let horizon = helm
            .0
            .projection
            .as_ref()
            .and_then(|profile| profile.horizon());
        let sin = simmath::sin(physics.yaw);
        let cos = simmath::cos(physics.yaw);
        let velocity = [
            sin * physics.forward_speed + cos * physics.lateral_speed,
            -cos * physics.forward_speed + sin * physics.lateral_speed,
        ];
        let mut projected = Vec::new();
        for system in &config.0.systems {
            if system.kind != crate::ship::system_registry::HELM_RADAR_KIND {
                continue;
            }
            let permitted = config.0.observations_for_system(&system.id, &boards.0);
            let records = config
                .0
                .systems
                .iter()
                .filter_map(|source| permitted.get(&source.id))
                .flat_map(|board| match board {
                    SystemBlackboard::Helm(board) => board.tracks.as_deref().unwrap_or_default(),
                    SystemBlackboard::TacticalRadar(board) => {
                        board.tracks.as_deref().unwrap_or_default()
                    }
                    SystemBlackboard::SensorRadar(board) => {
                        board.tracks.as_deref().unwrap_or_default()
                    }
                    _ => &[],
                })
                .cloned()
                .collect::<Vec<_>>();
            let approaches = horizon.map_or_else(Vec::new, |horizon| {
                phoenix_sim_gameplay::science::approaches::derive(
                    &records,
                    [physics.x, physics.z],
                    velocity,
                    horizon,
                )
            });
            projected.push((system.id.clone(), approaches));
        }
        for (owner, approaches) in projected {
            if let Some(SystemBlackboard::Helm(board)) = boards.0.get_mut(&owner) {
                board.approaches = approaches;
            }
        }
    }
}

#[test]
fn horizon_gating_matches_frozen_full_producer_and_change_ticks() {
    use crate::core::messages::{CommsBlackboard, SensorRadarBlackboard};
    use phoenix_model::console::{ConsoleFeed, ConsoleObservationKind};
    use phoenix_model::sensing_tracks::SensorTrackSnapshot;
    let config = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    let mut app = App::new();
    let entity = crate::entities::spawner::spawn_entity(
        &mut app.world_mut().commands(),
        &config,
        Vec3::ZERO,
        "approach-observer".into(),
        None,
    );
    app.world_mut().flush();
    let owner = SystemId("helm-radar".into());
    let tactical = SystemId("tactical-radar".into());
    let sensor = SystemId("sensor-radar".into());
    let track = |id: &str| -> SensorTrackSnapshot {
        serde_json::from_value(serde_json::json!({
            "contact_id": id, "observed_tick":2, "position":[100,20],
            "velocity":[-10,0], "position_error":1, "selectable":false,
            "stage":"classified", "identity":null, "identity_tick":null,
            "classification":"torpedo", "classification_tick":2,
            "assessment":null, "assessment_tick":null, "predicted":false,
            "samples":[{"origin":{"source":"public-origin", "system":"tactical-radar", "contact":id},
                "sample_id":"sample-2", "observed_tick":2, "observer_position":[0,0],
                "velocity_error":0.5,"drift_error_per_sec":0.1,"received_tick":null}]
        })).unwrap()
    };
    let current = track("z-current");
    let mut predicted = track("a-predicted");
    predicted.predicted = true;
    let mut held = track("old-original-clock");
    held.observed_tick = 0;
    let mut receding = track("receding");
    receding.velocity = Some([10.0, 0.0]);
    let mut unknown = track("unknown-motion");
    unknown.velocity = None;
    let mut no_error = track("no-sample-errors");
    no_error.samples.clear();
    {
        let mut config = app
            .world_mut()
            .get_mut::<ShipConfigComponent>(entity)
            .unwrap();
        let console = config.0.system(&owner).unwrap().console.clone().unwrap();
        let feeds = &mut config
            .0
            .consoles
            .iter_mut()
            .find(|row| row.id == console)
            .unwrap()
            .feeds;
        for (source, kind) in [
            (&tactical, ConsoleObservationKind::TacticalRadar),
            (&sensor, ConsoleObservationKind::SensorRadar),
        ] {
            feeds.retain(|feed| &feed.source != source);
            feeds.push(ConsoleFeed {
                source: source.clone(),
                kind,
            });
        }
    }
    {
        let mut boards = app
            .world_mut()
            .get_mut::<ShipSystemBlackboards>(entity)
            .unwrap();
        boards.0.insert(
            owner.clone(),
            SystemBlackboard::Helm(HelmBlackboard {
                tracks: Some(Vec::new()),
                // A disabled horizon must still actively clear old estimates.
                approaches: phoenix_sim_gameplay::science::approaches::derive(
                    std::slice::from_ref(&current),
                    [0.0, 0.0],
                    [0.0, 0.0],
                    30.0,
                ),
                ..Default::default()
            }),
        );
        boards.0.insert(
            tactical.clone(),
            SystemBlackboard::TacticalRadar(TacticalRadarBlackboard {
                tracks: Some(vec![
                    current.clone(),
                    predicted,
                    held,
                    receding,
                    unknown,
                    no_error,
                ]),
                ..Default::default()
            }),
        );
        boards.0.insert(
            sensor.clone(),
            SystemBlackboard::SensorRadar(SensorRadarBlackboard {
                tracks: Some(vec![current.clone()]),
                ..Default::default()
            }),
        );
        // Wrong typed publication and undeclared source cannot gain scope.
        boards.0.insert(
            SystemId("comms".into()),
            SystemBlackboard::TacticalRadar(TacticalRadarBlackboard {
                tracks: Some(vec![track("wrong-kind-secret")]),
                ..Default::default()
            }),
        );
        boards.0.insert(
            SystemId("undeclared-source".into()),
            SystemBlackboard::SensorRadar(SensorRadarBlackboard {
                tracks: Some(vec![track("undeclared-secret")]),
                ..Default::default()
            }),
        );
    }
    for stage in 0..11 {
        {
            let mut helm = app
                .world_mut()
                .get_mut::<crate::entities::spawner::HelmConsoleSection>(entity)
                .unwrap();
            helm.0.projection = match stage {
                0 => None,
                2 => Some(serde_json::from_value(serde_json::json!({"horizon_secs":5.0})).unwrap()),
                4 => Some(serde_json::from_value(serde_json::json!(false)).unwrap()),
                _ => {
                    Some(serde_json::from_value(serde_json::json!({"horizon_secs":30.0})).unwrap())
                }
            };
        }
        if stage == 3 {
            let mut conflict = current.clone();
            conflict.position[0] += 1.0;
            let mut boards = app
                .world_mut()
                .get_mut::<ShipSystemBlackboards>(entity)
                .unwrap();
            let SystemBlackboard::SensorRadar(board) = boards.0.get_mut(&sensor).unwrap() else {
                unreachable!()
            };
            board.tracks = Some(vec![conflict]);
        } else if stage == 5 {
            let encoded =
                serde_json::to_vec(&app.world().get::<ShipSystemBlackboards>(entity).unwrap().0)
                    .unwrap();
            app.world_mut()
                .get_mut::<ShipSystemBlackboards>(entity)
                .unwrap()
                .0 = serde_json::from_slice(&encoded).unwrap();
        } else if stage == 6 {
            let mut config = app
                .world_mut()
                .get_mut::<ShipConfigComponent>(entity)
                .unwrap();
            let console = config.0.system(&owner).unwrap().console.clone().unwrap();
            config
                .0
                .consoles
                .iter_mut()
                .find(|row| row.id == console)
                .unwrap()
                .feeds
                .retain(|feed| feed.source != tactical);
        } else if stage == 7 {
            // The upstream publisher has withheld its observations (e.g. on
            // concealment/loss). This consumer must not recover hidden truth.
            let mut boards = app
                .world_mut()
                .get_mut::<ShipSystemBlackboards>(entity)
                .unwrap();
            let SystemBlackboard::SensorRadar(board) = boards.0.get_mut(&sensor).unwrap() else {
                unreachable!()
            };
            board.tracks = None;
        } else if stage == 8 {
            app.world_mut()
                .get_mut::<ShipSystemBlackboards>(entity)
                .unwrap()
                .0
                .insert(
                    owner.clone(),
                    SystemBlackboard::Comms(CommsBlackboard::default()),
                );
        } else if stage == 9 {
            app.world_mut()
                .get_mut::<ShipSystemBlackboards>(entity)
                .unwrap()
                .0
                .remove(&owner);
        } else if stage == 10 {
            app.world_mut()
                .get_mut::<ShipConfigComponent>(entity)
                .unwrap()
                .0
                .systems
                .retain(|row| row.kind != crate::ship::system_registry::HELM_RADAR_KIND);
        }
        let original = app
            .world()
            .get::<ShipSystemBlackboards>(entity)
            .unwrap()
            .0
            .clone();
        let physics = *app.world().get::<ShipPhysics>(entity).unwrap();
        app.world_mut().clear_trackers();
        app.world_mut().run_system_once(cloning_publish).unwrap();
        let expected = app
            .world()
            .get::<ShipSystemBlackboards>(entity)
            .unwrap()
            .0
            .clone();
        let expected_changed = app
            .world()
            .entity(entity)
            .get_ref::<ShipSystemBlackboards>()
            .unwrap()
            .is_changed();
        app.world_mut()
            .get_mut::<ShipSystemBlackboards>(entity)
            .unwrap()
            .0 = original;
        app.world_mut().clear_trackers();
        app.world_mut().run_system_once(publish).unwrap();
        let actual = app.world().get::<ShipSystemBlackboards>(entity).unwrap();
        assert_eq!(actual.0, expected, "complete producer stage {stage}");
        assert_eq!(
            serde_json::to_value(&actual.0).unwrap(),
            serde_json::to_value(&expected).unwrap()
        );
        assert_eq!(
            app.world()
                .entity(entity)
                .get_ref::<ShipSystemBlackboards>()
                .unwrap()
                .is_changed(),
            expected_changed,
            "Changed stage {stage}"
        );
        assert_eq!(*app.world().get::<ShipPhysics>(entity).unwrap(), physics);
        if stage == 10 {
            assert!(!expected_changed);
        }
        if let Some(SystemBlackboard::Helm(board)) = actual.0.get(&owner) {
            assert_eq!(
                serde_json::to_vec(board).unwrap(),
                serde_json::to_vec(match &expected[&owner] {
                    SystemBlackboard::Helm(board) => board,
                    _ => unreachable!(),
                })
                .unwrap()
            );
            if matches!(stage, 0 | 4 | 7) {
                assert!(board.approaches.is_empty());
            }
            if matches!(stage, 1 | 2) {
                assert_eq!(
                    board.approaches.len(),
                    3,
                    "identical feed copies collapse; unsupported rows confer no estimate"
                );
                assert!(board
                    .approaches
                    .iter()
                    .any(|row| row.contact == "a-predicted" && row.predicted));
                assert!(board
                    .approaches
                    .iter()
                    .any(|row| row.contact == "old-original-clock" && row.observed_tick == 0));
                assert!(board
                    .approaches
                    .iter()
                    .all(|row| row.seconds == if stage == 2 { 5.0 } else { 10.0 }));
            }
            if stage == 3 {
                assert!(!board
                    .approaches
                    .iter()
                    .any(|row| row.contact == "z-current"));
            }
        }
    }
}
