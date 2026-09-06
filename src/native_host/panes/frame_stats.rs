//! Where a native host's frame goes (`--frame-stats`), and the experiment
//! toggles the numbers are read against.
//!
//! The native host draws every Station console through an embedded, CPU-
//! rasterised Ultralight view, pumped on the Bevy main thread once a frame
//! (`ultralight::drive_panes`). With three extra screens open that frame was
//! measured at ~5.7 fps (2026-09-06, four-screen rig), and this line is what
//! said where it went: ~50 ms of scalar pixel copy, ~50 ms of Ultralight
//! rasterisation and 25–50 ms of synchronous JS pushes of backed-up snapshots,
//! all on the main thread, against a render-thread residual of only 15–20 ms.
//! The copy and the push backlog are fixed (issues #1402, #1403); the line
//! stays, because the rasterisation floor (#1405) and the dedicated pane
//! thread (#1404) are judged by it too.
//!
//! # What is measured
//!
//! Once a second, one log line (`LogCat::Lobby`, info — run with `--log info`):
//!
//! * the Bevy frame period from `Time<Real>` (mean, p95, max, fps);
//! * the five phases of `drive_panes` — `update` (`Renderer::update`), `pump`
//!   (host→page pushes and the page→host drain), `render`
//!   (`Renderer::render`), `copy` (surface → texture bytes) and `publish`
//!   (the rest of the copy loop, chiefly marking the `Image` asset changed);
//! * how many panes copied, how many were forced whole, and the pixels copied
//!   per frame;
//! * how many `Image` assets Bevy was told changed per frame — each one is a
//!   full GPU texture re-creation on the render thread;
//! * how many `FixedUpdate` ticks a frame unpacked into and what they cost;
//! * the **residual**: frame period minus everything above. With pipelined
//!   rendering the main thread blocks until the render thread has finished
//!   the previous frame, so a large residual against small pane phases is the
//!   render thread — swapchain waits included — and nothing on this side.
//!
//! # Why a clock is fine here
//!
//! `Instant::now()` must not be read by a simulation system (`src/perf`'s
//! rule: a measured run and an unmeasured run produce the same simulation).
//! Everything stamped here is presentation: `drive_panes` runs in `Update` on
//! the pane host, and the fixed-loop bracket sits *around* the fixed schedules
//! in `RunFixedMainLoop`, outside every `SimSet`. Nothing measured feeds
//! authoritative state, and `tests/native_headless_digest.rs` is the standing
//! check that it does not.
//!
//! # The experiments
//!
//! [`PaneExperiments`] is a set of diagnostic toggles read once at boot from
//! [`EXPERIMENTS_ENV`] (a comma-separated list), each switching one suspected
//! cost off so the same build can be A/B'd against its own baseline line:
//!
//! | name | what it changes | what it tests |
//! |---|---|---|
//! | `novsync` | Station windows present with `AutoNoVsync` | serialised vblank waits across monitors |
//! | `raf33` | console pages run their render loop at 33 ms instead of 16 | page-side raster and script cost |
//!
//! Two earlier toggles — `untracked` (fetch the pane image untracked) and
//! `noforce` (trust Ultralight's dirty bounds over a push) — were measured
//! and retired: neither moved the frame, because every console repaints
//! nearly its whole surface every frame. Both remaining ones measured *worse*
//! than the baseline on the four-screen rig (CPU contention once the render
//! thread stops blocking; a slower page loop makes each push dearer) and are
//! kept only so the raster work in #1405 can re-check them.
//!
//! They are scaffolding, not features: once the measurement has picked the
//! fixes, the fixes land as ordinary code and the toggles go with them.

use std::fmt;
use std::time::{Duration, Instant};

use bevy::app::{RunFixedMainLoop, RunFixedMainLoopSystems};
use bevy::asset::AssetEvent;
use bevy::ecs::message::Messages;
use bevy::image::Image;
use bevy::prelude::*;
use bevy::window::PresentMode;
use vellum_perf::summarize;

