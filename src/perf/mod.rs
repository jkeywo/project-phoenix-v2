//! Performance measurement (issue #868).
//!
//! The measurement *contract* — series, summaries, provenance, baselines,
//! comparison, rendering — is `vellum-perf`, and phoenix is its second
//! consumer. What lives here is the part the crate excludes by charter: the
//! collectors, and where the baseline files live.
//!
//! Three collectors, one contract:
//!
//! - [`tick`] — the headless harness loop, native.
//! - [`assets`] — the shipped asset inventory, native, no run required.
//! - [`browser`] — boot, preload and frame timing in the browser host, wasm.
//!
//! Collection stays out of the simulation. Every collector here samples from
//! outside authoritative state, so a measured run and an unmeasured run
//! produce the same simulation.
//!
//! Values are benchmark evidence, not assertions. Nothing in the test suite
//! asserts on a duration; the tests cover the pure machinery around them.

#[cfg(not(target_arch = "wasm32"))]
pub mod assets;
#[cfg(target_arch = "wasm32")]
pub mod browser;
#[cfg(not(target_arch = "wasm32"))]
pub mod tick;

use vellum_perf::Profile;

#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;
#[cfg(not(target_arch = "wasm32"))]
use vellum_perf::{Baseline, Capture, Finding};

/// Where committed baselines live. One file per scenario, RON, reviewable in
/// a diff — the crate owns the types, this repository owns the layout.
pub const BASELINE_DIR: &str = "perf/baselines";

/// Build provenance for a capture, so two captures are only compared when
/// they are comparable.
///
/// `device` and `rev` come from the environment because only the environment
/// knows them: CI sets `GITHUB_SHA` and identifies its runner image, and a
/// developer's desktop is not a runner. An unset value stays empty rather
/// than guessing — the contract is that provenance is honest, not complete.
#[cfg(not(target_arch = "wasm32"))]
pub fn profile(runtime: &str) -> Profile {
    Profile {
        runtime: runtime.to_string(),
        build: build_flavour().to_string(),
        device: std::env::var("PHOENIX_PERF_DEVICE")
            .or_else(|_| std::env::var("RUNNER_OS"))
            .unwrap_or_default(),
        rev: std::env::var("GITHUB_SHA").unwrap_or_default(),
    }
}

/// The wasm build has no environment to read, so provenance the host knows
/// (the page's own build stamp, the runner) is supplied by the caller.
#[cfg(target_arch = "wasm32")]
pub fn profile(runtime: &str) -> Profile {
    Profile {
        runtime: runtime.to_string(),
        build: build_flavour().to_string(),
        device: String::new(),
        rev: String::new(),
    }
}

fn build_flavour() -> &'static str {
    if cfg!(debug_assertions) {
        "dev"
    } else {
        "release"
    }
}

/// The committed baseline path for one scenario.
#[cfg(not(target_arch = "wasm32"))]
pub fn baseline_path(scenario: &str) -> String {
    format!("{BASELINE_DIR}/{scenario}.ron")
}

/// Errors reading a baseline. A missing file is not one of them — see
/// [`load_baseline`].
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug)]
pub enum BaselineError {
    Io(String, std::io::Error),
    // Boxed: RON's spanned error carries its whole error enum plus a position,
    // and an unboxed variant makes every Ok result pay for the failure case.
    Parse(String, Box<ron::error::SpannedError>),
}

#[cfg(not(target_arch = "wasm32"))]
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
#[cfg(not(target_arch = "wasm32"))]
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
/// Returns the findings and their rendering, and leaves the decision about
/// exit codes to the caller — #868 is explicit that measurement informs
/// optimisation before it gates correctness.
#[cfg(not(target_arch = "wasm32"))]
pub fn report(capture: &Capture, baseline: &Baseline) -> (Vec<Finding>, String) {
    let findings = vellum_perf::compare(capture, baseline);
    let rendered = vellum_perf::render(&findings);
    (findings, rendered)
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use vellum_perf::{Expectation, Recorder, Statistic, Tolerance, Unit, Verdict};

    fn capture_with(metric: &str, unit: Unit, values: &[f64]) -> Capture {
        let mut recorder = Recorder::new();
        for value in values {
            recorder.sample(metric, unit.clone(), *value);
        }
        recorder.finish("test-scenario", profile("test"))
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

    /// Every committed baseline parses, and none is filed under a scenario
    /// name that disagrees with its own filename — a mismatch would compare a
    /// capture against expectations written for something else.
    #[test]
    fn every_committed_baseline_parses_and_is_self_consistent() {
        let dir = Path::new(BASELINE_DIR);
        let mut seen = 0;
        for entry in std::fs::read_dir(dir).expect("baseline directory exists") {
            let path = entry.expect("readable directory entry").path();
            if path.extension().and_then(|e| e.to_str()) != Some("ron") {
                continue;
            }
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .expect("baseline filename is utf-8")
                .to_string();
            let baseline = load_baseline(&path)
                .unwrap_or_else(|e| panic!("{e}"))
                .expect("the file exists, so it parses to Some");
            assert_eq!(
                baseline.scenario, stem,
                "baseline {stem:?} declares scenario {:?}",
                baseline.scenario
            );
            assert!(
                !baseline.expectations.is_empty(),
                "baseline {stem:?} expects nothing, so it can never report"
            );
            seen += 1;
        }
        assert!(seen > 0, "no baselines found under {BASELINE_DIR}");
    }

    #[test]
    fn a_metric_within_tolerance_passes() {
        let capture = capture_with("m", Unit::Millis, &[10.0, 10.0, 11.0]);
        let baseline = baseline_with("m", Unit::Millis, 10.0);
        let (findings, _) = report(&capture, &baseline);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].verdict, Verdict::Pass);
    }

    #[test]
    fn drift_beyond_tolerance_warns_rather_than_failing() {
        let capture = capture_with("m", Unit::Millis, &[14.0, 14.0, 14.0]);
        let baseline = baseline_with("m", Unit::Millis, 10.0);
        let (findings, _) = report(&capture, &baseline);
        assert_eq!(findings[0].verdict, Verdict::Warn);
    }

    #[test]
    fn a_unit_mismatch_is_incomparable_not_a_regression() {
        let capture = capture_with("m", Unit::Seconds, &[10.0]);
        let baseline = baseline_with("m", Unit::Millis, 10.0);
        let (findings, _) = report(&capture, &baseline);
        assert_eq!(findings[0].verdict, Verdict::Incomparable);
    }

    #[test]
    fn an_unbaselined_metric_produces_no_finding() {
        let capture = capture_with("something.new", Unit::Count, &[1.0]);
        let baseline = baseline_with("m", Unit::Millis, 10.0);
        let (findings, _) = report(&capture, &baseline);
        // The baselined metric is missing from the capture, so it is
        // incomparable; the new instrument contributes nothing at all.
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].metric, "m");
    }
}
