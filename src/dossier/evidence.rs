//! Gathered evidence, with its provenance (issue #1031, parent #851).
//!
//! A dossier that only ever shows authored background is a reference card.
//! [`super::projection`] folds that background; this module is the other half —
//! the record of what this crew **found out**, and of how.
//!
//! # Gathered knowledge is not projected knowledge, and the difference is the point
//!
//! Everything [`super::projection`] puts on a fact sheet is a restatement of
//! something the crew already had access to: the faction is on the roster, the
//! condition is what #1025 chose to publish, the promise is one the captain made
//! out loud. Nothing in that fold is *learned*. An evidence entry is the
//! opposite: it exists because the crew did something — ran a scan, got an
//! admission out of somebody, put two records side by side — and it is written
//! down only where a scenario says that act happened.
//!
//! That is why this is **state** and the projection is not. A run that scanned
//! the skyhook and a run that did not are different runs, and no amount of
//! re-folding the world recovers which one you are in. So it is stored, it is
//! snapshotted, and the census accounts for it.
//!
//! # Provenance is a field, not prose
//!
//! [`EvidenceProvenance`] is typed and closed. The whole Thin Margin arc turns
//! on the crew being able to say *how they know* — an official briefing and a
//! sensor return are different kinds of claim, and a mission whose evidence text
//! merely happened to begin *"Scan reports…"* could not sort, filter, or
//! contradict them. Baking the source into the text would also put it beyond
//! translation, which AGENTS.md rule 11 forbids on its own.
//!
//! # THE STORE IS APPEND-ONLY, AND DUPLICATES ARE A NO-OP
//!
//! Two properties, both load-bearing:
//!
//! 1. **Nothing is ever removed or edited.** There is no `forget`, no
//!    `retract`, and no mutable accessor. A crew's account of what they learned
//!    is the run's history; a scenario that wants to contradict an earlier
//!    finding appends the contradiction, which is what a later debrief needs to
//!    see. (It is also what keeps the ordering claim below true.)
//! 2. **Appending the same thing twice changes nothing.** The identity of an
//!    entry is `(subject, provenance, text)` — the *tick is not part of it* — so
//!    a second scan of the same structure yielding the same finding leaves the
//!    file as it was, stamped with the tick the crew FIRST learned it. This is a
//!    silent no-op rather than [`super::super::world::commitments`]' raise, and
//!    the asymmetry is deliberate: a duplicate promise is an authoring mistake
//!    (two beats claiming the same id mean different terms), whereas scanning
//!    the same thing twice is an ordinary player action that must not cost the
//!    scenario its call.
//!
//! # Ordering is gather order, and it is the same on every peer
//!
//! [`EvidenceLog::entries`] is a `Vec` in append order, for
//! [`CommitmentLedger`](crate::world::commitments::CommitmentLedger)'s reason: a
//! `HashMap`'s iteration order must never reach a payload or a fold, and the
//! order things were learned in is *already* a deterministic function of the run
//! — script appends ride the ordered per-call effect buffer, which applies in
//! authored order, inside a tick loop every peer reproduces. So two clients
//! render the same fact sheet without this module sorting anything, and the
//! per-subject view is a stable subsequence of the global one.
//!
//! A map keyed by subject was rejected for the same reason the ledger rejected
//! one: it would make "which of these two findings came first" depend on which
//! bucket they landed in, and the answer is the whole readout.
//!
//! Pure and Bevy-free. It is a field on
//! [`WorldContentRuntime`](crate::world::server::WorldContentRuntime), beside
//! the deadline table and the commitments ledger and for their reason — every
//! site that already borrows the content runtime to apply a call's effects can
//! append to it, and the state census sees no new registration.

use serde::{Deserialize, Serialize};

/// How this crew came to know something.
///
/// Closed, and closed on purpose: the four kinds are the vocabulary issue #1031
/// names, each has a `strings.csv` label the client resolves
/// (`PROVENANCE_LABELS` in `gui/components/ph-dossier-panel.js`), and a fifth
/// would need one too. An open string field would let a scenario invent
/// `"hearsay"` and have it render as nothing at all.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceProvenance {
    /// A sensor return. The crew pointed something at it and read the answer.
    Scan,
    /// Testimony: something a named party said on an open channel.
    Dialogue,
    /// A records comparison — two documents that do not agree, put side by side.
    Records,
    /// The mission briefing. Authored background the crew were handed, carried
    /// here rather than in the projected facts because *being told* is itself a
    /// provenance, and a crew that cannot tell a briefing from a scan cannot
    /// argue with the briefing.
    Briefing,
}

/// A provenance name no [`EvidenceProvenance`] answers to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnknownProvenance {
    /// The name as the script wrote it.
    pub name: String,
}

impl std::fmt::Display for UnknownProvenance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "unknown provenance '{}': expected one of {}",
            self.name,
            EvidenceProvenance::ALL
                .iter()
                .map(|p| p.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
}

