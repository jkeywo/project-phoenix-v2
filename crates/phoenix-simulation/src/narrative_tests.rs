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

/// The whole reason the objective emitter reads the manager's log: it
/// observes a transition whichever call site caused it, and reports each one
/// once.
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

/// The reason the manager keeps a transition log at all (issue #1338
/// review): an objective posted and resolved inside ONE fixed tick is a
/// single status field and two story beats. A diff of the tick's end state
/// could only ever report the completion, silently swallowing the posting —
/// and #1341/#1344 read this timeline as a sequence of transitions.
#[test]
fn an_objective_posted_and_completed_in_one_tick_reports_both_in_order() {
    let mut app = narrative_app();
    app.insert_resource(ObjectiveManagerRes(Default::default()));
    app.add_systems(Update, emit_scenario_narrative);
    {
        let mut objectives = app.world_mut().resource_mut::<ObjectiveManagerRes>();
        objectives
            .0
            .add("snap_decision", "world.probe.objective.snap", true, vec![]);
        objectives.0.complete("snap_decision");
    }
    app.update();

    let events = drain(&mut app);
    assert_eq!(
        events.len(),
        2,
        "both transitions are beats, not just the terminal one: {events:?}"
    );
    assert_eq!(events[0].kind, NarrativeKind::ObjectivePosted);
    assert_eq!(events[1].kind, NarrativeKind::ObjectiveCompleted);
    assert!(events.iter().all(|e| e.id == "snap_decision"));
    // The posting carries the objective's own fields, not a placeholder.
    assert_eq!(
        events[0].detail.get("text"),
        Some(&NarrativeValue::Text("world.probe.objective.snap".into()))
    );
    assert_eq!(
        events[0].detail.get("mandatory"),
        Some(&NarrativeValue::Flag(true))
    );

    // And nothing is re-reported on the next tick by the diff backstop.
    app.update();
    assert!(drain(&mut app).is_empty());
}

/// The same for a posting the world takes straight back: the objective
/// existed, so the posting is a beat. The REMOVAL is not (a layer unloading
/// its own bookkeeping is not a story event), so exactly one event survives —
/// where a diff would have reported nothing at all.
#[test]
fn an_objective_posted_and_removed_in_one_tick_still_reports_the_posting() {
    let mut app = narrative_app();
    app.insert_resource(ObjectiveManagerRes(Default::default()));
    app.add_systems(Update, emit_scenario_narrative);
    {
        let mut objectives = app.world_mut().resource_mut::<ObjectiveManagerRes>();
        objectives
            .0
            .add("layer_local", "world.probe.objective.layer", false, vec![]);
        assert!(objectives.0.remove("layer_local"));
    }
    app.update();

    let events = drain(&mut app);
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[0].kind, NarrativeKind::ObjectivePosted);
    assert_eq!(events[0].id, "layer_local");

    app.update();
    assert!(
        drain(&mut app).is_empty(),
        "the removed objective must leave the backstop's view"
    );
}

