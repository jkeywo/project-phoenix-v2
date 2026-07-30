//! Performance measurement for the headless harness (issue #868).
//!
//! The measurement *contract* — series, summaries, provenance, baselines,
//! comparison, rendering — is `vellum-perf`, and phoenix is its second
//! consumer. What lives here is the part the crate deliberately excludes:
//! the collector, and where the baseline file lives.
//!
//! Collection stays out of the simulation. Sampling happens in the harness
//! loop around `app.update()`, not in a Bevy system, so nothing in this file
//! can be observed by authoritative state or change a tick's outcome. A run
//! with measurement on and a run with it off produce the same simulation.
//!
//! Two metrics, both wall-clock:
//!
//! - `sim.tick` (millis, one sample per tick) — the distribution the p95
//!   expectation is written against.
//! - `sim.run` (seconds, one sample) — whole-run cost, which is what a
//!   scheduled CI job actually spends.
//!
//! Values here are benchmark evidence, not assertions. Nothing in the test
//! suite asserts on a duration; the tests cover the pure machinery around it.

use std::path::Path;
use std::time::Instant;

use vellum_perf::{Baseline, Capture, Finding, Profile, Recorder, Unit};

/// Metric name for per-tick wall time.
pub const TICK_METRIC: &str = "sim.tick";
/// Metric name for whole-run wall time.
pub const RUN_METRIC: &str = "sim.run";

/// Where committed baselines live. One file per scenario, RON, reviewable in
/// a diff — the crate owns the types, this repo owns the layout.
pub const BASELINE_DIR: &str = "perf/baselines";

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

/// Build provenance for a capture, so two captures are only compared when
/// they are comparable.
///
/// `device` and `rev` come from the environment because only the environment
/// knows them: CI sets `GITHUB_SHA` and identifies its runner image, and a
/// developer's desktop is not a runner. An unset value stays empty rather
/// than guessing — the contract is that provenance is honest, not complete.
pub fn profile() -> Profile {
    Profile {
        runtime: "headless-native".to_string(),
        build: if cfg!(debug_assertions) {
            "dev".to_string()
        } else {
            "release".to_string()
        },
        device: std::env::var("PHOENIX_PERF_DEVICE")
            .or_else(|_| std::env::var("RUNNER_OS"))
            .unwrap_or_default(),
        rev: std::env::var("GITHUB_SHA").unwrap_or_default(),
    }
}

/// The committed baseline path for one scenario.
pub fn baseline_path(scenario: &str) -> String {
    format!("{BASELINE_DIR}/{scenario}.ron")
}

/// Errors reading a baseline. A missing file is not one of them — see
/// [`load_baseline`].
#[derive(Debug)]
pub enum BaselineError {
    Io(String, std::io::Error),
    // Boxed: RON's spanned error carries its whole error enum plus a position,
    // and an unboxed variant makes every Ok result pay for the failure case.
    Parse(String, Box<ron::error::SpannedError>),
}

impl std::fmt::Display for BaselineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BaselineError::Io(path, e) => write!(f, "could not read baseline {path:?}: {e}"),
            BaselineError::Parse(path, e) => write!(f, "malformed baseline {path:?}: {e}"),
        }
    }
}

/// Read a baseline, or `None` when the scenario has no committed baseline yet.
///
/// A scenario measured before anyone has an opinion about its numbers is the
/// normal first state, not an error — the same reasoning that makes an
/// unbaselined metric produce no finding.
pub fn load_baseline(path: &Path) -> Result<Option<Baseline>, BaselineError> {
    let display = path.display().to_string();
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(BaselineError::Io(display, e)),
    };
    ron::from_str(&text)
        .map(Some)
        .map_err(|e| BaselineError::Parse(display, Box::new(e)))
}

