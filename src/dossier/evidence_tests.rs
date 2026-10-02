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