/// The diff is still there, and still earns its keep: an objective whose
/// state arrived without passing through the manager's own API — a world
/// load inserting a prepared manager, say — is reported by the backstop.
#[test]
fn the_diff_backstop_reports_state_that_never_passed_through_the_log() {
    let mut app = narrative_app();
    let mut prepared = crate::objectives::ObjectiveManager::new();
    prepared.add("prior", "world.probe.objective.prior", true, vec![]);
    prepared.complete("prior");
    // Drop the log, so the ONLY thing the emitter can see is the state.
    let _ = prepared.drain_transitions();
    app.insert_resource(ObjectiveManagerRes(prepared));
    app.add_systems(Update, emit_scenario_narrative);
    app.update();

    let events = drain(&mut app);
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[0].kind, NarrativeKind::ObjectiveCompleted);
    assert_eq!(events[0].id, "prior");
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
    app.add_systems(Update, emit_authored_and_marked_entity_narrative);

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
    app.add_systems(Update, emit_authored_and_marked_entity_narrative);
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
    app.add_systems(Update, emit_authored_and_marked_entity_narrative);
    {
        let mut queue = app
            .world_mut()
            .resource_mut::<EffectQueue<NarrativeRequest>>();
        queue.0.push(NarrativeRequest::Authored {
            kind: NarrativeKind::BeatFired,
            id: "storm_hits".into(),
            entity_uuid: None,
        });
        queue.0.push(NarrativeRequest::Authored {
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

// ── The scripted removal path (issue #1338 review) ────────────────────────
//
// `ctx.effects.destroy_entity(..)` writes no `BalanceEvent` and must not:
// an authorial removal is not a combat kill. These three fix what the
// absence of that event used to mean — a marked hull could be removed by a
// script and leave the timeline with a spawn and no fate.

/// Spawn a marked hull, run a tick to register the mark, and return the app
/// with the entity's id.
fn app_with_marked_hull(uuid: &str, narrative_id: &str) -> (App, Entity) {
    let mut app = narrative_app();
    app.init_resource::<EffectQueue<NarrativeRequest>>();
    app.add_systems(Update, emit_authored_and_marked_entity_narrative);
    let entity = app
        .world_mut()
        .spawn((
            EntityUuid(uuid.to_string()),
            NarrativeMark(narrative_id.to_string()),
        ))
        .id();
    app.update();
    drain(&mut app);
    (app, entity)
}

/// Queue one scripted-removal signal, as `ActionCmd::DestroyEntity`'s
/// applier arm does, and despawn the entity as the same arm does.
fn scripted_destroy(app: &mut App, uuid: &str, entity: Entity) {
    app.world_mut()
        .resource_mut::<EffectQueue<NarrativeRequest>>()
        .0
        .push(NarrativeRequest::ScriptedRemoval {
            entity_uuid: uuid.to_string(),
        });
    app.world_mut().entity_mut(entity).despawn();
}

/// The no-silent-vanish fallback: a script removing a marked hull the author
/// said nothing else about records its death exactly once — even though no
/// `BalanceEvent` was written for it.
#[test]
fn a_scripted_destroy_of_a_marked_hull_reports_one_death() {
    let (mut app, entity) = app_with_marked_hull("uuid-skyhook", "skyhook");
    scripted_destroy(&mut app, "uuid-skyhook", entity);
    app.update();

    let events = drain(&mut app);
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[0].kind, NarrativeKind::MarkedEntityDestroyed);
    assert_eq!(events[0].id, "skyhook");
    assert_eq!(events[0].source.entity.as_deref(), Some("uuid-skyhook"));

    // A second removal of the same hull (a re-fired trigger, a duplicate
    // schedule entry) adds no second death.
    app.world_mut()
        .resource_mut::<EffectQueue<NarrativeRequest>>()
        .0
        .push(NarrativeRequest::ScriptedRemoval {
            entity_uuid: "uuid-skyhook".into(),
        });
    app.update();
    assert!(drain(&mut app).is_empty(), "a hull dies once");
}

/// …and the author's own word wins. A `rescued` outcome recorded on the same
/// tick as the destroy — in EITHER queue order — leaves the rescue standing
/// alone, with no death invented behind it.
#[test]
fn an_authored_outcome_suppresses_the_scripted_death() {
    for outcome_first in [true, false] {
        let (mut app, entity) = app_with_marked_hull("uuid-lyra", "lyra");
        let authored = NarrativeRequest::Authored {
            kind: NarrativeKind::MarkedEntityRescued,
            id: "lyra".into(),
            entity_uuid: Some("uuid-lyra".into()),
        };
        if outcome_first {
            app.world_mut()
                .resource_mut::<EffectQueue<NarrativeRequest>>()
                .0
                .push(authored.clone());
            scripted_destroy(&mut app, "uuid-lyra", entity);
        } else {
            scripted_destroy(&mut app, "uuid-lyra", entity);
            app.world_mut()
                .resource_mut::<EffectQueue<NarrativeRequest>>()
                .0
                .push(authored.clone());
        }
        app.update();

        let events = drain(&mut app);
        assert_eq!(
            events.len(),
            1,
            "outcome_first={outcome_first}: the rescue must stand alone: {events:?}"
        );
        assert_eq!(events[0].kind, NarrativeKind::MarkedEntityRescued);
        assert!(
            !events
                .iter()
                .any(|e| e.kind == NarrativeKind::MarkedEntityDestroyed),
            "a rescue-by-despawn must never be recorded as a death"
        );
    }
}

/// An outcome authored on an EARLIER tick suppresses it too — the gate is
/// the run's timeline, not the tick's.
#[test]
fn an_outcome_authored_earlier_in_the_run_suppresses_the_scripted_death() {
    let (mut app, entity) = app_with_marked_hull("uuid-lyra", "lyra");
    app.world_mut()
        .resource_mut::<EffectQueue<NarrativeRequest>>()
        .0
        .push(NarrativeRequest::Authored {
            kind: NarrativeKind::MarkedEntityEscaped,
            id: "lyra".into(),
            entity_uuid: Some("uuid-lyra".into()),
        });
    app.update();
    let events = drain(&mut app);
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[0].kind, NarrativeKind::MarkedEntityEscaped);

    scripted_destroy(&mut app, "uuid-lyra", entity);
    app.update();
    assert!(
        drain(&mut app).is_empty(),
        "the hull's fate was already on the timeline"
    );
}

/// Marking is still the gate on this path: the traffic a scenario clears
/// away with `destroy_entity` produces nothing at all.
#[test]
fn a_scripted_destroy_of_an_unmarked_hull_stays_silent() {
    let mut app = narrative_app();
    app.init_resource::<EffectQueue<NarrativeRequest>>();
    app.add_systems(Update, emit_authored_and_marked_entity_narrative);
    let rock = app.world_mut().spawn(EntityUuid("uuid-rock".into())).id();
    app.update();
    assert!(
        drain(&mut app).is_empty(),
        "an unmarked hull has no spawn beat"
    );

    scripted_destroy(&mut app, "uuid-rock", rock);
    app.update();
    assert!(
        drain(&mut app).is_empty(),
        "an unmarked hull a script removes is not a story beat"
    );
}

// ── The continuous-task lifecycle (issue #1341) ───────────────────────────
//
// The emitter's four decisions — ordinal, station, vanished subject, and the
// one-terminal rule — exercised against a bare app, so what is being
// asserted is this system and not the tractor or the scan.

use crate::core::task_lifecycle::{
    TaskLifecycleRequest, TaskLifecycles, TaskSlot, TaskTerminalReason, TASK_VERB_SCAN,
    TASK_VERB_TRACTOR_HOLD,
};

const OPERATOR: &str = "uuid-tender";
const SUBJECT: &str = "uuid-hulk";

fn hold_slot() -> TaskSlot {
    TaskSlot::new(OPERATOR, "tractor", TASK_VERB_TRACTOR_HOLD)
}

/// A bare app carrying the lifecycle plumbing, the operator hull and the
/// subject hull, with the emitter as its only system.
fn lifecycle_app() -> App {
    let mut app = narrative_app();
    app.init_resource::<EffectQueue<TaskLifecycleRequest>>()
        .init_resource::<TaskLifecycles>()
        .init_resource::<crate::sim_tick::SimTick>()
        .add_systems(Update, emit_task_lifecycle_narrative);
    app.world_mut().spawn(EntityUuid(OPERATOR.into()));
    app.world_mut().spawn(EntityUuid(SUBJECT.into()));
    app
}

fn push(app: &mut App, request: TaskLifecycleRequest) {
    app.world_mut()
        .resource_mut::<EffectQueue<TaskLifecycleRequest>>()
        .0
        .push(request);
}

fn start(slot: TaskSlot, target: &str) -> TaskLifecycleRequest {
    TaskLifecycleRequest::Start {
        slot,
        target: Some(target.to_string()),
    }
}

fn end(slot: TaskSlot, reason: TaskTerminalReason) -> TaskLifecycleRequest {
    TaskLifecycleRequest::End { slot, reason }
}

/// The reason recorded on a terminal event.
fn reason_of(event: &NarrativeEvent) -> String {
    match event.detail.get("reason") {
        Some(NarrativeValue::Text(s)) => s.clone(),
        other => panic!("a terminal event must carry its reason, got {other:?}"),
    }
}

/// The core contract: one start, one terminal, sharing one key.
#[test]
fn an_activation_produces_one_start_and_one_terminal_sharing_a_key() {
    let mut app = lifecycle_app();
    push(&mut app, start(hold_slot(), SUBJECT));
    app.update();
    let events = drain(&mut app);
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[0].kind, NarrativeKind::TaskStarted);
    let key = events[0].id.clone();
    assert_eq!(events[0].source.entity.as_deref(), Some(OPERATOR));
    assert_eq!(events[0].source.system.as_deref(), Some("tractor"));
    assert_eq!(events[0].target.as_deref(), Some(SUBJECT));

    push(&mut app, end(hold_slot(), TaskTerminalReason::Released));
    app.update();
    let events = drain(&mut app);
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[0].kind, NarrativeKind::TaskCancelled);
    assert_eq!(
        events[0].id, key,
        "the terminal beat carries the start's key"
    );
    assert_eq!(reason_of(&events[0]), "released");
}

