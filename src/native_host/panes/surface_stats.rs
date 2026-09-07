//! Opt-in, bounded surface attribution. No timings enter simulation state.
//!
//! Raw events use integer nanoseconds from one monotonic origin. SDK update and
//! render are global iteration costs, never invented per-view raster timings.
//! A frame's identity travels with its owned buffer; moving or dropping it cannot
//! reattribute old pixels to the new view at the same pane id.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use bevy::app::AppExit;
use bevy::prelude::*;
use serde::Serialize;

use super::pane_thread::FrameRect;
use super::surface::PaneSurfaceError;
use crate::authoritative::{DeclareState, StateClass};

pub const CAPTURE_ENV: &str = "PHOENIX_SURFACE_CAPTURE";
/// Fixed event budget; a longer run explicitly truncates its raw detail.
pub const MAX_CAPTURE_EVENTS: usize = 262_144;

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct SurfaceIdentity {
    pub id: u32,
    pub epoch: u64,
    pub kind: &'static str,
    pub width: u32,
    pub height: u32,
    pub device_scale: f64,
    pub visible: bool,
}

/// Causes may coexist. These describe the existing force decision, not a new one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct FullCopyReasons {
    pub initial: bool,
    pub resize: bool,
    pub reveal: bool,
    pub bridge_push: bool,
    pub hud_push: bool,
    pub copy_retry: bool,
    pub buffer_retry: bool,
}

/// An observed dirty region and a copied region are different facts. The
/// pinned SDK wrapper exposes no pre-force bounds, so force/failure stays
/// unknown even when a full rectangle was copied successfully.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CopyObservation {
    pub outcome: &'static str,
    pub dirty_rect: Option<FrameRect>,
    pub copied_rect: Option<FrameRect>,
    pub forced: bool,
    pub reasons: FullCopyReasons,
}

impl CopyObservation {
    pub fn from_result(
        result: &Result<Option<FrameRect>, PaneSurfaceError>,
        forced: bool,
        reasons: FullCopyReasons,
    ) -> Self {
        let (outcome, copied_rect) = match result {
            Ok(Some(rect)) => ("copied", Some(*rect)),
            Ok(None) => ("clean", None),
            Err(_) => ("failed", None),
        };
        Self {
            outcome,
            copied_rect,
            dirty_rect: (!forced && result.is_ok()).then_some(copied_rect.unwrap_or_default()),
            forced,
            reasons,
        }
    }

    pub fn unobserved(outcome: &'static str, forced: bool, reasons: FullCopyReasons) -> Self {
        Self {
            outcome,
            dirty_rect: None,
            copied_rect: None,
            forced,
            reasons,
        }
    }

    pub fn operation(self, duration_ns: u64) -> Operation {
        Operation::Copy {
            outcome: self.outcome,
            dirty_pixels: self.dirty_rect.map(|rect| rect.pixel_count()),
            copied_pixels: self.copied_rect.map_or(0, |rect| rect.pixel_count()),
            dirty_rect: self.dirty_rect,
            copied_rect: self.copied_rect,
            forced: self.forced,
            reasons: self.reasons,
            duration_ns,
        }
    }

    /// Literal half-open raster coordinates, suitable for the ordinary
    /// measured host's Lobby log. No formatting or allocation in an unmeasured
    /// worker; the main adapter invokes this only while draining observations.
    pub fn log_line(self, surface: SurfaceIdentity) -> String {
        fn rect(rect: Option<FrameRect>, absent: &str) -> String {
            rect.map_or_else(
                || absent.into(),
                |r| format!("[{},{},{},{}]", r.left, r.top, r.right, r.bottom),
            )
        }
        format!("pane surface: id={} epoch={} kind={} raster={}x{} device_scale={} visible={} outcome={} dirty_rect={} copied_rect={} forced={} reasons={:?}",
            surface.id, surface.epoch, surface.kind, surface.width, surface.height, surface.device_scale, surface.visible, self.outcome,
            rect(self.dirty_rect, "unknown"), rect(self.copied_rect, "none"), self.forced, self.reasons)
    }
}

