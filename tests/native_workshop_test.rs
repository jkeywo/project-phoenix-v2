//! Real native host composition under the disposable Test clock. Kept in its
//! own executable: template caches/task pools must not leak into lib tests.
#![cfg(all(feature = "server", not(target_arch = "wasm32")))]

use bevy::prelude::*;
use project_phoenix::{
    boot::NativeRenderSurface,
    core::messages::GamePhase,
    native_host::workshop::test_clock::{TestClockPlugin, TestControl, TestControls},
    native_host::{build_native_host_app, preload_content_templates, NativeHostConfig},
    sim_digest::world_digest,
    sim_tick::SimTick,
};

fn request(app: &mut App, control: TestControl) {
    app.world_mut()
        .resource_mut::<TestControls>()
        .0
        .push_back(control);
    app.update();
}

#[test]
fn fresh_test_restarts_reproduce_fixed_ticks_on_backfill_without_live_layout_or_save_authority() {
    let preload = preload_content_templates(".").expect("shipped content preloads");
    let mut expected = None;
    for _ in 0..2 {
        let mut config = NativeHostConfig::new("assets/worlds/combat_test.toml");
        config.seed = Some(41);
        config.solo = true;
        config.deterministic = true;
        config.remember_layout = false;
        config.surface = NativeRenderSurface::Contract;
        let mut app =
            build_native_host_app(&config, &preload).expect("the ordinary native host builds");
        assert!(!app.world().contains_resource::<project_phoenix::native_host::layout_store_systems::BridgeLayoutStore>());
        assert!(!app
            .world()
            .contains_resource::<project_phoenix::save_slots_lifecycle::SaveCaptureConsumer>());
        assert!(!app
            .world()
            .contains_resource::<project_phoenix::native_host::transport::NativeTransportLink>());
        app.add_plugins(TestClockPlugin);
        app.finish();
        app.cleanup();
        request(&mut app, TestControl::Pause {});
        let initial = app.world().resource::<SimTick>().0;
        for offset in 1..=32 {
            request(&mut app, TestControl::Step {});
            assert_eq!(app.world().resource::<SimTick>().0, initial + offset);
            let digest = world_digest(app.world());
            app.update();
            assert_eq!(app.world().resource::<SimTick>().0, initial + offset);
            assert_eq!(
                world_digest(app.world()),
                digest,
                "a held render frame must not mutate canonical state"
            );
        }
        assert_eq!(
            app.world().resource::<State<GamePhase>>().get(),
            &GamePhase::InProgress
        );
        assert!(app
            .world()
            .resource::<project_phoenix::lobby::Sessions>()
            .0
            .players()
            .is_empty());
        let mut ships = app.world_mut().query_filtered::<&project_phoenix::ship_plugin::ActiveStationRatings, With<project_phoenix::server_app::LocalShip>>();
        let ratings: Vec<_> = ships.iter(app.world()).collect();
        assert_eq!(ratings.len(), 1);
        assert!(!ratings[0].0.is_empty());
        assert!(ratings[0]
            .0
            .values()
            .all(|rating| rating == project_phoenix::ship::rating::BACKFILL_RATING));
        let digest = world_digest(app.world());
        if let Some(expected) = expected {
            assert_eq!(digest, expected);
        }
        expected = Some(digest);
    }
}

/// The SDK composition test proves the embedded Authoring page. This separate
/// display smoke exercises the actual host executable, staged project and
/// inherited-pipe clock path through its ordinary native renderer.
#[cfg(feature = "host")]
#[test]
#[ignore = "opens the intended interactive Test Viewscreen; requires a native display"]
#[allow(clippy::disallowed_methods)] // Private test directory, never simulation identity.
fn actual_test_host_starts_unsaved_source_steps_and_retires_its_stage() {
    use project_phoenix::{
        native_host::workshop::test_process::TestProcess,
        workshop::{
            provider::{
                assets::Source, NativeWorkshopProvider, Operation, Response, WorkshopRequest,
                WorkspaceKind,
            },
            test_protocol::TestSelection,
            WorkshopDependencies,
        },
    };
    use std::{
        path::PathBuf,
        time::{Duration, Instant},
    };
    struct Private(PathBuf);
    impl Drop for Private {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let private = Private(
        std::env::temp_dir().join(format!("phoenix-workshop-render-{}", uuid::Uuid::new_v4())),
    );
    let mut provider = NativeWorkshopProvider::open(
        WorkspaceKind::Project,
        env!("CARGO_MANIFEST_DIR"),
        &private.0,
        WorkshopDependencies::default(),
    )
    .unwrap();
    let Response::Sources { mut files, .. } = provider
        .handle(WorkshopRequest {
            id: 1,
            operation: Operation::LoadSources,
        })
        .result
    else {
        panic!("native source provider did not load")
    };
    let world = "assets/worlds/combat_test.toml";
    let Source::Text(original) = &files[world] else {
        panic!("world must remain exact text")
    };
    let authored = format!("# Unsaved disposable Test smoke\n{original}");
    files.insert(world.into(), Source::Text(authored.clone()));
    let snapshot = provider
        .prepare_test(
            files,
            TestSelection {
                world: world.into(),
                ship: "assets/entities/alliance_cruiser.toml".into(),
                seed: 41,
            },
            None,
        )
        .unwrap();
    assert_eq!(snapshot.files[world], authored.as_bytes());
    assert!(snapshot
        .files
        .contains_key("assets/shaders/reference_grid.wgsl"));
    let stages = private.0.join("runs");
    let mut process = TestProcess::start(
        std::path::Path::new(env!("CARGO_BIN_EXE_phoenix-host")),
        &stages,
        snapshot,
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let state = process.status();
        assert!(state.running, "native Test failed: {:?}", state.error);
        if !state.starting {
            break;
        }
        assert!(Instant::now() < deadline, "native Test did not finish boot");
        std::thread::sleep(Duration::from_millis(50));
    }
    let paused = process.control(TestControl::Pause {}).unwrap();
    assert!(paused.paused);
    let stepped = process.control(TestControl::Step {}).unwrap();
    assert!(stepped.paused);
    assert_eq!(stepped.tick, paused.tick + 1);
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(process.status().tick, stepped.tick);
    assert_eq!(
        process
            .control(TestControl::Rate { multiplier: 4 })
            .unwrap()
            .multiplier,
        4
    );
    assert!(process.control(TestControl::Resume {}).unwrap().running);
    drop(process);
    assert_eq!(std::fs::read_dir(&stages).unwrap().count(), 0);
    assert!(
        !std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(world))
            .unwrap()
            .starts_with("# Unsaved disposable Test smoke")
    );
}
