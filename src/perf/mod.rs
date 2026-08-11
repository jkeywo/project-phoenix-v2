//! Performance measurement (issue #868).
//!
//! The measurement *contract* — series, summaries, provenance, baselines,
//! comparison, rendering — is `vellum-perf`, and phoenix is its second
//! consumer. What lives here is the part the crate excludes by charter: the
//! collectors, and where the baseline files live.
//!
//! Four collectors, one contract:
//!
//! - [`tick`] — the headless harness loop, native.
//! - [`assets`] — the shipped asset inventory, native, no run required.
//! - [`mesh`] — the mesh interior read through Bevy's own loader, native.
//! - [`browser`] — boot, preload and frame timing in the browser host, wasm.
//!
//! [`baseline`] is the fifth piece and not a collector: recording a baseline
//! *from* a capture, so the numbers a runner is held to are the numbers that
//! runner produced.
//!
//! Collection stays out of the simulation. Every collector here samples from
//! outside authoritative state, so a measured run and an unmeasured run
//! produce the same simulation.
//!
//! Values are benchmark evidence, not assertions. Nothing in the test suite
//! asserts on a duration; the tests cover the pure machinery around them.
//!
//! # Recording a baseline (issue #905, revised for the move-fast demo phase)
//!
//! Baselines are recorded on the machine that compares against them, and the CI
//! runner commits them: a scenario with no committed baseline yet is
//! bootstrapped by the perf job, which records the missing file from its own
//! capture and commits it on a push to `main` (with `[skip ci]`, so the
//! baseline commit does not start another run). An *existing* baseline is never
//! moved by CI — a baseline that changes is a budget decision, so it still
//! moves only through a deliberate `adopt` that lands in a reviewable diff:
//!
//! ```text
//! gh run download <run-id> -n perf-capture -D target/perf-artifact
//! cargo run --release --features perf --bin phoenix-perf -- \
//!     adopt --artifact target/perf-artifact
//! git diff perf/baselines
//! ```
//!
//! Adoption records the measurement and leaves the judgement alone: an
//! existing expectation keeps its statistic and tolerances and only its
//! `expected` moves. See [`baseline`].
//!
//! # When measurement gates (issue #905, revised for the demo phase)
//!
//! Whether a scenario's [`vellum_perf::Verdict::Fail`] *can* become a build
//! gate honestly still turns on both of these being true of it:
//!
//! 1. **Its metrics are a function of the checkout, not of the host.** Bytes on
//!    disk, LOD ladder depth, triangle counts and texture dimensions are the
//!    same on every machine that reads the same commit, so a drift is a real
//!    change to what a player downloads or what a GPU is handed. Wall-clock
//!    metrics are not: a shared runner's neighbours move them.
//! 2. **Its baseline was recorded by a machine whose measurement the comparing
//!    runner reproduces.** For a metric that passes (1) that is every machine,
//!    and the runner's own captures are the proof. For a wall-clock metric it
//!    is only a machine of the same class the baseline was recorded on.
//!
//! But *where* gating happens has moved. During the move-fast demo phase:
//!
//! - **The regular CI perf job (`.github/workflows/ci.yml`) gates on nothing.**
//!   Every scenario is warnings-first there; adverse drift from a baseline is
//!   filed as a GitHub issue to fix later, never a red build and never a silent
//!   re-record. The job stays out of `deploy`'s `needs`, as before.
//! - **The manual demo deploy (`.github/workflows/deploy-demo.yml`) is the
//!   gate.** It runs `phoenix-perf report --gate` on all four scenarios —
//!   including the wall-clock ones — and a fail blocks the demo deploy before
//!   it ships. A demo deploy is human-dispatched, so a wall-clock gate that
//!   fires on runner noise costs a re-dispatch rather than a disabled gate;
//!   that is the trade the demo accepts to ship a build whose budgets were
//!   actually checked. Its browser and headless captures are taken the same way
//!   CI takes them (native release binary; the smoke perf spec under WebDriver)
//!   so the comparison against the runner-recorded baseline stays valid.
//!
//! `Verdict::Incomparable` gates wherever `Fail` does — a metric that vanished
//! from a capture is a broken contract, and passing it is how a budget stops
//! being enforced without anyone deciding to stop enforcing it. `Warn` never
//! gates; the whole tolerance design assumes a warning is read — now, filed —
//! rather than obeyed.
//!
//! The mechanism is `phoenix-perf report --gate`, which is off unless asked
//! for; `deploy-demo.yml` is the workflow that asks.

