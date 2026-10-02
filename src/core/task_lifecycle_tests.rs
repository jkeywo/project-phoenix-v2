use super::*;

fn slot() -> TaskSlot {
    TaskSlot::new("uuid-tender", "tractor", TASK_VERB_TRACTOR_HOLD)
}

/// The coverage guard: every reason is deliberately classified and given a
/// `strings.csv` id. Adding a reason without touching `ALL` fails to compile
/// the array; adding one to `ALL` without bumping `REASON_COUNT` fails to
/// compile too — this test then forces the remaining two decisions.
#[test]
fn every_reason_is_classified_and_localizable() {
    assert_eq!(
        TaskTerminalReason::ALL.len(),
        TaskTerminalReason::REASON_COUNT
    );
    let mut labels = std::collections::BTreeSet::new();
    let mut ids = std::collections::BTreeSet::new();
    for reason in TaskTerminalReason::ALL {
        assert!(
            labels.insert(reason.as_str()),
            "duplicate terminal-reason label {:?}",
            reason.as_str()
        );
        assert!(
            ids.insert(reason.string_id()),
            "duplicate terminal-reason String Id {:?}",
            reason.string_id()
        );
        assert!(
            reason.string_id().starts_with("task.ended."),
            "{:?} must resolve through the task.ended.* family",
            reason
        );
    }
    // All four classes are reachable — a vocabulary that could never report
    // a cancellation would satisfy every other assertion here.
    let classes: std::collections::BTreeSet<TaskOutcome> = TaskTerminalReason::ALL
        .iter()
        .map(|r| r.outcome())
        .collect();
    assert_eq!(classes.len(), 4, "{classes:?}");
}

/// Only the two reasons a HOLD can give for a subject it once had are
/// re-read as a destruction. `NoSuchTarget` names a contact that was never
/// there, and must not be turned into a hull the timeline mourns.
#[test]
fn only_a_lost_hold_can_be_re_read_as_a_destroyed_subject() {
    let masking: Vec<&str> = TaskTerminalReason::ALL
        .iter()
        .filter(|r| r.masks_target_loss())
        .map(|r| r.as_str())
        .collect();
    assert_eq!(masking, vec!["target_lost", "out_of_range"]);
}

/// The key is a pure function of state the tick already decided, and the
/// ordinal is what separates a restart from the activation it replaced.
#[test]
fn a_restart_of_the_same_slot_mints_a_distinct_key() {
    let mut registry = TaskLifecycles::default();
    let first = registry.begin(slot(), Some("uuid-hulk".into()), None, 10);
    assert_eq!(
        first.key.as_str(),
        "uuid-tender/tractor/tractor_hold/uuid-hulk#0"
    );
    assert!(registry.end(&slot()).is_some());
    let second = registry.begin(slot(), Some("uuid-hulk".into()), None, 40);
    assert_eq!(
        second.key.as_str(),
        "uuid-tender/tractor/tractor_hold/uuid-hulk#1",
        "the same operator holding the same hull a second time is a SECOND task"
    );
    assert_ne!(first.key, second.key);
}

/// Two different slots run at once and keep separate identities — the
/// "simultaneous tasks stay separately identifiable" half of AC3.
#[test]
fn simultaneous_slots_do_not_share_an_activation() {
    let mut registry = TaskLifecycles::default();
    let hold = registry.begin(slot(), Some("uuid-hulk".into()), None, 1);
    let scan = registry.begin(
        TaskSlot::new("uuid-tender", "sensors", TASK_VERB_SCAN),
        Some("uuid-depot".into()),
        None,
        1,
    );
    assert_eq!(registry.len(), 2);
    assert_ne!(hold.key.as_str(), scan.key.as_str());
    // …and each ordinal counts its OWN slot, so the second task is not
    // numbered as though it were the first one's restart.
    assert_eq!(hold.key.ordinal, 0);
    assert_eq!(scan.key.ordinal, 0);
}

/// A terminal request for a slot with nothing live is dropped — the whole
/// of the "exactly one terminal event" rule.
#[test]
fn ending_an_unstarted_slot_yields_nothing() {
    let mut registry = TaskLifecycles::default();
    assert!(registry.end(&slot()).is_none());
    registry.begin(slot(), None, None, 0);
    assert!(registry.end(&slot()).is_some());
    assert!(
        registry.end(&slot()).is_none(),
        "a second report of the same ending must add nothing"
    );
}

/// `take_all` is the mission-end sweep: it empties the live set in slot
/// order but keeps the counters, so a resumed slot still mints a fresh key.
#[test]
fn take_all_empties_the_live_set_but_not_the_counters() {
    let mut registry = TaskLifecycles::default();
    registry.begin(slot(), None, None, 0);
    registry.begin(
        TaskSlot::new("uuid-tender", "sensors", TASK_VERB_SCAN),
        None,
        None,
        0,
    );
    let taken = registry.take_all();
    assert_eq!(taken.len(), 2);
    assert_eq!(
        taken
            .iter()
            .map(|a| a.key.slot.system.as_str())
            .collect::<Vec<_>>(),
        vec!["sensors", "tractor"],
        "the sweep walks in slot order, never in the order tasks opened"
    );
    assert!(registry.is_empty());
    assert_eq!(registry.begin(slot(), None, None, 0).key.ordinal, 1);
}

