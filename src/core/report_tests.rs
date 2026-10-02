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