use super::document::PaneDocumentOptions;
use crate::logging::{LogCat, LogFilterConfig};
use crate::sim_tick::SimTick;

/// The environment variable [`PaneExperiments::from_env`] reads.
pub const EXPERIMENTS_ENV: &str = "PHOENIX_FRAME_EXPERIMENTS";

/// How often [`log_frame_stats`] writes a line.
pub const REPORT_PERIOD: Duration = Duration::from_secs(1);

/// The page render-loop interval the `raf33` experiment injects. The default
/// `pane_boot.js` runs is 16 ms; see the note there on why it is a timer at all.
pub const EXPERIMENT_FRAME_MS: u32 = 33;

/// The diagnostic A/B toggles — see the module note's table.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PaneExperiments {
    /// Station windows present with `AutoNoVsync`.
    pub novsync: bool,
    /// Console pages run their render loop at [`EXPERIMENT_FRAME_MS`].
    pub raf33: bool,
}

impl PaneExperiments {
    /// Every name [`parse`](Self::parse) accepts, in display order.
    pub const NAMES: [&'static str; 2] = ["novsync", "raf33"];

    /// Parse a comma-separated list of experiment names. Whitespace and case
    /// are forgiven; an unknown name is refused, because a run that silently
    /// measured the baseline would be worse than no run.
    pub fn parse(list: &str) -> Result<Self, String> {
        let mut out = Self::default();
        for raw in list.split(',') {
            let name = raw.trim();
            if name.is_empty() {
                continue;
            }
            match name.to_ascii_lowercase().as_str() {
                "novsync" => out.novsync = true,
                "raf33" => out.raf33 = true,
                other => {
                    return Err(format!(
                        "unknown frame experiment {other:?}; the known ones are {}",
                        Self::NAMES.join(", ")
                    ))
                }
            }
        }
        Ok(out)
    }

    /// [`parse`](Self::parse) applied to [`EXPERIMENTS_ENV`]; an absent
    /// variable is no experiments.
    pub fn from_env() -> Result<Self, String> {
        match std::env::var(EXPERIMENTS_ENV) {
            Ok(list) => Self::parse(&list),
            Err(_) => Ok(Self::default()),
        }
    }

    /// Whether nothing is switched on.
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// The names switched on, in [`NAMES`](Self::NAMES) order.
    pub fn active(&self) -> Vec<&'static str> {
        [self.novsync, self.raf33]
            .into_iter()
            .zip(Self::NAMES)
            .filter_map(|(on, name)| on.then_some(name))
            .collect()
    }

    /// The present mode a Station window opens with under these experiments.
    pub fn station_present_mode(&self) -> PresentMode {
        if self.novsync {
            PresentMode::AutoNoVsync
        } else {
            PresentMode::default()
        }
    }

    /// The pane-document options these experiments ask for.
    pub fn document_options(&self) -> PaneDocumentOptions {
        PaneDocumentOptions {
            frame_ms: self.raf33.then_some(EXPERIMENT_FRAME_MS),
        }
    }
}

impl fmt::Display for PaneExperiments {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_empty() {
            f.write_str("none")
        } else {
            f.write_str(&self.active().join(","))
        }
    }
}

/// One frame of `drive_panes`, in milliseconds and counts.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PaneFrameSample {
    /// `Renderer::update` — loads, timers, page JavaScript.
    pub update_ms: f64,
    /// The push/drain loop over every pane.
    pub pump_ms: f64,
    /// `Renderer::render` — rasterising every repainted view.
    pub render_ms: f64,
    /// Every `copy_frame` call summed.
    pub copy_ms: f64,
    /// The copy loop less the copies: asset lookup and change marking.
    pub publish_ms: f64,
    /// Panes driven this frame.
    pub panes: usize,
    /// Panes whose copy wrote something.
    pub copied: usize,
    /// Panes copied whole because a push forced it.
    pub forced: usize,
    /// Pixels written across every pane.
    pub pixels: u64,
}