/// The dedupe, which is what makes "exactly one terminal event" true when
/// two sites report the same ending: `operate_tractor_ai`'s withdrawn order
/// and the `ReleaseTractor` it causes are one hold ending once, and the
/// FIRST report — the truer one — is what the timeline records.
#[test]
fn a_second_report_of_the_same_ending_adds_nothing() {
    let mut app = lifecycle_app();
    push(&mut app, start(hold_slot(), SUBJECT));
    app.update();
    drain(&mut app);

    push(
        &mut app,
        end(hold_slot(), TaskTerminalReason::OrderWithdrawn),
    );
    push(&mut app, end(hold_slot(), TaskTerminalReason::Released));
    app.update();
    let events = drain(&mut app);
    assert_eq!(events.len(), 1, "one hold ends once: {events:?}");
    assert_eq!(reason_of(&events[0]), "order_withdrawn");

    // …and a terminal report for a slot that never started says nothing at
    // all, rather than inventing an activation to close.
    push(&mut app, end(hold_slot(), TaskTerminalReason::Released));
    app.update();
    assert!(drain(&mut app).is_empty());
}

/// A restart closes the activation it replaces and opens a distinct key, so
/// both are separately identifiable in the timeline — AC3.
#[test]
fn a_restart_closes_the_old_activation_and_mints_a_new_key() {
    let mut app = lifecycle_app();
    push(&mut app, start(hold_slot(), SUBJECT));
    app.update();
    let first_key = drain(&mut app)[0].id.clone();

    push(&mut app, start(hold_slot(), SUBJECT));
    app.update();
    let events = drain(&mut app);
    assert_eq!(events.len(), 2, "{events:?}");
    assert_eq!(events[0].kind, NarrativeKind::TaskInterrupted);
    assert_eq!(events[0].id, first_key);
    assert_eq!(reason_of(&events[0]), "restarted");
    assert_eq!(events[1].kind, NarrativeKind::TaskStarted);
    assert_ne!(
        events[1].id, first_key,
        "the second hold of the same hull is a SECOND task"
    );
}

