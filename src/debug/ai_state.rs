//! AI-state projection — the per-ship doctrine pool surface (issue #1149,
//! PRD #1144).
//!
//! # What this is
//!
//! A read-only projection of the scored-objective doctrine pool each AI ship
//! carries on its viewscreen blackboard. The pool is computed every tick by
//! [`crate::ai::server::aggregate_doctrine_blackboards`] and already crosses the
//! wire on the viewscreen blackboard; it is rendered nowhere today. This surface
//! makes it diagnostic: for each AI-controlled ship it names every candidate
//! objective with its score, source and relevance, the chosen directive, and the
//! resolved target — so an AI tuner can see *why* the AI picked what it picked.
//!
//! # Reuses #1146's decision-trace helpers
//!
//! The chosen directive, each candidate's directive label, and each resolved
//! target are computed with the SAME [`crate::ai::decision_trace`] functions the
//! `ai`-log directive-change events use. A doctrine pool projected here and the
//! same pool logged by the doctrine emitter therefore agree by construction — no
//! second, drifting copy of "which directive won" or "what does this directive
//! name".
//!
//! # Determinism
//!
//! The counters are absent — there is nothing to record, only to read. Both the
//! flag-gated [`publish_ai_doctrine`] system and the one-shot projection the
//! headless report runs read the already-authoritative viewscreen pool and clone
//! it; neither touches `SimRng`, mutates the world, or is folded by
//! `world_digest` (the two resources below are declared `StateClass::Presentation`
//! at `DebugPlugin::build`). Enabling capture therefore leaves a seeded digest
//! byte-identical — proven by `tests/ai_doctrine.rs`.

use bevy::prelude::*;

use crate::ai::decision_trace;
use crate::core::messages::{ScoredObjective, SystemBlackboard};
use crate::debug::payload::{
    AiStatePayload, DoctrineCandidate, DoctrineChoice, HostBlockedView, HostMemoryEntry,
    HostPolicyView, HostTransitionView, ShipDoctrine, DEBUG_SCHEMA_VERSION,
};
use crate::server_app::ShipSystemBlackboards;

/// Project one scored objective into its wire form.
///
/// The directive label and resolved target come straight from the #1146
/// decision-trace helpers, so a candidate here reads exactly as the doctrine log
/// line does.
pub fn candidate(scored: &ScoredObjective) -> DoctrineCandidate {
    DoctrineCandidate {
        id: scored.id.clone(),
        score: scored.score,
        source: format!("{:?}", scored.source),
        relevance: scored.relevance.iter().map(|a| format!("{a:?}")).collect(),
        directive: decision_trace::directive_label(&scored.directive),
        target: decision_trace::directive_target(&scored.directive).map(str::to_string),
        mandatory: scored.snapshot.mandatory,
        status: format!("{:?}", scored.snapshot.status),
    }
}

/// Project one ship's whole doctrine pool.
///
/// `candidates` is sorted by descending score then id — deterministic, and the
/// winner reads first. `chosen` is [`decision_trace::top_directive`], the same
/// highest-positively-scored real directive the helm and weapons AI serve.
pub fn ship_doctrine(
    ship: String,
    uuid: Option<String>,
    scored: &[ScoredObjective],
) -> ShipDoctrine {
    let mut candidates: Vec<DoctrineCandidate> = scored.iter().map(candidate).collect();
    candidates.sort_by(|a, b| b.score.total_cmp(&a.score).then_with(|| a.id.cmp(&b.id)));
    let chosen = decision_trace::top_directive(scored).map(|o| DoctrineChoice {
        id: o.id.clone(),
        directive: decision_trace::directive_label(&o.directive),
        target: decision_trace::directive_target(&o.directive).map(str::to_string),
        score: o.score,
    });
    ShipDoctrine {
        ship,
        uuid,
        chosen,
        candidates,
    }
}

/// Fold a set of AI ships into the whole payload.
///
/// Pure and Bevy-free, so the headless projector tests can assert the payload
/// contents from an authored pool without an `App`. The Bevy system and the
/// headless report builder both funnel their ships through here, so they cannot
/// disagree. Ships are sorted by `(ship, uuid)` for a stable wire order.
pub fn collect_ai_doctrine(
    tick: u64,
    ships: impl IntoIterator<Item = (String, Option<String>, Vec<ScoredObjective>)>,
) -> AiStatePayload {
    let mut ships: Vec<ShipDoctrine> = ships
        .into_iter()
        .map(|(name, uuid, scored)| ship_doctrine(name, uuid, &scored))
        .collect();
    ships.sort_by(|a, b| a.ship.cmp(&b.ship).then_with(|| a.uuid.cmp(&b.uuid)));
    AiStatePayload {
        schema_version: DEBUG_SCHEMA_VERSION,
        tick,
        ships,
        // The per-host policy view is folded separately by
        // [`collect_host_policies`] and set on the payload by the caller, so the
        // doctrine fold (issue #1149) is untouched by issue #1152.
        hosts: Vec::new(),
    }
}

