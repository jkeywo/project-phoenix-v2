//! The structured post-mission report (issue #1344, PRD #1337).
//!
//! [`crate::core::narrative`] answers "what happened in the story", one beat at
//! a time. This module answers the question a crew asks *after* the story ends:
//! **what did we come away with** — one row per thing the mission was about, in
//! the order the author put them in, each carrying the fate it ended in.
//!
//! It exists because the alternative is a single word. Before this, an ending
//! was `victory` or `defeat`, and a mission that saved the stricken hauler but
//! let the traffic scatter had to pick one of the two. That is not a report; it
//! is a verdict with the reasoning thrown away.
//!
//! # What a row is, and what it deliberately is not
//!
//! A [`ReportRow`] carries five things and nothing else:
//!
//! * a stable authored `id`, so re-writing a row as the mission moves UPDATES
//!   it rather than appending a second one;
//! * a `heading_id` and an `outcome_id`, both `strings.csv` ids — the report is
//!   localized at the surface that renders it, never composed as English in
//!   Rust or in JavaScript (AGENTS.md rule 11);
//! * a [`ReportRowState`], the semantic fate a player surface styles on; and
//! * a signed `score`, which players NEVER see.
//!
//! The score is the whole reason the split exists. Design wants a diagnostic
//! number — did this row pull the run up or down, and by how much — and players
//! want an account of what happened to the people they were sent to help.
//! Showing a crew "Lyra Ascending: saved (+6)" turns the rescue into a coupon.
//! So the score rides the accumulator, folds into the headless total, and is
//! filtered out at the wire: [`MissionReport::to_json`] carries it,
//! `on_game_over_enter`'s wire projection does not.
//!
//! # Absence is a real answer
//!
//! A row that was never written is OMITTED, not written as "unresolved". A
//! Falling Skyway run that ends catastrophically before Lyra's fate is decided
//! reports no Lyra row at all — the mission never reached the point where the
//! question had an answer, and inventing one would be the report lying about
//! what the crew did. A report with NO rows is not a report: the ending falls
//! back to its declared victory/defeat framing, which is what keeps every other
//! scenario's ending exactly as it was until it opts in.
//!
//! # Determinism
//!
//! [`MissionReport`] is authoritative state, not presentation. A scenario
//! script writes it, so two instances on the same seed must hold the same rows
//! in the same order: `src/sim_digest.rs` folds it and `src/snapshot.rs`
//! captures and restores it. Rows live in a `Vec` in authored write order —
//! never a map — so the fold walks them in the order the mission produced them
//! rather than in a hash order.

use bevy::prelude::*;

/// The semantic fate one report row ended in.
///
/// A small CLOSED set, parsed at the script boundary
/// ([`Self::parse`]) so a typo raises there rather than reaching a player
/// surface as an unstyled row. It is the axis a surface may style on — the
/// row's *valence* — and deliberately not a score: the number is diagnostic and
/// the state is the story.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ReportRowState {
    /// The thing the row is about came through. Lyra pulled clear.
    Saved,
    /// It did not. Lyra was left in the lane when the band closed.
    Lost,
    /// Some of it came through and some did not — the shape a row about a
    /// GROUP wears (three claimants, two lifted). A row about one hull never
    /// reaches this state, which is why it is not simply "not saved".
    Partial,
    /// It resolved without a valence: the question was answered, and the answer
    /// was neither a win nor a loss. Kept out of `Partial` because a surface
    /// that colours partial as a near-miss would be wrong about this one.
    Neutral,
}

impl ReportRowState {
    /// Every state, in declaration order (also `Ord`'s order).
    pub const ALL: [ReportRowState; 4] = [
        ReportRowState::Saved,
        ReportRowState::Lost,
        ReportRowState::Partial,
        ReportRowState::Neutral,
    ];