/// The comparison report, warnings-first.
///
/// Returns the rendered findings and the verdict, and leaves the decision
/// about exit codes to the caller — #868 is explicit that measurement informs
/// optimisation before it gates correctness.
pub fn report(capture: &Capture, baseline: &Baseline) -> (Vec<Finding>, String) {
    let findings = vellum_perf::compare(capture, baseline);
    let rendered = vellum_perf::render(&findings);
    (findings, rendered)
}

#[cfg(test)]
mod tests {
    use super::*;
    use vellum_perf::{Expectation, Statistic, Tolerance, Verdict};

    fn capture_with(metric: &str, unit: Unit, values: &[f64]) -> Capture {
        let mut recorder = Recorder::new();
        for value in values {
            recorder.sample(metric, unit.clone(), *value);
        }
        recorder.finish("test-scenario", profile())
    }

    fn baseline_with(metric: &str, unit: Unit, expected: f64) -> Baseline {
        let mut baseline = Baseline {
            scenario: "test-scenario".to_string(),
            ..Default::default()
        };
        baseline.expectations.insert(
            metric.to_string(),
            Expectation {
                unit,
                statistic: Statistic::P95,
                expected,
                tolerance: Tolerance {
                    warn: 0.25,
                    fail: 1.0,
                },
            },
        );
        baseline
    }

    #[test]
    fn baseline_path_is_one_file_per_scenario() {
        assert_eq!(
            baseline_path("headless-default"),
            "perf/baselines/headless-default.ron"
        );
    }

    #[test]
    fn a_missing_baseline_is_absence_not_failure() {
        let path = Path::new("perf/baselines/no-such-scenario.ron");
        assert!(matches!(load_baseline(path), Ok(None)));
    }

    #[test]
    fn the_committed_baseline_parses() {
        let path = baseline_path("headless-default");
        let baseline = load_baseline(Path::new(&path))
            .expect("committed baseline parses")
            .expect("committed baseline exists");
        assert_eq!(baseline.scenario, "headless-default");
        assert!(baseline.expectations.contains_key(TICK_METRIC));
    }

    #[test]
    fn a_metric_within_tolerance_passes() {
        let capture = capture_with(TICK_METRIC, Unit::Millis, &[10.0, 10.0, 11.0]);
        let baseline = baseline_with(TICK_METRIC, Unit::Millis, 10.0);
        let (findings, _) = report(&capture, &baseline);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].verdict, Verdict::Pass);
    }

    #[test]
    fn drift_beyond_tolerance_warns_rather_than_failing() {
        let capture = capture_with(TICK_METRIC, Unit::Millis, &[14.0, 14.0, 14.0]);
        let baseline = baseline_with(TICK_METRIC, Unit::Millis, 10.0);
        let (findings, _) = report(&capture, &baseline);
        assert_eq!(findings[0].verdict, Verdict::Warn);
    }

    #[test]
    fn a_unit_mismatch_is_incomparable_not_a_regression() {
        let capture = capture_with(TICK_METRIC, Unit::Seconds, &[10.0]);
        let baseline = baseline_with(TICK_METRIC, Unit::Millis, 10.0);
        let (findings, _) = report(&capture, &baseline);
        assert_eq!(findings[0].verdict, Verdict::Incomparable);
    }

    #[test]
    fn an_unbaselined_metric_produces_no_finding() {
        let capture = capture_with("sim.something.new", Unit::Count, &[1.0]);
        let baseline = baseline_with(TICK_METRIC, Unit::Millis, 10.0);
        let (findings, _) = report(&capture, &baseline);
        // The baselined metric is missing from the capture, so it is
        // incomparable; the new instrument contributes nothing at all.
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].metric, TICK_METRIC);
    }

    #[test]
    fn the_sampler_records_one_tick_sample_per_tick() {
        let mut sampler = TickSampler::new();
        for _ in 0..3 {
            sampler.tick_begin();
            sampler.tick_end();
        }
        let capture = sampler.finish("test-scenario", profile());
        assert_eq!(capture.summaries[TICK_METRIC].summary.count, 3);
        assert_eq!(capture.summaries[RUN_METRIC].summary.count, 1);
    }
}
