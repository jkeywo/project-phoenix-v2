use super::*;

/// The coverage guard: every kind is deliberately classified as timeline or
/// fold-only, and `KIND_COUNT` matches the enum. Adding a kind without
/// touching `ALL` fails to compile the array; adding one to `ALL` without
/// bumping `KIND_COUNT` fails to compile too — this test then forces the
/// remaining decision, which is what the stream is for.
#[test]
fn every_kind_declares_whether_it_reaches_the_timeline() {
    assert_eq!(NarrativeKind::ALL.len(), NarrativeKind::KIND_COUNT);
    let fold_only: Vec<&str> = NarrativeKind::ALL
        .iter()
        .filter(|k| !k.in_timeline_stream())
        .map(|k| k.as_str())
        .collect();
    assert_eq!(
        fold_only,
        vec!["report_row_updated"],
        "a kind changed its timeline policy — say why in `in_timeline_stream`'s doc"
    );
}

/// Labels are the wire vocabulary, so they must be unique and stable.
#[test]
fn kind_labels_are_unique() {
    let mut seen = std::collections::BTreeSet::new();
    for kind in NarrativeKind::ALL {
        assert!(
            seen.insert(kind.as_str()),
            "duplicate narrative kind label {:?}",
            kind.as_str()
        );
    }
    assert_eq!(seen.len(), NarrativeKind::KIND_COUNT);
}

/// Only marked-entity outcomes are authorable, and a typo is an error
/// rather than a silently different beat.
#[test]
fn parse_outcome_accepts_only_the_entity_outcomes() {
    assert_eq!(
        NarrativeKind::parse_outcome("Rescued"),
        Ok(NarrativeKind::MarkedEntityRescued)
    );
    assert_eq!(
        NarrativeKind::parse_outcome(" abandoned "),
        Ok(NarrativeKind::MarkedEntityAbandoned)
    );
    // An Objective transition is observed, never declared.
    assert!(NarrativeKind::parse_outcome("completed").is_err());
    assert!(NarrativeKind::parse_outcome("wibble").is_err());
}

/// The fates a marked entity can be given, spelled out: these are the kinds
/// whose presence tells the scripted-removal fallback that the story has
/// already said what became of this hull. Spawning is not a fate, and
/// nothing outside the marked-entity family is one — a new kind that should
/// count has to be added here deliberately.
#[test]
fn only_the_marked_entity_fates_count_as_an_outcome() {
    let fates: Vec<&str> = NarrativeKind::ALL
        .iter()
        .filter(|k| k.is_marked_entity_outcome())
        .map(|k| k.as_str())
        .collect();
    assert_eq!(
        fates,
        vec![
            "marked_entity_disabled",
            "marked_entity_destroyed",
            "marked_entity_escaped",
            "marked_entity_rescued",
            "marked_entity_abandoned",
        ]
    );
    assert!(!NarrativeKind::MarkedEntitySpawned.is_marked_entity_outcome());
    assert!(!NarrativeKind::BeatFired.is_marked_entity_outcome());
}

/// The stamped JSON carries the sequence, the fixed tick, the derived time
/// and the event's own fields — the whole of PRD #1337's per-event shape.
#[test]
fn stamped_json_carries_sequence_tick_and_time() {
    let stamped = StampedNarrativeEvent {
        seq: 7,
        tick: 420,
        sim_t: 14.0,
        event: NarrativeEvent::new(NarrativeKind::ObjectivePosted, "reach_axiom")
            .text("text", "world.probe.objective.reach_axiom")
            .detail("mandatory", NarrativeValue::Flag(true)),
    };
    let json = stamped.to_json();
    assert!(
        json.starts_with("{\"seq\":7,\"tick\":420,\"sim_t\":14.0000,"),
        "{json}"
    );
    assert!(json.contains("\"kind\":\"objective_posted\""), "{json}");
    assert!(json.contains("\"id\":\"reach_axiom\""), "{json}");
    // The String Id passes through verbatim — never resolved to English.
    assert!(
        json.contains("\"text\":\"world.probe.objective.reach_axiom\""),
        "{json}"
    );
    assert!(json.contains("\"mandatory\":true"), "{json}");
    // No source and no target named.
    assert!(json.contains("\"source\":null"), "{json}");
    assert!(json.contains("\"target\":null"), "{json}");
}

/// The ndjson envelope matches every other headless stream record, so a
/// consumer splitting lines on `tick` sees narrative beats in tick order
/// beside the balance events.
#[test]
fn stream_json_uses_the_shared_envelope() {
    let stamped = StampedNarrativeEvent {
        seq: 0,
        tick: 12,
        sim_t: 0.4,
        event: NarrativeEvent::new(NarrativeKind::BeatFired, "storm_hits"),
    };
    let line = stamped.to_stream_json();
    assert!(
        line.starts_with("{\"tick\":12,\"sim_t\":0.4000,\"narrative\":{\"seq\":0,"),
        "{line}"
    );
}

/// A source with an entity/station/system encodes all three, so an event
/// that came off a console can be attributed to it.
#[test]
fn actor_encodes_the_three_source_axes() {
    let actor = NarrativeActor {
        entity: Some("uuid-1".into()),
        station: Some("tactical".into()),
        system: Some("comms".into()),
    };
    assert_eq!(
        actor.to_json(),
        "{\"entity\":\"uuid-1\",\"station\":\"tactical\",\"system\":\"comms\"}"
    );
    assert_eq!(NarrativeActor::default().to_json(), "null");
}

/// The fold is pure and order-preserving, and counts every kind it saw.
#[test]
fn fold_preserves_order_and_counts_kinds() {
    let events = vec![
        StampedNarrativeEvent {
            seq: 0,
            tick: 1,
            sim_t: 0.0,
            event: NarrativeEvent::new(NarrativeKind::ObjectivePosted, "a"),
        },
        StampedNarrativeEvent {
            seq: 1,
            tick: 2,
            sim_t: 0.1,
            event: NarrativeEvent::new(NarrativeKind::ObjectivePosted, "b"),
        },
        StampedNarrativeEvent {
            seq: 2,
            tick: 3,
            sim_t: 0.2,
            event: NarrativeEvent::new(NarrativeKind::ObjectiveCompleted, "a"),
        },
    ];
    let timeline = fold_narrative(&events);
    assert_eq!(
        timeline.events.iter().map(|e| e.seq).collect::<Vec<_>>(),
        vec![0, 1, 2]
    );
    assert_eq!(timeline.counts_by_kind.get("objective_posted"), Some(&2));
    assert_eq!(timeline.counts_by_kind.get("objective_completed"), Some(&1));
    assert_eq!(timeline.counts_by_kind.len(), 2);
}

/// An empty run still produces valid JSON with an explicit zero, so a
/// consumer never has to distinguish "absent" from "nothing happened".
#[test]
fn empty_timeline_encodes_as_an_explicit_zero() {
    let json = fold_narrative(&[]).to_json();
    assert_eq!(
        json,
        "{\"count\": 0, \"counts_by_kind\": {}, \"events\": []}"
    );
}
