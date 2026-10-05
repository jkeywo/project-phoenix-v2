use super::*;
use crate::core::messages::{AiDirective, ObjectiveStatus};
use crate::dossier::evidence::EvidenceProvenance;
use crate::world::commitments::CommitmentOutcome;
use crate::world::config::Trigger;
use crate::world::content::{TriggerState, WorldEvent};
use crate::world::deadlines::{Deadline, DeadlineHandler};
use crate::world::delayed::DelayedAction;
use crate::world::flags::parse_predicate;

const HZ: f32 = 60.0;

fn trigger_state(
    id: Option<&str>,
    condition: TriggerCondition,
    when: Option<&str>,
    repeat: bool,
    fired: bool,
) -> TriggerState {
    TriggerState {
        trigger: Trigger {
            condition,
            when: when.map(|w| parse_predicate(w).expect("test predicate parses")),
            id: id.map(str::to_string),
            repeat,
            cooldown_secs: None,
            gm_controls: None,
        },
        fired,
        origin_layer: None,
        seen_destroyed: Default::default(),
        last_fired_elapsed: None,
    }
}

/// Given an authored flag store, the payload contains every SET flag, sorted
/// by name, and nothing for an unset one.
#[test]
fn flags_are_projected_sorted_and_only_when_set() {
    let mut runtime = WorldContentRuntime::default();
    runtime.flags.set_flag("zeta");
    runtime.flags.set_flag_value("alpha", 5);
    runtime.flags.set_flag_value("cleared", 0); // removed by the store

    let payload = collect_scenario_state(&runtime, &ObjectiveManager::default());

    let names: Vec<&str> = payload.flags.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, vec!["alpha", "zeta"], "sorted, cleared flag absent");
    assert_eq!(payload.flags[0].value, 5);
    assert_eq!(payload.flags[1].value, 1);
}

/// Objectives carry id, status, base priority and directive, mandatory
/// first — the id/status/score/directive the AC names.
#[test]
fn objectives_carry_status_priority_and_directive() {
    let mut manager = ObjectiveManager::default();
    // Optional first, then a mandatory — so the mandatory-first ordering is
    // actually exercised rather than trivially satisfied.
    manager.add_full(
        "scan",
        "objective.scan",
        false,
        vec![],
        AiDirective::None,
        crate::objectives::UtilityConfig {
            base_priority: 2.0,
            ..Default::default()
        },
        Default::default(),
    );
    manager.add_full(
        "kill",
        "objective.kill",
        true,
        vec![],
        AiDirective::Destroy {
            target: "raider".into(),
        },
        crate::objectives::UtilityConfig {
            base_priority: 7.0,
            ..Default::default()
        },
        Default::default(),
    );
    manager.complete("scan");

    let runtime = WorldContentRuntime::default();
    let payload = collect_scenario_state(&runtime, &manager);

    assert_eq!(payload.objectives.len(), 2);
    // Mandatory first.
    let kill = &payload.objectives[0];
    assert_eq!(kill.id, "kill");
    assert_eq!(kill.status, ObjectiveStatus::Active);
    assert!(kill.mandatory);
    assert_eq!(kill.base_priority, 7.0);
    assert_eq!(
        kill.directive,
        AiDirective::Destroy {
            target: "raider".into()
        }
    );
    let scan = &payload.objectives[1];
    assert_eq!(scan.id, "scan");
    assert_eq!(scan.status, ObjectiveStatus::Completed);
    assert!(!scan.mandatory);
    assert_eq!(scan.base_priority, 2.0);
    assert_eq!(scan.directive, AiDirective::None);
}

