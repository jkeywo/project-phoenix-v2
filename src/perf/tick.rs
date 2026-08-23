//! The headless harness-loop collector (issue #868).
//!
//! Sampling happens in the harness loop around `app.update()`, not in a Bevy
//! system, so nothing here can be observed by authoritative state or change a
//! tick's outcome.

use std::time::Instant;

use vellum_perf::{Capture, Profile, Recorder, Unit};

/// Metric name for per-tick wall time.
pub const TICK_METRIC: &str = "sim.tick";
/// Metric name for whole-run wall time.
pub const RUN_METRIC: &str = "sim.run";

/// The runtime a headless capture records as its provenance.
pub const RUNTIME: &str = "headless-native";

/// Samples the harness loop feeds, one tick at a time.
///
/// Held by the caller rather than the app: the `App` is the simulation, and
/// the point of this module is that measurement never enters it.
pub struct TickSampler {
    recorder: Recorder,
    run_started: Instant,
    tick_started: Instant,
}

impl TickSampler {
    pub fn new() -> Self {
        let now = Instant::now();
        Self {
            recorder: Recorder::new(),
            run_started: now,
            tick_started: now,
        }
    }

    /// Called immediately before `app.update()`.
    pub fn tick_begin(&mut self) {
        self.tick_started = Instant::now();
    }

    /// Called immediately after `app.update()`.
    pub fn tick_end(&mut self) {
        let elapsed = self.tick_started.elapsed().as_secs_f64() * 1000.0;
        self.recorder.sample(TICK_METRIC, Unit::Millis, elapsed);
    }

    /// Borrow the recorder so a collector that did not do its own timing can
    /// fold its samples into this run's capture.
    ///
    /// The one such collector today is [`crate::perf::console`], which bridges
    /// samples the PRD #1144 debug pipeline already holds (issue #1169). It is
    /// exposed rather than the recorder being made public because the invariant
    /// this module exists for is unchanged: measurement stays outside the `App`,
    /// and the caller — not a Bevy system — decides what enters a capture.
    pub fn recorder_mut(&mut self) -> &mut Recorder {
        &mut self.recorder
    }

    /// Close the run and produce the capture artifact.
    pub fn finish(mut self, scenario: &str, profile: Profile) -> Capture {
        let run_secs = self.run_started.elapsed().as_secs_f64();
        self.recorder.sample(RUN_METRIC, Unit::Seconds, run_secs);
        self.recorder.finish(scenario, profile)
    }
}

impl Default for TickSampler {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::perf::profile;

    #[test]
    fn the_sampler_records_one_tick_sample_per_tick() {
        let mut sampler = TickSampler::new();
        for _ in 0..3 {
            sampler.tick_begin();
            sampler.tick_end();
        }
        let capture = sampler.finish("test-scenario", profile(RUNTIME));
        assert_eq!(capture.summaries[TICK_METRIC].summary.count, 3);
        assert_eq!(capture.summaries[RUN_METRIC].summary.count, 1);
    }
}