/// The two beats share their identity and point the same way; the terminal
/// one adds the reason, the class, and the span.
#[test]
fn the_two_beats_share_one_identity() {
    let mut registry = TaskLifecycles::default();
    let activation = registry.begin(
        slot(),
        Some("uuid-hulk".into()),
        Some("engineering".into()),
        30,
    );
    let start = start_event(&activation);
    let end = terminal_event(&activation, TaskTerminalReason::Released, 90);

    assert_eq!(start.kind, NarrativeKind::TaskStarted);
    assert_eq!(end.kind, NarrativeKind::TaskCancelled);
    assert_eq!(start.id, end.id, "one activation, one key");
    assert_eq!(start.target.as_deref(), Some("uuid-hulk"));
    assert_eq!(end.target.as_deref(), Some("uuid-hulk"));
    assert_eq!(start.source.entity.as_deref(), Some("uuid-tender"));
    assert_eq!(start.source.system.as_deref(), Some("tractor"));
    assert_eq!(start.source.station.as_deref(), Some("engineering"));
    assert_eq!(
        end.detail.get("reason"),
        Some(&NarrativeValue::Text("released".into()))
    );
    assert_eq!(
        end.detail.get("reason_text"),
        Some(&NarrativeValue::Text("task.ended.released".into())),
        "the localizable half of the reason is a String Id, never prose"
    );
    assert_eq!(
        end.detail.get("outcome"),
        Some(&NarrativeValue::Text("cancelled".into()))
    );
    assert_eq!(
        end.detail.get("duration_ticks"),
        Some(&NarrativeValue::Int(60))
    );
}

/// The poll rule: an unchanged FAILURE repeats, and nothing else does.
#[test]
fn only_an_unchanged_failure_counts_as_a_repeat() {
    let mut registry = TaskLifecycles::default();
    let scan = TaskSlot::new("uuid-tender", "sensors", TASK_VERB_SCAN);
    // The activation a terminal is recorded against — the registry now keeps
    // the whole thing, because the repeat COUNT has to be reported under the
    // same key the retained beat carried.
    let recorded = |registry: &mut TaskLifecycles, target: &str| {
        registry.begin(scan.clone(), Some(target.to_string()), None, 0)
    };
    assert!(
        !registry.repeats_last_failure(&scan, Some("uuid-hulk"), TaskTerminalReason::OutOfRange),
        "a slot that has never ended anything cannot be repeating itself"
    );

    let hulk = recorded(&mut registry, "uuid-hulk");
    registry.record_terminal(&hulk, TaskTerminalReason::OutOfRange);
    assert!(registry.repeats_last_failure(
        &scan,
        Some("uuid-hulk"),
        TaskTerminalReason::OutOfRange
    ));
    // A different subject, a different reason, or a different slot are all
    // different facts.
    assert!(!registry.repeats_last_failure(
        &scan,
        Some("uuid-depot"),
        TaskTerminalReason::OutOfRange
    ));
    assert!(!registry.repeats_last_failure(
        &scan,
        Some("uuid-hulk"),
        TaskTerminalReason::Unpowered
    ));
    assert!(!registry.repeats_last_failure(
        &slot(),
        Some("uuid-hulk"),
        TaskTerminalReason::OutOfRange
    ));

    // Only the FAILED class coalesces: a completion is work that happened
    // and a cancellation is a decision somebody made, however often either
    // recurs.
    for reason in TaskTerminalReason::ALL {
        registry.record_terminal(&hulk, reason);
        assert_eq!(
            registry.repeats_last_failure(&scan, Some("uuid-hulk"), reason),
            reason.outcome() == TaskOutcome::Failed,
            "{reason:?}"
        );
    }
}