/// A trigger whose `when` predicate references an unset flag is pending but
/// not eligible; setting the flag makes `when_holds` true. This is the
/// "armed vs waiting" evidence the surface exists for.
#[test]
fn triggers_report_pending_and_eligibility() {
    let mut runtime = WorldContentRuntime::default();
    runtime.triggers.push(trigger_state(
        Some("beat"),
        TriggerCondition::OnTimer { after_secs: 30.0 },
        Some("flag(ready)"),
        false,
        false,
    ));

    // Flag unset: armed but waiting on its gate.
    let payload = collect_scenario_state(&runtime, &ObjectiveManager::default());
    let trig = &payload.triggers[0];
    assert_eq!(trig.id.as_deref(), Some("beat"));
    assert_eq!(trig.condition, "on_timer(after_secs=30)");
    assert_eq!(trig.when.as_deref(), Some("flag(ready)"));
    assert!(trig.pending, "unfired once-only trigger is armed");
    assert!(!trig.fired);
    assert!(!trig.when_holds, "gate flag is unset");

    // Set the flag: now eligible.
    runtime.flags.set_flag("ready");
    let payload = collect_scenario_state(&runtime, &ObjectiveManager::default());
    assert!(payload.triggers[0].when_holds, "gate flag now holds");
}

/// A fired once-only trigger is no longer pending; a repeat trigger stays
/// armed after firing. A gateless trigger is always eligible.
#[test]
fn pending_reflects_lifecycle_and_gateless_is_eligible() {
    let mut runtime = WorldContentRuntime::default();
    runtime.triggers.push(trigger_state(
        None,
        TriggerCondition::OnWorldLoaded,
        None,
        false,
        true, // fired once-only
    ));
    runtime.triggers.push(trigger_state(
        Some("repeater"),
        TriggerCondition::OnFlagSet {
            name: "tick".into(),
        },
        None,
        true,
        true, // fired but repeatable
    ));

    let payload = collect_scenario_state(&runtime, &ObjectiveManager::default());
    assert!(!payload.triggers[0].pending, "fired once-only is spent");
    assert!(payload.triggers[0].when_holds, "no gate is always eligible");
    assert!(payload.triggers[1].pending, "a repeat trigger re-arms");
}

/// The delayed-action queue is projected with the action rendered and its
/// fire time.
#[test]
fn delayed_actions_are_projected() {
    let mut runtime = WorldContentRuntime::default();
    runtime.pending_delayed_actions.push(DelayedAction {
        action: TriggerAction::SetWorldFlag {
            name: "reinforce".into(),
        },
        origin_layer: None,
        entity_name: Some("carrier".into()),
        fire_at_elapsed: 45.5,
    });

    let payload = collect_scenario_state(&runtime, &ObjectiveManager::default());
    assert_eq!(payload.delayed_actions.len(), 1);
    assert_eq!(
        payload.delayed_actions[0].action,
        "set_world_flag(reinforce)"
    );
    assert_eq!(
        payload.delayed_actions[0].entity.as_deref(),
        Some("carrier")
    );
    assert_eq!(payload.delayed_actions[0].fire_at_secs, 45.5);
}

/// Armed deadlines are projected with their state; a cancelled one reads
/// `cancelled`.
#[test]
fn deadlines_are_projected_with_state() {
    let mut runtime = WorldContentRuntime::default();
    let authored = [
        Deadline {
            id: "window".into(),
            label: "world.deadline.window".into(),
            due_secs: 600,
            visible: true,
        },
        Deadline {
            id: "quiet".into(),
            label: String::new(),
            due_secs: 120,
            visible: false,
        },
    ];
    let handlers = [
        DeadlineHandler {
            deadline_id: "window".into(),
            handler: "on_window".into(),
            source_path: "w.rhai".into(),
        },
        DeadlineHandler {
            deadline_id: "quiet".into(),
            handler: "on_quiet".into(),
            source_path: "w.rhai".into(),
        },
    ];
    runtime.deadlines.arm(&authored, &handlers, 0, HZ);

    let payload = collect_scenario_state(&runtime, &ObjectiveManager::default());
    assert_eq!(payload.deadlines.len(), 2);
    let window = &payload.deadlines[0];
    assert_eq!(window.id, "window");
    assert_eq!(window.label, "world.deadline.window");
    assert!(window.visible);
    assert_eq!(window.due_tick, 600 * HZ as u64);
    assert_eq!(window.state, "pending");
    assert_eq!(payload.deadlines[1].label, "", "no label authored");
}

