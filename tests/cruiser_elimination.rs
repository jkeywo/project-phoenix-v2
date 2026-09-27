//! The competitive reference world runs through the ordinary host schedule.
#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]

use bevy::prelude::*;
use phoenix::core::messages::{GamePhase, StationId};
use phoenix::entities::spawner::{EntitySystemHull, EntityUuid};
use phoenix::headless::{build_headless_app, run, HeadlessArgs};
use phoenix::server_app::LocalShip;
use phoenix::ship_slots::AuthoredShipSlotId;
use phoenix::world::server::WorldContentRuntime;
use project_phoenix as phoenix;

fn boot(seed: u64, human: bool) -> App {
    let mut app = build_headless_app(&HeadlessArgs {
        world_path: "assets/worlds/cruiser_elimination.toml".into(),
        ship_path: "assets/entities/alliance_cruiser.toml".into(),
        seed: Some(seed),
        max_ticks: 100_000,
        deterministic: true,
        ..Default::default()
    })
    .unwrap();
    if human {
        let mut sessions = app.world_mut().resource_mut::<phoenix::lobby::Sessions>();
        sessions
            .0
            .register("human-helm".into(), "Helm".into())
            .unwrap();
        sessions
            .0
            .set_station("human-helm", Some(StationId("helm".into())));
    }
    run(&mut app, 3);
    let mut ships = app
        .world_mut()
        .query::<(&AuthoredShipSlotId, &EntitySystemHull)>();
    let mut slots: Vec<_> = ships
        .iter(app.world())
        .map(|(slot, hull)| {
            assert!(!hull.0.is_destroyed());
            slot.0.clone()
        })
        .collect();
    slots.sort();
    assert_eq!(
        slots,
        ["alliance_one", "alliance_two", "dynasty_one", "dynasty_two"]
    );
    let mut ships = app.world_mut().query::<(
        &AuthoredShipSlotId,
        &phoenix::entities::spawner::EntityTemplatePath,
        &phoenix::ship::state::ShipPhysics,
    )>();
    for (slot, template, physics) in ships.iter(app.world()) {
        let dynasty = slot.0.starts_with("dynasty");
        assert_eq!(
            template.0,
            if dynasty {
                "assets/entities/dynasty_player_cruiser.toml"
            } else {
                "assets/entities/alliance_cruiser.toml"
            }
        );
        assert!(
            if dynasty {
                phoenix::simmath::cos(physics.yaw) < -0.99
            } else {
                phoenix::simmath::cos(physics.yaw) > 0.99
            },
            "{} must retain its authored heading: {}",
            slot.0,
            physics.yaw
        );
    }
    if human {
        let (config, sources) = app
            .world_mut()
            .query_filtered::<(
                &phoenix::ship_plugin::ShipConfigComponent,
                &phoenix::ship_plugin::ShipSystemControlSources,
            ), With<LocalShip>>()
            .single(app.world())
            .unwrap();
        assert!(
            config
                .0
                .systems_for_station(&StationId("helm".into()))
                .any(|system| sources.0.source_for(&system.id)
                    == phoenix::ship::control_source::ControlSource::Human),
            "mixed run must contain a real Human-controlled Station"
        );
    }
    assert_eq!(
        *app.world().resource::<State<GamePhase>>().get(),
        GamePhase::InProgress
    );
    let identities: Vec<_> = app
        .world_mut()
        .query::<(&AuthoredShipSlotId, &EntityUuid)>()
        .iter(app.world())
        .map(|(slot, uuid)| (slot.0.clone(), uuid.0.clone()))
        .collect();
    let definitions = &app
        .world()
        .resource::<phoenix::world::server::ObjectiveManagerRes>()
        .0;
    let instances = &app
        .world()
        .resource::<phoenix::world::server::ObjectiveInstanceManagerRes>()
        .0;
    for (slot, uuid) in identities {
        let objectives =
            instances.project_snapshots_for_ship(&uuid, definitions.sorted_snapshots());
        assert_eq!(
            objectives.len(),
            1,
            "{slot} sees exactly its own team's goal"
        );
        assert_eq!(
            objectives[0].text,
            if slot.starts_with("dynasty") {
                "world.cruiser_elimination.dynasty_objective"
            } else {
                "world.cruiser_elimination.alliance_objective"
            }
        );
    }
    app
}

