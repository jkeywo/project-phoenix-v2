//! AI doctrine-pool projector integration + determinism guard (issue #1149,
//! PRD #1144).
//!
//! Two claims the pure unit tests in `debug::ai_state` cannot make:
//!
//! 1. **The projection works end-to-end through the Bevy system.** Driving
//!    `publish_ai_doctrine` over a ship whose viewscreen blackboard carries a
//!    scored-objective pool produces a capture whose JSON names every candidate
//!    with its score, directive and resolved target, and marks the chosen
//!    directive — the "why the AI picked what it picked" the surface exists for,
//!    read off the authoritative viewscreen blackboard.
//!
//! 2. **Enabling capture never moves the digest.** Two seeded headless runs of
//!    the same world — one with the AI-doctrine flag on, one off — fold to a
//!    byte-identical authoritative-state digest. Capture is a read-only
//!    projection off the doctrine blackboards; enabling it is inert to the fold.
//!    This follows the seeded-headless prior art in `tests/station_activity.rs`.

#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]

use bevy::prelude::*;

use project_phoenix::core::messages::{
    AiDirective, ObjectiveSnapshot, ObjectiveSource, ObjectiveStatus, ScoredObjective,
    SystemAffinity, SystemBlackboard, SystemId, ViewscreenBlackboard,
};
use project_phoenix::debug::ai_state::publish_ai_doctrine;
use project_phoenix::debug::{AiDoctrineCapture, DebugAiDoctrineEnabled};
use project_phoenix::entities::config::BehaviourConfig;
use project_phoenix::entities::spawner::{BehaviourSection, EntityName, EntityUuid};
use project_phoenix::headless::{build_headless_app, run, HeadlessArgs};
use project_phoenix::server_app::ShipSystemBlackboards;
use project_phoenix::ship::system_registry::viewscreen_system_id;
use project_phoenix::sim_digest::world_digest;
use project_phoenix::sim_tick::SimTick;
use std::collections::{BTreeMap, HashMap};

// ── The projection, end-to-end through the system ────────────────────────────

