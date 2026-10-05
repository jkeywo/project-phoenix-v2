//! Recording a baseline from a capture (issue #905).
//!
//! #868 committed baselines written by hand on the machine that happened to
//! measure them, and #905 is the correction: **a baseline belongs on the
//! machine that compares against it.** A number recorded on a developer
//! desktop and compared on a GitHub runner is not a budget, it is a warning
//! generator.
//!
//! There are two ways a baseline is recorded, and the split is deliberate:
//!
//! 1. **A missing baseline is bootstrapped by CI.** A scenario with no
//!    committed file yet has the runner record one from its own capture and
//!    commit it on a push to `main` (with `[skip ci]`). The file is then the
//!    runner's own opinion, produced by the same code path that compares.
//! 2. **An existing baseline moves only by a deliberate `adopt`.** Moving a
//!    budget is a decision, so CI never rewrites an existing file — a human
//!    adopts from the `perf-capture` artifact, and the result lands in a diff
//!    someone reads (a capture over an unchanged baseline is instead filed as a
//!    GitHub issue to fix later; see `.github/workflows/ci.yml`).
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
#[path = "baseline_tests.rs"]
mod tests;