/// The commitments board projects open and resolved promises with their
/// state and provenance ticks.
#[test]
fn commitments_are_projected() {
    let mut runtime = WorldContentRuntime::default();
    runtime
        .commitments
        .record(
            "passage",
            "strike_committee",
            "terms.passage",
            "resolves.passage",
            10,
        )
        .expect("record");
    runtime
        .commitments
        .record("aid", "colony", "terms.aid", "", 20)
        .expect("record");
    runtime
        .commitments
        .resolve("passage", CommitmentOutcome::Kept, 30);

    let payload = collect_scenario_state(&runtime, &ObjectiveManager::default());
    assert_eq!(payload.commitments.len(), 2);
    let passage = &payload.commitments[0];
    assert_eq!(passage.id, "passage");
    assert_eq!(passage.made_to, "strike_committee");
    assert_eq!(passage.terms, "terms.passage");
    assert_eq!(passage.state, "kept");
    assert_eq!(passage.made_at_tick, 10);
    assert_eq!(passage.resolved_at_tick, Some(30));
    let aid = &payload.commitments[1];
    assert_eq!(aid.state, "open");
    assert_eq!(aid.resolved_at_tick, None);
}

/// The comms dossier projects each finding with its provenance label.
#[test]
fn dossier_findings_are_projected() {
    let mut runtime = WorldContentRuntime::default();
    runtime.evidence.append(
        "uuid-1",
        "evidence.forged_manifest",
        EvidenceProvenance::Records,
        40,
    );
    runtime.evidence.append(
        "uuid-2",
        "evidence.scan_anomaly",
        EvidenceProvenance::Scan,
        50,
    );

    let payload = collect_scenario_state(&runtime, &ObjectiveManager::default());
    assert_eq!(payload.dossier.len(), 2);
    assert_eq!(payload.dossier[0].subject_uuid, "uuid-1");
    assert_eq!(payload.dossier[0].text, "evidence.forged_manifest");
    assert_eq!(payload.dossier[0].provenance, "records");
    assert_eq!(payload.dossier[0].gathered_at_tick, 40);
    assert_eq!(payload.dossier[1].provenance, "scan");
}

/// An empty runtime projects a version-stamped, empty payload.
#[test]
fn empty_runtime_projects_version_stamped_empty() {
    let payload = collect_scenario_state(
        &WorldContentRuntime::default(),
        &ObjectiveManager::default(),
    );
    assert_eq!(
        payload.schema_version,
        crate::debug::payload::DEBUG_SCHEMA_VERSION
    );
    assert!(payload.flags.is_empty());
    assert!(payload.objectives.is_empty());
    assert!(payload.triggers.is_empty());
}

// ── Renderers ────────────────────────────────────────────────────────────

#[test]
fn conditions_render_to_stable_strings() {
    assert_eq!(
        render_condition(&TriggerCondition::OnDestroyed {
            entity_name: "raider".into()
        }),
        "on_destroyed(raider)"
    );
    assert_eq!(
        render_condition(&TriggerCondition::OnAllDestroyed {
            group: "wing".into(),
            after_secs: 5.0
        }),
        "on_all_destroyed(wing, after_secs=5)"
    );
    assert_eq!(
        render_condition(&TriggerCondition::OnHullBelow {
            entity_name: "boss".into(),
            threshold: 0.25
        }),
        "on_hull_below(boss, threshold=0.25)"
    );
    assert_eq!(
        render_condition(&TriggerCondition::OnWaypointReached {
            entity_name: "convoy".into(),
            waypoint: Some("alpha".into())
        }),
        "on_waypoint_reached(convoy, waypoint=alpha)"
    );
    assert_eq!(
        render_condition(&TriggerCondition::OnWorldLoaded),
        "on_world_loaded"
    );
}

#[test]
fn predicates_render_to_stable_strings() {
    for (src, expected) in [
        ("flag(ready)", "flag(ready)"),
        ("counter(kills) >= 3", "counter(kills) >= 3"),
        ("flag(a) and counter(b) < 2", "(flag(a) and counter(b) < 2)"),
        ("not flag(x)", "!(flag(x))"),
    ] {
        let pred = parse_predicate(src).expect("parses");
        assert_eq!(render_predicate(&pred), expected, "for source {src:?}");
    }
}

// ── Trigger fire history recorder (issue #1151) ───────────────────────────

