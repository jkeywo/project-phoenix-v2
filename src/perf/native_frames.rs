//! Opt-in native presentation cadence capture (#1409).
//!
//! This observer never feeds authoritative state. It buffers vellum samples
//! until the native runner returns; there is no per-frame file I/O. Intervals
//! are between consecutive `First` schedules, including presentation waits,
//! and are not GPU durations or exclusive main-thread work.

use std::{
    io::Write,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use bevy::prelude::*;
use serde::Serialize;
use vellum_perf::{Capture, Recorder, Unit};

use crate::authoritative::{DeclareState, StateClass};

#[derive(Default)]
struct Samples {
    recorder: Recorder,
    previous: Option<Duration>,
    elapsed: Duration,
    workload: Vec<WorkloadObservation>,
    last_workload_second: Option<u64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WorkloadObservation {
    elapsed_seconds: f64,
    assets_ready: bool,
    asset_count: usize,
    failed_glbs: usize,
    windows: Vec<WindowObservation>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WindowObservation {
    title: String,
    monitor: Option<String>,
    width: u32,
    height: u32,
    scale: f64,
}

#[derive(Resource)]
struct Observer {
    samples: Arc<Mutex<Samples>>,
    start: Instant,
    duration: Duration,
}

#[derive(Resource, Default)]
struct FixedTicks(u64);

/// Held outside the App because native winit consumes its World on exit.
pub struct NativeFrameCapture {
    samples: Arc<Mutex<Samples>>,
    path: PathBuf,
    started_unix_ms: u128,
    origin: Instant,
    duration: Duration,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Artifact {
    complete: bool,
    started_unix_ms: u128,
    elapsed_seconds: f64,
    capture: Capture,
    workload: Vec<WorkloadObservation>,
}

impl NativeFrameCapture {
    /// Environment configuration is confined to the real native runner. Tests
    /// and offscreen/contract App construction do not start a capture.
    pub fn install_from_environment(app: &mut App) -> Result<Option<Self>, String> {
        let Some(path) = std::env::var_os("PHOENIX_FRAME_CAPTURE") else {
            return Ok(None);
        };
        let seconds = std::env::var("PHOENIX_FRAME_CAPTURE_SECONDS")
            .map_err(|_| "PHOENIX_FRAME_CAPTURE_SECONDS is required")?
            .parse::<f64>()
            .map_err(|_| "PHOENIX_FRAME_CAPTURE_SECONDS must be a number")?;
        if !seconds.is_finite() || !(1.0..=7200.0).contains(&seconds) {
            return Err("PHOENIX_FRAME_CAPTURE_SECONDS must be between 1 and 7200".into());
        }
        let path = PathBuf::from(path);
        // Never replace a previous result, including when a harness reuses a
        // run directory by accident. Reserve it before opening the window.
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|e| format!("cannot reserve capture {}: {e}", path.display()))?;
        let samples = Arc::new(Mutex::new(Samples::default()));
        let duration = Duration::from_secs_f64(seconds);
        let started_unix_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_millis();
        let origin = Instant::now();
        app.insert_resource(Observer {
            samples: samples.clone(),
            start: origin,
            duration,
        })
        .declare_state::<Observer>(StateClass::Presentation, "perf-native-frames")
        .declare_state::<FixedTicks>(StateClass::Presentation, "perf-native-frames")
        .init_resource::<FixedTicks>()
        .add_systems(First, record_frame)
        // Count completed steps outside the authoritative FixedUpdate graph.
        // An observer need not perturb that graph's topological tie-breaks.
        .add_systems(FixedLast, count_fixed_ticks);
        Ok(Some(Self {
            samples,
            path,
            started_unix_ms,
            origin,
            duration,
        }))
    }

    /// Shared monotonic origin for optional attribution collectors observing
    /// exactly the same cadence window. Never use it as simulation input.
    pub fn clock_origin(&self) -> Instant {
        self.origin
    }

    /// Wall-clock label for collectors sharing `clock_origin`.
    pub fn started_unix_ms(&self) -> u128 {
        self.started_unix_ms
    }

    /// A successful, fully observed bounded run is the only complete capture.
    /// Early operator exit remains useful evidence with `complete: false`.
    pub fn finish(self, exit: &AppExit) -> Result<(), String> {
        let mut samples = self.samples.lock().map_err(|e| e.to_string())?;
        let recorder = std::mem::take(&mut samples.recorder);
        let artifact = Artifact {
            complete: matches!(exit, AppExit::Success) && samples.elapsed >= self.duration,
            started_unix_ms: self.started_unix_ms,
            elapsed_seconds: samples.elapsed.as_secs_f64(),
            capture: recorder.finish("native-presentation", super::profile("native-window")),
            workload: std::mem::take(&mut samples.workload),
        };
        let file = std::fs::File::create(&self.path).map_err(|e| e.to_string())?;
        let mut writer = std::io::BufWriter::new(file);
        serde_json::to_writer(&mut writer, &artifact).map_err(|e| e.to_string())?;
        writer.flush().map_err(|e| e.to_string())
    }
}

impl Samples {
    fn record(&mut self, elapsed: Duration, fixed_ticks: u64) {
        if let Some(previous) = self.previous {
            self.recorder.sample(
                "native.frame",
                Unit::Millis,
                (elapsed - previous).as_secs_f64() * 1000.0,
            );
            self.recorder
                .sample("native.elapsed", Unit::Seconds, elapsed.as_secs_f64());
            self.recorder
                .sample("native.fixed_ticks", Unit::Count, fixed_ticks as f64);
        }
        self.previous = Some(elapsed);
        self.elapsed = elapsed;
    }
}

fn record_frame(
    observer: Res<Observer>,
    mut ticks: ResMut<FixedTicks>,
    mut exit: MessageWriter<AppExit>,
    preload: Option<Res<crate::server::asset_preload::AssetPreloadResource>>,
    windows: Query<&Window>,
    monitors: Query<&bevy::window::Monitor>,
) {
    let elapsed = observer.start.elapsed();
    // Poisoning is a failed observer, never a reason to change the mission.
    if let Ok(mut samples) = observer.samples.lock() {
        samples.record(elapsed, ticks.0);
        if samples.last_workload_second != Some(elapsed.as_secs()) {
            samples.last_workload_second = Some(elapsed.as_secs());
            let windows = windows
                .iter()
                .map(|window| {
                    let monitor = match window.mode {
                        bevy::window::WindowMode::BorderlessFullscreen(
                            bevy::window::MonitorSelection::Entity(entity),
                        ) => monitors.get(entity).ok().map(|monitor| {
                            format!(
                                "{}@{}x{}",
                                monitor.name.as_deref().unwrap_or("unnamed-display"),
                                monitor.physical_width,
                                monitor.physical_height
                            )
                        }),
                        _ => None,
                    };
                    WindowObservation {
                        title: window.title.clone(),
                        monitor,
                        width: window.resolution.physical_width(),
                        height: window.resolution.physical_height(),
                        scale: f64::from(window.resolution.scale_factor()),
                    }
                })
                .collect();
            samples.workload.push(WorkloadObservation {
                elapsed_seconds: elapsed.as_secs_f64(),
                assets_ready: preload
                    .as_ref()
                    .is_some_and(|p| p.started && p.complete && p.failed_glb_count() == 0),
                asset_count: preload.as_ref().map_or(0, |p| p.total_count),
                failed_glbs: preload.as_ref().map_or(0, |p| p.failed_glb_count()),
                windows,
            });
        }
    }
    ticks.0 = 0;
    if elapsed >= observer.duration {
        exit.write(AppExit::Success);
    }
}

fn count_fixed_ticks(mut ticks: ResMut<FixedTicks>) {
    ticks.0 += 1;
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)] // Random artifact names are not simulation identities.
mod tests {
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
            let artifact: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            std::fs::remove_file(path).unwrap();
            assert_eq!(artifact["complete"], complete);
            assert_eq!(
                artifact["capture"]["series"]["native.fixed_ticks"]["samples"],
                serde_json::json!([2.0])
            );
        }
    }
}
