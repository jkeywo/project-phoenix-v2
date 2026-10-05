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

    capture.0 = Some(json);
}

#[cfg(test)]
#[path = "ai_state_tests.rs"]
mod tests;
