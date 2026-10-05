//! Opt-in, bounded surface attribution. No timings enter simulation state.
//!
//! Raw events use integer nanoseconds from one monotonic origin. SDK update and
//! render are global iteration costs, never invented per-view raster timings.
//! A frame's identity travels with its owned buffer; moving or dropping it cannot
//! reattribute old pixels to the new view at the same pane id.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use bevy::app::AppExit;
use bevy::prelude::*;
use serde::Serialize;

#[cfg(test)]
use super::pane_thread::FrameRect;
#[cfg(test)]
use super::surface::PaneSurfaceError;
use crate::authoritative::{DeclareState, StateClass};

pub const CAPTURE_ENV: &str = "PHOENIX_SURFACE_CAPTURE";
pub use phoenix_platform::surface_stats::*;
#[derive(Resource, Clone, Debug, Deref, DerefMut)]
pub struct SurfaceObserver(pub phoenix_platform::surface_stats::SurfaceObserver);
impl SurfaceObserver {
    pub fn new(origin: Instant, limit: usize) -> Self {
        Self(phoenix_platform::surface_stats::SurfaceObserver::new(
            origin, limit,
        ))
    }
}
/// Held outside App because the winit runner consumes its World on shutdown.
pub struct SurfaceCapture {
    observer: SurfaceObserver,
    path: PathBuf,
    started_unix_ms: u128,
}

#[derive(Serialize)]
struct Artifact {
    schema: u32,
    successful_exit: bool,
    started_unix_ms: u128,
    elapsed_ns: u64,
    event_limit: usize,
    omitted_events: u64,
    /// Frames still owned elsewhere at closure, not silently counted as uploads
    /// or loss. The last measurement window may end with frames in flight.
    in_flight_at_close: u64,
    totals: BTreeMap<&'static str, u64>,
    events: Vec<SurfaceEvent>,
}

impl SurfaceCapture {
    pub fn install_from_environment(
        app: &mut App,
        shared_origin: Option<(Instant, u128)>,
    ) -> Result<Option<Self>, String> {
        let path = std::env::var_os(CAPTURE_ENV)
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("PHOENIX_FRAME_CAPTURE")
                    .map(|p| PathBuf::from(p).with_extension("surfaces.json"))
            });
        let Some(path) = path else {
            return Ok(None);
        };
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|e| format!("cannot reserve surface capture {}: {e}", path.display()))?;
        let (origin, started_unix_ms) = match shared_origin {
            Some(origin) => origin,
            None => (
                Instant::now(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_err(|e| e.to_string())?
                    .as_millis(),
            ),
        };
        let observer = SurfaceObserver::new(origin, MAX_CAPTURE_EVENTS);
        app.insert_resource(observer.clone())
            .declare_state::<SurfaceObserver>(
                StateClass::Presentation,
                "native-surface-attribution",
            )
            .add_systems(First, record_main_frame);
        Ok(Some(Self {
            observer,
            path,
            started_unix_ms,
        }))
    }

    /// No worker join: closure takes only the bounded recorder lock. A detached
    /// worker cannot hold capture completion hostage or append after closure.
    pub fn finish(self, exit: &AppExit) -> Result<(), String> {
        let artifact = self.close(exit)?;
        let json = crate::core::codec::to_json(&artifact).map_err(|e| e.to_string())?;
        std::fs::write(&self.path, json).map_err(|e| e.to_string())
    }

    fn close(&self, exit: &AppExit) -> Result<Artifact, String> {
        let mut recording = self.observer.close()?;
        let count = |name| recording.totals.get(name).copied().unwrap_or(0);
        let in_flight_at_close =
            count("produced").saturating_sub(count("uploaded") + count("discarded"));
        let artifact = Artifact {
            schema: 1,
            successful_exit: matches!(exit, AppExit::Success),
            started_unix_ms: self.started_unix_ms,
            elapsed_ns: self.observer.now_ns(),
            event_limit: self.observer.event_limit(),
            omitted_events: recording.omitted_events,
            in_flight_at_close,
            totals: std::mem::take(&mut recording.totals),
            events: std::mem::take(&mut recording.events),
        };
        drop(recording);
        Ok(artifact)
    }
}

fn record_main_frame(observer: Res<SurfaceObserver>) {
    observer.record(None, Operation::MainFrame);
}

#[cfg(test)]
#[path = "surface_stats_tests.rs"]
mod tests;