impl PaneFrameSample {
    /// The five phases summed.
    pub fn total_ms(&self) -> f64 {
        self.update_ms + self.pump_ms + self.render_ms + self.copy_ms + self.publish_ms
    }
}

/// One frame's fixed-schedule bracket.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FixedLoopSample {
    /// Wall time spent inside `RunFixedMainLoop`'s fixed schedules.
    pub ms: f64,
    /// `SimTick` steps taken inside it.
    pub ticks: u64,
}

/// A distribution over one report window.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PhaseStat {
    pub mean: f64,
    pub p95: f64,
    pub max: f64,
}

fn phase(samples: &[f64]) -> PhaseStat {
    if samples.is_empty() {
        return PhaseStat::default();
    }
    let s = summarize(samples);
    PhaseStat {
        mean: s.mean,
        p95: s.p95,
        max: s.max,
    }
}

/// The raw samples of one report window. Pure, so the reduction and the line
/// it prints are unit-tested with fabricated frames.
#[derive(Clone, Debug, Default)]
pub struct FrameStatsWindow {
    frames_ms: Vec<f64>,
    panes: Vec<PaneFrameSample>,
    fixed: Vec<FixedLoopSample>,
    modified: Vec<u32>,
}

impl FrameStatsWindow {
    /// One rendered frame's period.
    pub fn push_frame(&mut self, frame_ms: f64) {
        self.frames_ms.push(frame_ms);
    }

    /// One `drive_panes` pass.
    pub fn push_pane(&mut self, sample: PaneFrameSample) {
        self.panes.push(sample);
    }

    /// One frame's fixed-loop bracket.
    pub fn push_fixed(&mut self, sample: FixedLoopSample) {
        self.fixed.push(sample);
    }

    /// How many `Image` assets were reported changed in one frame.
    pub fn push_modified(&mut self, count: u32) {
        self.modified.push(count);
    }

    /// Frames recorded so far.
    pub fn frames(&self) -> usize {
        self.frames_ms.len()
    }

    /// Forget everything, keeping the allocations.
    pub fn clear(&mut self) {
        self.frames_ms.clear();
        self.panes.clear();
        self.fixed.clear();
        self.modified.clear();
    }

    /// Reduce the window, or `None` when no frame was recorded.
    pub fn report(&self, elapsed: Duration) -> Option<PaneFrameReport> {
        if self.frames_ms.is_empty() {
            return None;
        }
        let frames = self.frames_ms.len() as f64;
        let per_frame = |total: f64| total / frames;
        let pane_phase = |pick: fn(&PaneFrameSample) -> f64| {
            phase(&self.panes.iter().map(pick).collect::<Vec<_>>())
        };
        let frame = phase(&self.frames_ms);
        let pane_total_ms = per_frame(self.panes.iter().map(PaneFrameSample::total_ms).sum());
        let fixed_ms = per_frame(self.fixed.iter().map(|f| f.ms).sum());
        let ticks = per_frame(self.fixed.iter().map(|f| f.ticks as f64).sum());
        Some(PaneFrameReport {
            elapsed_s: elapsed.as_secs_f64(),
            frames: self.frames_ms.len(),
            frame,
            fps: if frame.mean > 0.0 {
                1000.0 / frame.mean
            } else {
                0.0
            },
            panes: per_frame(self.panes.iter().map(|s| s.panes as f64).sum()),
            update: pane_phase(|s| s.update_ms),
            pump: pane_phase(|s| s.pump_ms),
            render: pane_phase(|s| s.render_ms),
            copy: pane_phase(|s| s.copy_ms),
            publish: pane_phase(|s| s.publish_ms),
            pane_total_ms,
            copied: per_frame(self.panes.iter().map(|s| s.copied as f64).sum()),
            forced: per_frame(self.panes.iter().map(|s| s.forced as f64).sum()),
            megapixels: per_frame(self.panes.iter().map(|s| s.pixels as f64).sum()) / 1.0e6,
            modified: per_frame(self.modified.iter().map(|m| f64::from(*m)).sum()),
            ticks,
            fixed_ms,
            ms_per_tick: (ticks > 0.0).then(|| fixed_ms / ticks),
            residual_ms: frame.mean - pane_total_ms - fixed_ms,
        })
    }
}