fn obj(
    id: &str,
    score: f32,
    directive: AiDirective,
    relevance: Vec<SystemAffinity>,
) -> ScoredObjective {
    ScoredObjective {
        id: id.to_string(),
        score,
        directive,
        source: ObjectiveSource::Doctrine,
        relevance,
        snapshot: ObjectiveSnapshot {
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

/// A `ShipSystemBlackboards` whose viewscreen entry carries `scored` — the exact
/// place `aggregate_doctrine_blackboards` publishes the pool.
fn blackboards_with_pool(scored: Vec<ScoredObjective>) -> ShipSystemBlackboards {
    let mut map: HashMap<SystemId, SystemBlackboard> = HashMap::new();
    map.insert(
        viewscreen_system_id(),
        SystemBlackboard::Viewscreen(ViewscreenBlackboard {
            red_alert: false,
            hull_integrity_pct: 100.0,
            last_damage_taken_secs: None,
            last_weapon_fired_secs: None,
            last_attacker_uuid: None,
            scored_objectives: scored,
            combat_lock: None,
            science_target: None,
        }),
    );
    ShipSystemBlackboards(map)
}

/// Given a `BehaviourSection` ship whose viewscreen pool has a winning kill and a
/// lower-scored patrol, the published JSON names both candidates with their
/// score/directive/target and reports the kill as the chosen directive.
#[test]
fn system_projects_every_candidate_and_the_chosen_directive() {
    let pool = vec![
        obj(
            "kill",
            38.0,
            AiDirective::Destroy {
                target: "Ashrender".into(),
            },
            vec![SystemAffinity::Weapons],
        ),
        obj(
            "patrol",
            12.0,
            AiDirective::Patrol {
                anchors: vec!["picket".into()],
                loop_path: true,
            },
            vec![SystemAffinity::Helm],
        ),
    ];

    let mut app = App::new();
    app.insert_resource(SimTick(9));
    app.init_resource::<AiDoctrineCapture>();
    app.world_mut().spawn((
        blackboards_with_pool(pool),
        BehaviourSection(BehaviourConfig::default()),
        EntityName("Harrow".into()),
        EntityUuid("uuid-harrow".into()),
    ));
    app.add_systems(Update, publish_ai_doctrine);
    app.update();

    let json = app
        .world()
        .resource::<AiDoctrineCapture>()
        .0
        .clone()
        .expect("the system must publish a payload");

    // The versioned envelope and the ship the pool belongs to.
    assert!(json.contains("\"schema_version\":1"), "got {json}");
    assert!(json.contains("\"tick\":9"), "got {json}");
    assert!(json.contains("\"ship\":\"Harrow\""), "got {json}");
    assert!(json.contains("\"uuid\":\"uuid-harrow\""), "got {json}");

    // Every candidate with its score, directive and resolved target.
    assert!(json.contains("\"id\":\"kill\""), "got {json}");
    assert!(json.contains("\"score\":38.0"), "got {json}");
    assert!(
        json.contains("\"directive\":\"Destroy(Ashrender)\""),
        "got {json}"
    );
    assert!(json.contains("\"target\":\"Ashrender\""), "got {json}");
    assert!(json.contains("\"id\":\"patrol\""), "got {json}");
    assert!(
        json.contains("\"directive\":\"Patrol(picket loop)\""),
        "got {json}"
    );

    // The chosen directive is the top positively-scored real one.
    assert!(json.contains("\"chosen\":{"), "got {json}");
    // The chosen block names the kill, not the patrol.
    let chosen_at = json.find("\"chosen\":{").expect("chosen present");
    let chosen = &json[chosen_at..chosen_at + 120.min(json.len() - chosen_at)];
    assert!(chosen.contains("Destroy(Ashrender)"), "chosen was {chosen}");
}

// ── Determinism: enabling capture never moves the digest ─────────────────────

/// A fixed seed so both runs walk the identical RNG stream — the whole point.
const SEED: u64 = 0x4149_444f_4354_524e; // "AIDOCTRN"
/// Long enough to reach `InProgress` and let the doctrine aggregator score
/// pools, short enough to keep the test quick.
const TICKS: u64 = 600;

/// Build and run one seeded headless run, optionally with AI-doctrine capture
/// enabled. Returns the final authoritative-state digest and the captured JSON.
fn run_once(capture_enabled: bool) -> (u64, Option<String>) {
    let args = HeadlessArgs {
        seed: Some(SEED),
        deterministic: true,
        max_ticks: TICKS,
        ..Default::default()
    };
    let mut app = build_headless_app(&args).expect("headless app should build");
    if capture_enabled {
        // Overrides the default `false` `DebugPlugin` installed, turning the
        // flag-gated JSON publish on for every InProgress tick of this run.
        app.insert_resource(DebugAiDoctrineEnabled(true));
    }
    run(&mut app, TICKS);
    let digest = world_digest(app.world());
    let captured = app.world().resource::<AiDoctrineCapture>().0.clone();
    (digest, captured)
}

/// The AC guard: a seeded sweep produces byte-identical digests with capture on
/// and off. Enabling debug output is a read-only projection off the doctrine
/// blackboards that cannot perturb the simulation or its digest.
#[test]
fn enabling_capture_leaves_the_seeded_digest_identical() {
    let (digest_off, captured_off) = run_once(false);
    let (digest_on, captured_on) = run_once(true);

    assert_eq!(
        digest_off, digest_on,
        "enabling AI-doctrine capture moved the authoritative-state digest \
         — capture must be a read-only projection off authoritative state"
    );

    // The gating is real, not vacuous: capture off writes nothing, capture on
    // writes a versioned payload.
    assert!(
        captured_off.is_none(),
        "capture disabled must publish nothing"
    );
    let json = captured_on.expect("capture enabled must publish a payload");
    assert!(
        json.contains("\"schema_version\":1"),
        "the captured payload must carry the schema version; got: {json}"
    );
    assert!(
        json.contains("\"ships\""),
        "the captured payload must carry the per-ship doctrine surface; got: {json}"
    );
    assert!(
        json.contains("\"hosts\""),
        "the captured payload must carry the per-host policy surface (issue #1152); got: {json}"
    );
}

// ── The per-host policy surface, end-to-end through the system (issue #1152) ──

use project_phoenix::ai::policy::{AiPolicyRuntimeState, BlockedTransition, CommittedTransition};
use project_phoenix::ship::helm_ai::HelmBoostAiPolicyState;

/// Driving `publish_ai_doctrine` over a ship carrying a STATEFUL helm boost
/// policy-state component produces a capture whose `hosts` surface names that
/// host's current state, memory, the transition it committed and the guard
/// blocking the one it did not — the "stop being a black box" the surface exists
/// for, read off the authoritative runtime state.
#[test]
fn system_projects_stateful_host_policy_machines() {
    let mut memory = project_phoenix::world::flags::AiPolicyMemory::new();
    memory.set("engagements", 1.0);
    let runtime = AiPolicyRuntimeState {
        current: "surge".into(),
        entered_at_secs: 2.0,
        memory,
        last_transition: Some(CommittedTransition {
            from: "cruise".into(),
            to: "surge".into(),
            guard: "fact(hazard_urgency) > param(surge)".into(),
            at_secs: 2.0,
        }),
        blocked_transition: Some(BlockedTransition {
            from: "surge".into(),
            to: "cruise".into(),
            guard: "state_time >= param(dwell)".into(),
        }),
    };

    let mut app = App::new();
    app.insert_resource(SimTick(5));
    app.init_resource::<AiDoctrineCapture>();
    app.world_mut().spawn((
        HelmBoostAiPolicyState(runtime),
        EntityName("Harrow".into()),
        EntityUuid("uuid-harrow".into()),
    ));
    app.add_systems(Update, publish_ai_doctrine);
    app.update();

    let json = app
        .world()
        .resource::<AiDoctrineCapture>()
        .0
        .clone()
        .expect("the system must publish a payload");

    // The host is named off the AI host registry, in the current state.
    assert!(json.contains("\"host\":\"Helm boost\""), "got {json}");
    assert!(json.contains("\"state\":\"surge\""), "got {json}");
    assert!(json.contains("\"ship\":\"Harrow\""), "got {json}");
    // Memory reading, the committed transition, and the blocking guard.
    assert!(json.contains("\"key\":\"engagements\""), "got {json}");
    assert!(json.contains("\"last_transition\""), "got {json}");
    assert!(
        json.contains("fact(hazard_urgency) > param(surge)"),
        "got {json}"
    );
    assert!(json.contains("\"blocked_transition\""), "got {json}");
    assert!(json.contains("state_time >= param(dwell)"), "got {json}");
}

/// A stateless helm axis (an empty `current`, the machine never entered a state)
/// contributes NO host row — the surface is only meaningful for a real machine.
#[test]
fn system_omits_a_stateless_host() {
    let mut app = App::new();
    app.insert_resource(SimTick(1));
    app.init_resource::<AiDoctrineCapture>();
    app.world_mut().spawn((
        HelmBoostAiPolicyState(AiPolicyRuntimeState::default()),
        EntityName("Stateless".into()),
    ));
    app.add_systems(Update, publish_ai_doctrine);
    app.update();

    let json = app
        .world()
        .resource::<AiDoctrineCapture>()
        .0
        .clone()
        .expect("the system must publish a payload");
    assert!(json.contains("\"hosts\":[]"), "got {json}");
}
