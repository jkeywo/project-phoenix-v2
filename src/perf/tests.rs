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