/// Push a `TriggerState` onto the runtime and return its index, so a test can
/// mutate its fire fields to simulate the authoritative trigger pipeline.
fn push_trigger(
    runtime: &mut WorldContentRuntime,
    id: Option<&str>,
    condition: TriggerCondition,
    when: Option<&str>,
    repeat: bool,
) -> usize {
    runtime
        .triggers
        .push(trigger_state(id, condition, when, repeat, false));
    runtime.triggers.len() - 1
}

/// Simulate the authoritative fire of the trigger at `idx`: the same two
/// fields `evaluate_single_trigger` sets when a trigger fires.
fn simulate_fire(runtime: &mut WorldContentRuntime, idx: usize, elapsed: f32) {
    let event = match &runtime.triggers[idx].trigger.condition {
        TriggerCondition::OnTimer { .. } => WorldEvent::TimerElapsed {
            elapsed_secs: elapsed,
        },
        TriggerCondition::OnWorldLoaded => WorldEvent::WorldLoaded,
        TriggerCondition::OnFlagSet { name } => WorldEvent::FlagSet {
            name: name.clone(),
            origin_layer: None,
        },
        condition => panic!("unhandled fixture condition {condition:?}"),
    };
    runtime.triggers.evaluate(
        &[event],
        &runtime.name_to_uuid,
        &runtime.entity_groups,
        elapsed,
        |_| (vec![&runtime.flags], vec![None]),
    );
    assert_eq!(runtime.triggers[idx].last_fired_elapsed, Some(elapsed));
}

const DEPTH: usize = 4;

/// A fire records its fire time and the observed values of its `when` gate's
/// atoms — the evidence an author reconstructs "why did this beat fire" from.
#[test]
fn a_fire_records_its_time_and_when_atom_values() {
    let mut runtime = WorldContentRuntime::default();
    let idx = push_trigger(
        &mut runtime,
        Some("beat"),
        TriggerCondition::OnTimer { after_secs: 30.0 },
        Some("flag(ready) and counter(kills) >= 3"),
        false,
    );
    let mut recorder = TriggerFireRecorder::default();

    // First observation seeds the baseline; the trigger has not fired yet.
    recorder.sync_and_record(&runtime, DEPTH);
    assert!(recorder.fire_history(idx).is_empty(), "no fire yet");

    // The pipeline fires it at t=32.5 with the gate flags holding.
    runtime.flags.set_flag("ready");
    runtime.flags.set_flag_value("kills", 5);
    simulate_fire(&mut runtime, idx, 32.5);
    recorder.sync_and_record(&runtime, DEPTH);

    let history = recorder.fire_history(idx);
    assert_eq!(history.len(), 1, "one fire recorded");
    assert_eq!(history[0].fired_secs, 32.5);
    // Atom values quote the `when` vocabulary the surface already renders.
    let values: Vec<(&str, &str)> = history[0]
        .predicate_values
        .iter()
        .map(|v| (v.atom.as_str(), v.value.as_str()))
        .collect();
    assert!(
        values.contains(&("flag(ready)", "true")),
        "the gate flag's observed value, got {values:?}"
    );
    assert!(
        values.contains(&("counter(kills)", "5")),
        "the gate counter's observed reading, got {values:?}"
    );
}

/// A flag-referencing `condition` contributes its atom too — the "condition"
/// half of "the atoms in its condition/when that made it fire".
#[test]
fn a_flag_condition_atom_is_recorded() {
    let mut runtime = WorldContentRuntime::default();
    let idx = push_trigger(
        &mut runtime,
        Some("on_alarm"),
        TriggerCondition::OnFlagSet {
            name: "alarm".into(),
        },
        None,
        false,
    );
    let mut recorder = TriggerFireRecorder::default();
    recorder.sync_and_record(&runtime, DEPTH);

    runtime.flags.set_flag("alarm");
    simulate_fire(&mut runtime, idx, 10.0);
    recorder.sync_and_record(&runtime, DEPTH);

    let history = recorder.fire_history(idx);
    assert_eq!(history.len(), 1);
    assert_eq!(
        history[0].predicate_values,
        vec![PredicateValue {
            atom: "flag(alarm)".into(),
            value: "true".into(),
        }],
        "the condition's flag reads back in the flag(name) vocabulary"
    );
}