/// A coalesced repeat is COUNTED, never erased: the registry hands the total
/// back once, under the key of the terminal the repeats were folded into, so
/// issue #1341's "repeated tasks remain separately identifiable" holds
/// without the timeline growing with the cadence.
#[test]
fn coalesced_repeats_are_counted_and_drained_once() {
    let mut registry = TaskLifecycles::default();
    let scan = TaskSlot::new("uuid-tender", "sensors", TASK_VERB_SCAN);
    let activation = registry.begin(scan.clone(), Some("uuid-hulk".into()), None, 7);
    registry.record_terminal(&activation, TaskTerminalReason::OutOfRange);

    assert!(!registry.has_pending_repeats(), "nothing has repeated yet");
    assert!(registry.take_repeats(&scan).is_none());

    for _ in 0..40 {
        registry.note_repeat(&scan);
    }
    assert!(registry.has_pending_repeats());
    let (repeated, reason, count) = registry
        .take_repeats(&scan)
        .expect("40 coalesced attempts must be recoverable");
    assert_eq!(count, 40);
    assert_eq!(reason, TaskTerminalReason::OutOfRange);
    assert_eq!(
        repeated.key, activation.key,
        "the count is reported under the key of the beat it was folded into"
    );
    // Drained, not duplicated…
    assert!(registry.take_repeats(&scan).is_none());
    assert!(!registry.has_pending_repeats());
    // …but the retained terminal itself survives, so the NEXT identical
    // attempt still coalesces rather than re-opening the flood.
    assert!(registry.repeats_last_failure(
        &scan,
        Some("uuid-hulk"),
        TaskTerminalReason::OutOfRange
    ));

    // The census beat carries the count, the total including the recorded
    // attempt, and the retained activation's key.
    let event = repeats_event(&repeated, reason, count);
    assert_eq!(event.kind, NarrativeKind::TaskRepeated);
    assert!(!event.kind.is_task_lifecycle(), "it terminates nothing");
    assert_eq!(event.id, activation.key.as_str());
    assert_eq!(event.detail.get("repeats"), Some(&NarrativeValue::Int(40)));
    assert_eq!(event.detail.get("attempts"), Some(&NarrativeValue::Int(41)));
    assert_eq!(
        event.detail.get("reason"),
        Some(&NarrativeValue::Text("out_of_range".into()))
    );
}

/// A run that stops with several slots still polling reports each one's
/// count, in slot order — never in the order they happened to start.
#[test]
fn every_slots_repeats_drain_in_slot_order() {
    let mut registry = TaskLifecycles::default();
    let tractor = slot();
    let scan = TaskSlot::new("uuid-tender", "sensors", TASK_VERB_SCAN);
    // Opened tractor-first, so the drain order cannot be the insertion one.
    let held = registry.begin(tractor.clone(), Some("uuid-hulk".into()), None, 0);
    let read = registry.begin(scan.clone(), Some("uuid-hulk".into()), None, 0);
    registry.record_terminal(&held, TaskTerminalReason::OutOfRange);
    registry.record_terminal(&read, TaskTerminalReason::NoSuchTarget);
    registry.note_repeat(&tractor);
    registry.note_repeat(&scan);
    registry.note_repeat(&scan);

    let drained = registry.take_all_repeats();
    assert_eq!(
        drained
            .iter()
            .map(|(a, _, n)| (a.key.slot.system.as_str(), *n))
            .collect::<Vec<_>>(),
        vec![("sensors", 2), ("tractor", 1)],
        "slot order, and each slot's own count"
    );
    assert!(registry.take_all_repeats().is_empty(), "drained once");
}

/// A task with no subject encodes a fixed five-field key rather than a
/// shorter one, so a reader never has to count separators.
#[test]
fn a_subjectless_task_still_encodes_five_fields() {
    let mut registry = TaskLifecycles::default();
    let activation = registry.begin(slot(), None, None, 0);
    assert_eq!(
        activation.key.as_str(),
        "uuid-tender/tractor/tractor_hold/-#0"
    );
    assert!(start_event(&activation).target.is_none());
}

#[test]
fn physical_work_preserves_transition_order_and_subject_policy() {
    let start = |target: Option<&str>| TaskLifecycleRequest::Start {
        slot: slot(),
        target: target.map(str::to_owned),
    };
    let end = |reason| TaskLifecycleRequest::End {
        slot: slot(),
        reason,
    };
    let cases = [
        (false, None, Some("a"), true, vec![start(Some("a"))]),
        (true, Some("a"), Some("a"), true, vec![]),
        (
            true,
            Some("a"),
            Some("b"),
            true,
            vec![end(TaskTerminalReason::TargetLost), start(Some("b"))],
        ),
        (
            true,
            Some("a"),
            None,
            true,
            vec![end(TaskTerminalReason::TargetLost), start(None)],
        ),
        (false, None, None, true, vec![]),
        (false, None, None, false, vec![start(None)]),
        (true, Some("a"), Some("b"), false, vec![]),
    ];
    for (active, prior, target, follow_subject, expected) in cases {
        assert_eq!(
            physical_work_reports(
                slot(),
                PriorActivation {
                    active,
                    target: prior
                },
                PhysicalWork::Formed {
                    target,
                    follow_subject
                }
            ),
            expected
        );
    }
    for target in [None, Some("a")] {
        for active in [false, true] {
            let mut expected = Vec::new();
            if !active {
                expected.push(start(target));
            }
            expected.push(end(TaskTerminalReason::Unpowered));
            assert_eq!(
                physical_work_reports(
                    slot(),
                    PriorActivation { active, target },
                    PhysicalWork::Refused {
                        target,
                        reason: TaskTerminalReason::Unpowered
                    }
                ),
                expected
            );
        }
    }
}
