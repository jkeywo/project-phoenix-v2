//! The narrative-event emitters (issue #1338, PRD #1337).
//!
//! [`crate::core::narrative`] owns the vocabulary — what a mission-timeline
//! event IS. This module owns the four systems that produce them, and it lives
//! at the crate root rather than under `core` because two of them read scenario
//! state (`world::server`) that `core` must not depend on.
//!
//! # Why three of the four are observers, not chokepoint writes
//!
//! [`crate::core::balance::BalanceEvent`] is written *at* its chokepoints, and
//! that is right for combat: a hit has exactly one site, and the site knows the
//! attacker. Objective, deadline and marked-entity outcomes are not like that.
//!
//! * An Objective can be completed from a trigger action, a Rhai effect, a
//!   comms response, or the Helm AI satisfying its own directive
//!   (`ship::helm_ai`) — four call sites today, and the count has only ever
//!   gone up. Threading a message writer through all of them would put the
//!   timeline's completeness at the mercy of the next one added.
//! * The systems that own those sites are at Bevy's 16-parameter limit already,
//!   which is why `ScriptedCommsAux` / `CommsRespondAux` exist at all.
//! * The state itself is an ordered, deterministic `Vec` in both cases
//!   (`ObjectiveManager::sorted_snapshots`, `DeadlineTable::records`), so
//!   diffing it against the previous tick observes *every* path and can invent
//!   nothing.
//!
//! So [`emit_scenario_narrative`] diffs, and the per-system `Local` it diffs
//! against is deliberately NOT a resource: it is derived, non-authoritative,
//! and keeping it out of the world keeps it out of the #894 census and the
//! snapshot both.
//!
//! Authored beats and authored entity outcomes DO have one true chokepoint —
//! the script boundary — so they ride the #1223 effect-queue pattern instead:
//! `world::server::apply_dispatch_result` pushes a
//! [`NarrativeRequest`](crate::core::narrative::NarrativeRequest) and
//! [`drain_narrative_requests`] turns it into an event.
//!
//! # Determinism
//!
//! Every emitter reads state the fixed tick already decided and writes only
//! `Messages<NarrativeEvent>`, which nothing authoritative reads. Iteration
//! order is fixed at every site: the two diffs walk ordered `Vec`s, the marked
//! entity pass sorts its query results by authored id before emitting, and the
//! effect queue is drained front-to-back. No wall clock, no RNG, no `HashMap`
//! walk.

use bevy::prelude::*;
use std::collections::BTreeMap;

use crate::core::balance::BalanceEvent;
use crate::core::messages::ObjectiveStatus;
use crate::core::narrative::{
    NarrativeActor, NarrativeEvent, NarrativeKind, NarrativeMark, NarrativeRequest, NarrativeValue,
};
use crate::effect_queue::EffectQueue;
use crate::entities::spawner::EntityUuid;
use crate::world::deadlines::DeadlineState;
use crate::world::server::{ObjectiveManagerRes, WorldContentRuntime};

/// The narrative kind an objective's status maps to when it is first seen, or
/// when it changes. `Active` is only a *transition* to report on the tick the
/// objective appears — an objective that stays Active reports nothing.
fn kind_for_status(status: &ObjectiveStatus) -> NarrativeKind {
    match status {
        ObjectiveStatus::Active => NarrativeKind::ObjectivePosted,
        ObjectiveStatus::Completed => NarrativeKind::ObjectiveCompleted,
        ObjectiveStatus::Failed => NarrativeKind::ObjectiveFailed,
    }
}