/// One report window reduced. Every rate is per rendered frame.
#[derive(Clone, Debug, PartialEq)]
pub struct PaneFrameReport {
    pub elapsed_s: f64,
    pub frames: usize,
    pub frame: PhaseStat,
    pub fps: f64,
    pub panes: f64,
    pub update: PhaseStat,
    pub pump: PhaseStat,
    pub render: PhaseStat,
    pub copy: PhaseStat,
    pub publish: PhaseStat,
    /// The five pane phases summed, per frame.
    pub pane_total_ms: f64,
    pub copied: f64,
    pub forced: f64,
    pub megapixels: f64,
    /// `Image` assets reported changed, per frame.
    pub modified: f64,
    pub ticks: f64,
    pub fixed_ms: f64,
    pub ms_per_tick: Option<f64>,
    /// Frame period less the pane phases and the fixed loop: the render
    /// thread's share, swapchain waits included.
    pub residual_ms: f64,
}

impl fmt::Display for PaneFrameReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let ph = |p: &PhaseStat| format!("{:.1}/{:.1}", p.mean, p.p95);
        write!(
            f,
            "frame stats: {:.2} s, {} frames | frame {:.1} ms mean, {:.1} p95, {:.1} max ({:.1} fps) \
             | panes {:.1}: update {}, pump {}, render {}, copy {}, publish {} ms (mean/p95; \
             {:.1} ms/frame) | copied {:.1}/frame, forced {:.1}, {:.2} Mpx/frame \
             | image assets changed {:.1}/frame | fixed {:.1} ticks/frame, {:.1} ms/frame{} \
             | residual {:.1} ms/frame",
            self.elapsed_s,
            self.frames,
            self.frame.mean,
            self.frame.p95,
            self.frame.max,
            self.fps,
            self.panes,
            ph(&self.update),
            ph(&self.pump),
            ph(&self.render),
            ph(&self.copy),
            ph(&self.publish),
            self.pane_total_ms,
            self.copied,
            self.forced,
            self.megapixels,
            self.modified,
            self.ticks,
            self.fixed_ms,
            match self.ms_per_tick {
                Some(per) => format!(" ({per:.2} ms/tick)"),
                None => String::new(),
            },
            self.residual_ms,
        )
    }
}

/// The live accumulator. Present only on a host run with `--frame-stats`, so
/// its absence is the whole of "measurement off".
#[derive(Resource, Default)]
pub struct PaneFrameStats {
    /// The current window's samples.
    pub window: FrameStatsWindow,
    /// When the current window opened.
    started: Option<Instant>,
    /// `Image` change events counted since the last frame was closed.
    pending_modified: u32,
    /// The fixed-loop bracket's opening stamp and `SimTick`.
    fixed_start: Option<(Instant, u64)>,
}

impl PaneFrameStats {
    /// Record this frame's `drive_panes` pass.
    pub fn record_pane(&mut self, sample: PaneFrameSample) {
        self.window.push_pane(sample);
    }
}

/// Open the fixed-loop bracket: `RunFixedMainLoopSystems::BeforeFixedMainLoop`.
pub fn mark_fixed_loop_start(stats: Option<ResMut<PaneFrameStats>>, tick: Option<Res<SimTick>>) {
    if let Some(mut stats) = stats {
        stats.fixed_start = Some((Instant::now(), tick.map_or(0, |t| t.0)));
    }
}

