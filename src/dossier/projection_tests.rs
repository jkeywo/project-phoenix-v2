use super::*;
use crate::dossier::evidence::EvidenceProvenance;

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

fn finding(text: &str, provenance: EvidenceProvenance, tick: u64) -> EvidenceEntry {
    EvidenceEntry {
        subject_uuid: "skyhook-1".into(),
        text: text.into(),
        provenance,
        gathered_at_tick: tick,
    }
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
        "a crew who have found nothing out have an empty file, not a missing one"
    );
}

/// The editorial order: who they are, whether you can talk to them, what
/// state they are in, what you owe them.
#[test]
fn facts_fold_in_a_fixed_editorial_order() {
    let mut s = subject();
    s.faction_label = Some("faction.alliance.display_name".into());
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

// ── Gathered evidence (issue #1031) ──────────────────────────────────────

/// **AC4's server half.** Evidence rides its own list, in gather order, with
/// the provenance beside each entry — never folded into `facts`, because the
/// separation between what the crew were handed and what they earned is the
/// readout.
#[test]
fn evidence_projects_into_its_own_list_in_gather_order_and_never_into_the_facts() {
    let mut s = subject();
    s.faction_label = Some("faction.alliance.display_name".into());
    s.evidence = vec![
        finding(
            "world.probe.evidence.brief",
            EvidenceProvenance::Briefing,
            1,
        ),
        finding("world.probe.evidence.scan", EvidenceProvenance::Scan, 400),
        finding(
            "world.probe.evidence.foreman",
            EvidenceProvenance::Dialogue,
            900,
        ),
    ];

    let dossier = project(&s);
    assert_eq!(
        labels(&dossier.facts),
        vec![FACT_FACTION],
        "a finding is not a fact row — the two lists are separate all the way \
             to the panel"
    );
    assert_eq!(
        dossier
            .evidence
            .iter()
            .map(|e| (e.text.as_str(), e.provenance.as_str(), e.gathered_at_tick))
            .collect::<Vec<_>>(),
        vec![
            ("world.probe.evidence.brief", "briefing", 1),
            ("world.probe.evidence.scan", "scan", 400),
            ("world.probe.evidence.foreman", "dialogue", 900),
        ],
        "gather order, never re-sorted by provenance or by tick"
    );
}

/// The provenance crosses the wire under its own script name, so a
/// scenario's `provenance: "scan"`, the client's `PROVENANCE_LABELS` key and
/// a save all spell it identically. Asserted over the whole vocabulary, so a
/// fifth kind cannot ship with a name the client has never heard of.
#[test]
fn every_provenance_reaches_the_wire_under_its_own_script_name() {
    let mut s = subject();
    s.evidence = EvidenceProvenance::ALL
        .iter()
        .enumerate()
        .map(|(i, p)| finding(&format!("world.probe.{i}"), *p, i as u64))
        .collect();
    assert_eq!(
        project(&s)
            .evidence
            .iter()
            .map(|e| e.provenance.clone())
            .collect::<Vec<_>>(),
        EvidenceProvenance::ALL
            .iter()
            .map(|p| p.as_str().to_string())
            .collect::<Vec<_>>()
    );
}

/// **AC5, the hidden-truth guarantee restated with evidence in the port.**
///
/// The withheld condition is still structurally unreachable — the input is
/// still `InfrastructureSnapshot::from_state`'s `None` — and appending a
/// finding does not change that by one row. A crew who learned something
/// about this structure learned exactly what the scenario said they learned;
/// the number it is keeping back is still on nothing.
#[test]
fn appending_evidence_does_not_open_a_path_for_the_withheld_condition() {
    use crate::infrastructure::{InfrastructureConfig, InfrastructureState};

    let hidden = InfrastructureState::from_config(&InfrastructureConfig {
        condition_max: 100.0,
        condition: Some(31.0),
        publish: false,
        ..InfrastructureConfig::default()
    });

    let mut s = subject();
    s.condition = InfrastructureSnapshot::from_state(&hidden)
        .as_ref()
        .map(|published| SubjectCondition::from_published(published, |_| None, |_| None));
    s.evidence = vec![finding(
        "world.probe.evidence.scan",
        EvidenceProvenance::Scan,
        400,
    )];

    let dossier = project(&s);
    assert_eq!(dossier.evidence.len(), 1, "the crew did learn something");
    assert!(
        dossier.facts.is_empty(),
        "and the sheet still carries no condition row"
    );
    assert!(
        !dossier
            .facts
            .iter()
            .any(|f| matches!(f.value, DossierValue::Fraction(_))),
        "the withheld 0.31 rides on no fact"
    );
    assert!(
        !dossier
            .evidence
            .iter()
            .any(|e| e.text.contains("31") || e.provenance.contains("31")),
        "and nothing about it leaked into the evidence list either — an entry \
             carries what a SCENARIO said the crew found, and nothing this module read"
    );
}