/// The per-trigger ring is bounded: firing past its depth keeps the most
/// recent `depth` fires and evicts the oldest.
#[test]
fn the_ring_caps_at_the_authored_depth() {
    let mut runtime = WorldContentRuntime::default();
    let idx = push_trigger(
        &mut runtime,
        Some("beacon"),
        TriggerCondition::OnTimer { after_secs: 1.0 },
        None,
        true, // repeat, so it can fire many times
    );
    let mut recorder = TriggerFireRecorder::default();
    recorder.sync_and_record(&runtime, 2); // depth 2

    for elapsed in [10.0_f32, 20.0, 30.0, 40.0] {
        simulate_fire(&mut runtime, idx, elapsed);
        recorder.sync_and_record(&runtime, 2);
    }

    let history = recorder.fire_history(idx);
    assert_eq!(history.len(), 2, "ring bounded at depth 2");
    let times: Vec<f32> = history.iter().map(|f| f.fired_secs).collect();
    assert_eq!(
        times,
        vec![30.0, 40.0],
        "the two most recent fires, oldest first"
    );
}

/// A trigger that fired BEFORE capture began is not mis-recorded on the first
/// captured tick: the first observation only seeds the baseline.
#[test]
fn a_pre_capture_fire_is_not_recorded() {
    let mut runtime = WorldContentRuntime::default();
    let idx = push_trigger(
        &mut runtime,
        Some("already"),
        TriggerCondition::OnWorldLoaded,
        None,
        false,
    );
    // It fired at t=5 before the recorder ever saw it.
    simulate_fire(&mut runtime, idx, 5.0);

    let mut recorder = TriggerFireRecorder::default();
    recorder.sync_and_record(&runtime, DEPTH);
    assert!(
        recorder.fire_history(idx).is_empty(),
        "a fire before capture must not be attributed to the first tick"
    );
}

/// A `ResetTrigger` (last_fired_elapsed Some→None) is not a fire, and a
/// subsequent genuine re-fire after the reset IS recorded.
#[test]
fn a_reset_is_not_a_fire_but_the_next_fire_is() {
    let mut runtime = WorldContentRuntime::default();
    let idx = push_trigger(
        &mut runtime,
        Some("resettable"),
        TriggerCondition::OnWorldLoaded,
        None,
        false,
    );
    let mut recorder = TriggerFireRecorder::default();
    recorder.sync_and_record(&runtime, DEPTH);

    // Fire, then reset (what `reset_triggers_by_id` does to the fire fields).
    simulate_fire(&mut runtime, idx, 5.0);
    recorder.sync_and_record(&runtime, DEPTH);
    assert_eq!(runtime.triggers.reset_by_id("resettable"), 1);
    recorder.sync_and_record(&runtime, DEPTH);
    assert_eq!(
        recorder.fire_history(idx).len(),
        1,
        "the reset is not a fire"
    );

    // A fresh fire after the reset is recorded.
    simulate_fire(&mut runtime, idx, 12.0);
    recorder.sync_and_record(&runtime, DEPTH);
    let history = recorder.fire_history(idx);
    assert_eq!(history.len(), 2);
    assert_eq!(history[1].fired_secs, 12.0);
}

/// The recorded fire history flows into the collected payload at the matching
/// trigger index — the additive `fire_history` field the dock reads.
#[test]
fn fire_history_lands_on_the_collected_trigger() {
    let mut runtime = WorldContentRuntime::default();
    let idx = push_trigger(
        &mut runtime,
        Some("beat"),
        TriggerCondition::OnTimer { after_secs: 30.0 },
        Some("flag(ready)"),
        false,
    );
    let mut recorder = TriggerFireRecorder::default();
    recorder.sync_and_record(&runtime, DEPTH);
    runtime.flags.set_flag("ready");
    simulate_fire(&mut runtime, idx, 30.0);
    recorder.sync_and_record(&runtime, DEPTH);

    let payload =
        collect_scenario_state_with_fires(&runtime, &ObjectiveManager::default(), &recorder);
    assert_eq!(payload.triggers[idx].fire_history.len(), 1);
    assert_eq!(payload.triggers[idx].fire_history[0].fired_secs, 30.0);
    // The public two-arg form carries an empty fire history (no recorder).
    let plain = collect_scenario_state(&runtime, &ObjectiveManager::default());
    assert!(plain.triggers[idx].fire_history.is_empty());
}

