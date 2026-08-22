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
    AiStatePayload, DoctrineCandidate, DoctrineChoice, ShipDoctrine, DEBUG_SCHEMA_VERSION,
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
/// (`wasm_toggle_ai_doctrine`) and from a connected phone
/// (`DebugFlag::AiDoctrine`), read back in `ServerMessage::DebugState`.
#[derive(Resource, Default, Debug)]
pub struct DebugAiDoctrineEnabled(pub bool);

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

/// Project each AI ship's doctrine pool to JSON when capture is enabled
/// (flag-gated).
///
/// Read-only: it never touches an authoritative resource, so its running or not
/// cannot move the digest. Queries `BehaviourSection` ships — exactly the set the
/// doctrine aggregator scores, so every ship with a doctrine pool is covered
/// (including the doctrine-driven player ship, whose merged mission pool a tuner
/// wants to see). On the browser host it also feeds the WASM bridge thread-local
/// the dock reads; every target keeps the JSON in [`AiDoctrineCapture`].
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
    mut capture: ResMut<AiDoctrineCapture>,
) {
    let tick = sim_tick.map_or(0, |t| t.0);
    let payload = collect_ai_doctrine(
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
    }
}
