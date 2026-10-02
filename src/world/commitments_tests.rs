use super::*;

fn made() -> CommitmentLedger {
    let mut ledger = CommitmentLedger::default();
    ledger
        .record(
            "safe_passage",
            "skyway_strike_committee",
            "world.probe.commitment.safe_passage.terms",
            "world.probe.commitment.safe_passage.resolves",
            120,
        )
        .expect("a fresh id records");
    ledger
        .record(
            "surface_records",
            "skyway_strike_committee",
            "world.probe.commitment.surface_records.terms",
            "world.probe.commitment.surface_records.resolves",
            180,
        )
        .expect("a second fresh id records");
    ledger
}

// ── AC1: id, party, terms and resolution condition; duplicates are an error

#[test]
fn a_recorded_promise_keeps_its_party_terms_and_condition() {
    let ledger = made();
    let promise = ledger
        .get("safe_passage")
        .expect("the id is the lookup key");
    assert_eq!(
        promise.made_to, "skyway_strike_committee",
        "the party travels as the script wrote it — not resolved to a hull"
    );
    assert_eq!(promise.terms, "world.probe.commitment.safe_passage.terms");
    assert_eq!(
        promise.resolves_when, "world.probe.commitment.safe_passage.resolves",
        "what would count as keeping it is data, not an implication of a handler"
    );
    assert_eq!(promise.state, CommitmentState::Open);
    assert_eq!(
        promise.made_at_tick, 120,
        "stamped with the tick it was made"
    );
    assert_eq!(
        promise.resolved_at_tick, None,
        "an open promise has no resolution tick"
    );
    assert_eq!(
        ledger.records.len(),
        2,
        "recording is append-only, in the order the promises were made"
    );
}

#[test]
fn a_duplicate_id_is_an_error_rather_than_an_overwrite() {
    let mut ledger = made();
    assert_eq!(
        ledger.record(
            "safe_passage",
            "somebody_else",
            "other.terms",
            "other.when",
            300
        ),
        Err(DuplicateCommitment {
            id: "safe_passage".into()
        }),
        "a second promise under a live id is refused"
    );
    assert_eq!(
        ledger.get("safe_passage").map(|c| c.made_to.as_str()),
        Some("skyway_strike_committee"),
        "and the terms the crew were actually given survive the attempt"
    );

    // A RESOLVED promise still occupies its id: the run made it.
    ledger.resolve("safe_passage", CommitmentOutcome::Kept, 400);
    assert!(
        ledger
            .record("safe_passage", "anyone", "t", "w", 500)
            .is_err(),
        "re-using a spent id would erase that the promise was ever kept"
    );
    assert_eq!(
        ledger.records.len(),
        2,
        "nothing was appended by either attempt"
    );
}

// ── AC3: three states; kept/broken write distinct campaign flags ──────────

#[test]
fn keeping_a_promise_writes_its_kept_flag_and_stamps_the_tick() {
    let mut ledger = made();
    assert_eq!(
        ledger.resolve("safe_passage", CommitmentOutcome::Kept, 900),
        Some("commitment.safe_passage.kept".to_string()),
        "resolution names the campaign flag the caller must set"
    );
    let promise = ledger.get("safe_passage").expect("still on the books");
    assert_eq!(promise.state, CommitmentState::Kept);
    assert_eq!(promise.resolved_at_tick, Some(900));
}

#[test]
fn breaking_a_promise_writes_a_different_flag_from_keeping_one() {
    let mut ledger = made();
    assert_eq!(
        ledger.resolve("surface_records", CommitmentOutcome::Broken, 1200),
        Some("commitment.surface_records.broken".to_string()),
    );
    assert_ne!(
        broken_flag("surface_records"),
        kept_flag("surface_records"),
        "a trigger firing on 'the promise resolved' must be able to tell which way"
    );
    assert_eq!(
        ledger.get("surface_records").map(|c| c.state),
        Some(CommitmentState::Broken)
    );
}