/// Close the fixed-loop bracket: `RunFixedMainLoopSystems::AfterFixedMainLoop`.
/// `SimTick` outside the fixed schedules reads the number of completed steps,
/// so its difference across the bracket is the steps this frame unpacked into.
pub fn mark_fixed_loop_end(stats: Option<ResMut<PaneFrameStats>>, tick: Option<Res<SimTick>>) {
    let Some(mut stats) = stats else {
        return;
    };
    if let Some((started, tick_before)) = stats.fixed_start.take() {
        let ticks = tick.map_or(0, |t| t.0).saturating_sub(tick_before);
        let ms = started.elapsed().as_secs_f64() * 1000.0;
        stats.window.push_fixed(FixedLoopSample { ms, ticks });
    }
}

/// Count the `Image` assets Bevy was told changed. Each such event is a full
/// GPU texture re-creation on the render thread, so this is the number any fix
/// that keeps a pane texture in place (#1404) is judged by.
pub fn count_image_modified(
    mut events: MessageReader<AssetEvent<Image>>,
    stats: Option<ResMut<PaneFrameStats>>,
) {
    let modified = events
        .read()
        .filter(|event| matches!(event, AssetEvent::Modified { .. }))
        .count() as u32;
    if let Some(mut stats) = stats {
        stats.pending_modified += modified;
    }
}

/// Close this frame's accounting and, once per [`REPORT_PERIOD`], log the line.
pub fn log_frame_stats(
    time: Res<Time<Real>>,
    stats: Option<ResMut<PaneFrameStats>>,
    log: Option<Res<LogFilterConfig>>,
) {
    let Some(mut stats) = stats else {
        return;
    };
    let frame_ms = time.delta_secs_f64() * 1000.0;
    if frame_ms > 0.0 {
        stats.window.push_frame(frame_ms);
    }
    let modified = std::mem::take(&mut stats.pending_modified);
    stats.window.push_modified(modified);

    let now = Instant::now();
    let started = *stats.started.get_or_insert(now);
    let elapsed = now.duration_since(started);
    if elapsed < REPORT_PERIOD {
        return;
    }
    if let Some(report) = stats.window.report(elapsed) {
        crate::pinfo!(log, LogCat::Lobby, "{report}");
    }
    stats.window.clear();
    stats.started = Some(now);
}

/// Installs the accumulator and the three systems around it. Added only for a
/// windowed host run with `--frame-stats`; `drive_panes` finds the resource and
/// starts stamping its phases.
pub struct PaneFrameStatsPlugin;

