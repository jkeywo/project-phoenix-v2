//! The dossier projection (issue #1030, parent #851 "Falling Skyway").
//!
//! A **dossier** is what this crew knows about one subject — a ship, a station,
//! a piece of infrastructure. It is not a store. Nothing writes a dossier down
//! and nothing keeps one between ticks: [`project`] folds authoritative state
//! that other subsystems already own into a view, every tick, from scratch.
//!
//! # The rule: what the crew can already read is known; everything else has no field
//!
//! The mission this exists for turns on a secret — an official maintenance
//! record that does not match the structure it describes — so the one thing this
//! module must never do is leak the thing the scenario is keeping back. It is
//! held to that **structurally** rather than by filtering:
//!
//! * [`DossierSubject`] is the *whole input port*. It has a field for the
//!   subject's identity, its faction label, its comms standing, its **published**
//!   condition and the promises made to it — and no field for anything else.
//!   A withheld disposition, an unrevealed future spawn, an authored flag the
//!   scenario has not mirrored anywhere the crew can see: none of them has
//!   anywhere to go. Leaking one would take a new field on this struct, in a
//!   diff, next to this paragraph.
//! * The condition input arrives as
//!   [`InfrastructureSnapshot`](crate::messages::InfrastructureSnapshot) — the
//!   type #1025 mints through `from_state`, which returns `None` for a structure
//!   whose scenario authored `publish = false`. The dossier therefore cannot
//!   read a condition track that is off the wire, because it never holds one.
//! * Labels are the second gate. A published operational flag or capacity is a
//!   machine name in the author's namespace (#1025 says so on both fields), so
//!   it reaches a fact sheet only where the scenario also authored a
//!   `strings.csv` label for it. An unlabelled capacity is published *data* that
//!   is not published *prose*, and the projection keeps the distinction.
//!
//! The same phrasing #1029's ledger uses applies here and is the reason a whole
//! [`Commitment`] can be handed over: a caller that can see a promise exists is a
//! caller the crew already told.
//!
//! # Evidence is #1031's, and it appends
//!
//! [`DossierSnapshot::evidence`](crate::messages::DossierSnapshot::evidence) is
//! carried from this slice and left empty by it. #1031 appends entries with
//! provenance; nothing about the fold, the payload, the console state or the
//! panel changes shape when it does. That is the whole reason the list exists
//! before anything writes to it.
//!
//! Pure and Bevy-free. The adapter that gathers the live inputs is
//! [`super::server`].

use crate::messages::{DossierFactSnapshot, DossierSnapshot, DossierValue, InfrastructureSnapshot};
use crate::world::commitments::{Commitment, CommitmentState};

/// `strings.csv` id: the subject's faction.
pub const FACT_FACTION: &str = "dossier.fact.faction";
/// `strings.csv` id: whether the subject is inside comms range right now.
pub const FACT_COMMS: &str = "dossier.fact.comms";
/// `strings.csv` id: the subject's structural condition.
pub const FACT_CONDITION: &str = "dossier.fact.condition";
/// `strings.csv` id: a promise to this subject that is still owed.
pub const FACT_COMMITMENT_OPEN: &str = "dossier.fact.commitment_open";
/// `strings.csv` id: a promise to this subject the captain kept.
pub const FACT_COMMITMENT_KEPT: &str = "dossier.fact.commitment_kept";
/// `strings.csv` id: a promise to this subject the captain broke.
pub const FACT_COMMITMENT_BROKEN: &str = "dossier.fact.commitment_broken";

/// Every fact label the projection itself can emit.
///
/// A closed list, written out rather than composed, for
/// `operations::verb_label`'s reason: a composed `format!("dossier.fact.{x}")`
/// is invisible to `scripts/check-strings.mjs` and would let a new fact ship
/// with no row behind it. Every *other* label a dossier carries is one a
/// scenario authored beside the value it labels, which is where that value's
/// own string row is authored too.
pub const SHARED_FACT_LABELS: [&str; 6] = [
    FACT_FACTION,
    FACT_COMMS,
    FACT_CONDITION,
    FACT_COMMITMENT_OPEN,
    FACT_COMMITMENT_KEPT,
    FACT_COMMITMENT_BROKEN,
];