/// Two slots run at once and neither ends the other — the simultaneous half
/// of AC3.
#[test]
fn simultaneous_tasks_on_one_hull_stay_separate() {
    let mut app = lifecycle_app();
    let scan_slot = TaskSlot::new(OPERATOR, "sensors", TASK_VERB_SCAN);
    push(&mut app, start(hold_slot(), SUBJECT));
    push(&mut app, start(scan_slot.clone(), SUBJECT));
    push(&mut app, end(scan_slot, TaskTerminalReason::Completed));
    app.update();

    let events = drain(&mut app);
    // Stable slot sort: the sensors slot sorts before the tractor slot, and
    // its own start-then-end order is preserved inside it.
    let shape: Vec<(&str, &str)> = events
        .iter()
        .map(|e| (e.kind.as_str(), e.source.system.as_deref().unwrap_or("")))
        .collect();
    assert_eq!(
        shape,
        vec![
            ("task_started", "sensors"),
            ("task_completed", "sensors"),
            ("task_started", "tractor"),
        ],
        "{events:?}"
    );
    // The hold is untouched by the scan finishing.
    assert_eq!(
        app.world().resource::<TaskLifecycles>().len(),
        1,
        "the tractor hold must still be running"
    );
}

/// The subject leaving the world is a target-DESTROYED interruption, even
/// though the owning system can only see "out of range".
#[test]
fn a_vanished_subject_upgrades_the_reason_it_masked() {
    let mut app = lifecycle_app();
    push(&mut app, start(hold_slot(), SUBJECT));
    app.update();
    drain(&mut app);

    let subject = app
        .world_mut()
        .query::<(Entity, &EntityUuid)>()
        .iter(app.world())
        .find(|(_, uuid)| uuid.0 == SUBJECT)
        .map(|(entity, _)| entity)
        .expect("the fixture spawns the subject");
    app.world_mut().entity_mut(subject).despawn();

    push(&mut app, end(hold_slot(), TaskTerminalReason::OutOfRange));
    app.update();
    let events = drain(&mut app);
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[0].kind, NarrativeKind::TaskInterrupted);
    assert_eq!(
        reason_of(&events[0]),
        "target_destroyed",
        "a hull that left the world is not merely distant"
    );
}

/// …and it ends the task even when nothing reports it at all — the case a
/// scripted removal produces, where the owning system may not run again
/// before the timeline is read.
#[test]
fn a_vanished_subject_ends_the_task_unreported() {
    let mut app = lifecycle_app();
    push(&mut app, start(hold_slot(), SUBJECT));
    app.update();
    drain(&mut app);

    let subject = app
        .world_mut()
        .query::<(Entity, &EntityUuid)>()
        .iter(app.world())
        .find(|(_, uuid)| uuid.0 == SUBJECT)
        .map(|(entity, _)| entity)
        .expect("the fixture spawns the subject");
    app.world_mut().entity_mut(subject).despawn();
    app.update();

    let events = drain(&mut app);
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(reason_of(&events[0]), "target_destroyed");
    assert!(app.world().resource::<TaskLifecycles>().is_empty());

    // And it happens once: the sweep closed the activation, so the next tick
    // has nothing left to close.
    app.update();
    assert!(drain(&mut app).is_empty());
}