/// Project one stateful fine-system AI host's policy machine into its wire form
/// (issue #1152).
///
/// A read-only clone of the authoritative [`AiPolicyRuntimeState`]: the current
/// state, its private memory (sorted by key for a stable wire order), and the
/// last-committed / most-recently-blocked transitions the machine tick recorded.
/// `host` is the registry name from [`crate::entities::ai_flag_hosts`], so the
/// view is keyed off that registry rather than a new parallel index.
pub fn host_policy_view(
    ship: String,
    uuid: Option<String>,
    host: &str,
    runtime: &crate::ai::policy::AiPolicyRuntimeState,
) -> HostPolicyView {
    let mut memory: Vec<HostMemoryEntry> = runtime
        .memory
        .iter()
        .map(|(key, value)| HostMemoryEntry {
            key: key.to_string(),
            value,
        })
        .collect();
    memory.sort_by(|a, b| a.key.cmp(&b.key));
    HostPolicyView {
        ship,
        uuid,
        host: host.to_string(),
        state: runtime.current.clone(),
        entered_at_secs: runtime.entered_at_secs,
        memory,
        last_transition: runtime
            .last_transition
            .as_ref()
            .map(|t| HostTransitionView {
                from: t.from.clone(),
                to: t.to.clone(),
                guard: t.guard.clone(),
                at_secs: t.at_secs,
            }),
        blocked_transition: runtime
            .blocked_transition
            .as_ref()
            .map(|b| HostBlockedView {
                from: b.from.clone(),
                to: b.to.clone(),
                guard: b.guard.clone(),
            }),
    }
}

/// Fold a set of stateful fine-system AI hosts into the per-host policy view
/// (issue #1152).
///
/// Pure and Bevy-free, so the headless projector tests can assert the surface
/// from an authored runtime state without an `App`. The Bevy system and the
/// headless report both funnel their hosts through here, so they cannot
/// disagree. Sorted by `(ship, uuid, host)` for a byte-identical wire order.
pub fn collect_host_policies(
    hosts: impl IntoIterator<
        Item = (
            String,
            Option<String>,
            &'static str,
            crate::ai::policy::AiPolicyRuntimeState,
        ),
    >,
) -> Vec<HostPolicyView> {
    let mut hosts: Vec<HostPolicyView> = hosts
        .into_iter()
        .map(|(ship, uuid, host, runtime)| host_policy_view(ship, uuid, host, &runtime))
        .collect();
    hosts.sort_by(|a, b| {
        a.ship
            .cmp(&b.ship)
            .then_with(|| a.uuid.cmp(&b.uuid))
            .then_with(|| a.host.cmp(&b.host))
    });
    hosts
}

/// Flatten one ship's three helm policy-state axes into the owned
/// `(ship, uuid, host, runtime)` rows [`collect_host_policies`] folds (issue
/// #1152).
///
/// Shared by the live publish system and the headless report, so the two project
/// the identical per-host surface off the same authoritative
/// [`AiPolicyRuntimeState`]s. Each axis is named by its
/// [`crate::entities::ai_flag_hosts`] registry host, keeping the view keyed off
/// that registry. Only a STATEFUL axis contributes a row: a stateless policy's
/// machine tick never enters a state, so its `current` stays the empty default
/// and it has no policy machine to show.
pub fn host_rows_for_entity(
    name: Option<&crate::entities::spawner::EntityName>,
    uuid: Option<&crate::entities::spawner::EntityUuid>,
    engines: Option<&crate::ai::policy::AiPolicyRuntimeState>,
    steering: Option<&crate::ai::policy::AiPolicyRuntimeState>,
    boost: Option<&crate::ai::policy::AiPolicyRuntimeState>,
    out: &mut Vec<(
        String,
        Option<String>,
        &'static str,
        crate::ai::policy::AiPolicyRuntimeState,
    )>,
) {
    let ship = name
        .map(|n| n.0.clone())
        .unwrap_or_else(|| "<unnamed>".to_string());
    let uuid = uuid.map(|u| u.0.clone());
    for (host, runtime) in [
        (crate::entities::ai_flag_hosts::HELM_ENGINES.system, engines),
        (
            crate::entities::ai_flag_hosts::HELM_STEERING.system,
            steering,
        ),
        (crate::entities::ai_flag_hosts::HELM_BOOST.system, boost),
    ] {
        let Some(runtime) = runtime else { continue };
        if runtime.current.is_empty() {
            continue;
        }
        out.push((ship.clone(), uuid.clone(), host, runtime.clone()));
    }
}

/// This ship's scored-objective pool, read off its viewscreen blackboard.
///
/// Empty when the ship has no viewscreen entry (a static point-defence platform
/// authors a viewscreen only for its combat lock, with no doctrine pool). A
/// clone: this is a read-only projection, it never borrows into the authoritative
/// blackboard beyond the read.
pub fn ship_scored_pool(blackboards: &ShipSystemBlackboards) -> Vec<ScoredObjective> {
    match blackboards
        .0
        .get(&crate::ship::system_registry::viewscreen_system_id())
    {
        Some(SystemBlackboard::Viewscreen(v)) => v.scored_objectives.clone(),
        _ => Vec::new(),
    }
}

