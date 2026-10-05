use super::*;
use crate::command_admission::HostSlot;
use crate::gm_action::{GmAction, GmActionGrant, GmActionId, GmActionJournal, GmActionOrder};

/// One real canonical grant. `HostSlot(7)` is deliberately not `1`, so a
/// projection that leaked the transport origin would be visible.
fn pause(sequence: u64, active: bool) -> GmActionGrant {
    let from = HostSlot(7);
    GmActionGrant {
        from,
        sequenced_by: HostSlot(1),
        operator_id: format!("gm-{}", sequence % 2 + 1),
        correlation: GmActionId::new(format!("corr-{sequence}")).unwrap(),
        recovery_generation: 0,
        apply_tick: sequence,
        order: GmActionOrder::new(from, sequence),
        action: GmAction::SetSessionPaused { active },
    }
}

/// The projection with no world behind it, which is what a journal-only
/// fixture has: no placement stopwatch, nothing that can be rebuilt, and no
/// fixture here places or removes anything.
fn journal_projection_for_test(log: &GmActionLog) -> GmJournalProjection {
    journal_projection(log, None, 60.0, None)
}

fn applied(grants: impl IntoIterator<Item = GmActionGrant>) -> GmActionLog {
    let mut journal = GmActionJournal::default();
    let mut last = 0;
    for grant in grants {
        last = grant.apply_tick;
        journal.insert(grant).unwrap();
    }
    journal.apply_through(last)
}

#[test]
fn projects_public_attribution_without_transport_identity() {
    let projection = journal_projection_for_test(&applied([pause(1, true)]));
    let row = &projection.entries[0];
    assert_eq!(row.operator_id, "gm-2");
    assert_eq!(row.correlation, "corr-1");
    assert_eq!(row.action_kind, GmActionKind::SessionPause);
    assert_eq!(row.tick, 1);
    assert_eq!(row.sequence, Some(1));
    assert_eq!(row.outcome, GmActionOutcome::Applied);
    // Structural, not a substring search: the published row carries these
    // public fields and nothing else, so a transport field added to
    // `LoggedGmAction` later cannot quietly ride along.
    let json = serde_json::to_value(&projection).unwrap();
    let mut keys: Vec<_> = json["entries"][0]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "action_kind",
            "correlation",
            "operator_id",
            "outcome",
            "sequence",
            "tick"
        ]
    );
}

#[test]
fn reports_the_real_applied_and_no_op_outcomes_in_canonical_order() {
    // Pause, pause again (nothing to change), resume.
    let projection =
        journal_projection_for_test(&applied([pause(1, true), pause(2, true), pause(3, false)]));
    assert_eq!(projection.total, 3);
    assert_eq!(projection.capacity, MAX_GM_ACTIONS_PER_RUN);
    assert_eq!(
        projection
            .entries
            .iter()
            .map(|entry| (entry.sequence, entry.outcome))
            .collect::<Vec<_>>(),
        vec![
            (Some(1), GmActionOutcome::Applied),
            (Some(2), GmActionOutcome::NoOp),
            (Some(3), GmActionOutcome::Applied),
        ]
    );
}

#[test]
fn trims_the_oldest_rows_while_still_reporting_the_full_total() {
    let count = GM_JOURNAL_WINDOW as u64 + 5;
    let projection = journal_projection_for_test(&applied(
        (1..=count).map(|n| pause(n, !n.is_multiple_of(2))),
    ));
    assert_eq!(projection.total, count as usize);
    assert_eq!(projection.entries.len(), GM_JOURNAL_WINDOW);
    assert_eq!(projection.entries[0].sequence, Some(6));
    assert_eq!(
        projection.entries[GM_JOURNAL_WINDOW - 1].sequence,
        Some(count)
    );
}
