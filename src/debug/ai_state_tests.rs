use super::*;
use crate::core::messages::{
    AiDirective, ObjectiveSnapshot, ObjectiveSource, ObjectiveStatus, SystemAffinity,
};
use std::collections::BTreeMap;

fn obj(id: &str, score: f32, directive: AiDirective) -> ScoredObjective {
    ScoredObjective {
        id: id.to_string(),
        score,
        directive,
        source: ObjectiveSource::Doctrine,
        relevance: vec![SystemAffinity::Weapons],
        snapshot: ObjectiveSnapshot {
            progress: None,
            unassigned: false,
            id: id.to_string(),
            text: String::new(),
            text_params: BTreeMap::new(),
            mandatory: true,
            status: ObjectiveStatus::Active,
            targets: Vec::new(),
            source: ObjectiveSource::Doctrine,
        },
    }
}

/// The whole point: every candidate carries its score, directive and resolved
/// target, and the chosen directive is the top positively-scored real one.
#[test]
fn ship_doctrine_carries_score_directive_and_target_for_every_candidate() {
    let pool = vec![
        obj(
            "kill",
            38.0,
            AiDirective::Destroy {
                target: "Ashrender".into(),
            },
        ),
        obj(
            "patrol",
            12.0,
            AiDirective::Patrol {
                anchors: vec!["picket".into()],
                loop_path: true,
            },
        ),
    ];
    let ship = ship_doctrine("Harrow".into(), Some("uuid-1".into()), &pool);

    assert_eq!(ship.ship, "Harrow");
    assert_eq!(ship.uuid.as_deref(), Some("uuid-1"));
    // Sorted by descending score: the kill reads first.
    assert_eq!(ship.candidates.len(), 2);
    assert_eq!(ship.candidates[0].id, "kill");
    assert_eq!(ship.candidates[0].score, 38.0);
    assert_eq!(ship.candidates[0].directive, "Destroy(Ashrender)");
    assert_eq!(ship.candidates[0].target.as_deref(), Some("Ashrender"));
    assert_eq!(ship.candidates[0].source, "Doctrine");
    assert_eq!(ship.candidates[0].relevance, vec!["Weapons".to_string()]);
    assert!(ship.candidates[0].mandatory);
    assert_eq!(ship.candidates[0].status, "Active");
    // The resolved winner.
    let chosen = ship.chosen.expect("a real directive should be chosen");
    assert_eq!(chosen.id, "kill");
    assert_eq!(chosen.directive, "Destroy(Ashrender)");
    assert_eq!(chosen.target.as_deref(), Some("Ashrender"));
    assert_eq!(chosen.score, 38.0);
}

/// A pool whose only directives gated out to zero has candidates but no
/// chosen directive — a tuner still sees the empty-handed pool.
#[test]
fn gated_out_pool_has_candidates_but_no_choice() {
    let pool = vec![obj(
        "kill",
        0.0,
        AiDirective::Destroy { target: "x".into() },
    )];
    let ship = ship_doctrine("Idle".into(), None, &pool);
    assert_eq!(ship.candidates.len(), 1);
    assert!(ship.chosen.is_none());
}

/// The fold sorts ships by name, so two hosts folding the same world produce
/// the same wire order.
#[test]
fn collect_sorts_ships_by_name() {
    let payload = collect_ai_doctrine(
        7,
        vec![
            ("Zephyr".to_string(), None, Vec::new()),
            ("Ashrender".to_string(), None, Vec::new()),
        ],
    );
    assert_eq!(payload.tick, 7);
    assert_eq!(payload.schema_version, DEBUG_SCHEMA_VERSION);
    assert_eq!(payload.ships.len(), 2);
    assert_eq!(payload.ships[0].ship, "Ashrender");
    assert_eq!(payload.ships[1].ship, "Zephyr");
    // The doctrine fold leaves the per-host surface (issue #1152) empty.
    assert!(payload.hosts.is_empty());
}

// ── Per-host policy view (issue #1152) ───────────────────────────────────

use crate::ai::policy::{
    AiPolicyRuntimeState, BlockedTransition as PolicyBlocked,
    CommittedTransition as PolicyCommitted,
};

