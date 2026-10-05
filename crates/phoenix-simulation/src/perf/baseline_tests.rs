use super::*;
use vellum_perf::{MetricSummary, Recorder, Tolerance};

fn capture_of(scenario: &str, samples: &[(&str, Unit, &[f64])]) -> Capture {
    let mut recorder = Recorder::new();
    for (metric, unit, values) in samples {
        for value in *values {
            recorder.sample(metric, unit.clone(), *value);
        }
    }
    recorder.finish(scenario, crate::perf::profile("test-runtime"))
}

fn baseline_of(scenario: &str, expectations: &[(&str, Expectation)]) -> Baseline {
    Baseline {
        scenario: scenario.to_string(),
        expectations: expectations
            .iter()
            .map(|(m, e)| (m.to_string(), e.clone()))
            .collect(),
    }
}

#[test]
fn a_new_baseline_takes_every_metric_the_capture_measured() {
    let capture = capture_of(
        "s",
        &[
            ("a.count", Unit::Count, &[1.0, 5.0]),
            ("a.time", Unit::Millis, &[10.0]),
        ],
    );
    let baseline = adopt(&capture, None);
    assert_eq!(baseline.scenario, "s");
    assert_eq!(baseline.expectations.len(), 2);
    // Counts read `max`, durations read `p95`.
    assert_eq!(baseline.expectations["a.count"].statistic, Statistic::Max);
    assert_eq!(baseline.expectations["a.count"].expected, 5.0);
    assert_eq!(baseline.expectations["a.time"].statistic, Statistic::P95);
    assert_eq!(baseline.expectations["a.time"].expected, 10.0);
}

/// The point of the whole module: re-recording moves the number and leaves
/// the human's judgement about it alone.
#[test]
fn re_recording_moves_the_number_and_keeps_the_judgement() {
    let existing = baseline_of(
        "s",
        &[(
            "m",
            Expectation {
                unit: Unit::Millis,
                statistic: Statistic::Mean,
                expected: 0.78,
                tolerance: Tolerance {
                    warn: 0.5,
                    fail: 2.0,
                },
            },
        )],
    );
    let capture = capture_of("s", &[("m", Unit::Millis, &[4.0, 6.0])]);

    let adopted = adopt(&capture, Some(&existing));
    let e = &adopted.expectations["m"];
    assert_eq!(e.expected, 5.0, "the capture's mean, not its p95");
    assert_eq!(e.statistic, Statistic::Mean);
    assert_eq!(e.tolerance.warn, 0.5);
    assert_eq!(e.tolerance.fail, 2.0);
}

#[test]
fn an_expectation_the_capture_did_not_measure_is_kept_and_reported() {
    let existing = baseline_of(
        "s",
        &[(
            "gone",
            Expectation {
                unit: Unit::Count,
                statistic: Statistic::Max,
                expected: 3.0,
                tolerance: Tolerance::default(),
            },
        )],
    );
    let capture = capture_of("s", &[("here", Unit::Count, &[1.0])]);

    let adopted = adopt(&capture, Some(&existing));
    assert_eq!(adopted.expectations["gone"].expected, 3.0);
    assert!(adopted.expectations.contains_key("here"));
    assert_eq!(unmeasured(&capture, Some(&existing)), vec!["gone"]);
}

#[test]
fn nothing_is_unmeasured_when_there_was_no_baseline() {
    let capture = capture_of("s", &[("m", Unit::Count, &[1.0])]);
    assert!(unmeasured(&capture, None).is_empty());
}

/// The round trip that matters: what adoption writes is what the reader
/// loads back, byte-for-byte in meaning.
#[test]
fn a_rendered_baseline_parses_back_to_itself() {
    let capture = capture_of(
        "round-trip",
        &[
            ("a.bytes", Unit::Bytes, &[10.0, 400.0]),
            ("a.time", Unit::Millis, &[1.0, 2.0, 3.0]),
            ("a.custom", Unit::Custom("widgets".into()), &[7.0]),
        ],
    );
    let baseline = adopt(&capture, None);
    let text = render(&baseline, &capture.profile, None);

    let parsed: Baseline = ron::from_str(&text).expect("the rendered file parses");
    assert_eq!(parsed, baseline);
}

#[test]
fn the_provenance_block_records_where_the_numbers_came_from() {
    let capture = capture_of("s", &[("m", Unit::Count, &[1.0])]);
    let text = render(&adopt(&capture, None), &capture.profile, None);
    assert!(text.contains(PROVENANCE_MARKER));
    assert!(text.contains("test-runtime"));
}

/// Prose is why baselines are committed rather than generated, so it
/// survives the tool that regenerates them.
#[test]
fn hand_written_prose_survives_a_re_record() {
    let previous = "// Why these numbers are what they are.\n\
                        // A second line of reasoning.\n\
                        (\n    scenario: \"s\",\n    expectations: {},\n)\n";
    let capture = capture_of("s", &[("m", Unit::Count, &[2.0])]);
    let text = render(&adopt(&capture, None), &capture.profile, Some(previous));

    assert!(text.starts_with("// Why these numbers are what they are."));
    assert!(text.contains("A second line of reasoning."));
    assert!(ron::from_str::<Baseline>(&text).is_ok());
}