/// The mission ending under a live task closes it, exactly once — the
/// GameOver half of AC2, which the headless report boundary does NOT cover
/// (that is the max-ticks half, `headless::report::finalize_task_lifecycles`).
///
/// This branch is the live one on any host that keeps ticking after the run
/// is decided, so it also has to leave the registry empty: a second tick in
/// GameOver must not close the same activation again.
#[test]
fn the_mission_ending_closes_every_live_task_once() {
    let mut app = lifecycle_app();
    app.insert_resource(State::new(GamePhase::InProgress));
    let scan_slot = TaskSlot::new(OPERATOR, "sensors", TASK_VERB_SCAN);
    push(&mut app, start(hold_slot(), SUBJECT));
    push(&mut app, start(scan_slot, SUBJECT));
    app.update();
    assert_eq!(drain(&mut app).len(), 2);
    assert_eq!(app.world().resource::<TaskLifecycles>().len(), 2);

    // The run is decided, and the schedule keeps running under it.
    app.insert_resource(State::new(GamePhase::GameOver));
    app.update();
    let events = drain(&mut app);
    assert_eq!(
        events.len(),
        2,
        "both live tasks end when the mission does: {events:?}"
    );
    for event in &events {
        assert_eq!(event.kind, NarrativeKind::TaskInterrupted);
        assert_eq!(reason_of(event), "mission_ended");
    }
    // Slot order, never the order they were opened in.
    assert_eq!(
        events
            .iter()
            .map(|e| e.source.system.as_deref().unwrap_or(""))
            .collect::<Vec<_>>(),
        vec!["sensors", "tractor"]
    );
    assert!(
        app.world().resource::<TaskLifecycles>().is_empty(),
        "the sweep must empty the registry, or the task's own terminal is \
             swallowed by the dedupe while the sim keeps ticking"
    );

    // …and a second tick under GameOver adds nothing.
    app.update();
    assert!(drain(&mut app).is_empty());
}

/// A standing order that can never be fulfilled is a POLL, not a beat: the
/// Sensors AI host re-issues its `ScanTarget` on every authored snapshot
/// while its objective stays unsatisfied, so an out-of-range Scan would
/// otherwise mint a whole activation per cadence, unbounded, into the
/// report, the counts and the ndjson stream.
#[test]
fn a_repeated_identical_failure_is_recorded_once() {
    let mut app = lifecycle_app();
    let scan_slot = TaskSlot::new(OPERATOR, "sensors", TASK_VERB_SCAN);
    let mut events = Vec::new();
    for _ in 0..200 {
        push(&mut app, start(scan_slot.clone(), SUBJECT));
        push(
            &mut app,
            end(scan_slot.clone(), TaskTerminalReason::OutOfRange),
        );
        app.update();
        events.extend(drain(&mut app));
    }
    assert_eq!(
        events.len(),
        2,
        "200 cadences of one unchanged refusal are one beat: {events:?}"
    );
    assert_eq!(events[0].kind, NarrativeKind::TaskStarted);
    assert_eq!(reason_of(&events[1]), "out_of_range");
    // The suppressed repeats did not spend ordinals either, so the key the
    // reader sees is the first activation's.
    assert!(events[0].id.ends_with("#0"), "{:?}", events[0].id);
    // …but they were COUNTED, not erased: 199 attempts are still waiting to
    // be reported under that same key. Nothing counts in the dark.
    assert!(app
        .world()
        .resource::<TaskLifecycles>()
        .has_pending_repeats());
}