impl EvidenceProvenance {
    /// Every provenance, in the order a fact sheet's legend would read them.
    ///
    /// Written out rather than derived so the wire names, the `strings.csv`
    /// rows and this list are visible in one place; the test below pins that
    /// each is distinct and that `parse` round-trips every one.
    pub const ALL: [Self; 4] = [Self::Scan, Self::Dialogue, Self::Records, Self::Briefing];

    /// The script/wire name. Hand-written for
    /// [`CommitmentState::as_str`](crate::world::commitments::CommitmentState::as_str)'s
    /// reason: the exact strings a scenario types are visible at the point they
    /// are promised, rather than being an accident of the variant spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Scan => "scan",
            Self::Dialogue => "dialogue",
            Self::Records => "records",
            Self::Briefing => "briefing",
        }
    }

    /// Read a script's provenance name, or say what was expected.
    ///
    /// An error rather than a defaulting fallback: a mistyped `"scans"` that
    /// silently became a scan would put a claim on a fact sheet under a
    /// provenance nobody authored, and the crew's ability to say how they know
    /// is the thing this slice exists to protect.
    pub fn parse(name: &str) -> Result<Self, UnknownProvenance> {
        Self::ALL
            .into_iter()
            .find(|p| p.as_str() == name)
            .ok_or_else(|| UnknownProvenance {
                name: name.to_string(),
            })
    }
}

/// One thing this crew found out about one subject.
///
/// Every field is something the crew themselves produced or were handed: what
/// was learned (by `strings.csv` id), how, about whom, and when. There is
/// deliberately no "confidence", no "true value" and no hidden pair — the same
/// refusal [`Commitment`](crate::world::commitments::Commitment) makes, for the
/// same reason. A scenario in which the crew are misled authors the misleading
/// finding as its own entry, with the provenance that misled them, and the
/// contradiction arrives later as a second entry the crew can compare it
/// against. That is what makes the whole record safe to hand to a console:
/// inspecting it cannot reveal anything the crew were not already shown.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceEntry {
    /// The subject's UUID.
    ///
    /// A UUID here, where a promise's `made_to` is deliberately a *name*,
    /// because the two records mean different things. A promise is made to a
    /// party that outlives the hull it was given to; a finding is about the
    /// specific thing that was examined, and if that thing is gone there is no
    /// sheet for the finding to sit on. Resolved once, by the applier that holds
    /// `name_to_uuid`, so script keeps writing names.
    pub subject_uuid: String,
    /// `strings.csv` id for what was learned. Never English (AGENTS.md rule 11).
    pub text: String,
    /// How the crew learned it.
    pub provenance: EvidenceProvenance,
    /// The `SimTick` it was learned on.
    ///
    /// Stamped at the script surface off the same call clock a promise's
    /// `made_at_tick` uses, so a later debrief can order a run's findings
    /// against its promises without re-simulating it. On a duplicate append the
    /// FIRST tick is kept: the crew learned it then, and confirming it later
    /// does not move when they found out.
    pub gathered_at_tick: u64,
}

/// Everything this run has found out, in the order it found it out.
///
/// See the module docs for why this is a `Vec` rather than a map keyed by
/// subject, and why nothing here can remove an entry.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceLog {
    /// The findings, oldest first.
    #[serde(default)]
    pub entries: Vec<EvidenceEntry>,
}

impl EvidenceLog {
    /// Whether the crew have found out anything at all.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Write one finding onto the subject's file, returning `false` when it was
    /// already there.
    ///
    /// The duplicate answer is a no-op rather than an error — see the module
    /// docs — and the caller uses the `bool` only to log; nothing in the
    /// simulation branches on it.
    pub fn append(
        &mut self,
        subject_uuid: &str,
        text: &str,
        provenance: EvidenceProvenance,
        now_tick: u64,
    ) -> bool {
        if self.holds(subject_uuid, text, provenance) {
            return false;
        }
        self.entries.push(EvidenceEntry {
            subject_uuid: subject_uuid.to_string(),
            text: text.to_string(),
            provenance,
            gathered_at_tick: now_tick,
        });
        true
    }

    /// Whether this exact finding is already on file — the identity duplicate
    /// appends are judged against, stated once so the check and the doc comment
    /// above it cannot drift apart.
    pub fn holds(&self, subject_uuid: &str, text: &str, provenance: EvidenceProvenance) -> bool {
        self.entries
            .iter()
            .any(|e| e.subject_uuid == subject_uuid && e.text == text && e.provenance == provenance)
    }