#[test]
fn not_yet_kept_is_never_the_same_answer_as_failed() {
    let mut ledger = made();
    assert_eq!(ledger.state_of("safe_passage"), "open");
    assert_eq!(
        ledger.state_of("never_promised"),
        "unknown",
        "a promise that was never made is a fourth answer, and it is the \
             duplicate guard"
    );

    ledger.resolve("safe_passage", CommitmentOutcome::Broken, 900);
    assert_eq!(ledger.state_of("safe_passage"), "broken");
    assert_eq!(
        ledger.state_of("surface_records"),
        "open",
        "an unfinished errand is not a betrayal"
    );
}

#[test]
fn resolving_twice_leaves_the_first_resolution_standing() {
    let mut ledger = made();
    ledger.resolve("safe_passage", CommitmentOutcome::Kept, 900);
    assert_eq!(
        ledger.resolve("safe_passage", CommitmentOutcome::Broken, 1500),
        None,
        "a late handler cannot convert a kept promise into a broken one"
    );
    let promise = ledger.get("safe_passage").expect("still on the books");
    assert_eq!(promise.state, CommitmentState::Kept);
    assert_eq!(
        promise.resolved_at_tick,
        Some(900),
        "and the tick it was actually settled on stands"
    );
    assert_eq!(
        ledger.resolve("never_promised", CommitmentOutcome::Kept, 900),
        None,
        "an unknown id resolves nothing and writes no flag"
    );
}

// ── AC6: the ledger is inspectable ───────────────────────────────────────

#[test]
fn open_promises_are_listed_oldest_first_and_resolved_ones_drop_out() {
    let mut ledger = made();
    assert_eq!(
        ledger.open().map(|c| c.id.as_str()).collect::<Vec<_>>(),
        vec!["safe_passage", "surface_records"],
        "still owed, in the order the captain gave their word"
    );

    ledger.resolve("safe_passage", CommitmentOutcome::Kept, 900);
    assert_eq!(
        ledger.open().map(|c| c.id.as_str()).collect::<Vec<_>>(),
        vec!["surface_records"],
        "a settled promise is no longer owed"
    );
    assert_eq!(
        ledger.records.len(),
        2,
        "but it is still on the books — the run made it"
    );
    assert!(!ledger.is_empty());
    assert!(CommitmentLedger::default().is_empty());
}

// ── The mutation front-end the script surface and the adapter share ──────

#[test]
fn apply_dispatches_both_mutations_through_the_same_body() {
    let mut ledger = CommitmentLedger::default();
    let record = CommitmentChange {
        id: "safe_passage".into(),
        mutation: CommitmentMutation::Record {
            made_to: "skyway_strike_committee".into(),
            terms: "t".into(),
            resolves_when: "w".into(),
        },
    };
    assert_eq!(
        ledger.apply(&record, 120),
        Ok(None),
        "recording asks the caller to write no flag"
    );
    assert_eq!(ledger.state_of("safe_passage"), "open");

    assert_eq!(
        ledger.apply(&record, 130),
        Err(DuplicateCommitment {
            id: "safe_passage".into()
        }),
        "and the duplicate rule holds through the buffered front-end too"
    );

    assert_eq!(
        ledger.apply(
            &CommitmentChange {
                id: "safe_passage".into(),
                mutation: CommitmentMutation::Resolve {
                    outcome: CommitmentOutcome::Kept
                },
            },
            900,
        ),
        Ok(Some("commitment.safe_passage.kept".to_string())),
    );
}

// ── AC9: the state is serialisable so a save can carry it ────────────────

#[test]
fn the_ledger_round_trips_through_serialization() {
    let mut ledger = made();
    ledger.resolve("safe_passage", CommitmentOutcome::Kept, 900);
    ledger.resolve("surface_records", CommitmentOutcome::Broken, 1500);

    let json = serde_json::to_string(&ledger).expect("serialises");
    let restored: CommitmentLedger = serde_json::from_str(&json).expect("deserialises");
    assert_eq!(
        restored, ledger,
        "every field a run writes — party, terms, condition, state and both \
             tick stamps — round-trips"
    );
}
