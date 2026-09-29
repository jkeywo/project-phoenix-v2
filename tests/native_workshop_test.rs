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

#[test]
fn disposable_native_test_controls_one_authored_slot_and_runs_other_present_slots() {
    use project_phoenix::lockstep::FleetSlotOf;
    use project_phoenix::ship_slots::{AuthoredShipSlotId, FrozenShipSlots, LaunchSource};
    const WORLD: &str = "tests/fixtures/worlds/multi_ship_slots_direct_launch.toml";
    const HULL: &str = "assets/entities/alliance_cruiser.toml";
    let preload = preload_content_templates(".").expect("shipped content preloads");
    let mut config = NativeHostConfig::new(WORLD);
    config.ship_path = Some(HULL.into());
    config.solo = true;
    config.deterministic = true;
    config.remember_layout = false;
    config.surface = NativeRenderSurface::Contract;
    let mut app = build_native_host_app(&config, &preload).expect("ordinary native host builds");
    let frozen = FrozenShipSlots::for_workshop_test(
        &app.world()
            .resource::<project_phoenix::world::config::WorldConfig>()
            .ship_slots,
        "wing",
        HULL,
    )
    .expect("controlled slot is offered");
    assert_eq!(frozen.0.len(), 2);
    assert_eq!(frozen.0[0].source, LaunchSource::Backfill);
    assert_eq!(frozen.0[1].source, LaunchSource::Claimed);
    app.insert_resource(frozen);
    app.add_plugins(TestClockPlugin);
    app.finish();
    app.cleanup();
    request(&mut app, TestControl::Pause {});
    for _ in 0..4 {
        request(&mut app, TestControl::Step {});
    }
    assert_eq!(
        app.world().resource::<State<GamePhase>>().get(),
        &GamePhase::InProgress
    );
    let mut ships = app.world_mut().query::<(
        &AuthoredShipSlotId,
        &FleetSlotOf,
        Has<project_phoenix::server_app::LocalShip>,
    )>();
    let mut roster: Vec<_> = ships
        .iter(app.world())
        .map(|(slot, _, local)| (slot.0.clone(), local))
        .collect();
    roster.sort();
    assert_eq!(roster, [("lead".into(), false), ("wing".into(), true)]);
}

/// The SDK composition test proves the embedded Authoring page. This separate
/// display smoke exercises the actual host executable, staged project and
/// inherited-pipe clock path through its ordinary native renderer.
#[cfg(feature = "host")]
#[test]
#[ignore = "requires a native GPU adapter for the real offscreen child and pipe transport"]
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
    let authored = format!(
        r#"# Unsaved disposable Test smoke
{original}
[[gm_role_preset]]
id = "test-native-role"
label = "workshop.test_heading"
panels = ["gm-map-panel", "gm-activity"]
"#
    );
    files.insert(world.into(), Source::Text(authored.clone()));
    let snapshot = provider
        .prepare_test(
            files,
            TestSelection {
                world: world.into(),
                slot: None,
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
    let documents = project_phoenix::delivery::serve::HostedDocuments::default();
    let mut process = TestProcess::start_with_delivery(
        std::path::Path::new(env!("CARGO_BIN_EXE_phoenix-host")),
        &stages,
        snapshot,
        Some((documents.clone(), "http://127.0.0.1:7".into())),
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
    let frame_path = process
        .status()
        .frame_url
        .unwrap()
        .strip_prefix("http://127.0.0.1:7")
        .unwrap()
        .to_owned();
    let presentation_path = process
        .status()
        .presentation_url
        .unwrap()
        .strip_prefix("http://127.0.0.1:7")
        .unwrap()
        .to_owned();
    loop {
        let state = process.status();
        assert!(state.running, "native Test failed: {:?}", state.error);
        let drawn = documents.resource(&frame_path).is_some_and(|frame| {
            let pixels = image::load_from_memory(&frame.body).unwrap().into_rgba8();
            let (width, height) = pixels.dimensions();
            let mut colours = std::collections::HashSet::new();
            let mut brightest = 0;
            for y in height * 3 / 10..height * 7 / 10 {
                for x in width * 3 / 10..width * 7 / 10 {
                    let px = pixels.get_pixel(x, y).0;
                    brightest = brightest.max(*px[..3].iter().max().unwrap());
                    if colours.len() < 8 {
                        colours.insert(px);
                    }
                }
            }
            colours.len() > 1 && brightest > 16
        });
        if drawn {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "native dock transport never received a lit non-flat scene"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    process
        .control(TestControl::View {
            view: project_phoenix::workshop::test_protocol::TestView::GameMaster,
        })
        .unwrap();
    loop {
        if documents
            .resource(&presentation_path)
            .is_some_and(|resource| {
                let payload =
                    project_phoenix::core::codec::decode_workshop_test_presentation(&resource.body)
                        .unwrap();
                payload.channels.contains_key("gm_entity")
                    && payload.channels.contains_key("hud")
                    && payload.role_presets.contains("test-native-role")
            })
        {
            break;
        }
        assert!(process.status().running);
        assert!(
            Instant::now() < deadline,
            "native dock did not receive GM/HUD/captured role projections"
        );
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
    assert!(documents.resource(&frame_path).is_none());
    assert!(documents.resource(&presentation_path).is_none());
    assert_eq!(std::fs::read_dir(&stages).unwrap().count(), 0);
    assert!(
        !std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(world))
            .unwrap()
            .starts_with("# Unsaved disposable Test smoke")
    );
}