/// Everything the projection may see about one subject — **the whole input
/// port**.
///
/// See the module docs. Every field here names a surface the crew has some other
/// authoritative access to: the entity's own name and description already ride
/// the entity snapshot, the faction label is authored display text, the comms
/// standing is the roster's, the condition is what #1025 chose to publish, and a
/// promise is one the captain made out loud.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DossierSubject {
    /// The subject entity's UUID.
    pub uuid: String,
    /// `strings.csv` id for its crew-facing name, or empty.
    pub name: String,
    /// `strings.csv` id for its authored `[target] description`, or empty.
    pub summary: String,
    /// `strings.csv` id for its faction's crew-facing label. `None` for a
    /// factionless entity **and** for one whose faction authored no
    /// `display_name` — the crew have no word for it either way.
    pub faction_label: Option<String>,
    /// The subject's standing on the hail roster: `Some(in_range)` when it is a
    /// contact at all, `None` when it is not hailable.
    pub comms_in_range: Option<bool>,
    /// The subject's **published** condition track, already gated by #1025's
    /// `publish` flag, paired with the labels the scenario authored for its
    /// thresholds and capacities.
    pub condition: Option<SubjectCondition>,
    /// Promises made to this subject, oldest first — the ledger's own order.
    pub commitments: Vec<Commitment>,
}

/// A subject's published condition track plus the crew-facing labels for it
/// (issue #1030).
///
/// Built by the adapter from an [`InfrastructureSnapshot`] and the authored
/// labels beside it; see [`SubjectCondition::from_published`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SubjectCondition {
    /// Structural condition as a fraction of the authored ceiling.
    pub condition_fraction: f32,
    /// `(label id, held)` for each operational flag that authored a label, in
    /// authored order.
    pub flags: Vec<(String, bool)>,
    /// `(label id, amount)` for each capacity that authored a label, in authored
    /// order.
    pub capacities: Vec<(String, i64)>,
}

impl SubjectCondition {
    /// Pair a published snapshot with the authored labels for its flags and
    /// capacities, dropping every entry that has none.
    ///
    /// `flag_labels` / `capacity_labels` resolve a machine id to its authored
    /// `strings.csv` label. Taking them as lookups rather than reading the live
    /// config keeps this side of the boundary honest: the *values* can only ever
    /// be ones `InfrastructureSnapshot::from_state` already published, and the
    /// labels can only ever be ones an author wrote for the crew.
    pub fn from_published(
        published: &InfrastructureSnapshot,
        flag_labels: impl Fn(&str) -> Option<String>,
        capacity_labels: impl Fn(&str) -> Option<String>,
    ) -> Self {
        Self {
            condition_fraction: published.condition_fraction,
            flags: published
                .flags
                .iter()
                .filter_map(|(id, held)| flag_labels(id).map(|label| (label, *held)))
                .collect(),
            capacities: published
                .capacities
                .iter()
                .filter_map(|(id, amount)| capacity_labels(id).map(|label| (label, *amount)))
                .collect(),
        }
    }
}

/// The `strings.csv` label for a promise in each of the ledger's three states.
fn commitment_label(state: CommitmentState) -> &'static str {
    match state {
        CommitmentState::Open => FACT_COMMITMENT_OPEN,
        CommitmentState::Kept => FACT_COMMITMENT_KEPT,
        CommitmentState::Broken => FACT_COMMITMENT_BROKEN,
    }
}