impl Plugin for PaneFrameStatsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PaneFrameStats>()
            .add_systems(
                RunFixedMainLoop,
                (
                    mark_fixed_loop_start.in_set(RunFixedMainLoopSystems::BeforeFixedMainLoop),
                    mark_fixed_loop_end.in_set(RunFixedMainLoopSystems::AfterFixedMainLoop),
                ),
            )
            .add_systems(
                PostUpdate,
                (
                    count_image_modified.run_if(resource_exists::<Messages<AssetEvent<Image>>>),
                    log_frame_stats,
                )
                    .chain(),
            );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn experiments_parse_by_name_forgiving_case_and_whitespace() {
        assert_eq!(
            PaneExperiments::parse("").unwrap(),
            PaneExperiments::default()
        );
        assert!(PaneExperiments::parse(" , ").unwrap().is_empty());
        let both = PaneExperiments::parse(" raf33, NOVSYNC ").unwrap();
        assert!(both.novsync && both.raf33);
        assert_eq!(both.active(), vec!["novsync", "raf33"]);
        assert_eq!(both.to_string(), "novsync,raf33");
        assert_eq!(PaneExperiments::default().to_string(), "none");
        let all = PaneExperiments::parse(&PaneExperiments::NAMES.join(",")).unwrap();
        assert_eq!(all.active(), PaneExperiments::NAMES.to_vec());
    }

    #[test]
    fn an_unknown_or_retired_experiment_is_refused_and_names_the_known_ones() {
        for retired in ["untracked", "noforce", "novsinc"] {
            let err = PaneExperiments::parse(&format!("raf33,{retired}")).unwrap_err();
            assert!(err.contains(retired), "{err}");
            for name in PaneExperiments::NAMES {
                assert!(err.contains(name), "{err} should name {name}");
            }
        }
    }

    #[test]
    fn each_experiment_maps_to_exactly_its_knob() {
        let none = PaneExperiments::default();
        assert_eq!(none.station_present_mode(), PresentMode::default());
        assert_eq!(none.document_options(), PaneDocumentOptions::default());

        let novsync = PaneExperiments::parse("novsync").unwrap();
        assert_eq!(novsync.station_present_mode(), PresentMode::AutoNoVsync);
        assert_eq!(novsync.document_options(), PaneDocumentOptions::default());

        let raf33 = PaneExperiments::parse("raf33").unwrap();
        assert_eq!(raf33.station_present_mode(), PresentMode::default());
        assert_eq!(raf33.document_options().frame_ms, Some(EXPERIMENT_FRAME_MS));
    }

    #[test]
    fn an_empty_window_has_no_report() {
        let window = FrameStatsWindow::default();
        assert!(window.report(Duration::from_secs(1)).is_none());
    }

    fn fabricated() -> FrameStatsWindow {
        let mut window = FrameStatsWindow::default();
        for _ in 0..2 {
            window.push_frame(100.0);
            window.push_pane(PaneFrameSample {
                update_ms: 1.0,
                pump_ms: 2.0,
                render_ms: 3.0,
                copy_ms: 4.0,
                publish_ms: 5.0,
                panes: 4,
                copied: 4,
                forced: 2,
                pixels: 1_000_000,
            });
            window.push_fixed(FixedLoopSample { ms: 12.0, ticks: 6 });
            window.push_modified(4);
        }
        window
    }

    #[test]
    fn a_window_reduces_to_per_frame_rates_and_a_residual() {
        let report = fabricated().report(Duration::from_secs(1)).unwrap();
        assert_eq!(report.frames, 2);
        assert_eq!(report.frame.mean, 100.0);
        assert_eq!(report.fps, 10.0);
        assert_eq!(report.panes, 4.0);
        assert_eq!(report.update.mean, 1.0);
        assert_eq!(report.publish.p95, 5.0);
        assert_eq!(report.pane_total_ms, 15.0);
        assert_eq!(report.copied, 4.0);
        assert_eq!(report.forced, 2.0);
        assert_eq!(report.megapixels, 1.0);
        assert_eq!(report.modified, 4.0);
        assert_eq!(report.ticks, 6.0);
        assert_eq!(report.fixed_ms, 12.0);
        assert_eq!(report.ms_per_tick, Some(2.0));
        assert_eq!(report.residual_ms, 100.0 - 15.0 - 12.0);
    }

    #[test]
    fn the_line_carries_every_number_an_operator_reads_it_for() {
        let line = fabricated()
            .report(Duration::from_secs(1))
            .unwrap()
            .to_string();
        for needle in [
            "2 frames",
            "frame 100.0 ms mean",
            "(10.0 fps)",
            "panes 4.0",
            "update 1.0/1.0",
            "15.0 ms/frame",
            "copied 4.0/frame",
            "forced 2.0",
            "1.00 Mpx/frame",
            "image assets changed 4.0/frame",
            "fixed 6.0 ticks/frame",
            "(2.00 ms/tick)",
            "residual 73.0 ms/frame",
        ] {
            assert!(line.contains(needle), "{line:?} should contain {needle:?}");
        }
    }

    #[test]
    fn a_window_with_no_ticks_reports_no_per_tick_cost() {
        let mut window = FrameStatsWindow::default();
        window.push_frame(16.0);
        window.push_fixed(FixedLoopSample { ms: 0.5, ticks: 0 });
        let report = window.report(Duration::from_millis(16)).unwrap();
        assert_eq!(report.ms_per_tick, None);
        assert!(!report.to_string().contains("ms/tick"));
    }

    #[test]
    fn clearing_forgets_the_frames() {
        let mut window = fabricated();
        assert_eq!(window.frames(), 2);
        window.clear();
        assert_eq!(window.frames(), 0);
        assert!(window.report(Duration::from_secs(1)).is_none());
    }
}