/// The other half of that rule, and the half issue #1341's AC3 turns on: the
/// coalesced attempts are reported, once, under the key of the beat they
/// were folded into.
///
/// The emitter cannot tell an AI cadence retry from an engineer pressing
/// Engage a second time on a target that is still out of reach — both are
/// one slot, one subject, one unchanged refusal, both halves minted by one
/// tick — so it must bound the repeat without making it invisible.
#[test]
fn the_coalesced_repeats_are_reported_once_when_the_standing_failure_ends() {
    let mut app = lifecycle_app();
    let scan_slot = TaskSlot::new(OPERATOR, "sensors", TASK_VERB_SCAN);
    // Drained every iteration: `Messages` is double-buffered, so an event
    // left unread for two updates expires on its own.
    let mut first = Vec::new();
    for _ in 0..12 {
        push(&mut app, start(scan_slot.clone(), SUBJECT));
        push(
            &mut app,
            end(scan_slot.clone(), TaskTerminalReason::OutOfRange),
        );
        app.update();
        first.extend(drain(&mut app));
    }
    assert_eq!(first.len(), 2, "one pair for the standing order");
    let key = first[0].id.clone();

    // The order finally reaches its subject. That ENDS the standing failure,
    // so the eleven suppressed attempts are reported first — under the key
    // they repeated — and the reading that came back is its own activation.
    push(&mut app, start(scan_slot.clone(), SUBJECT));
    push(&mut app, end(scan_slot, TaskTerminalReason::Completed));
    app.update();
    let events = drain(&mut app);
    assert_eq!(
        events
            .iter()
            .map(|e| (e.kind, e.id.as_str()))
            .collect::<Vec<_>>(),
        vec![
            (NarrativeKind::TaskRepeated, key.as_str()),
            (
                NarrativeKind::TaskStarted,
                "uuid-tender/sensors/scan/uuid-hulk#1"
            ),
            (
                NarrativeKind::TaskCompleted,
                "uuid-tender/sensors/scan/uuid-hulk#1"
            ),
        ],
        "the census beat, then the activation that broke the standing failure"
    );
    assert_eq!(
        events[0].detail.get("repeats"),
        Some(&NarrativeValue::Int(11)),
        "eleven attempts were folded into the recorded refusal"
    );
    assert_eq!(
        events[0].detail.get("attempts"),
        Some(&NarrativeValue::Int(12))
    );
    assert!(
        !app.world()
            .resource::<TaskLifecycles>()
            .has_pending_repeats(),
        "reported once, not once per later beat"
    );
}

/// A run that stops with the standing order still being polled has nothing
/// left to break the failure, so the mission ending is what reports the
/// count. (`headless::report::finalize_task_lifecycles` covers the other way
/// a run stops, where no further tick exists at all.)
#[test]
fn a_still_polling_order_reports_its_repeats_when_the_mission_ends() {
    let mut app = lifecycle_app();
    app.insert_resource(State::new(GamePhase::InProgress));
    let scan_slot = TaskSlot::new(OPERATOR, "sensors", TASK_VERB_SCAN);
    // Drained every iteration: `Messages` is double-buffered, so an event
    // left unread for two updates expires on its own.
    let mut recorded = Vec::new();
    for _ in 0..5 {
        push(&mut app, start(scan_slot.clone(), SUBJECT));
        push(
            &mut app,
            end(scan_slot.clone(), TaskTerminalReason::OutOfRange),
        );
        app.update();
        recorded.extend(drain(&mut app));
    }
    assert_eq!(recorded.len(), 2);

    app.insert_resource(State::new(GamePhase::GameOver));
    app.update();
    let events = drain(&mut app);
    assert_eq!(events.len(), 1, "one census beat: {events:?}");
    assert_eq!(events[0].kind, NarrativeKind::TaskRepeated);
    assert_eq!(
        events[0].detail.get("repeats"),
        Some(&NarrativeValue::Int(4))
    );
    // …and the tick after adds nothing, however long the run sits in
    // GameOver.
    app.update();
    assert!(drain(&mut app).is_empty());
}