/// Fold one subject's authoritative facts into the dossier a console renders.
///
/// Deterministic and total: the same input always produces the same fact list in
/// the same order, and a subject with nothing known about it projects an *empty*
/// dossier rather than no dossier. "Empty, not missing" is a real distinction on
/// screen — a crew that opens a sheet and finds nothing on it has learned
/// something; a subject that silently vanishes from the list has told them
/// nothing at all.
///
/// Fact order is fixed here rather than sorted, because it is an editorial
/// order: who they are, whether you can talk to them, what state they are in,
/// what you owe them.
pub fn project(subject: &DossierSubject) -> DossierSnapshot {
    let mut facts = Vec::new();

    if let Some(label) = &subject.faction_label {
        facts.push(DossierFactSnapshot {
            label: FACT_FACTION.to_string(),
            value: DossierValue::Text(label.clone()),
        });
    }

    if let Some(in_range) = subject.comms_in_range {
        facts.push(DossierFactSnapshot {
            label: FACT_COMMS.to_string(),
            value: DossierValue::Flag(in_range),
        });
    }

    if let Some(condition) = &subject.condition {
        facts.push(DossierFactSnapshot {
            label: FACT_CONDITION.to_string(),
            value: DossierValue::Fraction(condition.condition_fraction),
        });
        for (label, held) in &condition.flags {
            facts.push(DossierFactSnapshot {
                label: label.clone(),
                value: DossierValue::Flag(*held),
            });
        }
        for (label, amount) in &condition.capacities {
            facts.push(DossierFactSnapshot {
                label: label.clone(),
                value: DossierValue::Count(*amount),
            });
        }
    }

    for commitment in &subject.commitments {
        facts.push(DossierFactSnapshot {
            label: commitment_label(commitment.state).to_string(),
            value: DossierValue::Text(commitment.terms.clone()),
        });
    }

    DossierSnapshot {
        uuid: subject.uuid.clone(),
        name: subject.name.clone(),
        summary: subject.summary.clone(),
        facts,
        // #1031's, and appended there. Empty is the whole contract this slice
        // owes that one.
        evidence: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn subject() -> DossierSubject {
        DossierSubject {
            uuid: "skyhook-1".into(),
            name: "world.entity.skyhook.name".into(),
            ..DossierSubject::default()
        }
    }

    fn promise(id: &str, terms: &str, state: CommitmentState) -> Commitment {
        Commitment {
            id: id.into(),
            made_to: "world.entity.strike_committee.name".into(),
            terms: terms.into(),
            resolves_when: "world.probe.resolves".into(),
            state,
            made_at_tick: 10,
            resolved_at_tick: None,
        }
    }

    fn labels(facts: &[DossierFactSnapshot]) -> Vec<&str> {
        facts.iter().map(|f| f.label.as_str()).collect()
    }

    /// **AC2.** A subject nobody knows anything about still has a sheet. The
    /// list carries it, the name resolves, and the fact list is empty rather
    /// than the whole dossier being absent.
    #[test]
    fn a_subject_with_no_known_facts_projects_an_empty_dossier_not_a_missing_one() {
        let dossier = project(&subject());
        assert_eq!(dossier.uuid, "skyhook-1");
        assert_eq!(dossier.name, "world.entity.skyhook.name");
        assert!(
            dossier.facts.is_empty(),
            "nothing is known, so nothing is claimed"
        );
        assert!(
            dossier.evidence.is_empty(),
            "evidence is #1031's to append; this slice always leaves it empty"
        );
    }

    /// The editorial order: who they are, whether you can talk to them, what
    /// state they are in, what you owe them.
    #[test]
    fn facts_fold_in_a_fixed_editorial_order() {
        let mut s = subject();
        s.faction_label = Some("faction.federation.display_name".into());
        s.comms_in_range = Some(true);
        s.condition = Some(SubjectCondition {
            condition_fraction: 0.42,
            flags: vec![("world.skyhook.transfer_capable.label".into(), false)],
            capacities: vec![("world.skyhook.berths.label".into(), 4)],
        });
        s.commitments = vec![promise(
            "safe_passage",
            "world.probe.terms",
            CommitmentState::Open,
        )];

        let dossier = project(&s);
        assert_eq!(
            labels(&dossier.facts),
            vec![
                FACT_FACTION,
                FACT_COMMS,
                FACT_CONDITION,
                "world.skyhook.transfer_capable.label",
                "world.skyhook.berths.label",
                FACT_COMMITMENT_OPEN,
            ]
        );
        assert_eq!(
            dossier.facts[2].value,
            DossierValue::Fraction(0.42),
            "the condition rides as a fraction — the client renders the percentage"
        );
        assert_eq!(
            dossier.facts[4].value,
            DossierValue::Count(4),
            "a capacity is a whole number, not a formatted string"
        );
    }

    /// **AC1, the hidden-truth guarantee, stated as a property of the INPUT.**
    ///
    /// #1025's `from_state` is the publish gate, and it is the only way a
    /// condition track can reach a [`SubjectCondition`]. A structure the
    /// scenario keeps off the wire produces `None` there, so the dossier has no
    /// condition to fold and the withheld number is on no fact.
    ///
    /// The wire-shape half of the same guarantee — that the payload has no field
    /// a secret could ride in at all — is asserted in `codec.rs`, where the
    /// serialisation lives.
    #[test]
    fn an_unpublished_condition_track_cannot_reach_the_projection() {
        use crate::infrastructure::{InfrastructureConfig, InfrastructureState};

        let hidden = InfrastructureState::from_config(&InfrastructureConfig {
            condition_max: 100.0,
            condition: Some(31.0),
            publish: false,
            ..InfrastructureConfig::default()
        });
        assert!(
            InfrastructureSnapshot::from_state(&hidden).is_none(),
            "the publish gate is #1025's, and this projection is downstream of it"
        );

        let mut s = subject();
        s.condition = InfrastructureSnapshot::from_state(&hidden)
            .as_ref()
            .map(|published| SubjectCondition::from_published(published, |_| None, |_| None));

        let dossier = project(&s);
        assert!(
            !labels(&dossier.facts).contains(&FACT_CONDITION),
            "a structure kept off the wire has no condition row"
        );
        assert!(
            !dossier
                .facts
                .iter()
                .any(|f| matches!(f.value, DossierValue::Fraction(_))),
            "and the withheld 0.31 is on no fact of any label"
        );
    }

    /// The second gate. A published flag or capacity is a machine id in the
    /// author's namespace; without an authored crew-facing label there is
    /// nothing to call it, so it does not become a row.
    #[test]
    fn an_unlabelled_flag_or_capacity_is_published_data_but_not_a_dossier_row() {
        let published = InfrastructureSnapshot {
            condition_fraction: 0.8,
            flags: vec![
                ("transfer_capable".into(), true),
                ("docking_capable".into(), false),
            ],
            capacities: vec![("berths".into(), 4), ("throughput".into(), 900)],
        };
        let condition = SubjectCondition::from_published(
            &published,
            |id| (id == "transfer_capable").then(|| "world.skyhook.transfer.label".to_string()),
            |id| (id == "berths").then(|| "world.skyhook.berths.label".to_string()),
        );

        let mut s = subject();
        s.condition = Some(condition);
        let dossier = project(&s);

        assert_eq!(
            labels(&dossier.facts),
            vec![
                FACT_CONDITION,
                "world.skyhook.transfer.label",
                "world.skyhook.berths.label",
            ],
            "only the labelled half becomes prose"
        );
        assert!(
            !labels(&dossier.facts)
                .iter()
                .any(|l| *l == "docking_capable" || *l == "throughput"),
            "and a machine id is never itself used as a label"
        );
    }

    /// A promise's STATE is carried by which label it folds under, so the panel
    /// tells "still owed" from "kept" from "broken" without a second field —
    /// the three-states-never-two rule #1029's ledger is built on, preserved
    /// across the projection.
    #[test]
    fn each_promise_folds_under_the_label_for_the_state_it_is_in() {
        let mut s = subject();
        s.commitments = vec![
            promise("a", "world.probe.a", CommitmentState::Open),
            promise("b", "world.probe.b", CommitmentState::Kept),
            promise("c", "world.probe.c", CommitmentState::Broken),
        ];
        let dossier = project(&s);
        assert_eq!(
            labels(&dossier.facts),
            vec![
                FACT_COMMITMENT_OPEN,
                FACT_COMMITMENT_KEPT,
                FACT_COMMITMENT_BROKEN,
            ]
        );
        assert_eq!(
            dossier.facts[0].value,
            DossierValue::Text("world.probe.a".into()),
            "the value is the TERMS the crew were given, by string id"
        );
    }

    /// Every shared label is distinct and none is composed at runtime — the
    /// property `scripts/check-strings.mjs` relies on to find them all.
    #[test]
    fn the_shared_fact_labels_are_a_closed_distinct_set() {
        let mut sorted = SHARED_FACT_LABELS.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), SHARED_FACT_LABELS.len());
        for label in SHARED_FACT_LABELS {
            assert!(label.starts_with("dossier.fact."));
        }
    }
}
