//! Bevy's supported render-pass diagnostic path. Values are delivered samples:
//! Bevy retains the latest completed render batch, so this is not an assertion
//! that every submitted GPU frame has been observed.
use bevy::{diagnostic::DiagnosticsStore, prelude::*};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    sync::{atomic::Ordering, Arc, Mutex},
};

#[derive(Default, Serialize)]
pub(crate) struct Capture {
    pub adapter: Option<String>,
    pub device_features: Option<String>,
    pub truncated: bool,
    pub history_may_be_truncated: bool,
    pub samples: Vec<Sample>,
}

#[derive(Serialize)]
pub(crate) struct Sample {
    pub path: String,
    pub received_ns: u128,
    pub update: u64,
    pub value: f64,
}

pub(crate) struct GpuPlugin {
    pub control: Arc<super::timing::Control>,
    pub capture: Arc<Mutex<Capture>>,
}

impl Plugin for GpuPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(bevy::render::diagnostic::RenderDiagnosticsPlugin);
        let control = self.control.clone();
        let capture = self.capture.clone();
        let mut seen = BTreeMap::new();
        app.add_systems(PostUpdate, move |store: Res<DiagnosticsStore>| {
            let now = std::time::Instant::now();
            let received_ns = now.duration_since(control.epoch).as_nanos();
            let mut capture = capture.lock().unwrap();
            for diagnostic in store.iter() {
                let path = diagnostic.path().to_string();
                if !path.starts_with("render/") {
                    continue;
                }
                let Some(latest) = diagnostic.measurement() else {
                    continue;
                };
                let previous = seen.get(&path).copied();
                if previous == Some(latest.time) {
                    continue;
                }
                // A completed render batch can contain multiple measurements
                // of the same path, all with the same timestamp. Advance the
                // cursor only after retaining the whole path's new history.
                let new_rows: Vec<_> = diagnostic
                    .measurements()
                    .filter(|measurement| previous.is_none_or(|time| measurement.time > time))
                    .collect();
                if control.active_at(now) {
                    if new_rows.len() >= diagnostic.get_max_history_length() {
                        capture.history_may_be_truncated = true;
                    }
                    for measurement in new_rows {
                        if capture.samples.len() < 500_000 {
                            capture.samples.push(Sample {
                                path: path.clone(),
                                received_ns,
                                update: control.update.load(Ordering::Relaxed),
                                value: measurement.value,
                            });
                        } else {
                            capture.truncated = true;
                        }
                    }
                }
                seen.insert(path, latest.time);
            }
        });
    }

    fn finish(&self, app: &mut App) {
        let Some(render) = app.get_sub_app(bevy::render::RenderApp) else {
            return;
        };
        let mut capture = self.capture.lock().unwrap();
        if let Some(adapter) = render
            .world()
            .get_resource::<bevy::render::renderer::RenderAdapterInfo>()
        {
            capture.adapter = Some(format!(
                "{}; {:?}; {:?}; {}; {}",
                adapter.name,
                adapter.backend,
                adapter.device_type,
                adapter.driver,
                adapter.driver_info
            ));
        }
        if let Some(device) = render
            .world()
            .get_resource::<bevy::render::renderer::RenderDevice>()
        {
            capture.device_features = Some(format!("{:?}", device.features()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::diagnostic::{Diagnostic, DiagnosticMeasurement, DiagnosticPath};

    #[test]
    fn repeated_passes_in_one_batch_are_all_retained_once() {
        let control = Arc::new(super::super::timing::Control::new(100));
        control.active.store(true, Ordering::Relaxed);
        let capture = Arc::new(Mutex::new(Capture::default()));
        let mut app = App::new();
        app.init_resource::<DiagnosticsStore>()
            .add_plugins(GpuPlugin {
                control,
                capture: capture.clone(),
            });
        app.finish();
        app.cleanup();
        let path = DiagnosticPath::new("render/main/elapsed_gpu");
        let mut diagnostic = Diagnostic::new(path);
        let time = std::time::Instant::now();
        for value in [1.0, 2.0, 3.0] {
            diagnostic.add_measurement(DiagnosticMeasurement { time, value });
        }
        app.world_mut()
            .resource_mut::<DiagnosticsStore>()
            .add(diagnostic);
        app.update();
        app.update();
        let captured = capture.lock().unwrap();
        assert_eq!(
            captured
                .samples
                .iter()
                .map(|sample| sample.value)
                .collect::<Vec<_>>(),
            vec![1.0, 2.0, 3.0]
        );
        assert!(!captured.history_may_be_truncated);
    }
}