/// Emit Objective and deadline transitions by diffing the scenario's own state
/// against what this system last saw.
///
/// One system for both because they are the same shape and the same argument:
/// ordered authored records whose *status field* is the story. Each `Local` map
/// is this system's private previous-tick view; neither is world state.
///
/// An objective is reported the first time it is seen (`ObjectivePosted` for an
/// Active one) and again on every status change. An objective the world
/// REMOVES (a layer unloading its own, `ObjectiveManager::remove`) is dropped
/// from the view without an event: nothing happened in the story, the scenario
/// simply took its bookkeeping back.
pub fn emit_scenario_narrative(
    objectives: Option<Res<ObjectiveManagerRes>>,
    runtime: Option<Res<WorldContentRuntime>>,
    mut seen_objectives: Local<BTreeMap<String, ObjectiveStatus>>,
    mut seen_deadlines: Local<BTreeMap<String, DeadlineState>>,
    mut out: MessageWriter<NarrativeEvent>,
) {
    if let Some(objectives) = objectives.as_deref() {
        // `sorted_snapshots` is mandatory-first then authored order — a total
        // order that does not depend on when each objective was added.
        let snapshots = objectives.0.sorted_snapshots();
        let mut live: BTreeMap<String, ObjectiveStatus> = BTreeMap::new();
        for snap in &snapshots {
            live.insert(snap.id.clone(), snap.status.clone());
            let changed = match seen_objectives.get(&snap.id) {
                None => true,
                Some(prev) => prev != &snap.status,
            };
            if !changed {
                continue;
            }
            // `text` is the objective's `strings.csv` id, carried verbatim —
            // AGENTS.md rule 11 and PRD #1337's "String Ids, not prose".
            let mut event = NarrativeEvent::new(kind_for_status(&snap.status), snap.id.clone())
                .text("text", snap.text.clone())
                .detail("mandatory", NarrativeValue::Flag(snap.mandatory));
            if let Some(first) = snap.targets.first() {
                event = event.to_target(first.clone());
            }
            out.write(event);
        }
        // Rebuild rather than merge, so a removed objective leaves the view.
        *seen_objectives = live;
    }

    if let Some(runtime) = runtime.as_deref() {
        let mut live: BTreeMap<String, DeadlineState> = BTreeMap::new();
        for record in &runtime.deadlines.records {
            let key = record.presentation_id();
            live.insert(key.clone(), record.state);
            let previously = seen_deadlines.get(&key).copied();
            // Only ONE deadline transition is a story beat: firing. Arming is
            // the world loading, and a cancellation is the scenario deciding
            // the beat will not happen — the authored `narrative_beat` call is
            // where an author says that mattered.
            if record.state == DeadlineState::Fired && previously != Some(DeadlineState::Fired) {
                out.write(
                    NarrativeEvent::new(NarrativeKind::DeadlineFired, key.clone())
                        .text("label", record.label.clone())
                        .detail("due_tick", NarrativeValue::Int(record.due_tick as i64)),
                );
            }
        }
        *seen_deadlines = live;
    }
}

/// Emit marked-entity spawn and death.
///
/// The `Local` map is uuid → authored narrative id, and it is what makes death
/// reportable at all: by the time anything can observe a destroyed ship it has
/// been despawned, so the mark has to have been remembered while it was alive.
/// It is a `Local` rather than a resource for the reason
/// [`emit_scenario_narrative`]'s is — see this module's docs.
///
/// Death rides `BalanceEvent::EntityDestroyed` rather than a second kill
/// chokepoint: that event is already emitted exactly once per death, at the
/// kill site, carrying the killer credit. Reading it here is not "inferring a
/// story event from a shot" — the gate is the authored [`NarrativeMark`], and
/// an unmarked hull dying produces nothing.
pub fn emit_marked_entity_narrative(
    marks: Query<(&EntityUuid, &NarrativeMark)>,
    mut known: Local<BTreeMap<String, String>>,
    mut balance: MessageReader<BalanceEvent>,
    mut out: MessageWriter<NarrativeEvent>,
) {
    // Newly marked entities, sorted by authored id so the emission order does
    // not depend on archetype layout.
    let mut fresh: Vec<(String, String)> = marks
        .iter()
        .filter(|(uuid, _)| !known.contains_key(&uuid.0))
        .map(|(uuid, mark)| (mark.0.clone(), uuid.0.clone()))
        .collect();
    fresh.sort();
    for (narrative_id, uuid) in fresh {
        known.insert(uuid.clone(), narrative_id.clone());
        out.write(
            NarrativeEvent::new(NarrativeKind::MarkedEntitySpawned, narrative_id)
                .from_actor(NarrativeActor::entity(uuid)),
        );
    }

    for event in balance.read() {
        let BalanceEvent::EntityDestroyed { victim, killer } = event else {
            continue;
        };
        let Some(narrative_id) = known.get(victim).cloned() else {
            continue;
        };
        let mut ev = NarrativeEvent::new(NarrativeKind::MarkedEntityDestroyed, narrative_id)
            .from_actor(NarrativeActor::entity(victim.clone()));
        if let Some(killer) = killer {
            ev = ev.to_target(killer.clone());
        }
        out.write(ev);
    }
}