fn destroy(app: &mut App, slots: &[&str]) {
    let mut ships = app
        .world_mut()
        .query::<(&AuthoredShipSlotId, &EntityUuid, &mut EntitySystemHull)>();
    let mut events = Vec::new();
    for (slot, uuid, mut hull) in ships.iter_mut(app.world_mut()) {
        if slots.contains(&slot.0.as_str()) {
            let ids: Vec<_> = hull.0.iter().map(|(id, _)| id.clone()).collect();
            for id in ids {
                hull.0.set_hp(&id, 0.0);
            }
            events.push(phoenix::ai::server::AiEntityDestroyed {
                entity_uuid: uuid.0.clone(),
            });
        }
    }
    assert_eq!(events.len(), slots.len());
    app.world_mut()
        .resource_mut::<Messages<phoenix::ai::server::AiEntityDestroyed>>()
        .write_batch(events);
    run(app, 5);
}

fn result(app: &App, expected: &str) {
    assert_eq!(
        *app.world().resource::<State<GamePhase>>().get(),
        GamePhase::GameOver
    );
    let runtime = app.world().resource::<WorldContentRuntime>();
    assert_eq!(runtime.flags.counter("match_resolved"), 1);
    assert_eq!(runtime.flags.counter(expected), 1);
    let report = app
        .world()
        .resource::<phoenix::core::report::MissionReport>();
    assert_eq!(report.rows().len(), 2);
    for row in report.rows() {
        let verdict = if expected == "match_draw" {
            "draw"
        } else if expected == format!("{}_victory", row.id) {
            "victory"
        } else {
            "defeat"
        };
        assert_eq!(
            row.outcome_id,
            format!("world.cruiser_elimination.{verdict}")
        );
    }
}

#[test]
fn competitive_world_results_and_destroyed_crew_keep_their_identity() {
    let mut app = boot(1549, true);
    let original = app
        .world_mut()
        .query_filtered::<Entity, With<LocalShip>>()
        .single(app.world())
        .unwrap();
    destroy(&mut app, &["alliance_one"]);
    assert_eq!(
        *app.world().resource::<State<GamePhase>>().get(),
        GamePhase::InProgress
    );
    assert!(
        app.world()
            .resource::<phoenix::crew_spectator::CrewSpectator>()
            .active
    );
    assert!(app.world().get::<LocalShip>(original).is_some());
    destroy(&mut app, &["alliance_two"]);
    result(&app, "dynasty_victory");

    let mut app = boot(1550, false);
    destroy(&mut app, &["dynasty_one", "dynasty_two"]);
    result(&app, "alliance_victory");

    let mut app = boot(1551, false);
    destroy(
        &mut app,
        &["alliance_one", "dynasty_one", "alliance_two", "dynasty_two"],
    );
    result(&app, "match_draw");
    let runtime = app.world().resource::<WorldContentRuntime>();
    assert_eq!(runtime.flags.counter("alliance_victory"), 0);
    assert_eq!(runtime.flags.counter("dynasty_victory"), 0);
}

#[test]
fn competitive_world_finishes_without_a_gm_with_full_and_mixed_backfill() {
    for human in [false, true] {
        let mut app = boot(1549, human);
        run(&mut app, 18_000);
        let runtime = app.world().resource::<WorldContentRuntime>();
        let outcome = ["alliance_victory", "dynasty_victory", "match_draw"]
            .into_iter()
            .find(|name| runtime.flags.counter(name) == 1);
        if outcome.is_none() {
            let path = std::env::temp_dir().join(format!("cruiser-elimination-human-{human}.json"));
            std::fs::write(
                &path,
                serde_json::to_vec_pretty(&phoenix::snapshot::capture(app.world())).unwrap(),
            )
            .unwrap();
            panic!(
                "ordinary Backfill combat did not resolve; diagnostic snapshot {}",
                path.display()
            );
        }
        let outcome = outcome.unwrap();
        result(&app, outcome);
    }
}