/// Re-recording twice must not stack provenance blocks on top of each
/// other — the generated part is replaced, not appended to.
#[test]
fn re_recording_replaces_the_generated_block_rather_than_stacking_it() {
    let capture = capture_of("s", &[("m", Unit::Count, &[2.0])]);
    let baseline = adopt(&capture, None);
    let once = render(&baseline, &capture.profile, Some("// Prose.\n(\n)\n"));
    let twice = render(&baseline, &capture.profile, Some(&once));

    assert_eq!(twice.matches(PROVENANCE_MARKER).count(), 1);
    assert_eq!(once, twice, "adoption is idempotent");
}

/// A committed baseline is compared across a Windows desktop and a Linux
/// runner, so a recording must not decide the line ending from the machine
/// that happened to make it.
#[test]
fn a_recording_uses_the_same_line_ending_on_every_platform() {
    let capture = capture_of("s", &[("m", Unit::Count, &[1.0])]);
    let text = render(&adopt(&capture, None), &capture.profile, None);
    assert!(
        !text.contains('\r'),
        "a rendered baseline carries a carriage return: {text:?}"
    );
}

/// The committed baselines must survive the documented adoption command,
/// which means their reasoning lives in the header rather than inside the
/// RON value — see the module documentation.
#[test]
fn the_committed_baselines_keep_their_reasoning_where_adoption_preserves_it() {
    let dir = std::path::Path::new(crate::perf::BASELINE_DIR);
    let mut seen = 0;
    for entry in crate::repo_fixtures::fs::read_dir(dir).expect("baseline directory exists") {
        let path = entry.expect("readable directory entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("ron") {
            continue;
        }
        let text = crate::repo_fixtures::fs::read_to_string(&path)
            .expect("a committed baseline is readable");
        // The RON value begins at the first line that is neither blank nor
        // a comment; every comment from there on is inside the generated
        // body, and adoption regenerates that from the data.
        let body_starts = text
            .lines()
            .position(|line| {
                let line = line.trim_start();
                !line.is_empty() && !line.starts_with("//")
            })
            .expect("a committed baseline carries a RON value");
        let lost: Vec<&str> = text
            .lines()
            .skip(body_starts)
            .filter(|line| line.trim_start().starts_with("//"))
            .collect();
        assert!(
            lost.is_empty(),
            "{}: comment(s) inside the RON value would be deleted by \
                 `phoenix-perf adopt`; move them into the header above \
                 {PROVENANCE_MARKER:?}:\n{}",
            path.display(),
            lost.join("\n")
        );
        seen += 1;
    }
    assert!(seen > 0, "no baselines found under {dir:?}");
}

/// The round trip the workflow depends on, against the real committed
/// files: re-recording a baseline from a capture that measured exactly
/// what it already expects gives back the same baseline, and gives it back
/// the same way twice.
///
/// This is what makes `git diff perf/baselines` after an adoption
/// readable. If it were false, every recording would show movement
/// whether or not a number moved, and the diff would stop being evidence.
#[test]
fn adopting_a_committed_baselines_own_numbers_changes_nothing() {
    let dir = std::path::Path::new(crate::perf::BASELINE_DIR);
    let mut seen = 0;
    for entry in crate::repo_fixtures::fs::read_dir(dir).expect("baseline directory exists") {
        let path = entry.expect("readable directory entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("ron") {
            continue;
        }
        let text = crate::repo_fixtures::fs::read_to_string(&path)
            .expect("a committed baseline is readable");
        let committed: Baseline = ron::from_str(&text).expect("a committed baseline parses");

        // A capture that measured precisely what the file expects. One
        // sample per metric is enough: every statistic of a single sample
        // is that sample.
        let mut recorder = Recorder::new();
        for (metric, expectation) in &committed.expectations {
            recorder.sample(metric, expectation.unit.clone(), expectation.expected);
        }
        let capture = recorder.finish(&committed.scenario, crate::perf::profile("round-trip"));

        let once = render(
            &adopt(&capture, Some(&committed)),
            &capture.profile,
            Some(&text),
        );
        let parsed: Baseline = ron::from_str(&once).expect("the re-recording parses");
        assert_eq!(
            parsed,
            committed,
            "{}: re-recording its own numbers changed the baseline",
            path.display()
        );

        let twice = render(
            &adopt(&capture, Some(&parsed)),
            &capture.profile,
            Some(&once),
        );
        assert_eq!(
            once,
            twice,
            "{}: adoption is not idempotent",
            path.display()
        );

        let prose = human_header(&text);
        for line in prose
            .lines()
            .filter(|line| !line.trim().is_empty() && line.trim() != "//")
        {
            assert!(
                once.contains(line),
                "{}: re-recording dropped a line of the hand-written header:\n{line}",
                path.display()
            );
        }
        seen += 1;
    }
    assert!(seen > 0, "no baselines found under {dir:?}");
}

/// A baseline with no prior file still renders a legal header.
#[test]
fn a_first_recording_needs_no_previous_file() {
    let capture = Capture {
        scenario: "fresh".into(),
        profile: Profile::default(),
        series: BTreeMap::new(),
        summaries: BTreeMap::from([(
            "m".to_string(),
            MetricSummary {
                unit: Unit::Count,
                summary: vellum_perf::summarize(&[9.0]),
            },
        )]),
    };
    let text = render(&adopt(&capture, None), &capture.profile, None);
    let parsed: Baseline = ron::from_str(&text).expect("the rendered file parses");
    assert_eq!(parsed.scenario, "fresh");
    assert_eq!(parsed.expectations["m"].expected, 9.0);
    assert!(text.contains("(unrecorded"));
}