/// …but only while nothing about it changes. A different subject, a
/// different reason, or the work starting to succeed are all beats again —
/// and a task that spanned ticks is never a poll, however its predecessor
/// ended.
#[test]
fn a_changed_or_spanning_task_is_still_a_beat() {
    let mut app = lifecycle_app();
    let scan_slot = TaskSlot::new(OPERATOR, "sensors", TASK_VERB_SCAN);
    let refuse = |app: &mut App, target: &str, reason| {
        push(app, start(scan_slot.clone(), target));
        push(app, end(scan_slot.clone(), reason));
        app.update();
    };

    refuse(&mut app, SUBJECT, TaskTerminalReason::OutOfRange);
    assert_eq!(drain(&mut app).len(), 2, "the first refusal is a beat");
    refuse(&mut app, SUBJECT, TaskTerminalReason::OutOfRange);
    assert!(
        drain(&mut app).is_empty(),
        "the unchanged repeat is not a beat of its own"
    );

    // A different subject — which also ENDS the standing failure, so the one
    // suppressed repeat is reported alongside the new pair.
    refuse(&mut app, OPERATOR, TaskTerminalReason::OutOfRange);
    let changed = drain(&mut app);
    assert_eq!(
        changed.iter().map(|e| e.kind).collect::<Vec<_>>(),
        vec![
            NarrativeKind::TaskRepeated,
            NarrativeKind::TaskStarted,
            NarrativeKind::TaskFailed
        ],
        "the census of what was coalesced, then the beat that changed: \
             {changed:?}"
    );
    assert_eq!(
        changed[0].detail.get("repeats"),
        Some(&NarrativeValue::Int(1))
    );
    // A different reason for the same subject.
    refuse(&mut app, SUBJECT, TaskTerminalReason::Unpowered);
    assert_eq!(drain(&mut app).len(), 2);
    // The work succeeding is never coalesced, however often it recurs.
    refuse(&mut app, SUBJECT, TaskTerminalReason::Completed);
    assert_eq!(drain(&mut app).len(), 2);
    refuse(&mut app, SUBJECT, TaskTerminalReason::Completed);
    assert_eq!(drain(&mut app).len(), 2);

    // And a task that spanned ticks is a real activation ending, even when
    // the last thing this slot recorded was the identical failure.
    refuse(&mut app, SUBJECT, TaskTerminalReason::OutOfRange);
    drain(&mut app);
    push(&mut app, start(scan_slot.clone(), SUBJECT));
    app.update();
    assert_eq!(drain(&mut app).len(), 1, "the start of a spanning task");
    push(&mut app, end(scan_slot, TaskTerminalReason::OutOfRange));
    app.update();
    assert_eq!(drain(&mut app).len(), 1, "and its own terminal");
}

/// A quiet tick with nothing running writes nothing — the emitter must not
/// be a per-tick heartbeat.
#[test]
fn an_idle_run_produces_no_lifecycle_events() {
    let mut app = lifecycle_app();
    for _ in 0..5 {
        app.update();
    }
    assert!(drain(&mut app).is_empty());
}

/// A marked hull that is SHOT and then tidied away by a script reports one
/// death, not two — the combat event is the real one and it records the
/// fate.
#[test]
fn a_combat_death_suppresses_a_later_scripted_removal() {
    let (mut app, entity) = app_with_marked_hull("uuid-wick", "wick");
    app.world_mut()
        .resource_mut::<Messages<BalanceEvent>>()
        .write(BalanceEvent::EntityDestroyed {
            victim: "uuid-wick".into(),
            killer: Some("uuid-raider".into()),
        });
    scripted_destroy(&mut app, "uuid-wick", entity);
    app.update();

    let events = drain(&mut app);
    assert_eq!(events.len(), 1, "one hull, one death: {events:?}");
    assert_eq!(events[0].kind, NarrativeKind::MarkedEntityDestroyed);
    assert_eq!(
        events[0].target.as_deref(),
        Some("uuid-raider"),
        "the surviving death is the combat one, which carries killer credit"
    );
}

// ── The ship's-computer message (issue #1342) ─────────────────────────

use crate::core::computer_message::ComputerMessageSeverity;
use crate::core::messages::StationId;

/// A bare app carrying just what `tick_computer_message` needs — no
/// `WorldConfig`, so tick_hz falls back to `SchedClock::ZERO`'s 60 Hz.
fn computer_message_app() -> App {
    let mut app = App::new();
    app.add_message::<NarrativeEvent>()
        .init_resource::<ActiveComputerMessage>()
        .init_resource::<EffectQueue<ComputerMessageRequest>>()
        .init_resource::<SimTick>();
    app.add_systems(Update, tick_computer_message);
    app
}

fn push_request(app: &mut App, req: ComputerMessageRequest) {
    app.world_mut()
        .resource_mut::<EffectQueue<ComputerMessageRequest>>()
        .0
        .push(req);
}

fn request(id: &str, secs: i64) -> ComputerMessageRequest {
    ComputerMessageRequest {
        id: id.into(),
        text: "world.probe.computer_message.text".into(),
        severity: ComputerMessageSeverity::Advisory,
        duration_secs: secs,
        station: None,
    }
}

fn set_tick(app: &mut App, tick: u64) {
    app.world_mut().resource_mut::<SimTick>().0 = tick;
}

