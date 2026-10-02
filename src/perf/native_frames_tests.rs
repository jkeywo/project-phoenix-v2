use super::*;

#[test]
fn workload_observation_requires_preload_and_records_actual_fullscreen_pixels() {
    use bevy::ecs::system::RunSystemOnce;
    use bevy::window::{Monitor, MonitorSelection, WindowMode, WindowResolution};
    let samples = Arc::new(Mutex::new(Samples::default()));
    let mut app = App::new();
    app.add_message::<AppExit>()
        .insert_resource(Observer {
            samples: samples.clone(),
            start: Instant::now(),
            duration: Duration::from_secs(60),
        })
        .init_resource::<FixedTicks>();
    let monitor = app
        .world_mut()
        .spawn(Monitor {
            name: Some("panel".into()),
            physical_width: 1920,
            physical_height: 1200,
            physical_position: IVec2::ZERO,
            refresh_rate_millihertz: Some(60_000),
            scale_factor: 1.25,
            video_modes: Vec::new(),
        })
        .id();
    app.world_mut().spawn(Window {
        mode: WindowMode::BorderlessFullscreen(MonitorSelection::Entity(monitor)),
        resolution: WindowResolution::new(1920, 1200).with_scale_factor_override(1.25),
        ..default()
    });
    app.world_mut().run_system_once(record_frame).unwrap();
    assert!(!samples.lock().unwrap().workload[0].assets_ready);
    let mut preload = crate::server::asset_preload::AssetPreloadResource::default();
    preload.started = true;
    preload.complete = true;
    preload.total_count = 10;
    app.insert_resource(preload);
    samples.lock().unwrap().last_workload_second = None;
    app.world_mut().run_system_once(record_frame).unwrap();
    let observed = samples.lock().unwrap();
    let ready = &observed.workload[1];
    assert!(ready.assets_ready);
    assert_eq!(ready.asset_count, 10);
    assert_eq!(ready.windows[0].monitor.as_deref(), Some("panel@1920x1200"));
    assert_eq!(
        (ready.windows[0].width, ready.windows[0].height),
        (1920, 1200)
    );
    assert_eq!(ready.windows[0].scale, 1.25);
}

#[test]
fn first_schedule_establishes_origin_and_catchup_belongs_to_completed_interval() {
    let mut samples = Samples::default();
    samples.record(Duration::from_millis(100), 0);
    samples.record(Duration::from_millis(120), 3);
    samples.record(Duration::from_millis(130), 1);
    let capture = samples
        .recorder
        .finish("test", super::super::profile("test"));
    assert_eq!(capture.series["native.frame"].samples, vec![20.0, 10.0]);
    assert_eq!(capture.series["native.elapsed"].samples, vec![0.12, 0.13]);
    assert_eq!(capture.series["native.fixed_ticks"].samples, vec![3.0, 1.0]);
}

#[test]
fn artifact_completion_requires_the_whole_interval_and_a_successful_runner_exit() {
    #[derive(serde::Deserialize)]
    struct Probe {
        complete: bool,
        capture: Capture,
    }
    for (elapsed, exit, complete) in [
        (500, AppExit::Success, false),
        (1000, AppExit::error(), false),
        (1000, AppExit::Success, true),
    ] {
        let path = std::env::temp_dir().join(format!(
            "phoenix-frame-capture-{}.json",
            uuid::Uuid::new_v4()
        ));
        let mut samples = Samples::default();
        samples.record(Duration::ZERO, 0);
        samples.record(Duration::from_millis(elapsed), 2);
        let capture = NativeFrameCapture {
            samples: Arc::new(Mutex::new(samples)),
            path: path.clone(),
            started_unix_ms: 0,
            origin: Instant::now(),
            duration: Duration::from_secs(1),
        };
        capture.finish(&exit).unwrap();
        let artifact: Probe =
            crate::core::codec::decode_presentation_capture(&std::fs::read(&path).unwrap())
                .unwrap();
        std::fs::remove_file(path).unwrap();
        assert_eq!(artifact.complete, complete);
        assert_eq!(
            artifact.capture.series["native.fixed_ticks"].samples,
            vec![2.0]
        );
    }
}
