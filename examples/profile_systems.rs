//! On-rig named-system attribution. This example uses production App builders;
//! its control and wrapped runs are separate from ordinary binary baselines.
//! Usage: profile_systems <headless|native> <control|wrapped> <world> <new-dir>
//! Native also requires <explicit-zero-profile>. Assets resolve from the CWD.

#[cfg(not(target_arch = "wasm32"))]
#[path = "profile_systems/timing.rs"]
mod timing;

#[cfg(not(target_arch = "wasm32"))]
#[path = "profile_systems/gpu.rs"]
mod gpu;

#[cfg(target_arch = "wasm32")]
fn main() {
    panic!("profile_systems is a native-only diagnostic");
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    use bevy::prelude::*;
    use project_phoenix::{headless, native_host};
    use std::{
        sync::{atomic::Ordering, Arc, Mutex},
        time::{Duration, Instant},
    };

    #[derive(serde::Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Update {
        update: u64,
        tick: u64,
        duration_ms: f64,
    }
    #[derive(serde::Serialize)]
    struct Continuation {
        tick: u64,
        digest: String,
    }
    #[derive(serde::Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Artifact<'a> {
        runtime: &'a str,
        mode: &'a str,
        world: &'a str,
        trace_truncated: bool,
        systems: Vec<&'a timing::Row>,
        updates: Vec<Update>,
        continuation: Option<Continuation>,
        window_seconds: Option<[f64; 2]>,
        native_update_tag: &'static str,
        render_diagnostics: &'a gpu::Capture,
        gpu_interpretation: &'static str,
        interpretation: &'static str,
    }

    let args: Vec<_> = std::env::args().skip(1).collect();
    assert!(
        args.len() >= 4,
        "profile_systems <headless|native> <control|wrapped> <world> <new-dir> [zero-profile]"
    );
    let native = match args[0].as_str() {
        "native" => true,
        "headless" => false,
        _ => panic!("unknown runtime"),
    };
    let enabled = match args[1].as_str() {
        "wrapped" => true,
        "control" => false,
        _ => panic!("unknown mode"),
    };
    let output = std::path::PathBuf::from(&args[3]);
    std::fs::create_dir(&output).expect("fresh output directory");
    // At most 20k entries per system, with explicit truncation. Headless runs
    // retain every normal 3601-update span; native traces cap memory even if
    // an uncapped renderer produces unusually many iterations.
    let mut control = Arc::new(timing::Control::new(20_000));
    let rows = Arc::new(Mutex::new(Vec::<Arc<timing::Row>>::new()));
    let gpu = Arc::new(Mutex::new(gpu::Capture::default()));
    let mut updates = Vec::new();
    let mut continuation = None;
    if native {
        struct TimerPlugin {
            control: Arc<timing::Control>,
            rows: Arc<Mutex<Vec<Arc<timing::Row>>>>,
        }
        impl Plugin for TimerPlugin {
            fn build(&self, _: &mut App) {}
            fn finish(&self, app: &mut App) {
                let mut rows = timing::instrument_world(app.world_mut(), self.control.clone());
                if let Some(render) = app.get_sub_app_mut(bevy::render::RenderApp) {
                    rows.extend(timing::instrument_world(
                        render.world_mut(),
                        self.control.clone(),
                    ));
                }
                self.rows.lock().unwrap().extend(rows);
            }
        }
        native_host::pin_content_root(".").unwrap();
        let preload = native_host::preload_content_templates(".").unwrap();
        let mut config = native_host::NativeHostConfig::new(&args[2]);
        config.ship_path = Some("assets/entities/alliance_destroyer.toml".into());
        config.seed = Some(42);
        config.solo = true;
        config.surface = project_phoenix::boot::NativeRenderSurface::Window;
        config.log_spec = "warn".into();
        let profile =
            std::fs::read_to_string(args.get(4).expect("explicit zero-pane profile required"))
                .unwrap();
        config.bridge_profile = Some(
            native_host::bridge_profile::BridgeProfile::from_toml(&profile)
                .unwrap()
                .validate()
                .unwrap(),
        );
        let mut app = native_host::build_native_host_app(&config, &preload).unwrap();
        // Install the same cadence collector as the normal native runner,
        // retaining its origin for all other observers before cloning Control.
        std::env::set_var("PHOENIX_FRAME_CAPTURE", output.join("frames.json"));
        std::env::set_var("PHOENIX_FRAME_CAPTURE_SECONDS", "70");
        let capture =
            project_phoenix::perf::native_frames::NativeFrameCapture::install_from_environment(
                &mut app,
            )
            .unwrap()
            .unwrap();
        let setup = Arc::get_mut(&mut control).unwrap();
        setup.epoch = capture.clock_origin();
        setup.window = Some((Duration::from_secs(40), Duration::from_secs(70)));
        if enabled {
            app.add_plugins(gpu::GpuPlugin {
                control: control.clone(),
                capture: gpu.clone(),
            });
            app.add_plugins(TimerPlugin {
                control: control.clone(),
                rows: rows.clone(),
            });
        }
        let observation = control.clone();
        app.add_systems(First, move || {
            observation.update.fetch_add(1, Ordering::Relaxed);
        });
        let exit = app.run();
        capture.finish(&exit).unwrap();
    } else {
        let headless::ParseOutcome::Run(config) = headless::parse_args(
            [
                "--world",
                args[2].as_str(),
                "--ship",
                "assets/entities/alliance_destroyer.toml",
                "--seed",
                "42",
                "--sim-seconds",
                "60",
                "--console-latency",
                "--log",
                "off",
            ]
            .into_iter()
            .map(str::to_owned),
        )
        .unwrap() else {
            unreachable!()
        };
        let mut app = headless::build_headless_app(&config).unwrap();
        app.finish();
        app.cleanup();
        if enabled {
            rows.lock()
                .unwrap()
                .extend(timing::instrument_world(app.world_mut(), control.clone()));
        }
        for update in 0..config.max_ticks {
            control.update.store(update, Ordering::Relaxed);
            control.active.store(update >= 300, Ordering::Relaxed);
            let start = Instant::now();
            app.update();
            let duration_ms = start.elapsed().as_secs_f64() * 1000.0;
            if update >= 300 {
                updates.push(Update {
                    update,
                    tick: app
                        .world()
                        .resource::<project_phoenix::sim_tick::SimTick>()
                        .0,
                    duration_ms,
                });
            }
            if app
                .world()
                .resource::<State<project_phoenix::core::messages::GamePhase>>()
                .get()
                == &project_phoenix::core::messages::GamePhase::GameOver
            {
                break;
            }
        }
        continuation = Some(Continuation {
            tick: app
                .world()
                .resource::<project_phoenix::sim_tick::SimTick>()
                .0,
            digest: format!(
                "{:016x}",
                project_phoenix::sim_digest::world_digest(app.world())
            ),
        });
        std::fs::write(
            output.join("report.json"),
            headless::build_report(&mut app, &config, 0.0).to_json(),
        )
        .unwrap();
    }
    control.active.store(false, Ordering::Relaxed);
    let rows = rows.lock().unwrap();
    let rows: Vec<_> = rows.iter().map(|row| row.as_ref()).collect();
    let gpu_capture = gpu.lock().unwrap();
    let artifact = Artifact { runtime: &args[0], mode: &args[1], world: &args[2],
        trace_truncated: control.truncated.load(Ordering::Relaxed), systems: rows, updates, continuation,
        window_seconds: control.window.map(|(start, end)| [start.as_secs_f64(), end.as_secs_f64()]),
        native_update_tag: "Main counter observed at span start, not a causal render-frame ID; unordered First systems can see the previous value. Native windows use the exact frames.json monotonic origin.",
        render_diagnostics: &gpu_capture,
        gpu_interpretation: "elapsed_gpu paths contain supported GPU timestamp milliseconds. Their absence means no GPU result. Other paths are CPU milliseconds or pipeline counts. Nested render-pass paths overlap; samples are delivered asynchronously, not aligned to every main frame.",
        interpretation: "Overlapping system wall spans, not exclusive CPU or GPU durations. ExtractSchedule deferred work is nested inside ExtractCommands and must be counted once." };
    let file = std::fs::File::create(output.join("systems.json")).unwrap();
    let mut writer = std::io::BufWriter::new(file);
    let json = project_phoenix::core::codec::encode_presentation_capture(&artifact).unwrap();
    std::io::Write::write_all(&mut writer, json.as_bytes()).unwrap();
    std::io::Write::flush(&mut writer).unwrap();
}