/// Whether the AI doctrine-pool debug output is being rendered (issue #1149).
///
/// Gates only the JSON publish; the pool it projects is authoritative state that
/// exists whatever this says. Flipped from the host cog's Debug tab
/// (the generic Debug Surface setter) and from a connected phone
/// (`DebugSurface::AiDoctrine`), read back in `ServerMessage::DebugState`.
#[derive(Resource, Default, Debug)]
pub struct DebugAiDoctrineEnabled(pub bool);

impl crate::debug::catalogue::DebugSurfaceState for DebugAiDoctrineEnabled {
    fn is_enabled(&self) -> bool {
        self.0
    }

    fn set_enabled(&mut self, enabled: bool) {
        self.0 = enabled;
    }
}

/// Module-owned adapter for the AI-doctrine Debug Surface.
pub const DEBUG_AI_DOCTRINE_ADAPTER: crate::debug::catalogue::DebugSurfaceAdapter =
    crate::debug::catalogue::DebugSurfaceAdapter::for_resource::<DebugAiDoctrineEnabled>(
        crate::core::debug_surface::DebugSurface::AiDoctrine,
    );

/// The latest AI doctrine-pool JSON, when capture is enabled (issue #1149).
///
/// The target-agnostic sink, matching `StationActivityCapture`: on the browser
/// host the publish system ALSO writes the WASM bridge thread-local the dock
/// reads, but every target keeps the JSON here so the determinism guard can read
/// it without a browser. `None` until the first publish; never folded into the
/// digest. (The headless *report* does its own one-shot projection off the world
/// rather than reading this, so the report carries the surface with the flag off
/// too.)
#[derive(Resource, Default, Debug)]
pub struct AiDoctrineCapture(pub Option<String>);

/// Project each AI ship's doctrine pool AND every stateful fine-system host's
/// policy machine to JSON when capture is enabled (flag-gated).
///
/// Read-only: it never touches an authoritative resource, so its running or not
/// cannot move the digest. Queries `BehaviourSection` ships for the doctrine pool
/// — exactly the set the doctrine aggregator scores, so every ship with a
/// doctrine pool is covered (including the doctrine-driven player ship, whose
/// merged mission pool a tuner wants to see) — and, for the per-host policy view
/// (issue #1152), the ships carrying the helm Engines/Steering/Boost policy-state
/// components, the fine-system hosts that run a `#882` state machine. On the
/// browser host it also feeds the WASM bridge thread-local the dock reads; every
/// target keeps the JSON in [`AiDoctrineCapture`].
pub fn publish_ai_doctrine(
    sim_tick: Option<Res<crate::sim_tick::SimTick>>,
    ships: Query<
        (
            &ShipSystemBlackboards,
            Option<&crate::entities::spawner::EntityName>,
            Option<&crate::entities::spawner::EntityUuid>,
        ),
        With<crate::entities::spawner::BehaviourSection>,
    >,
    host_states: Query<
        (
            Option<&crate::entities::spawner::EntityName>,
            Option<&crate::entities::spawner::EntityUuid>,
            Option<&crate::ship::helm_ai::HelmEnginesAiPolicyState>,
            Option<&crate::ship::helm_ai::HelmSteeringAiPolicyState>,
            Option<&crate::ship::helm_ai::HelmBoostAiPolicyState>,
        ),
        Or<(
            With<crate::ship::helm_ai::HelmEnginesAiPolicyState>,
            With<crate::ship::helm_ai::HelmSteeringAiPolicyState>,
            With<crate::ship::helm_ai::HelmBoostAiPolicyState>,
        )>,
    >,
    mut capture: ResMut<AiDoctrineCapture>,
) {
    let tick = sim_tick.map_or(0, |t| t.0);
    let mut payload = collect_ai_doctrine(
        tick,
        ships.iter().map(|(blackboards, name, uuid)| {
            (
                name.map(|n| n.0.clone())
                    .unwrap_or_else(|| "<unnamed>".to_string()),
                uuid.map(|u| u.0.clone()),
                ship_scored_pool(blackboards),
            )
        }),
    );
    let mut host_rows = Vec::new();
    for (name, uuid, engines, steering, boost) in host_states.iter() {
        host_rows_for_entity(
            name,
            uuid,
            engines.map(|c| &c.0),
            steering.map(|c| &c.0),
            boost.map(|c| &c.0),
            &mut host_rows,
        );
    }
    payload.hosts = collect_host_policies(host_rows);
    let json = crate::core::codec::encode_ai_doctrine(&payload);

    #[cfg(all(target_arch = "wasm32", feature = "server"))]
    crate::server::bridge::set_ai_doctrine_string(json.clone());

    capture.0 = Some(json);
}

#[cfg(test)]
mod tests {
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
}
