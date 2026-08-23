//! The console-latency collector (issue #1169, on the issue #868 contract).
//!
//! # Why this collector is shaped differently from its four neighbours
//!
//! [`crate::perf::tick`], [`crate::perf::assets`], [`crate::perf::mesh`] and
//! [`crate::perf::browser`] all *take* their own measurements. This one does
//! not: the samples already exist, in
//! [`crate::debug::ConsoleLatencyTracker`], because the debug observability
//! pipeline (PRD #1144) needs them for its own payload and its headless report.
//! Measuring them a second time here would give two numbers for one thing, and
//! the interesting failure is the day they disagree.
//!
//! So this module is a *bridge*: it hands the tracker's retained raw samples to
//! a `vellum_perf::Recorder`, one at a time, and lets the crate compute the
//! statistics a baseline compares. The debug payload's own p50/p75/max are
//! computed by the same nearest-rank definition
//! (`crate::debug::console_latency::percentile`), so the two views of one run
//! agree by construction rather than by coincidence.
//!
//! # What the metric means
//!
//! [`CONSOLE_ACK_METRIC`] carries only the **`SimHost`** surface: the wall time
//! from a tick's command admission to the end of that tick's
//! `SimSet::Broadcast`. That is deliberate, and it is the honest choice:
//!
//! * It is the only segment a headless CI run can produce at all — there is no
//!   browser and no phone, so no client-measured segment exists.
//! * It is the only segment that is a function of the CHECKOUT rather than of a
//!   player's network. A phone's WebRTC round trip on a stranger's wifi is a
//!   real number and a useless budget; the simulation's own service window is
//!   the part a regression in this repository can move.
//! * It is the only PER-COMMAND segment. The client's `send_to_ack` ends when
//!   the issuing console next received server state, so it is a
//!   perceived-feedback proxy bounded below by the host's broadcast cadence
//!   (issue #1169 review, finding C1) — it would move for reasons no processing
//!   change caused, and fail to move for ones that did.
//!
//! The client-measured segments stay where they can be read in context — the
//! debug dock and the run report — and are not smuggled into a budget that
//! could not fairly hold anyone to them.
//!
//! # The metric must not charge for its own production
//!
//! The samples are free to this collector, but they were not free to the run: the
//! flag-gated JSON publish clones labels, sorts each retained window and encodes,
//! and it used to do that EVERY tick, inside the window `sim.tick` measures
//! (issue #1169 review, finding C4). It is now throttled to 4 Hz of simulation
//! time and the per-command action label is a `&'static str` rather than a
//! `format!`. Measured on `probe_duel` at 1800 ticks, six interleaved runs per
//! build: the pre-fix build sat +0.7% / +3.2% (p50 / p95) above a build with no
//! console-latency measurement at all; the fixed build does not sit above it on
//! any statistic.
//!
//! Per the module contract in [`crate::perf`], this collector *exposes* the
//! metric; whether it becomes a budget is a separate, deliberate decision that
//! lands in a reviewable `perf/baselines/*.ron` diff via `phoenix-perf adopt`.
//! Nothing here writes a baseline.

use vellum_perf::{Recorder, Unit};

use crate::debug::ConsoleLatencyTracker;

/// Metric name for the simulation's admission→broadcast service window.
///
/// Named for what a *player* is waiting on ("ack") rather than for the schedule
/// points that bracket it, so it reads alongside `sim.tick` and `browser.frame`
/// as a latency a person could notice. Millis, lower is better — see
/// `crate::perf::metric_direction`.
pub const CONSOLE_ACK_METRIC: &str = "sim.console_ack";

/// Feed every retained host-surface sample into `recorder`.
///
/// One `recorder.sample` per retained sample rather than one per summary: a
/// `Recorder` summarises what it is given, and handing it a pre-computed p50
/// would make the capture's `count` a lie and its own percentiles meaningless.
///
/// A tracker with nothing in it contributes nothing, which is exactly right —
/// an unmeasured run must produce no metric rather than a zero, so that a
/// baselined budget reports `Incomparable` ("this was not checked") instead of
/// a spurious pass.
pub fn sample_console_latency(recorder: &mut Recorder, tracker: &ConsoleLatencyTracker) {
    for (_action, ms) in tracker.host_samples() {
        recorder.sample(CONSOLE_ACK_METRIC, Unit::Millis, ms as f64);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::messages::LatencySurface;
    use crate::perf::profile;

    #[test]
    fn every_retained_host_sample_reaches_the_recorder() {
        let mut tracker = ConsoleLatencyTracker::default();
        tracker.record_host("FirePhaser", 1.0);
        tracker.record_host("FirePhaser", 3.0);
        tracker.record_host("SetThrottle", 2.0);

        let mut recorder = Recorder::new();
        sample_console_latency(&mut recorder, &tracker);
        let capture = recorder.finish("test-scenario", profile("test"));

        let summary = &capture.summaries[CONSOLE_ACK_METRIC].summary;
        assert_eq!(summary.count, 3, "one perf sample per retained sample");
        assert_eq!(summary.max, 3.0);
    }

    /// A run that measured nothing must contribute NO metric — a zeroed one
    /// would read as a budget that passed.
    #[test]
    fn an_unmeasured_run_contributes_no_metric() {
        let mut recorder = Recorder::new();
        sample_console_latency(&mut recorder, &ConsoleLatencyTracker::default());
        assert!(recorder.is_empty());
    }

    /// Client-reported samples must never reach the budget: they measure a
    /// player's network, which no checkout controls — and since the #1169 review
    /// they are additionally a *perceived-feedback proxy* bounded below by the
    /// host's broadcast cadence, which would make them meaningless as a
    /// processing budget even over a perfect link.
    #[test]
    fn client_measured_samples_stay_out_of_the_budget() {
        let mut tracker = ConsoleLatencyTracker::default();
        tracker.record_client(
            LatencySurface::PhoneConsole,
            &crate::core::messages::ConsoleLatencySample {
                action: "fire_phaser".into(),
                input_to_send_ms: 5.0,
                send_to_ack_ms: 500.0,
            },
        );

        let mut recorder = Recorder::new();
        sample_console_latency(&mut recorder, &tracker);
        assert!(
            recorder.is_empty(),
            "a phone's round trip is not a budget this repository can be held to"
        );
    }
}