    /// The stable snake_case label written to the wire and to the report JSON.
    /// Hand-written, not derived, so the vocabulary is visible where it is
    /// promised — the same convention
    /// [`crate::core::narrative::NarrativeKind::as_str`] follows.
    pub fn as_str(self) -> &'static str {
        match self {
            ReportRowState::Saved => "saved",
            ReportRowState::Lost => "lost",
            ReportRowState::Partial => "partial",
            ReportRowState::Neutral => "neutral",
        }
    }

    /// Parse the author-facing word on `ctx.effects.report_row(#{ state: … })`.
    ///
    /// `Err` on anything else, so a misspelled state raises at the script
    /// boundary — discarding the call under the shared failure policy — rather
    /// than recording a row nothing can style.
    pub fn parse(s: &str) -> Result<Self, String> {
        match s.trim().to_ascii_lowercase().as_str() {
            "saved" => Ok(ReportRowState::Saved),
            "lost" => Ok(ReportRowState::Lost),
            "partial" => Ok(ReportRowState::Partial),
            "neutral" => Ok(ReportRowState::Neutral),
            other => Err(format!(
                "unknown report row state '{other}' (expected one of: saved, \
                 lost, partial, neutral)"
            )),
        }
    }
}

/// One row of the structured post-mission report.
///
/// See the module docs for why the score is here and not on the wire.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReportRow {
    /// The author's own stable identifier for this row. Writing the same id
    /// twice updates the row in place and keeps its original position, so a row
    /// can be re-stated as the mission moves without the report growing.
    pub id: String,
    /// `strings.csv` id for what the row is ABOUT ("Lyra Ascending").
    pub heading_id: String,
    /// `strings.csv` id for how it ENDED ("Pulled clear of the lane").
    pub outcome_id: String,
    /// The semantic fate a player surface styles on.
    pub state: ReportRowState,
    /// The hidden signed diagnostic score. Never leaves the host: headless
    /// output and the digest carry it, the wire does not.
    pub score: i32,
}

impl ReportRow {
    /// Encode as a JSON object, score included — the HEADLESS shape. Hand-rolled
    /// like every sibling in [`crate::core::balance`]: `serde_json` is confined
    /// to `codec.rs`.
    pub fn to_json(&self) -> String {
        format!(
            "{{\"id\": {:?}, \"heading\": {:?}, \"outcome\": {:?}, \"state\": {:?}, \"score\": {}}}",
            self.id,
            self.heading_id,
            self.outcome_id,
            self.state.as_str(),
            self.score,
        )
    }
}

/// The report a run accumulates, in authored row order.
///
/// Written only through [`Self::set_row`], from the one chokepoint that owns
/// the script boundary's report queue, so "the rows are in write order" is a
/// property of the type rather than of every caller remembering it.
#[derive(Resource, Clone, Debug, Default, PartialEq, Eq)]
pub struct MissionReport {
    /// Authored write order. A `Vec` and not a map: the order IS the report,
    /// and it is also what the digest fold walks.
    rows: Vec<ReportRow>,
}

impl MissionReport {
    /// Build from already-ordered rows — the snapshot restore path.
    pub fn from_rows(rows: Vec<ReportRow>) -> Self {
        Self { rows }
    }

    /// Write `row`, replacing any row with the same id IN PLACE.
    ///
    /// Returns whether anything actually changed, so the caller can emit a
    /// [`crate::core::narrative::NarrativeKind::ReportRowUpdated`] beat only
    /// when the row moved. A scenario that re-states an unchanged row every
    /// tick is a rate, and the timeline should not carry it.
    pub fn set_row(&mut self, row: ReportRow) -> bool {
        match self.rows.iter_mut().find(|existing| existing.id == row.id) {
            Some(existing) => {
                if *existing == row {
                    return false;
                }
                *existing = row;
                true
            }
            None => {
                self.rows.push(row);
                true
            }
        }
    }

    /// The rows, in authored order.
    pub fn rows(&self) -> &[ReportRow] {
        &self.rows
    }

    /// Whether the scenario authored no report at all. An empty report is NOT a
    /// report — see the module docs — and every caller that decides whether an
    /// ending is report-bearing asks this.
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// The hidden total: the sum of every VISIBLE row's score. Saturating
    /// rather than wrapping, because a report that overflowed into a negative
    /// total would be a diagnostic that lies.
    pub fn total(&self) -> i32 {
        self.rows
            .iter()
            .fold(0i32, |acc, row| acc.saturating_add(row.score))
    }