/// Turn every queued authored narrative request into an event.
///
/// Drained in full every tick (`std::mem::take`), front to back, so the queue
/// is structurally empty at the fold point — the `ClearedAtFold` contract every
/// [`EffectQueue`] carries.
pub fn drain_narrative_requests(
    queue: Option<ResMut<EffectQueue<NarrativeRequest>>>,
    mut out: MessageWriter<NarrativeEvent>,
) {
    let Some(mut queue) = queue else {
        return;
    };
    if queue.0.is_empty() {
        return;
    }
    for request in std::mem::take(&mut queue.0) {
        let mut event = NarrativeEvent::new(request.kind, request.id);
        if let Some(uuid) = request.entity_uuid {
            event = event.from_actor(NarrativeActor::entity(uuid));
        }
        out.write(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::message::Messages;

    /// Build a bare app carrying just the narrative message and the systems
    /// under test — no simulation plugins, so the assertions are about these
    /// systems and nothing else.
    fn narrative_app() -> App {
        let mut app = App::new();
        app.add_message::<NarrativeEvent>()
            .add_message::<BalanceEvent>();
        app
    }

    /// Drain the events written so far, in write order.
    fn drain(app: &mut App) -> Vec<NarrativeEvent> {
        let mut messages = app.world_mut().resource_mut::<Messages<NarrativeEvent>>();
        messages.drain().collect()
    }

    /// The whole reason the objective emitter is a diff: it observes a
    /// transition whichever call site caused it, and reports each one once.
    #[test]
    fn objective_posted_then_completed_reports_each_transition_once() {
        let mut app = narrative_app();
        app.insert_resource(ObjectiveManagerRes(Default::default()));
        app.add_systems(Update, emit_scenario_narrative);

        app.world_mut().resource_mut::<ObjectiveManagerRes>().0.add(
            "reach_axiom",
            "world.probe.objective.reach",
            true,
            vec![],
        );
        app.update();
        let events = drain(&mut app);
        assert_eq!(events.len(), 1, "{events:?}");
        assert_eq!(events[0].kind, NarrativeKind::ObjectivePosted);
        assert_eq!(events[0].id, "reach_axiom");
        assert_eq!(
            events[0].detail.get("text"),
            Some(&NarrativeValue::Text("world.probe.objective.reach".into())),
            "the objective's String Id must pass through verbatim"
        );

        // An unchanged objective is not re-reported.
        app.update();
        assert!(drain(&mut app).is_empty());

        app.world_mut()
            .resource_mut::<ObjectiveManagerRes>()
            .0
            .complete("reach_axiom");
        app.update();
        let events = drain(&mut app);
        assert_eq!(events.len(), 1, "{events:?}");
        assert_eq!(events[0].kind, NarrativeKind::ObjectiveCompleted);

        app.update();
        assert!(drain(&mut app).is_empty(), "completion must report once");
    }

    /// A failed objective is its own beat, distinct from a completed one.
    #[test]
    fn objective_failure_is_its_own_kind() {
        let mut app = narrative_app();
        app.insert_resource(ObjectiveManagerRes(Default::default()));
        app.add_systems(Update, emit_scenario_narrative);
        app.world_mut().resource_mut::<ObjectiveManagerRes>().0.add(
            "hold_the_line",
            "world.probe.objective.hold",
            false,
            vec![],
        );
        app.update();
        drain(&mut app);
        app.world_mut()
            .resource_mut::<ObjectiveManagerRes>()
            .0
            .fail("hold_the_line");
        app.update();
        let events = drain(&mut app);
        assert_eq!(events.len(), 1, "{events:?}");
        assert_eq!(events[0].kind, NarrativeKind::ObjectiveFailed);
        assert_eq!(
            events[0].detail.get("mandatory"),
            Some(&NarrativeValue::Flag(false))
        );
    }

    /// Firing is the only deadline transition that is a story beat, and it is
    /// reported once.
    #[test]
    fn only_a_fired_deadline_reaches_the_timeline() {
        use crate::world::deadlines::DeadlineRecord;

        let mut app = narrative_app();
        let mut runtime = WorldContentRuntime::default();
        runtime.deadlines.records.push(DeadlineRecord {
            id: "window_opens".into(),
            origin_layer: None,
            label: "world.probe.deadline.window_opens.label".into(),
            visible: true,
            due_tick: 600,
            state: DeadlineState::Pending,
            armed: None,
        });
        runtime.deadlines.records.push(DeadlineRecord {
            id: "called_off".into(),
            origin_layer: None,
            label: "world.probe.deadline.called_off.label".into(),
            visible: false,
            due_tick: 900,
            state: DeadlineState::Pending,
            armed: None,
        });
        app.insert_resource(runtime);
        app.add_systems(Update, emit_scenario_narrative);

        // Pending deadlines say nothing.
        app.update();
        assert!(drain(&mut app).is_empty());

        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime.deadlines.records[0].state = DeadlineState::Fired;
            runtime.deadlines.records[1].state = DeadlineState::Cancelled;
        }
        app.update();
        let events = drain(&mut app);
        assert_eq!(
            events.len(),
            1,
            "a cancelled deadline is not a beat: {events:?}"
        );
        assert_eq!(events[0].kind, NarrativeKind::DeadlineFired);
        assert_eq!(events[0].id, "window_opens");
        assert_eq!(
            events[0].detail.get("label"),
            Some(&NarrativeValue::Text(
                "world.probe.deadline.window_opens.label".into()
            ))
        );

        app.update();
        assert!(drain(&mut app).is_empty(), "firing must report once");
    }

    /// Marking is the gate: an unmarked hull dying produces nothing, a marked
    /// one produces exactly one spawn beat and one death beat.
    #[test]
    fn only_marked_entities_produce_spawn_and_death_beats() {
        let mut app = narrative_app();
        app.add_systems(Update, emit_marked_entity_narrative);

        app.world_mut()
            .spawn((EntityUuid("uuid-lyra".into()), NarrativeMark("lyra".into())));
        app.world_mut().spawn(EntityUuid("uuid-rock".into()));
        app.update();
        let events = drain(&mut app);
        assert_eq!(events.len(), 1, "{events:?}");
        assert_eq!(events[0].kind, NarrativeKind::MarkedEntitySpawned);
        assert_eq!(events[0].id, "lyra");
        assert_eq!(events[0].source.entity.as_deref(), Some("uuid-lyra"));

        // Spawns report once.
        app.update();
        assert!(drain(&mut app).is_empty());

        app.world_mut()
            .resource_mut::<Messages<BalanceEvent>>()
            .write(BalanceEvent::EntityDestroyed {
                victim: "uuid-rock".into(),
                killer: Some("uuid-lyra".into()),
            });
        app.world_mut()
            .resource_mut::<Messages<BalanceEvent>>()
            .write(BalanceEvent::EntityDestroyed {
                victim: "uuid-lyra".into(),
                killer: None,
            });
        app.update();
        let events = drain(&mut app);
        assert_eq!(
            events.len(),
            1,
            "the unmarked rock's death is not a story beat: {events:?}"
        );
        assert_eq!(events[0].kind, NarrativeKind::MarkedEntityDestroyed);
        assert_eq!(events[0].id, "lyra");
    }

    /// A marked entity that dies after despawning still reports: the mark is
    /// remembered from while it was alive, which is the whole reason the map
    /// exists.
    #[test]
    fn a_despawned_marked_entity_still_reports_its_death() {
        let mut app = narrative_app();
        app.add_systems(Update, emit_marked_entity_narrative);
        let entity = app
            .world_mut()
            .spawn((EntityUuid("uuid-wick".into()), NarrativeMark("wick".into())))
            .id();
        app.update();
        drain(&mut app);

        app.world_mut().entity_mut(entity).despawn();
        app.world_mut()
            .resource_mut::<Messages<BalanceEvent>>()
            .write(BalanceEvent::EntityDestroyed {
                victim: "uuid-wick".into(),
                killer: Some("uuid-raider".into()),
            });
        app.update();
        let events = drain(&mut app);
        assert_eq!(events.len(), 1, "{events:?}");
        assert_eq!(events[0].kind, NarrativeKind::MarkedEntityDestroyed);
        assert_eq!(events[0].target.as_deref(), Some("uuid-raider"));
    }

    /// The authored queue drains front to back and leaves nothing behind.
    #[test]
    fn authored_requests_drain_in_order_and_empty_the_queue() {
        let mut app = narrative_app();
        app.init_resource::<EffectQueue<NarrativeRequest>>();
        app.add_systems(Update, drain_narrative_requests);
        {
            let mut queue = app
                .world_mut()
                .resource_mut::<EffectQueue<NarrativeRequest>>();
            queue.0.push(NarrativeRequest {
                kind: NarrativeKind::BeatFired,
                id: "storm_hits".into(),
                entity_uuid: None,
            });
            queue.0.push(NarrativeRequest {
                kind: NarrativeKind::MarkedEntityRescued,
                id: "lyra".into(),
                entity_uuid: Some("uuid-lyra".into()),
            });
        }
        app.update();
        let events = drain(&mut app);
        assert_eq!(events.len(), 2, "{events:?}");
        assert_eq!(events[0].kind, NarrativeKind::BeatFired);
        assert_eq!(events[0].id, "storm_hits");
        assert_eq!(events[1].kind, NarrativeKind::MarkedEntityRescued);
        assert_eq!(events[1].source.entity.as_deref(), Some("uuid-lyra"));
        assert!(app
            .world()
            .resource::<EffectQueue<NarrativeRequest>>()
            .0
            .is_empty());
    }
}