/// When the roster shrinks (a world reload), the recorder rebuilds rather than
/// mis-aligning old rings onto new triggers.
#[test]
fn a_shrinking_roster_rebuilds_the_records() {
    let mut runtime = WorldContentRuntime::default();
    let a = push_trigger(
        &mut runtime,
        Some("a"),
        TriggerCondition::OnWorldLoaded,
        None,
        true,
    );
    let _b = push_trigger(
        &mut runtime,
        Some("b"),
        TriggerCondition::OnWorldLoaded,
        None,
        true,
    );
    let mut recorder = TriggerFireRecorder::default();
    recorder.sync_and_record(&runtime, DEPTH);
    simulate_fire(&mut runtime, a, 5.0);
    recorder.sync_and_record(&runtime, DEPTH);
    assert_eq!(recorder.fire_history(a).len(), 1);

    // A reload leaves a single trigger; the recorder must not carry the old
    // ring onto it.
    let retained = runtime.triggers[0].clone();
    runtime.triggers.replace_declarative(vec![retained]);
    recorder.sync_and_record(&runtime, DEPTH);
    assert!(
        recorder.fire_history(0).is_empty(),
        "the rebuilt record starts empty and re-seeds its baseline"
    );
}

#[test]
fn restoring_continuation_discards_observer_history_without_recording_a_fire() {
    let mut runtime = WorldContentRuntime::default();
    let idx = push_trigger(
        &mut runtime,
        Some("beat"),
        TriggerCondition::OnWorldLoaded,
        None,
        true,
    );
    let mut recorder = TriggerFireRecorder::default();
    recorder.sync_and_record(&runtime, DEPTH);
    simulate_fire(&mut runtime, idx, 5.0);
    recorder.sync_and_record(&runtime, DEPTH);
    assert_eq!(recorder.fire_history(idx).len(), 1);

    let mut continuation = runtime.triggers.capture();
    continuation[idx].last_fired_elapsed = Some(20.0);
    runtime.triggers.restore(&continuation).unwrap();
    recorder.sync_and_record(&runtime, DEPTH);
    assert!(
        recorder.fire_history(idx).is_empty(),
        "restore seeds a baseline, not a fire"
    );
    simulate_fire(&mut runtime, idx, 21.0);
    recorder.sync_and_record(&runtime, DEPTH);
    assert_eq!(recorder.fire_history(idx).len(), 1);
    assert_eq!(recorder.fire_history(idx)[0].fired_secs, 21.0);
}

#[test]
fn equal_length_layer_replacement_discards_history_of_the_old_indices() {
    let mut runtime = WorldContentRuntime::default();
    for owner in ["a", "b", "c"] {
        let mut state = trigger_state(
            Some(owner),
            TriggerCondition::OnWorldLoaded,
            None,
            true,
            false,
        );
        state.origin_layer = Some(owner.into());
        runtime.triggers.push(state);
    }
    let mut recorder = TriggerFireRecorder::default();
    recorder.sync_and_record(&runtime, DEPTH);
    simulate_fire(&mut runtime, 2, 5.0);
    recorder.sync_and_record(&runtime, DEPTH);
    assert_eq!(recorder.fire_history(2).len(), 1);
    runtime.triggers.remove_layer("b");
    let mut replacement = trigger_state(
        Some("b"),
        TriggerCondition::OnWorldLoaded,
        None,
        true,
        false,
    );
    replacement.origin_layer = Some("b".into());
    runtime.triggers.push(replacement);
    assert_eq!(runtime.triggers.len(), 3);
    recorder.sync_and_record(&runtime, DEPTH);
    assert!(recorder.fire_history(1).is_empty());
    assert!(
        recorder.fire_history(2).is_empty(),
        "B cannot inherit C's former index history"
    );
}