    /// Encode as the run report's `report` object — rows (with scores) and the
    /// total. The headless shape; see the module docs for why the wire's is
    /// narrower.
    pub fn to_json(&self) -> String {
        let rows = self
            .rows
            .iter()
            .map(ReportRow::to_json)
            .collect::<Vec<_>>()
            .join(", ");
        format!("{{\"rows\": [{}], \"total\": {}}}", rows, self.total())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str, state: ReportRowState, score: i32) -> ReportRow {
        ReportRow {
            id: id.to_string(),
            heading_id: format!("world.probe.report.{id}.heading"),
            outcome_id: format!("world.probe.report.{id}.{}", state.as_str()),
            state,
            score,
        }
    }

    /// The coverage guard for the state vocabulary: every variant has a label
    /// and every label parses back to the variant that produced it, so the
    /// script boundary and the wire cannot drift apart.
    #[test]
    fn every_state_round_trips_through_its_label() {
        for state in ReportRowState::ALL {
            assert_eq!(ReportRowState::parse(state.as_str()), Ok(state));
        }
    }

    #[test]
    fn state_parsing_is_case_and_space_insensitive_but_closed() {
        assert_eq!(ReportRowState::parse("  SAVED "), Ok(ReportRowState::Saved));
        assert!(ReportRowState::parse("rescued").is_err());
        assert!(ReportRowState::parse("").is_err());
    }

    #[test]
    fn a_fresh_report_is_empty_and_totals_zero() {
        let report = MissionReport::default();
        assert!(report.is_empty());
        assert_eq!(report.total(), 0);
        assert_eq!(report.rows(), &[]);
    }

    /// The property the whole accumulator rests on: authored order survives
    /// re-statement. A row re-scored later must not jump to the end of the
    /// report.
    #[test]
    fn rewriting_a_row_updates_it_in_place_and_keeps_its_position() {
        let mut report = MissionReport::default();
        report.set_row(row("lyra", ReportRowState::Lost, -6));
        report.set_row(row("traffic", ReportRowState::Partial, 2));
        assert!(report.set_row(row("lyra", ReportRowState::Saved, 6)));

        let ids: Vec<&str> = report.rows().iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, vec!["lyra", "traffic"]);
        assert_eq!(report.rows()[0].state, ReportRowState::Saved);
        assert_eq!(report.rows()[0].score, 6);
    }

    /// Re-stating an UNCHANGED row reports no change, which is what keeps a
    /// per-tick restatement out of the narrative timeline.
    #[test]
    fn rewriting_an_identical_row_reports_no_change() {
        let mut report = MissionReport::default();
        assert!(report.set_row(row("lyra", ReportRowState::Saved, 6)));
        assert!(!report.set_row(row("lyra", ReportRowState::Saved, 6)));
        assert_eq!(report.rows().len(), 1);
    }

    /// AC4's arithmetic: the total is the sum of the VISIBLE rows' hidden
    /// scores — nothing else contributes, and an omitted row contributes
    /// nothing because it is not there.
    #[test]
    fn the_total_is_the_sum_of_the_visible_rows_scores() {
        let mut report = MissionReport::default();
        report.set_row(row("lyra", ReportRowState::Saved, 6));
        report.set_row(row("traffic", ReportRowState::Lost, -4));
        report.set_row(row("records", ReportRowState::Neutral, 0));
        assert_eq!(report.total(), 2);
    }

    #[test]
    fn the_json_carries_every_row_in_order_with_its_score_and_the_total() {
        let mut report = MissionReport::default();
        report.set_row(row("lyra", ReportRowState::Saved, 6));
        report.set_row(row("traffic", ReportRowState::Lost, -4));
        let json = report.to_json();

        assert!(json.contains("\"id\": \"lyra\""), "{json}");
        assert!(json.contains("\"state\": \"saved\""), "{json}");
        assert!(json.contains("\"score\": 6"), "{json}");
        assert!(json.contains("\"score\": -4"), "{json}");
        assert!(json.contains("\"total\": 2"), "{json}");
        assert!(
            json.find("\"lyra\"") < json.find("\"traffic\""),
            "rows must serialise in authored order: {json}"
        );
    }

    #[test]
    fn an_empty_report_still_encodes_as_an_object() {
        assert_eq!(
            MissionReport::default().to_json(),
            "{\"rows\": [], \"total\": 0}"
        );
    }
}