/// A stateful runtime state in `surge`, entered from `cruise` at t=2s, with
/// two memory readings and a blocked return-to-`cruise` guard.
fn surge_runtime() -> AiPolicyRuntimeState {
    let mut memory = crate::world::flags::AiPolicyMemory::new();
    // Deliberately inserted out of key order; the projection must sort.
    memory.set("peak_hazard", 0.8);
    memory.set("engagements", 2.0);
    AiPolicyRuntimeState {
        current: "surge".into(),
        entered_at_secs: 2.0,
        memory,
        last_transition: Some(PolicyCommitted {
            from: "cruise".into(),
            to: "surge".into(),
            guard: "fact(hazard_urgency) > param(surge)".into(),
            at_secs: 2.0,
        }),
        blocked_transition: Some(PolicyBlocked {
            from: "surge".into(),
            to: "cruise".into(),
            guard: "state_time >= param(dwell)".into(),
        }),
    }
}

/// The projection carries the state, sorted memory, and both transitions.
#[test]
fn host_policy_view_projects_state_memory_and_transitions() {
    let view = host_policy_view(
        "Harrow".into(),
        Some("uuid-1".into()),
        "Helm boost",
        &surge_runtime(),
    );
    assert_eq!(view.ship, "Harrow");
    assert_eq!(view.uuid.as_deref(), Some("uuid-1"));
    assert_eq!(view.host, "Helm boost");
    assert_eq!(view.state, "surge");
    assert_eq!(view.entered_at_secs, 2.0);

    // Memory is sorted by key regardless of insertion order.
    assert_eq!(view.memory.len(), 2);
    assert_eq!(view.memory[0].key, "engagements");
    assert_eq!(view.memory[0].value, 2.0);
    assert_eq!(view.memory[1].key, "peak_hazard");

    let last = view.last_transition.expect("a committed transition");
    assert_eq!(last.from, "cruise");
    assert_eq!(last.to, "surge");
    assert_eq!(last.guard, "fact(hazard_urgency) > param(surge)");
    assert_eq!(last.at_secs, 2.0);

    let blocked = view.blocked_transition.expect("a blocking guard");
    assert_eq!(blocked.to, "cruise");
    assert_eq!(blocked.guard, "state_time >= param(dwell)");
}

/// A machine that has taken no transition yet carries neither record.
#[test]
fn host_policy_view_of_a_fresh_machine_has_no_transitions() {
    let runtime = AiPolicyRuntimeState {
        current: "cruise".into(),
        ..Default::default()
    };
    let view = host_policy_view("Idle".into(), None, "Helm engines", &runtime);
    assert_eq!(view.state, "cruise");
    assert!(view.memory.is_empty());
    assert!(view.last_transition.is_none());
    assert!(view.blocked_transition.is_none());
}

/// The fold sorts hosts by `(ship, uuid, host)`, so two hosts folding the
/// same world produce a byte-identical wire order.
#[test]
fn collect_host_policies_sorts_by_ship_then_host() {
    let rt = surge_runtime();
    let hosts = collect_host_policies(vec![
        ("Zephyr".to_string(), None, "Helm boost", rt.clone()),
        (
            "Ashrender".to_string(),
            Some("u".into()),
            "Helm steering",
            rt.clone(),
        ),
        (
            "Ashrender".to_string(),
            Some("u".into()),
            "Helm boost",
            rt.clone(),
        ),
    ]);
    let keys: Vec<(&str, &str)> = hosts
        .iter()
        .map(|h| (h.ship.as_str(), h.host.as_str()))
        .collect();
    assert_eq!(
        keys,
        vec![
            ("Ashrender", "Helm boost"),
            ("Ashrender", "Helm steering"),
            ("Zephyr", "Helm boost"),
        ]
    );
}

/// The shared flatten emits a row per STATEFUL axis and skips a stateless one
/// (an empty `current`) and an absent axis.
#[test]
fn host_rows_for_entity_skips_stateless_and_absent_axes() {
    let stateful = surge_runtime();
    let stateless = AiPolicyRuntimeState::default(); // current == ""
    let name = crate::entities::spawner::EntityName("Harrow".into());
    let mut out = Vec::new();
    host_rows_for_entity(
        Some(&name),
        None,
        Some(&stateful),  // engines: stateful → a row
        Some(&stateless), // steering: stateless → skipped
        None,             // boost: absent → skipped
        &mut out,
    );
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].0, "Harrow");
    assert_eq!(out[0].2, "Helm engines");
}
