//! Recording a baseline from a capture (issue #905).
//!
//! #868 committed baselines written by hand on the machine that happened to
//! measure them, and #905 is the correction: **a baseline belongs on the
//! machine that compares against it.** A number recorded on a developer
//! desktop and compared on a GitHub runner is not a budget, it is a warning
//! generator.
//!
//! CI cannot commit, so the loop is deliberately two-sided:
//!
//! 1. The runner renders the baseline it *would* record from its own captures
//!    and uploads it in the `perf-capture` artifact. That file is the runner's
//!    own opinion, produced by the same code path that compares.
//! 2. A human downloads the artifact and runs `phoenix-perf adopt`, which
//!    rewrites `perf/baselines/*.ron` from it. The result lands in a diff
//!    someone reads, which is the whole reason baselines are committed files.
//!
//! ```text
//! gh run download <run-id> -n perf-capture -D target/perf-artifact
//! cargo run --release --features perf --bin phoenix-perf -- \
//!     adopt --artifact target/perf-artifact
//! git diff perf/baselines
//! ```
//!
//! **Adoption records the measurement; a human owns the judgement.** When a
//! baseline already exists, its statistic and tolerances are carried over
//! untouched and only `expected` moves. That split is what makes re-recording
//! safe to repeat: widening `browser.frame`'s tolerance because a
//! sub-millisecond p95 makes a ratio meaningless is a decision, and the next
//! adoption must not quietly undo it.
//!
//! An expectation the capture did not measure is **kept, not dropped**.
//! Deleting a budget is a decision too, and a capture that has lost a metric
//! is at least as likely to be a broken collector as a retired one — leaving
//! it in means the next report says `Incomparable` out loud instead of going
//! quiet.
//!
//! **Commentary belongs in the header, above [`PROVENANCE_MARKER`].** The RON
//! value is regenerated from the data on every adoption, so a comment written
//! *inside* it — next to the expectation it explains — is deleted the first
//! time a runner's numbers are recorded. Comments are not in RON's data model,
//! so there is nothing to carry them on; the header is where reasoning
//! survives, and every committed baseline keeps its reasoning there.
//! `the_committed_baselines_keep_their_reasoning_where_adoption_preserves_it`
//! holds that line.

use std::collections::BTreeMap;

use vellum_perf::{Baseline, Capture, Expectation, Profile, Statistic, Unit};

/// The first line of the generated provenance block. Everything from this line
/// to the end of the header is rewritten on each adoption; everything above it
/// is the human's prose and survives.
pub const PROVENANCE_MARKER: &str = "// ── Recorded from a capture ";

/// Which summary statistic a *brand-new* expectation reads.
///
/// Not a budget value — the numbers all come from the capture — but a default
/// worth stating: wall-clock metrics get `p95`, because a mean flatters a
/// stall and a max reports the one tick that loaded the assets; everything
/// else gets `max`, because a byte count or a triangle count has no noise to
/// filter and the largest is the one that hurts. An existing expectation's
/// statistic always wins over this.
fn default_statistic(unit: &Unit) -> Statistic {
    match unit {
        Unit::Millis | Unit::Seconds | Unit::PerSecond => Statistic::P95,
        Unit::Bytes | Unit::Count | Unit::Custom(_) => Statistic::Max,
    }
}

/// The baseline `capture` says this scenario should have.
pub fn adopt(capture: &Capture, existing: Option<&Baseline>) -> Baseline {
    let mut expectations: BTreeMap<String, Expectation> = BTreeMap::new();

    for (metric, measured) in &capture.summaries {
        let prior = existing.and_then(|b| b.expectations.get(metric));
        let statistic = prior
            .map(|e| e.statistic)
            .unwrap_or_else(|| default_statistic(&measured.unit));
        let tolerance = prior.map(|e| e.tolerance).unwrap_or_default();
        expectations.insert(
            metric.clone(),
            Expectation {
                unit: measured.unit.clone(),
                statistic,
                expected: statistic.read(&measured.summary),
                tolerance,
            },
        );
    }

    // Expectations the capture never measured, carried over verbatim.
    if let Some(existing) = existing {
        for (metric, expectation) in &existing.expectations {
            expectations
                .entry(metric.clone())
                .or_insert_with(|| expectation.clone());
        }
    }

    Baseline {
        scenario: capture.scenario.clone(),
        expectations,
    }
}

/// Metrics an existing baseline expects that the capture did not measure.
///
/// Returned so the caller can say so: a silently carried-over expectation is
/// the one way this can hide a broken collector.
pub fn unmeasured(capture: &Capture, existing: Option<&Baseline>) -> Vec<String> {
    let Some(existing) = existing else {
        return Vec::new();
    };
    existing
        .expectations
        .keys()
        .filter(|metric| !capture.summaries.contains_key(*metric))
        .cloned()
        .collect()
}

