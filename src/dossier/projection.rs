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
//!   condition, the promises made to it and the evidence the crew gathered on it
//!   — and no field for anything else. A withheld disposition, an unrevealed
//!   future spawn, an authored flag the scenario has not mirrored anywhere the
//!   crew can see: none of them has anywhere to go. Leaking one would take a new
//!   field on this struct, in a diff, next to this paragraph.
//! * The condition input arrives as
//!   [`InfrastructureSnapshot`](crate::core::messages::InfrastructureSnapshot) — the
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
//! # Evidence is GATHERED knowledge, and it does not weaken the rule (issue #1031)
//!
//! [`DossierSubject::evidence`] is the port's sixth and last field, and it is
//! the only one that is not a re-statement of something folded from elsewhere:
//! it is what the crew *found out*, carried out of
//! [`crate::dossier::evidence`]'s append-only log.
//!
//! Adding it does not open a door the five above are closed to, and the reason
//! is worth stating precisely rather than assuming. **An evidence entry exists
//! only because a scenario said the crew did something.** Its one writer is
//! `ctx.dossier.append(…)`, authored at the beat where a scan returns or a
//! witness talks; nothing here or in [`super::server`] derives an entry from
//! world state, reads one out of a withheld field, or synthesises one when a
//! secret becomes true. So the invariant is unchanged in substance — everything
//! on a sheet is something the crew already have access to — and merely widened
//! in *how*: the first five fields are surfaces the crew can consult, and this
//! one is the record of what they went and got.
//!
//! The two structural gates stand exactly as they did. A withheld condition
//! track still reaches no field, because
//! [`InfrastructureSnapshot::from_state`](crate::core::messages::InfrastructureSnapshot)
//! still returns `None` for it and *nothing about evidence touches that path*;
//! and the payload's whole key set is still pinned in `codec.rs`, which is where
//! a field a secret could ride in would have to appear. What a scenario CAN now
//! do is reveal its own secret — deliberately, in a beat, as an entry with a
//! provenance saying how the crew got it. That is the mechanism the mission
//! wanted, and issue #1030's rule always said appending was the only path to it.
//!
//! Pure and Bevy-free. The adapter that gathers the live inputs is
//! [`super::server`].

use crate::core::messages::{
    DossierEvidenceSnapshot, DossierFactSnapshot, DossierSnapshot, DossierValue,
    InfrastructureSnapshot,
};
use crate::dossier::evidence::EvidenceEntry;
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
/// standing is the roster's, the condition is what #1025 chose to publish, a
/// promise is one the captain made out loud — and the evidence is what the crew
/// themselves went and found out.
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
    /// What the crew have found out about this subject, oldest first — the
    /// log's own order, restricted to this subject (issue #1031).
    ///
    /// Whole [`EvidenceEntry`] records for the reason whole [`Commitment`]s are
    /// handed over: there is nothing on one to withhold. Every field was
    /// produced by the crew's own act of finding out, so a caller that can see
    /// an entry is a caller who was there when it was gathered.
    pub evidence: Vec<EvidenceEntry>,
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
///
/// Evidence is kept in its own list rather than folded into `facts` (issue
/// #1031), and that separation is the whole readout: a crew looking at a sheet
/// must be able to see which lines they were handed and which they earned. It
/// stays in gather order — never re-sorted by provenance, never interleaved
/// with the facts — so the sheet reads as the story of what the crew did.
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
        evidence: subject
            .evidence
            .iter()
            .map(|entry| DossierEvidenceSnapshot {
                text: entry.text.clone(),
                // The typed provenance crosses the wire under its own script
                // name, so the panel's PROVENANCE_LABELS table, a scenario's
                // `provenance: "scan"`, and a save all spell it identically.
                provenance: entry.provenance.as_str().to_string(),
                gathered_at_tick: entry.gathered_at_tick,
            })
            .collect(),
    }
}

#[cfg(test)]
#[path = "projection_tests.rs"]
mod tests;
