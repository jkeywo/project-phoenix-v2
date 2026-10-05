//! Bounded surface observation and frame-lifetime attribution.
use crate::{frames::FrameRect, surface::PaneSurfaceError};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::Instant,
};
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

#[derive(Clone, Debug)]
pub struct SurfaceObserver(Arc<Shared>);

#[derive(Clone, Debug)]
pub struct SurfaceRecording {
    pub events: Vec<SurfaceEvent>,
    pub totals: BTreeMap<&'static str, u64>,
    pub omitted_events: u64,
    pub closed: bool,
}
impl SurfaceObserver {
    pub fn event_limit(&self) -> usize {
        self.0.limit
    }
    pub fn snapshot(&self) -> Result<SurfaceRecording, String> {
        let recording = self.0.recording.lock().map_err(|e| e.to_string())?;
        Ok(SurfaceRecording {
            events: recording.events.clone(),
            totals: recording.totals.clone(),
            omitted_events: recording.omitted_events,
            closed: recording.closed,
        })
    }
    /// Freeze the recorder and transfer its accumulated events to the caller.
    pub fn close(&self) -> Result<SurfaceRecording, String> {
        let mut recording = self.0.recording.lock().map_err(|e| e.to_string())?;
        recording.closed = true;
        Ok(SurfaceRecording {
            events: std::mem::take(&mut recording.events),
            totals: std::mem::take(&mut recording.totals),
            omitted_events: recording.omitted_events,
            closed: true,
        })
    }

    pub fn new(origin: Instant, limit: usize) -> Self {
        Self(Arc::new(Shared {
            origin,
            limit: limit.min(MAX_CAPTURE_EVENTS),
            recording: Mutex::new(Recording::default()),
        }))
    }

    pub fn now_ns(&self) -> u64 {
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

    pub fn events(&self) -> Vec<SurfaceEvent> {
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

pub fn elapsed_ns(start: Option<Instant>) -> u64 {
    start.map_or(0, |at| at.elapsed().as_nanos().min(u64::MAX as u128) as u64)
}