/// Showing the first message emits exactly one `shown` beat, carrying the
/// String Id, severity and duration verbatim.
#[test]
fn showing_a_message_emits_one_posted_event() {
    let mut app = computer_message_app();
    push_request(&mut app, request("hail_debris", 10));
    app.update();
    let events = drain(&mut app);
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[0].kind, NarrativeKind::ComputerMessagePosted);
    assert_eq!(events[0].id, "hail_debris");
    assert_eq!(
        events[0].detail.get("text"),
        Some(&NarrativeValue::Text(
            "world.probe.computer_message.text".into()
        ))
    );
    assert_eq!(
        events[0].detail.get("severity"),
        Some(&NarrativeValue::Text("advisory".into()))
    );
    assert_eq!(
        events[0].detail.get("duration_secs"),
        Some(&NarrativeValue::Int(10))
    );
    assert!(
        app.world()
            .resource::<ActiveComputerMessage>()
            .current
            .is_some(),
        "the state is now authoritatively showing something"
    );
}

/// A second message supersedes the first, in one tick: the superseded
/// event names the OLD id and the reason, and the posted event follows it
/// for the NEW id — both in one tick, in that order.
#[test]
fn a_second_message_supersedes_the_first_in_the_same_tick() {
    let mut app = computer_message_app();
    push_request(&mut app, request("first", 100));
    app.update();
    drain(&mut app);

    push_request(&mut app, request("second", 5));
    app.update();
    let events = drain(&mut app);
    assert_eq!(events.len(), 2, "{events:?}");
    assert_eq!(events[0].kind, NarrativeKind::ComputerMessageCleared);
    assert_eq!(events[0].id, "first");
    assert_eq!(
        events[0].detail.get("reason"),
        Some(&NarrativeValue::Text("superseded".into()))
    );
    assert_eq!(
        events[0].detail.get("superseded_by"),
        Some(&NarrativeValue::Text("second".into()))
    );
    assert_eq!(events[1].kind, NarrativeKind::ComputerMessagePosted);
    assert_eq!(events[1].id, "second");
}

/// Expiry is measured in simulation ticks: nothing is reported before the
/// due tick, and exactly one `expired` event fires on it.
#[test]
fn expiry_reports_once_on_its_due_tick() {
    let mut app = computer_message_app();
    push_request(&mut app, request("hail_debris", 10));
    app.update();
    drain(&mut app);

    // One tick early: nothing.
    set_tick(&mut app, 599);
    app.update();
    assert!(drain(&mut app).is_empty());

    // Due: exactly one expired event.
    set_tick(&mut app, 600);
    app.update();
    let events = drain(&mut app);
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[0].kind, NarrativeKind::ComputerMessageCleared);
    assert_eq!(events[0].id, "hail_debris");
    assert_eq!(
        events[0].detail.get("reason"),
        Some(&NarrativeValue::Text("expired".into()))
    );
    assert!(app
        .world()
        .resource::<ActiveComputerMessage>()
        .current
        .is_none());

    // And it does not re-report on a later tick.
    set_tick(&mut app, 700);
    app.update();
    assert!(drain(&mut app).is_empty(), "expiry must report once");
}

/// The optional Station cue rides the posted event's detail.
#[test]
fn a_station_cue_rides_the_posted_event() {
    let mut app = computer_message_app();
    push_request(
        &mut app,
        ComputerMessageRequest {
            id: "charge_ready".into(),
            text: "world.probe.computer_message.charge".into(),
            severity: ComputerMessageSeverity::Critical,
            duration_secs: 8,
            station: Some(StationId("tactical".into())),
        },
    );
    app.update();
    let events = drain(&mut app);
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(
        events[0].detail.get("station"),
        Some(&NarrativeValue::Text("tactical".into()))
    );
    assert_eq!(
        events[0].detail.get("severity"),
        Some(&NarrativeValue::Text("critical".into()))
    );
}

/// A tick with no queued request and nothing due is silent.
#[test]
fn a_quiet_tick_emits_nothing() {
    let mut app = computer_message_app();
    app.update();
    assert!(drain(&mut app).is_empty());
}

/// `clear_active_computer_message` empties the state unconditionally and
/// is not itself a narrative beat. Modelled on its real registration —
/// `OnEnter(GamePhase::GameOver)` / `OnEnter(GamePhase::Lobby)` — rather
/// than chained onto `tick_computer_message`'s own per-tick schedule,
/// which would clear a message on the very tick it was shown.
#[test]
fn clear_active_computer_message_empties_state_silently() {
    let mut active = ActiveComputerMessage::default();
    active.show(&request("mission_wrap", 30), 0, 60.0);
    assert!(active.current.is_some());

    let mut app = App::new();
    app.add_message::<NarrativeEvent>().insert_resource(active);
    app.add_systems(Update, clear_active_computer_message);
    app.update();

    assert!(drain(&mut app).is_empty(), "clearing is not itself a beat");
    assert!(app
        .world()
        .resource::<ActiveComputerMessage>()
        .current
        .is_none());
}
