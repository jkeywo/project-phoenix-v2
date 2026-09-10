//! Public presentation of the ONE canonical GM action journal (issue #1441).
//!
//! There is no second journal here and no second store. Every field below is
//! copied out of [`crate::gm_action::GmActionLog`], the derived terminal-result
//! log that [`crate::gm_action::GmActionJournal::applied_log`] recomputes from
//! the durable, snapshot-folded journal — so a capture/restore that rewinds the
//! journal rewinds this projection with it, and a later entry that the restored
//! prefix never contained cannot survive as a visible row.
//!
//! What it deliberately does NOT carry is the transport half of a grant:
//! [`crate::gm_action::GmActionGrant`]'s `from`, `sequenced_by` and
//! `recovery_generation`, and the [`crate::command_admission::HostSlot`] inside
//! [`crate::gm_action::GmActionOrder::origin`]. Those are session/technical
//! identities, not facilitation history; a GM reading who did what needs the
//! public operator id (which [`crate::gm_roster::GmRoster`] names) and the
//! deterministic apply order, not the slot a peer happened to occupy.

use serde::{Deserialize, Serialize};

use crate::gm_action::{
    GmActionKind, GmActionLog, GmActionOutcome, GmActionRefusalReason, GmAffectedField,
    GmUndoReference, LoggedGmAction, MAX_GM_ACTIONS_PER_RUN,
};

/// `skip_serializing_if` for a `bool` that is absent when false.
fn is_false(value: &bool) -> bool {
    !*value
}

/// How many of the journal's terminal facts one projection carries.
///
/// The same presentation bound the Session result feed already uses. `total`
/// and `capacity` below keep the trimming honest rather than implying the run
/// only ever contained what the window shows.
pub const GM_JOURNAL_WINDOW: usize = 128;

/// One public row of the canonical journal.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GmJournalEntry {
    /// The operator who originated the action. A crew-public GM identity
    /// (`gm_roster`), never a session token or a reconnect credential.
    pub operator_id: String,
    /// The operator-chosen correlation, which is what makes one row stably
    /// selectable across republishes.
    pub correlation: String,
    pub action_kind: GmActionKind,
    /// The action's stable target identity, for the families that have one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    /// The agreed simulation tick the action applied at.
    pub tick: u64,
    /// The owner-assigned total order WITHIN that tick. Only the contiguous
    /// sequence: `GmActionOrder::origin` is a `HostSlot` and stays private.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sequence: Option<u64>,
    pub outcome: GmActionOutcome,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<GmActionRefusalReason>,
    /// The EXACT field this action changed and the values on either side of it
    /// (issue #1442), for the families that have a typed inverse.
    ///
    /// Present only on an `Applied` fact of such a family — the journal records
    /// it nowhere else, because nowhere else is there a true pair to record.
    /// The page echoes this value back verbatim when it asks for an inverse,
    /// which is what lets the canonical reducer refuse a request built on a
    /// stale reading rather than acting on it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub affected: Option<GmAffectedField>,
    /// The original action this fact reversed (issue #1442), when it is itself
    /// an inverse. Its operator id is the SECOND operator on that history: the
    /// one who acted, beside `operator_id`, who undid it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub undo_of: Option<GmUndoReference>,
    /// Whether an applied inverse of THIS entry already exists.
    ///
    /// Derived from the same canonical log rather than stored, so it cannot
    /// disagree with it, and published because otherwise every reader would
    /// have to re-derive it — including one that offers an Undo control and
    /// would otherwise offer it twice.
    #[serde(default, skip_serializing_if = "is_false")]
    pub inverted: bool,
}

impl GmJournalEntry {
    /// Project one durable fact, dropping everything that is not public.
    ///
    /// `inverted` is supplied by the caller because it is a fact about the
    /// whole log, not about this entry.
    pub fn from_logged(entry: &LoggedGmAction, inverted: bool) -> Self {
        Self {
            operator_id: entry.operator_id.clone(),
            correlation: entry.correlation.as_str().to_string(),
            action_kind: entry.action_kind,
            target: entry.target.clone(),
            tick: entry.tick,
            sequence: entry.order.map(|order| order.sequence),
            outcome: entry.outcome,
            reason: entry.reason,
            affected: entry.affected.clone(),
            undo_of: entry.undo_of.clone(),
            inverted,
        }
    }
}

/// The absolute, presentation-bounded view of the canonical journal.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GmJournalProjection {
    /// The durable journal's own protocol cap, so "bounded" is a stated fact
    /// rather than something a GM has to infer from a truncated list.
    pub capacity: usize,
    /// How many terminal facts the canonical applied log holds right now.
    pub total: usize,
    /// The most recent [`GM_JOURNAL_WINDOW`] of them, oldest first.
    pub entries: Vec<GmJournalEntry>,
}

/// Build the projection from the canonical applied log.
///
/// `Pending` facts are excluded: an action whose authentic consumer has not yet
/// answered has no terminal outcome to attribute, and the journal panel's whole
/// claim is that every row it shows really happened.
pub fn journal_projection(log: &GmActionLog) -> GmJournalProjection {
    let terminal: Vec<&LoggedGmAction> = log
        .entries()
        .iter()
        .filter(|entry| entry.outcome != GmActionOutcome::Pending)
        .collect();
    // Which originals an APPLIED inverse has already reversed. Taken over the
    // whole log rather than the window: an undo that has scrolled out of the
    // presentation bound still happened, and an entry that stopped saying so
    // would invite a second undo the reducer would only refuse.
    //
    // Keyed on (operator, correlation), not correlation alone: a correlation is
    // scoped to the operator that minted it (`GmActionJournal::grant_for`
    // refuses a duplicate only within one operator's own lane), so two GMs may
    // legitimately both be running a "faction-1". Matching on the bare string
    // would mark the OTHER operator's untouched action as already reversed and
    // hide its Undo control, which `undo_precheck` would then happily allow.
    let inverted: std::collections::BTreeSet<(&str, &str)> = log
        .entries()
        .iter()
        .filter(|entry| entry.outcome == GmActionOutcome::Applied)
        .filter_map(|entry| entry.undo_of.as_ref())
        .map(|undo| (undo.operator_id.as_str(), undo.correlation.as_str()))
        .collect();
    let total = terminal.len();
    let window = terminal
        .iter()
        .skip(total.saturating_sub(GM_JOURNAL_WINDOW))
        .map(|entry| {
            GmJournalEntry::from_logged(
                entry,
                inverted.contains(&(entry.operator_id.as_str(), entry.correlation.as_str())),
            )
        })
        .collect();
    GmJournalProjection {
        capacity: MAX_GM_ACTIONS_PER_RUN,
        total,
        entries: window,
    }
}

#[cfg(test)]
mod tests {
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
        let projection = journal_projection(&applied([pause(1, true)]));
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
            journal_projection(&applied([pause(1, true), pause(2, true), pause(3, false)]));
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
        let projection = journal_projection(&applied(
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
}