/// Render a baseline as the RON file that gets committed.
///
/// `existing_text` is the file being replaced, if there is one: its
/// hand-written header survives, its generated provenance block does not.
/// Comments are not part of RON's data model, so preserving them is textual by
/// necessity — the alternative is a tool that deletes the reasoning every time
/// it records a number.
pub fn render(baseline: &Baseline, profile: &Profile, existing_text: Option<&str>) -> String {
    let mut out = String::new();
    if let Some(prose) = existing_text.map(human_header) {
        let prose = trim_trailing_blank_comments(&prose);
        if !prose.is_empty() {
            out.push_str(prose);
            out.push_str("\n//\n");
        }
    }
    out.push_str(&provenance(baseline, profile));
    out.push_str(&body(baseline));
    out
}

/// Drop trailing blank and comment-only-blank (`//`) lines from kept prose.
///
/// The separator [`render`] writes between prose and the generated block is a
/// bare `//` line, which [`human_header`] then keeps as prose on the next
/// adoption. Without this trim, each re-record would push another blank
/// comment line in and adoption would stop being idempotent — every recording
/// a diff, whether or not a number moved.
fn trim_trailing_blank_comments(prose: &str) -> &str {
    let mut kept = prose;
    loop {
        kept = kept.trim_end_matches('\n');
        if kept.is_empty() {
            return "";
        }
        let last = kept.rsplit('\n').next().unwrap_or_default();
        if !last.trim().is_empty() && last.trim() != "//" {
            return kept;
        }
        kept = &kept[..kept.len() - last.len()];
    }
}

/// The hand-written part of an existing baseline's header: everything before
/// the generated block, and before the RON value itself.
fn human_header(text: &str) -> String {
    let mut kept = String::new();
    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with(PROVENANCE_MARKER) {
            break;
        }
        // The RON value begins at the first line that is not a comment or
        // blank; nothing after it is header.
        if !trimmed.is_empty() && !trimmed.starts_with("//") {
            break;
        }
        kept.push_str(line);
        kept.push('\n');
    }
    kept
}

fn provenance(baseline: &Baseline, profile: &Profile) -> String {
    let mut out = format!("{PROVENANCE_MARKER}─────────────────────────────────\n");
    out.push_str(&format!(
        "// Written by `phoenix-perf adopt` from a {} capture of scenario {:?}.\n",
        if profile.runtime.is_empty() {
            "(unrecorded runtime)"
        } else {
            &profile.runtime
        },
        baseline.scenario,
    ));
    out.push_str(&format!(
        "// build: {}   device: {}\n",
        blank_as_unrecorded(&profile.build),
        blank_as_unrecorded(&profile.device),
    ));
    out.push_str(&format!(
        "// rev:   {}\n",
        blank_as_unrecorded(&profile.rev)
    ));
    out.push_str(
        "//\n// Edit the prose ABOVE this block; re-recording rewrites from here down, and\n\
         // carries every statistic and tolerance over unchanged. The numbers are the\n\
         // capture's; the tolerances are yours.\n",
    );
    out
}

fn blank_as_unrecorded(value: &str) -> &str {
    if value.is_empty() {
        "(unrecorded)"
    } else {
        value
    }
}

/// The RON value itself.
///
/// The line ending is pinned to `\n` rather than left to RON's
/// platform default: a baseline is a committed file compared across a Windows
/// desktop and a Linux runner, and a recording that swapped every line ending
/// would put the whole file in the diff without moving a single number.
fn body(baseline: &Baseline) -> String {
    let config = ron::ser::PrettyConfig::new()
        .struct_names(false)
        .new_line("\n");
    let mut text =
        ron::ser::to_string_pretty(baseline, config).expect("a baseline serialises to RON");
    text.push('\n');
    text
}

#[cfg(test)]
mod tests {
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
        for entry in std::fs::read_dir(dir).expect("baseline directory exists") {
            let path = entry.expect("readable directory entry").path();
            if path.extension().and_then(|e| e.to_str()) != Some("ron") {
                continue;
            }
            let text = std::fs::read_to_string(&path).expect("a committed baseline is readable");
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
        for entry in std::fs::read_dir(dir).expect("baseline directory exists") {
            let path = entry.expect("readable directory entry").path();
            if path.extension().and_then(|e| e.to_str()) != Some("ron") {
                continue;
            }
            let text = std::fs::read_to_string(&path).expect("a committed baseline is readable");
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
}