#[cfg(not(target_arch = "wasm32"))]
pub mod assets;
#[cfg(not(target_arch = "wasm32"))]
pub mod baseline;
#[cfg(target_arch = "wasm32")]
pub mod browser;
#[cfg(not(target_arch = "wasm32"))]
pub mod mesh;
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

/// Which side of an expectation is adverse for a metric.
///
/// `vellum-perf` intentionally compares by absolute drift. Phoenix adds this
/// policy at its boundary because a smaller download or faster frame is an
/// improvement, while a deeper LOD ladder is improved by growing. Unknown
/// metrics retain vellum's symmetric comparison until their meaning is made
/// explicit here.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MetricDirection {
    LowerIsBetter,
    HigherIsBetter,
    Symmetric,
}

#[cfg(not(target_arch = "wasm32"))]
fn metric_direction(metric: &str) -> MetricDirection {
    match metric {
        "sim.tick"
        | "sim.run"
        | "browser.boot"
        | "browser.preload"
        | "browser.frame"
        | "assets.glb.bytes"
        | "assets.glb.total.bytes"
        | "assets.glb.without_lod"
        | "assets.mesh.triangles"
        | "assets.mesh.triangles.total"
        | "assets.texture.count"
        | "assets.texture.pixels" => MetricDirection::LowerIsBetter,
        "assets.lod.levels" => MetricDirection::HigherIsBetter,
        _ => MetricDirection::Symmetric,
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn normalize_direction(findings: &mut [Finding]) {
    for finding in findings {
        // Incomparability is a broken measurement contract, not drift, and
        // must never be normalized into a pass.
        if finding.verdict == vellum_perf::Verdict::Incomparable {
            continue;
        }
        let favourable = match metric_direction(&finding.metric) {
            MetricDirection::LowerIsBetter => finding.got <= finding.expected,
            MetricDirection::HigherIsBetter => finding.got >= finding.expected,
            MetricDirection::Symmetric => false,
        };
        if favourable {
            finding.verdict = vellum_perf::Verdict::Pass;
        }
    }
}

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
/// optimisation before it gates correctness. The module documentation above
/// records which scenarios have since earned a gate, and why the rest have
/// not.
#[cfg(not(target_arch = "wasm32"))]
pub fn report(capture: &Capture, baseline: &Baseline) -> (Vec<Finding>, String) {
    let mut findings = vellum_perf::compare(capture, baseline);
    normalize_direction(&mut findings);
    let rendered = vellum_perf::render(&findings);
    (findings, rendered)
}

/// Whether these findings should fail a build, *for a caller that asked to
/// gate*. Nothing here decides to ask.
///
/// `Incomparable` gates alongside `Fail`: a baselined metric missing from the
/// capture, or arriving in the wrong unit, means the budget was not checked at
/// all — and letting that through is how a gate stops gating without anyone
/// deciding to stop. `Warn` never gates; the whole tolerance design assumes a
/// warning is read rather than obeyed.
#[cfg(not(target_arch = "wasm32"))]
pub fn gates(findings: &[Finding]) -> bool {
    matches!(
        vellum_perf::worst(findings),
        vellum_perf::Verdict::Fail | vellum_perf::Verdict::Incomparable
    )
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
    fn lower_is_better_reduction_passes_even_beyond_fail_tolerance() {
        let capture = capture_with("assets.glb.total.bytes", Unit::Bytes, &[1.0]);
        let baseline = baseline_with("assets.glb.total.bytes", Unit::Bytes, 10.0);
        let (findings, _) = report(&capture, &baseline);
        assert_eq!(findings[0].verdict, Verdict::Pass);
        assert!(!gates(&findings));
    }

    #[test]
    fn lower_is_better_increase_still_fails() {
        let capture = capture_with("assets.glb.total.bytes", Unit::Bytes, &[30.0]);
        let baseline = baseline_with("assets.glb.total.bytes", Unit::Bytes, 10.0);
        let (findings, _) = report(&capture, &baseline);
        assert_eq!(findings[0].verdict, Verdict::Fail);
        assert!(gates(&findings));
    }

    #[test]
    fn deeper_lod_ladder_passes_but_shallower_ladder_fails() {
        let deeper = capture_with("assets.lod.levels", Unit::Count, &[30.0]);
        let shallower = capture_with("assets.lod.levels", Unit::Count, &[1.0]);
        let mut baseline = baseline_with("assets.lod.levels", Unit::Count, 10.0);
        baseline
            .expectations
            .get_mut("assets.lod.levels")
            .expect("the fixture inserts this expectation")
            .tolerance
            .fail = 0.5;

        let (deeper_findings, _) = report(&deeper, &baseline);
        let (shallower_findings, _) = report(&shallower, &baseline);
        assert_eq!(deeper_findings[0].verdict, Verdict::Pass);
        assert_eq!(shallower_findings[0].verdict, Verdict::Fail);
    }

    #[test]
    fn incomparable_known_metric_remains_incomparable() {
        let capture = capture_with("assets.glb.total.bytes", Unit::Count, &[1.0]);
        let baseline = baseline_with("assets.glb.total.bytes", Unit::Bytes, 10.0);
        let (findings, _) = report(&capture, &baseline);
        assert_eq!(findings[0].verdict, Verdict::Incomparable);
        assert!(gates(&findings));
    }

    #[test]
    fn unknown_metrics_keep_symmetric_comparison() {
        let capture = capture_with("unknown.metric", Unit::Count, &[1.0]);
        let mut baseline = baseline_with("unknown.metric", Unit::Count, 10.0);
        baseline
            .expectations
            .get_mut("unknown.metric")
            .expect("the fixture inserts this expectation")
            .tolerance
            .fail = 0.5;
        let (findings, _) = report(&capture, &baseline);
        assert_eq!(findings[0].verdict, Verdict::Fail);
        assert!(gates(&findings));
    }

    /// Adding a metric to a committed baseline is a budget decision. Force the
    /// same change to choose its direction rather than silently inheriting a
    /// symmetric policy that reports improvements as adverse drift.
    #[test]
    fn every_committed_metric_has_an_explicit_direction() {
        for entry in std::fs::read_dir(BASELINE_DIR).expect("baseline directory exists") {
            let path = entry.expect("readable directory entry").path();
            if path.extension().and_then(|e| e.to_str()) != Some("ron") {
                continue;
            }
            let baseline = load_baseline(&path)
                .unwrap_or_else(|e| panic!("{e}"))
                .expect("the baseline exists");
            for metric in baseline.expectations.keys() {
                assert_ne!(
                    metric_direction(metric),
                    MetricDirection::Symmetric,
                    "committed metric {metric:?} in {} has no direction policy",
                    path.display()
                );
            }
        }
    }

    /// The gating rule from the module documentation, as code: drift that only
    /// warns must never fail a build, however far out it is.
    #[test]
    fn a_warning_never_gates_however_loud() {
        let capture = capture_with("m", Unit::Millis, &[19.0]);
        let baseline = baseline_with("m", Unit::Millis, 10.0);
        let (findings, _) = report(&capture, &baseline);
        assert_eq!(findings[0].verdict, Verdict::Warn);
        assert!(!gates(&findings));
    }

    #[test]
    fn drift_past_the_fail_tolerance_gates() {
        let capture = capture_with("m", Unit::Millis, &[30.0]);
        let baseline = baseline_with("m", Unit::Millis, 10.0);
        let (findings, _) = report(&capture, &baseline);
        assert_eq!(findings[0].verdict, Verdict::Fail);
        assert!(gates(&findings));
    }

    /// A budget that could not be checked is not a budget that passed.
    #[test]
    fn a_metric_the_capture_never_measured_gates() {
        let capture = capture_with("something.else", Unit::Millis, &[1.0]);
        let baseline = baseline_with("m", Unit::Millis, 10.0);
        let (findings, _) = report(&capture, &baseline);
        assert_eq!(findings[0].verdict, Verdict::Incomparable);
        assert!(gates(&findings));
    }

    #[test]
    fn nothing_to_report_gates_nothing() {
        assert!(!gates(&[]));
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