impl FullCopyReasons {
    pub fn merge(&mut self, other: Self) {
        self.initial |= other.initial;
        self.resize |= other.resize;
        self.reveal |= other.reveal;
        self.bridge_push |= other.bridge_push;
        self.hud_push |= other.hud_push;
        self.copy_retry |= other.copy_retry;
        self.buffer_retry |= other.buffer_retry;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DiscardReason {
    Closed,
    StaleEpoch,
    SupersededDeferral,
    DeferralExhausted,
    RefusedLayout,
    NoRenderer,
    /// Receiver teardown, a refused sink, or another path with no named outcome.
    BufferDropped,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Operation {
    MainFrame,
    MainPass {
        drain_ns: u64,
        queue_ns: u64,
        frames: u64,
    },
    Iteration {
        update_ns: u64,
        pump_ns: u64,
        render_ns: u64,
        copy_ns: u64,
        publish_ns: u64,
        total_ns: u64,
    },
    Lifecycle {
        action: &'static str,
    },
    HudSlot {
        revision: u64,
        has_script: bool,
    },
    Push {
        channel: &'static str,
        revision: Option<u64>,
        applied: u64,
        failed: u64,
        deferred_messages: u64,
        duration_ns: u64,
    },
    Copy {
        outcome: &'static str,
        dirty_pixels: Option<u64>,
        copied_pixels: u64,
        dirty_rect: Option<FrameRect>,
        copied_rect: Option<FrameRect>,
        forced: bool,
        reasons: FullCopyReasons,
        duration_ns: u64,
    },
    Produced {
        pixels: u64,
        forced: bool,
        reasons: FullCopyReasons,
        hud_revision: Option<u64>,
    },
    Drained {
        age_ns: u64,
    },
    Extracted {
        age_ns: u64,
    },
    Deferred {
        age_ns: u64,
        attempts: u8,
    },
    PromotedFull {
        pixels: u64,
    },
    Uploaded {
        age_ns: u64,
        pixels: u64,
        full: bool,
        write_texture_ns: u64,
    },
    Discarded {
        age_ns: u64,
        reason: DiscardReason,
    },
}

impl Operation {
    fn name(&self) -> &'static str {
        match self {
            Self::MainFrame => "main_frames",
            Self::MainPass { .. } => "main_passes",
            Self::Iteration { .. } => "worker_iterations",
            Self::Lifecycle { .. } => "lifecycle_events",
            Self::HudSlot { .. } => "hud_slot_revisions",
            Self::Push { .. } => "push_batches",
            Self::Copy { .. } => "copy_decisions",
            Self::Produced { .. } => "produced",
            Self::Drained { .. } => "drained",
            Self::Extracted { .. } => "extracted",
            Self::Deferred { .. } => "deferred_attempts",
            Self::PromotedFull { .. } => "promoted_full",
            Self::Uploaded { .. } => "uploaded",
            Self::Discarded { .. } => "discarded",
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct SurfaceEvent {
    pub at_ns: u64,
    pub surface: Option<SurfaceIdentity>,
    pub frame: Option<u64>,
    #[serde(flatten)]
    pub operation: Operation,
}

#[derive(Debug, Default)]
struct Recording {
    events: Vec<SurfaceEvent>,
    totals: BTreeMap<&'static str, u64>,
    omitted_events: u64,
    next_frame: u64,
    closed: bool,
}

impl Recording {
    fn push(&mut self, event: SurfaceEvent, limit: usize) {
        if self.closed {
            return;
        }
        *self.totals.entry(event.operation.name()).or_default() += 1;
        if self.events.len() < limit {
            self.events.push(event);
        } else {
            self.omitted_events += 1;
        }
    }
}

#[derive(Debug)]
struct Shared {
    origin: Instant,
    limit: usize,
    recording: Mutex<Recording>,
}

#[derive(Resource, Clone, Debug)]
pub struct SurfaceObserver(Arc<Shared>);

impl SurfaceObserver {
    pub fn new(origin: Instant, limit: usize) -> Self {
        Self(Arc::new(Shared {
            origin,
            limit: limit.min(MAX_CAPTURE_EVENTS),
            recording: Mutex::new(Recording::default()),
        }))
    }

    fn now_ns(&self) -> u64 {
        self.0.origin.elapsed().as_nanos().min(u64::MAX as u128) as u64
    }

    pub fn record(&self, surface: Option<SurfaceIdentity>, operation: Operation) {
        self.record_at(self.now_ns(), surface, None, operation);
    }

    fn record_at(
        &self,
        at_ns: u64,
        surface: Option<SurfaceIdentity>,
        frame: Option<u64>,
        operation: Operation,
    ) {
        self.0
            .recording
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(
                SurfaceEvent {
                    at_ns,
                    surface,
                    frame,
                    operation,
                },
                self.0.limit,
            );
    }

    pub fn produced(
        &self,
        surface: SurfaceIdentity,
        pixels: u64,
        forced: bool,
        reasons: FullCopyReasons,
        hud_revision: Option<u64>,
    ) -> FrameTrace {
        let at_ns = self.now_ns();
        let mut recording = self.0.recording.lock().unwrap_or_else(|e| e.into_inner());
        let frame = recording.next_frame;
        recording.next_frame += 1;
        recording.push(
            SurfaceEvent {
                at_ns,
                surface: Some(surface),
                frame: Some(frame),
                operation: Operation::Produced {
                    pixels,
                    forced,
                    reasons,
                    hud_revision,
                },
            },
            self.0.limit,
        );
        FrameTrace {
            observer: self.clone(),
            surface,
            frame,
            produced_ns: at_ns,
            terminal: false,
        }
    }

    #[cfg(test)]
    pub(crate) fn events(&self) -> Vec<SurfaceEvent> {
        self.0.recording.lock().unwrap().events.clone()
    }
}

/// Non-cloneable, like the buffer it accompanies: one terminal outcome at most.
#[derive(Debug)]
pub struct FrameTrace {
    observer: SurfaceObserver,
    surface: SurfaceIdentity,
    frame: u64,
    produced_ns: u64,
    terminal: bool,
}

impl FrameTrace {
    fn record(&self, make: impl FnOnce(u64) -> Operation) {
        let now = self.observer.now_ns();
        self.observer.record_at(
            now,
            Some(self.surface),
            Some(self.frame),
            make(now.saturating_sub(self.produced_ns)),
        );
    }
    pub fn drained(&self) {
        self.record(|age_ns| Operation::Drained { age_ns });
    }
    pub fn extracted(&self) {
        self.record(|age_ns| Operation::Extracted { age_ns });
    }
    pub fn deferred(&self, attempts: u8) {
        self.record(|age_ns| Operation::Deferred { age_ns, attempts });
    }
    pub fn promoted(&self, pixels: u64) {
        self.record(|_| Operation::PromotedFull { pixels });
    }
    pub fn uploaded(&mut self, pixels: u64, full: bool, write_texture_ns: u64) {
        if self.terminal {
            return;
        }
        self.terminal = true;
        self.record(|age_ns| Operation::Uploaded {
            age_ns,
            pixels,
            full,
            write_texture_ns,
        });
    }
    pub fn discarded(&mut self, reason: DiscardReason) {
        if self.terminal {
            return;
        }
        self.terminal = true;
        self.record(|age_ns| Operation::Discarded { age_ns, reason });
    }
}

impl Drop for FrameTrace {
    fn drop(&mut self) {
        self.discarded(DiscardReason::BufferDropped);
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
        let json = crate::core::codec::encode_presentation_capture(&artifact)
            .map_err(|e| e.to_string())?;
        std::fs::write(&self.path, json).map_err(|e| e.to_string())
    }

    fn close(&self, exit: &AppExit) -> Result<Artifact, String> {
        let mut recording = self
            .observer
            .0
            .recording
            .lock()
            .map_err(|e| e.to_string())?;
        recording.closed = true;
        let count = |name| recording.totals.get(name).copied().unwrap_or(0);
        let in_flight_at_close =
            count("produced").saturating_sub(count("uploaded") + count("discarded"));
        let artifact = Artifact {
            schema: 1,
            successful_exit: matches!(exit, AppExit::Success),
            started_unix_ms: self.started_unix_ms,
            elapsed_ns: self.observer.now_ns(),
            event_limit: self.observer.0.limit,
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

pub fn elapsed_ns(start: Option<Instant>) -> u64 {
    start.map_or(0, |at| at.elapsed().as_nanos().min(u64::MAX as u128) as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn surface() -> SurfaceIdentity {
        SurfaceIdentity {
            id: 7,
            epoch: 2,
            kind: "console",
            width: 10,
            height: 20,
            device_scale: 1.5,
            visible: true,
        }
    }

    #[test]
    fn rectangle_log_and_capture_keep_dirty_knowledge_separate_from_copied_bounds() {
        let rect = FrameRect {
            left: 2,
            top: 3,
            right: 7,
            bottom: 9,
        };
        let partial =
            CopyObservation::from_result(&Ok(Some(rect)), false, FullCopyReasons::default());
        let json = serde_json::to_value(partial.operation(42)).unwrap();
        assert_eq!(
            json["dirty_rect"],
            serde_json::json!({ "left": 2, "top": 3, "right": 7, "bottom": 9 })
        );
        assert_eq!(json["copied_rect"], json["dirty_rect"]);
        assert_eq!(json["dirty_pixels"], 30);
        assert_eq!(json["copied_pixels"], 30);
        assert_eq!(json["duration_ns"], 42);
        assert!(partial.log_line(surface()).contains("id=7 epoch=2 kind=console raster=10x20 device_scale=1.5 visible=true outcome=copied dirty_rect=[2,3,7,9] copied_rect=[2,3,7,9] forced=false"));

        let forced = CopyObservation::from_result(
            &Ok(Some(FrameRect::full(10, 20))),
            true,
            FullCopyReasons {
                reveal: true,
                ..Default::default()
            },
        );
        let json = serde_json::to_value(forced.operation(0)).unwrap();
        assert!(json["dirty_rect"].is_null() && json["dirty_pixels"].is_null());
        assert_eq!(json["copied_pixels"], 200);
        assert!(forced
            .log_line(surface())
            .contains("dirty_rect=unknown copied_rect=[0,0,10,20] forced=true"));
        assert_eq!(json["reasons"]["reveal"], true);

        let clean = CopyObservation::from_result(&Ok(None), false, FullCopyReasons::default());
        assert_eq!(clean.dirty_rect, Some(FrameRect::default()));
        assert!(clean
            .log_line(surface())
            .contains("outcome=clean dirty_rect=[0,0,0,0] copied_rect=none"));
        let failed = CopyObservation::from_result(
            &Err(PaneSurfaceError::Frame("lock".into())),
            false,
            FullCopyReasons::default(),
        );
        assert!(failed
            .log_line(surface())
            .contains("outcome=failed dirty_rect=unknown copied_rect=none"));
        let json = serde_json::to_value(failed.operation(1)).unwrap();
        assert!(json["dirty_pixels"].is_null());
        assert_eq!(json["copied_pixels"], 0);
    }

    #[test]
    fn buffer_lifetime_records_exactly_one_terminal_outcome_and_original_identity() {
        let observer = SurfaceObserver::new(Instant::now(), 32);
        let mut frame = observer.produced(surface(), 200, true, FullCopyReasons::default(), None);
        frame.drained();
        frame.extracted();
        frame.deferred(1);
        frame.uploaded(200, true, 42);
        frame.discarded(DiscardReason::StaleEpoch);
        drop(frame);
        let events = observer.events();
        assert_eq!(events.len(), 5);
        assert!(events
            .iter()
            .all(|e| e.surface == Some(surface()) && e.frame == Some(0)));
        assert!(matches!(
            events.last().unwrap().operation,
            Operation::Uploaded {
                write_texture_ns: 42,
                ..
            }
        ));
    }

    #[test]
    fn bounded_recording_keeps_exact_totals_and_discloses_missing_raw_events() {
        let observer = SurfaceObserver::new(Instant::now(), 1);
        let frame = observer.produced(surface(), 1, false, FullCopyReasons::default(), None);
        drop(frame);
        let recording = observer.0.recording.lock().unwrap();
        assert_eq!(recording.events.len(), 1);
        assert_eq!(recording.omitted_events, 1);
        assert_eq!(recording.totals["produced"], 1);
        assert_eq!(recording.totals["discarded"], 1);
    }

    #[test]
    fn closure_discloses_in_flight_frames_and_refuses_late_worker_events() {
        let observer = SurfaceObserver::new(Instant::now(), 32);
        let frame = observer.produced(surface(), 200, true, FullCopyReasons::default(), None);
        let capture = SurfaceCapture {
            observer: observer.clone(),
            path: PathBuf::new(),
            started_unix_ms: 0,
        };
        let artifact = capture.close(&AppExit::Success).unwrap();
        assert!(artifact.successful_exit);
        assert_eq!(artifact.in_flight_at_close, 1);
        assert_eq!(artifact.totals["produced"], 1);
        assert_eq!(artifact.omitted_events, 0);
        drop(frame);
        observer.record(None, Operation::MainFrame);
        let recording = observer.0.recording.lock().unwrap();
        assert!(recording.closed);
        assert!(recording.events.is_empty());
        assert!(
            recording.totals.is_empty(),
            "the finished artifact has one immutable boundary"
        );
    }
}