    /// One subject's file, oldest first.
    ///
    /// A filtered view rather than a stored per-subject bucket, the way
    /// [`CommitmentLedger::open`](crate::world::commitments::CommitmentLedger::open)
    /// is: the order is the global gather order restricted to this subject, so
    /// the sheet reads in the order the crew learned things and cannot disagree
    /// with the log it came from.
    pub fn for_subject<'a>(
        &'a self,
        subject_uuid: &'a str,
    ) -> impl Iterator<Item = &'a EvidenceEntry> {
        self.entries
            .iter()
            .filter(move |e| e.subject_uuid == subject_uuid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn log() -> EvidenceLog {
        EvidenceLog::default()
    }

    /// **AC1.** An entry carries all four: what was learned, how, about whom,
    /// and when.
    #[test]
    fn an_entry_carries_its_text_its_provenance_its_subject_and_its_tick() {
        let mut log = log();
        assert!(log.append(
            "skyhook-1",
            "world.probe.evidence.stress_fracture",
            EvidenceProvenance::Scan,
            420
        ));
        assert_eq!(
            log.entries,
            vec![EvidenceEntry {
                subject_uuid: "skyhook-1".into(),
                text: "world.probe.evidence.stress_fracture".into(),
                provenance: EvidenceProvenance::Scan,
                gathered_at_tick: 420,
            }]
        );
    }

    /// **AC3.** The same finding twice is one line, stamped with the tick the
    /// crew first learned it — confirming something later does not move when
    /// they found out.
    #[test]
    fn appending_the_same_finding_twice_leaves_one_entry_stamped_at_the_first_tick() {
        let mut log = log();
        assert!(log.append("skyhook-1", "world.probe.a", EvidenceProvenance::Scan, 100));
        assert!(
            !log.append("skyhook-1", "world.probe.a", EvidenceProvenance::Scan, 900),
            "the second append reports that it changed nothing"
        );
        assert_eq!(log.entries.len(), 1);
        assert_eq!(log.entries[0].gathered_at_tick, 100);
    }

    /// The identity is all three parts. The SAME text learned another way is a
    /// different fact about the crew — it is the second, independent source —
    /// and the same provenance about another subject is another subject's file.
    #[test]
    fn the_duplicate_identity_is_subject_and_provenance_and_text_together() {
        let mut log = log();
        assert!(log.append("skyhook-1", "world.probe.a", EvidenceProvenance::Scan, 10));
        assert!(
            log.append(
                "skyhook-1",
                "world.probe.a",
                EvidenceProvenance::Dialogue,
                20
            ),
            "the same claim, corroborated from a second source, is a second entry"
        );
        assert!(
            log.append("depot-2", "world.probe.a", EvidenceProvenance::Scan, 30),
            "and another subject's file is another subject's file"
        );
        assert!(
            log.append("skyhook-1", "world.probe.b", EvidenceProvenance::Scan, 40),
            "and a different finding from the same scan is a fourth"
        );
        assert_eq!(log.entries.len(), 4);
    }

    /// **AC6.** Per-subject order is the global gather order restricted to that
    /// subject — a stable subsequence, never a re-sort — so two peers replaying
    /// the same run render the same sheet.
    #[test]
    fn a_subjects_file_reads_in_the_order_the_crew_learned_things() {
        let mut log = log();
        log.append("a", "world.probe.1", EvidenceProvenance::Briefing, 1);
        log.append("b", "world.probe.2", EvidenceProvenance::Scan, 2);
        log.append("a", "world.probe.3", EvidenceProvenance::Dialogue, 3);
        log.append("a", "world.probe.4", EvidenceProvenance::Records, 4);

        assert_eq!(
            log.for_subject("a")
                .map(|e| e.text.as_str())
                .collect::<Vec<_>>(),
            vec!["world.probe.1", "world.probe.3", "world.probe.4"],
        );
        assert_eq!(
            log.for_subject("b")
                .map(|e| e.text.as_str())
                .collect::<Vec<_>>(),
            vec!["world.probe.2"],
        );
        assert_eq!(
            log.for_subject("nobody").count(),
            0,
            "a subject nothing was learned about has an empty file, not a missing one"
        );
    }

    /// The wire vocabulary: four distinct names, each of which parses back to
    /// the variant that produced it, and nothing else parses at all.
    #[test]
    fn every_provenance_round_trips_through_its_script_name() {
        let mut names: Vec<&str> = EvidenceProvenance::ALL.iter().map(|p| p.as_str()).collect();
        assert_eq!(names.len(), 4);
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), 4, "the four names are distinct");

        for provenance in EvidenceProvenance::ALL {
            assert_eq!(
                EvidenceProvenance::parse(provenance.as_str()),
                Ok(provenance)
            );
        }
        let err = EvidenceProvenance::parse("hearsay").expect_err("not a provenance");
        assert_eq!(err.name, "hearsay");
        assert!(
            format!("{err}").contains("scan, dialogue, records, briefing"),
            "the message names what WAS expected: {err}"
        );
    }

    /// The serde names are the script names, so a save, a payload and a scenario
    /// all spell a provenance the same way.
    #[test]
    fn a_provenance_serialises_under_its_script_name() {
        for provenance in EvidenceProvenance::ALL {
            assert_eq!(
                serde_json::to_string(&provenance).unwrap(),
                format!("\"{}\"", provenance.as_str())
            );
        }
    }

    /// The whole log round-trips, which is what #863 persists it as.
    #[test]
    fn the_log_round_trips_through_serde_in_order() {
        let mut log = log();
        log.append("a", "world.probe.1", EvidenceProvenance::Scan, 60);
        log.append("b", "world.probe.2", EvidenceProvenance::Briefing, 120);
        let json = serde_json::to_string(&log).unwrap();
        assert_eq!(serde_json::from_str::<EvidenceLog>(&json).unwrap(), log);
    }
}
