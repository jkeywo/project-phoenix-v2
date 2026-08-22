use super::*;
/// A single-shot `on_destroyed` trigger for `name`.
///
/// It took the `TriggerAction` the trigger would dispatch until issue #985
/// deleted `Trigger::actions`; a fire runs the trigger's SCRIPT HANDLER now,
/// so what these tests observe is the latch, not an effect.
fn dest_trigger(name: &str) -> Trigger {
    Trigger {
        condition: TriggerCondition::OnDestroyed {
            entity_name: name.into(),
        },
        when: None,
        id: None,
        repeat: false,
        cooldown_secs: None,
    }
}

// ── evaluate_triggers ─────────────────────────────────────────────────

#[test]
fn on_destroyed_fires_when_entity_destroyed() {
    let mut states = vec![TriggerState {
        trigger: dest_trigger("raider"),
        fired: false,
        origin_layer: None,
        seen_destroyed: HashSet::new(),
        last_fired_elapsed: None,
    }];
    let mut name_to_uuid = HashMap::new();
    name_to_uuid.insert("raider".into(), "uuid-1".into());
    let events = vec![WorldEvent::Destroyed {
        uuid: "uuid-1".into(),
    }];
    let fired = evaluate_triggers(&mut states, &events, &name_to_uuid);
    assert_eq!(fired.len(), 1);
    assert!(states[0].fired);
}

#[test]
fn on_destroyed_does_not_fire_for_different_entity() {
    let mut states = vec![TriggerState {
        trigger: dest_trigger("raider"),
        fired: false,
        origin_layer: None,
        seen_destroyed: HashSet::new(),
        last_fired_elapsed: None,
    }];
    let mut name_to_uuid = HashMap::new();
    name_to_uuid.insert("raider".into(), "uuid-1".into());
    name_to_uuid.insert("station".into(), "uuid-2".into());
    let events = vec![WorldEvent::Destroyed {
        uuid: "uuid-2".into(),
    }];
    let fired = evaluate_triggers(&mut states, &events, &name_to_uuid);
    assert!(fired.is_empty());
    assert!(!states[0].fired);
}

#[test]
fn trigger_fires_only_once_single_shot() {
    let mut states = vec![TriggerState {
        trigger: dest_trigger("raider"),
        fired: false,
        origin_layer: None,
        seen_destroyed: HashSet::new(),
        last_fired_elapsed: None,
    }];
    let mut name_to_uuid = HashMap::new();
    name_to_uuid.insert("raider".into(), "uuid-1".into());
    let events = vec![WorldEvent::Destroyed {
        uuid: "uuid-1".into(),
    }];
    let fired1 = evaluate_triggers(&mut states, &events, &name_to_uuid);
    let fired2 = evaluate_triggers(&mut states, &events, &name_to_uuid);
    assert_eq!(fired1.len(), 1);
    assert!(fired2.is_empty());
}

// ── Trigger lifecycle: once (default) / repeat / cooldown / reset ──────

/// A repeatable `on_timer` trigger with an optional cooldown.
fn repeat_timer_trigger(cooldown_secs: Option<f32>) -> Trigger {
    Trigger {
        condition: TriggerCondition::OnTimer { after_secs: 0.0 },
        when: None,
        id: Some("pulse".into()),
        repeat: true,
        cooldown_secs,
    }
}

#[test]
fn repeat_trigger_re_fires_each_time_condition_holds() {
    let mut states = vec![TriggerState {
        trigger: repeat_timer_trigger(None),
        fired: false,
        origin_layer: None,
        seen_destroyed: HashSet::new(),
        last_fired_elapsed: None,
    }];
    let name_to_uuid = HashMap::new();
    let ev = vec![WorldEvent::TimerElapsed { elapsed_secs: 5.0 }];
    // Same condition holds on two consecutive evaluations — a repeat
    // trigger fires both times (a once-only trigger would fire only once).
    let f1 =
        evaluate_triggers_with_flags(&mut states, &ev, &name_to_uuid, &[], &HashMap::new(), 5.0);
    let f2 =
        evaluate_triggers_with_flags(&mut states, &ev, &name_to_uuid, &[], &HashMap::new(), 6.0);
    assert_eq!(f1.len(), 1, "first fire");
    assert_eq!(f2.len(), 1, "repeat trigger re-fires while condition holds");
}

#[test]
fn cooldown_gates_repeat_until_enough_time_elapses() {
    let mut states = vec![TriggerState {
        trigger: repeat_timer_trigger(Some(10.0)),
        fired: false,
        origin_layer: None,
        seen_destroyed: HashSet::new(),
        last_fired_elapsed: None,
    }];
    let name_to_uuid = HashMap::new();
    let ev = vec![WorldEvent::TimerElapsed { elapsed_secs: 0.0 }];

    // First fire at elapsed = 0.0.
    let f0 =
        evaluate_triggers_with_flags(&mut states, &ev, &name_to_uuid, &[], &HashMap::new(), 0.0);
    assert_eq!(f0.len(), 1);
    assert_eq!(states[0].last_fired_elapsed, Some(0.0));

    // Elapsed = 5.0 < cooldown 10.0 → still cooling down, must NOT fire.
    let f5 =
        evaluate_triggers_with_flags(&mut states, &ev, &name_to_uuid, &[], &HashMap::new(), 5.0);
    assert!(f5.is_empty(), "cooldown gates the re-fire");

    // Elapsed = 10.0 ≥ last(0.0) + cooldown(10.0) → re-fires.
    let f10 =
        evaluate_triggers_with_flags(&mut states, &ev, &name_to_uuid, &[], &HashMap::new(), 10.0);
    assert_eq!(f10.len(), 1, "re-fires once cooldown elapses");
    assert_eq!(states[0].last_fired_elapsed, Some(10.0));
}

#[test]
fn reset_re_arms_a_fired_once_only_trigger() {
    let mut states = vec![TriggerState {
        trigger: Trigger {
            condition: TriggerCondition::OnDestroyed {
                entity_name: "raider".into(),
            },
            when: None,
            id: Some("kill_watch".into()),
            repeat: false,
            cooldown_secs: None,
        },
        fired: false,
        origin_layer: None,
        seen_destroyed: HashSet::new(),
        last_fired_elapsed: None,
    }];
    let mut name_to_uuid = HashMap::new();
    name_to_uuid.insert("raider".into(), "uuid-1".into());
    let events = vec![WorldEvent::Destroyed {
        uuid: "uuid-1".into(),
    }];

    let f1 = evaluate_triggers(&mut states, &events, &name_to_uuid);
    assert_eq!(f1.len(), 1);
    assert!(states[0].fired);

    // Without reset the once-only trigger stays fired.
    let f2 = evaluate_triggers(&mut states, &events, &name_to_uuid);
    assert!(f2.is_empty());

    // Reset by id re-arms it; it fires again on the next matching event.
    let n = reset_triggers_by_id(&mut states, "kill_watch");
    assert_eq!(n, 1);
    assert!(!states[0].fired);
    assert_eq!(states[0].last_fired_elapsed, None);
    let f3 = evaluate_triggers(&mut states, &events, &name_to_uuid);
    assert_eq!(f3.len(), 1, "re-armed trigger fires again");
}

#[test]
fn reset_unknown_id_is_a_noop() {
    let mut states = vec![TriggerState {
        trigger: dest_trigger("raider"),
        fired: true,
        origin_layer: None,
        seen_destroyed: HashSet::new(),
        last_fired_elapsed: Some(3.0),
    }];
    let n = reset_triggers_by_id(&mut states, "nonexistent");
    assert_eq!(n, 0);
    assert!(states[0].fired, "unknown id must not touch any trigger");
}

// ── OnAllDestroyed ─────────────────────────────────────────────────────

fn all_destroyed_trigger(name: &str) -> Trigger {
    Trigger {
        condition: TriggerCondition::OnAllDestroyed {
            group: name.into(),
            after_secs: 0.0,
        },
        when: None,
        id: None,
        repeat: false,
        cooldown_secs: None,
    }
}

#[test]
fn on_all_destroyed_fires_only_after_last_named_entity_dies() {
    let mut states = vec![TriggerState {
        trigger: all_destroyed_trigger("a"),
        fired: false,
        origin_layer: None,
        seen_destroyed: HashSet::new(),
        last_fired_elapsed: None,
    }];
    let mut name_to_uuid = HashMap::new();
    name_to_uuid.insert("a".into(), "uuid-a".into());
    name_to_uuid.insert("b".into(), "uuid-b".into());

    let entity_groups: HashMap<String, HashSet<String>> = [(
        "a".to_string(),
        ["a".to_string(), "b".to_string()].into_iter().collect(),
    )]
    .into_iter()
    .collect();

    // First destruction: only `a` dies — must NOT fire.
    let events1 = vec![WorldEvent::Destroyed {
        uuid: "uuid-a".into(),
    }];
    let fired1 = evaluate_triggers_with_flags(
        &mut states,
        &events1,
        &name_to_uuid,
        &[],
        &entity_groups,
        0.0,
    );
    assert!(
        fired1.is_empty(),
        "OnAllDestroyed must not fire while any named entity still alive"
    );
    assert!(!states[0].fired);
    assert!(states[0].seen_destroyed.contains("a"));

    // Second destruction: `b` dies — now must fire.
    let events2 = vec![WorldEvent::Destroyed {
        uuid: "uuid-b".into(),
    }];
    let fired2 = evaluate_triggers_with_flags(
        &mut states,
        &events2,
        &name_to_uuid,
        &[],
        &entity_groups,
        0.0,
    );
    assert_eq!(fired2.len(), 1);
    assert!(states[0].fired);
}

#[test]
fn on_all_destroyed_is_single_shot() {
    let mut states = vec![TriggerState {
        trigger: all_destroyed_trigger("a"),
        fired: false,
        origin_layer: None,
        seen_destroyed: HashSet::new(),
        last_fired_elapsed: None,
    }];
    let mut name_to_uuid = HashMap::new();
    name_to_uuid.insert("a".into(), "uuid-a".into());
    let events = vec![WorldEvent::Destroyed {
        uuid: "uuid-a".into(),
    }];
    let fired1 = evaluate_triggers(&mut states, &events, &name_to_uuid);
    let fired2 = evaluate_triggers(&mut states, &events, &name_to_uuid);
    assert_eq!(fired1.len(), 1);
    assert!(
        fired2.is_empty(),
        "OnAllDestroyed must not re-fire after firing once"
    );
}

#[test]
fn on_all_destroyed_never_fires_for_unspawned_entity() {
    let mut states = vec![TriggerState {
        trigger: all_destroyed_trigger("a"),
        fired: false,
        origin_layer: None,
        seen_destroyed: HashSet::new(),
        last_fired_elapsed: None,
    }];
    let mut name_to_uuid = HashMap::new();
    name_to_uuid.insert("a".into(), "uuid-a".into());
    // "b" never registered.

    let entity_groups: HashMap<String, HashSet<String>> = [(
        "a".to_string(),
        ["a".to_string(), "b".to_string()].into_iter().collect(),
    )]
    .into_iter()
    .collect();

    let events = vec![
        WorldEvent::Destroyed {
            uuid: "uuid-a".into(),
        },
        WorldEvent::Destroyed {
            uuid: "uuid-b".into(),
        },
    ];
    let fired = evaluate_triggers_with_flags(
        &mut states,
        &events,
        &name_to_uuid,
        &[],
        &entity_groups,
        0.0,
    );
    assert!(
        fired.is_empty(),
        "OnAllDestroyed must not fire when a named entity was never registered"
    );
    assert!(states[0].seen_destroyed.contains("a"));
    assert!(!states[0].seen_destroyed.contains("b"));
}

#[test]
fn on_all_destroyed_fires_when_all_named_entities_die_in_one_batch() {
    // NOTE: With new group-based API, this test uses single entity "a".
    // For proper multi-entity testing, use evaluate_triggers_with_flags
    // with entity_groups = {"a" => {"a", "b", "c"}}.
    let mut states = vec![TriggerState {
        trigger: all_destroyed_trigger("a"),
        fired: false,
        origin_layer: None,
        seen_destroyed: HashSet::new(),
        last_fired_elapsed: None,
    }];
    let mut name_to_uuid = HashMap::new();
    name_to_uuid.insert("a".into(), "uuid-a".into());
    name_to_uuid.insert("b".into(), "uuid-b".into());
    name_to_uuid.insert("c".into(), "uuid-c".into());

    let events = vec![
        WorldEvent::Destroyed {
            uuid: "uuid-a".into(),
        },
        WorldEvent::Destroyed {
            uuid: "uuid-b".into(),
        },
        WorldEvent::Destroyed {
            uuid: "uuid-c".into(),
        },
    ];
    let fired = evaluate_triggers(&mut states, &events, &name_to_uuid);
    assert_eq!(fired.len(), 1);
    assert!(states[0].fired);
}

#[test]
fn on_all_destroyed_ignores_destruction_events_for_other_entities() {
    let mut states = vec![TriggerState {
        trigger: all_destroyed_trigger("a"),
        fired: false,
        origin_layer: None,
        seen_destroyed: HashSet::new(),
        last_fired_elapsed: None,
    }];
    let mut name_to_uuid = HashMap::new();
    name_to_uuid.insert("a".into(), "uuid-a".into());
    name_to_uuid.insert("other".into(), "uuid-other".into());

    // Some other entity dies — must not register against this trigger.
    let events = vec![WorldEvent::Destroyed {
        uuid: "uuid-other".into(),
    }];
    let fired = evaluate_triggers(&mut states, &events, &name_to_uuid);
    assert!(fired.is_empty());
    assert!(!states[0].seen_destroyed.contains("a"));
    assert!(states[0].seen_destroyed.is_empty());
}

#[test]
fn on_all_destroyed_when_predicate_gates_firing_but_seen_set_persists() {
    // Pins the design decision: `seen_destroyed` accumulates BEFORE the
    // `when` predicate is evaluated. The trigger fires as soon as both
    // (a) every named entity has been destroyed AND
    // (b) the `when` predicate evaluates true,
    // even if those become true on different ticks.
    use crate::world::flags::{parse_predicate, FlagStore};

    let predicate = parse_predicate("flag(armed)").expect("parse");
    let mut states = vec![TriggerState {
        trigger: Trigger {
            condition: TriggerCondition::OnAllDestroyed {
                group: "a".into(),
                after_secs: 0.0,
            },
            when: Some(predicate),
            id: None,
            repeat: false,
            cooldown_secs: None,
        },
        fired: false,
        origin_layer: None,
        seen_destroyed: HashSet::new(),
        last_fired_elapsed: None,
    }];
    let mut name_to_uuid = HashMap::new();
    name_to_uuid.insert("a".into(), "uuid-a".into());
    name_to_uuid.insert("b".into(), "uuid-b".into());

    // Tick 1: both entities die, but `armed` flag is unset → must NOT fire.
    let events = vec![
        WorldEvent::Destroyed {
            uuid: "uuid-a".into(),
        },
        WorldEvent::Destroyed {
            uuid: "uuid-b".into(),
        },
    ];
    let flag_store_unset = FlagStore::default();
    let chain_unset: [&FlagStore; 1] = [&flag_store_unset];
    let entity_groups: HashMap<String, HashSet<String>> = [(
        "a".to_string(),
        ["a".to_string(), "b".to_string()].into_iter().collect(),
    )]
    .into_iter()
    .collect();
    let fired_tick1 = evaluate_triggers_with_flags(
        &mut states,
        &events,
        &name_to_uuid,
        &chain_unset,
        &entity_groups,
        0.0,
    );
    assert!(
        fired_tick1.is_empty(),
        "must not fire while `when` predicate is false"
    );
    assert!(!states[0].fired);
    // But the seen_destroyed set must still have grown.
    assert!(states[0].seen_destroyed.contains("a"));
    assert!(states[0].seen_destroyed.contains("b"));

    // Tick 2: no new events, but the flag is now set → must fire from
    // the persisted `seen_destroyed` set.
    let mut flag_store_set = FlagStore::default();
    flag_store_set.set_flag("armed");
    let chain_set: [&FlagStore; 1] = [&flag_store_set];
    let fired_tick2 = evaluate_triggers_with_flags(
        &mut states,
        &[],
        &name_to_uuid,
        &chain_set,
        &entity_groups,
        0.0,
    );
    assert_eq!(fired_tick2.len(), 1);
    assert!(states[0].fired);
}

#[test]
fn on_all_destroyed_after_secs_gates_until_time_elapsed() {
    let mut states = vec![TriggerState {
        trigger: Trigger {
            condition: TriggerCondition::OnAllDestroyed {
                group: "waves".into(),
                after_secs: 10.0,
            },
            when: None,
            id: None,
            repeat: false,
            cooldown_secs: None,
        },
        fired: false,
        origin_layer: None,
        seen_destroyed: HashSet::new(),
        last_fired_elapsed: None,
    }];
    let mut name_to_uuid = HashMap::new();
    name_to_uuid.insert("raider".into(), "uuid-r".into());
    let entity_groups: HashMap<String, HashSet<String>> = [(
        "waves".to_string(),
        ["raider".to_string()].into_iter().collect(),
    )]
    .into_iter()
    .collect();

    // Destroy the only group member while elapsed is below the gate.
    let events = vec![WorldEvent::Destroyed {
        uuid: "uuid-r".into(),
    }];

    // elapsed = 5.0 < after_secs = 10.0 → trigger must NOT fire.
    let fired = evaluate_triggers_with_flags(
        &mut states,
        &events,
        &name_to_uuid,
        &[],
        &entity_groups,
        5.0,
    );
    assert!(
        fired.is_empty(),
        "OnAllDestroyed must not fire before after_secs gate"
    );
    assert!(!states[0].fired);
    assert!(states[0].seen_destroyed.contains("raider"));

    // No new events, but elapsed is now 15.0 > after_secs → trigger fires.
    let fired2 =
        evaluate_triggers_with_flags(&mut states, &[], &name_to_uuid, &[], &entity_groups, 15.0);
    assert_eq!(fired2.len(), 1);
    assert!(states[0].fired);
}

#[test]
fn on_timer_fires_when_elapsed_exceeds_threshold() {
    let mut states = vec![TriggerState {
        trigger: Trigger {
            condition: TriggerCondition::OnTimer { after_secs: 30.0 },
            when: None,
            id: None,
            repeat: false,
            cooldown_secs: None,
        },
        fired: false,
        origin_layer: None,
        seen_destroyed: HashSet::new(),
        last_fired_elapsed: None,
    }];
    let name_to_uuid = HashMap::new();
    let before = vec![WorldEvent::TimerElapsed { elapsed_secs: 10.0 }];
    let fired = evaluate_triggers(&mut states, &before, &name_to_uuid);
    assert!(fired.is_empty());
    let after = vec![WorldEvent::TimerElapsed { elapsed_secs: 60.0 }];
    let fired = evaluate_triggers(&mut states, &after, &name_to_uuid);
    assert_eq!(fired.len(), 1);
}

#[test]
fn on_attacked_fires_when_entity_attacked() {
    let mut states = vec![TriggerState {
        trigger: Trigger {
            condition: TriggerCondition::OnAttacked {
                entity_name: "raider".into(),
            },
            when: None,
            id: None,
            repeat: false,
            cooldown_secs: None,
        },
        fired: false,
        origin_layer: None,
        seen_destroyed: HashSet::new(),
        last_fired_elapsed: None,
    }];
    let mut name_to_uuid = HashMap::new();
    name_to_uuid.insert("raider".into(), "uuid-1".into());
    let events = vec![WorldEvent::Attacked {
        uuid: "uuid-1".into(),
        attacker_uuid: "uuid-player".into(),
    }];
    let fired = evaluate_triggers(&mut states, &events, &name_to_uuid);
    assert_eq!(fired.len(), 1);
}

#[test]
fn on_hailed_fires_when_entity_hailed() {
    let mut states = vec![TriggerState {
        trigger: Trigger {
            condition: TriggerCondition::OnHailed {
                entity_name: "starbase".into(),
            },
            when: None,
            id: None,
            repeat: false,
            cooldown_secs: None,
        },
        fired: false,
        origin_layer: None,
        seen_destroyed: HashSet::new(),
        last_fired_elapsed: None,
    }];
    let mut name_to_uuid = HashMap::new();
    name_to_uuid.insert("starbase".into(), "uuid-sb".into());
    let events = vec![WorldEvent::Hailed {
        target_uuid: "uuid-sb".into(),
    }];
    let fired = evaluate_triggers(&mut states, &events, &name_to_uuid);
    assert_eq!(fired.len(), 1);
}

#[test]
fn only_matching_trigger_fires() {
    let mut states = vec![
        TriggerState {
            trigger: dest_trigger("raider"),
            fired: false,
            origin_layer: None,
            seen_destroyed: HashSet::new(),
            last_fired_elapsed: None,
        },
        TriggerState {
            trigger: dest_trigger("station"),
            fired: false,
            origin_layer: None,
            seen_destroyed: HashSet::new(),
            last_fired_elapsed: None,
        },
    ];
    let mut name_to_uuid = HashMap::new();
    name_to_uuid.insert("raider".into(), "uuid-r".into());
    name_to_uuid.insert("station".into(), "uuid-s".into());
    let events = vec![WorldEvent::Destroyed {
        uuid: "uuid-r".into(),
    }];
    let fired = evaluate_triggers(&mut states, &events, &name_to_uuid);
    assert_eq!(fired.len(), 1);
    assert!(states[0].fired);
    assert!(!states[1].fired);
}

#[test]
fn on_destroyed_does_not_fire_if_entity_name_unknown() {
    let mut states = vec![TriggerState {
        trigger: dest_trigger("ghost"),
        fired: false,
        origin_layer: None,
        seen_destroyed: HashSet::new(),
        last_fired_elapsed: None,
    }];
    let name_to_uuid = HashMap::new();
    let events = vec![WorldEvent::Destroyed {
        uuid: "uuid-x".into(),
    }];
    let fired = evaluate_triggers(&mut states, &events, &name_to_uuid);
    assert!(fired.is_empty());
}

// ── trigger_states_from_world ─────────────────────────────────────────

// ── on_world_loaded (issue #415) ──────────────────────────────────────

#[test]
fn on_world_loaded_matches_world_loaded_event() {
    let mut states = vec![TriggerState {
        trigger: Trigger {
            condition: TriggerCondition::OnWorldLoaded,
            when: None,
            id: None,
            repeat: false,
            cooldown_secs: None,
        },
        fired: false,
        origin_layer: None,
        seen_destroyed: HashSet::new(),
        last_fired_elapsed: None,
    }];
    let name_to_uuid = HashMap::new();
    let events = vec![WorldEvent::WorldLoaded];
    let fired = evaluate_triggers(&mut states, &events, &name_to_uuid);
    assert_eq!(fired.len(), 1);
    assert!(states[0].fired);
}

#[test]
fn on_world_loaded_does_not_match_unrelated_events() {
    let mut states = vec![TriggerState {
        trigger: Trigger {
            condition: TriggerCondition::OnWorldLoaded,
            when: None,
            id: None,
            repeat: false,
            cooldown_secs: None,
        },
        fired: false,
        origin_layer: None,
        seen_destroyed: HashSet::new(),
        last_fired_elapsed: None,
    }];
    let name_to_uuid = HashMap::new();
    let events = vec![
        WorldEvent::Destroyed { uuid: "x".into() },
        WorldEvent::TimerElapsed { elapsed_secs: 5.0 },
        WorldEvent::FlagSet {
            name: "f".into(),
            origin_layer: None,
        },
    ];
    let fired = evaluate_triggers(&mut states, &events, &name_to_uuid);
    assert!(fired.is_empty());
    assert!(!states[0].fired);
}

#[test]
fn on_world_loaded_is_single_shot() {
    let mut states = vec![TriggerState {
        trigger: Trigger {
            condition: TriggerCondition::OnWorldLoaded,
            when: None,
            id: None,
            repeat: false,
            cooldown_secs: None,
        },
        fired: false,
        origin_layer: None,
        seen_destroyed: HashSet::new(),
        last_fired_elapsed: None,
    }];
    let name_to_uuid = HashMap::new();
    let events = vec![WorldEvent::WorldLoaded];
    let fired1 = evaluate_triggers(&mut states, &events, &name_to_uuid);
    let fired2 = evaluate_triggers(&mut states, &events, &name_to_uuid);
    assert_eq!(fired1.len(), 1);
    assert!(fired2.is_empty());
}

// ── on_entered_region / on_exited_region (issue #416) ─────────────────

#[test]
fn on_entered_region_matches_entered_region_event_with_resolved_uuid() {
    let mut states = vec![TriggerState {
        trigger: Trigger {
            condition: TriggerCondition::OnEnteredRegion {
                entity_name: "nebula".into(),
            },
            when: None,
            id: None,
            repeat: false,
            cooldown_secs: None,
        },
        fired: false,
        origin_layer: None,
        seen_destroyed: HashSet::new(),
        last_fired_elapsed: None,
    }];
    let mut name_to_uuid = HashMap::new();
    name_to_uuid.insert("nebula".into(), "uuid-nebula".into());
    let events = vec![WorldEvent::EnteredRegion {
        uuid: "uuid-nebula".into(),
    }];
    let fired = evaluate_triggers(&mut states, &events, &name_to_uuid);
    assert_eq!(fired.len(), 1);
    assert!(states[0].fired);
}

#[test]
fn on_entered_region_does_not_match_different_uuid() {
    let mut states = vec![TriggerState {
        trigger: Trigger {
            condition: TriggerCondition::OnEnteredRegion {
                entity_name: "nebula".into(),
            },
            when: None,
            id: None,
            repeat: false,
            cooldown_secs: None,
        },
        fired: false,
        origin_layer: None,
        seen_destroyed: HashSet::new(),
        last_fired_elapsed: None,
    }];
    let mut name_to_uuid = HashMap::new();
    name_to_uuid.insert("nebula".into(), "uuid-nebula".into());
    let events = vec![WorldEvent::EnteredRegion {
        uuid: "uuid-other".into(),
    }];
    let fired = evaluate_triggers(&mut states, &events, &name_to_uuid);
    assert!(fired.is_empty());
    assert!(!states[0].fired);
}

#[test]
fn on_entered_region_does_not_match_exited_region_event() {
    let mut states = vec![TriggerState {
        trigger: Trigger {
            condition: TriggerCondition::OnEnteredRegion {
                entity_name: "nebula".into(),
            },
            when: None,
            id: None,
            repeat: false,
            cooldown_secs: None,
        },
        fired: false,
        origin_layer: None,
        seen_destroyed: HashSet::new(),
        last_fired_elapsed: None,
    }];
    let mut name_to_uuid = HashMap::new();
    name_to_uuid.insert("nebula".into(), "uuid-nebula".into());
    let events = vec![WorldEvent::ExitedRegion {
        uuid: "uuid-nebula".into(),
    }];
    let fired = evaluate_triggers(&mut states, &events, &name_to_uuid);
    assert!(fired.is_empty());
}

#[test]
fn on_exited_region_matches_exited_region_event_with_resolved_uuid() {
    let mut states = vec![TriggerState {
        trigger: Trigger {
            condition: TriggerCondition::OnExitedRegion {
                entity_name: "nebula".into(),
            },
            when: None,
            id: None,
            repeat: false,
            cooldown_secs: None,
        },
        fired: false,
        origin_layer: None,
        seen_destroyed: HashSet::new(),
        last_fired_elapsed: None,
    }];
    let mut name_to_uuid = HashMap::new();
    name_to_uuid.insert("nebula".into(), "uuid-nebula".into());
    let events = vec![WorldEvent::ExitedRegion {
        uuid: "uuid-nebula".into(),
    }];
    let fired = evaluate_triggers(&mut states, &events, &name_to_uuid);
    assert_eq!(fired.len(), 1);
}

#[test]
fn on_entered_region_does_not_match_unresolved_name() {
    let mut states = vec![TriggerState {
        trigger: Trigger {
            condition: TriggerCondition::OnEnteredRegion {
                entity_name: "unknown".into(),
            },
            when: None,
            id: None,
            repeat: false,
            cooldown_secs: None,
        },
        fired: false,
        origin_layer: None,
        seen_destroyed: HashSet::new(),
        last_fired_elapsed: None,
    }];
    let name_to_uuid = HashMap::new(); // empty — name does not resolve
    let events = vec![WorldEvent::EnteredRegion {
        uuid: "any-uuid".into(),
    }];
    let fired = evaluate_triggers(&mut states, &events, &name_to_uuid);
    assert!(fired.is_empty());
}

// ── parent: layer walker on flag-transition conditions (PRD #397 fix 1) ──

#[test]
fn on_flag_set_with_parent_prefix_matches_event_in_loader_layer() {
    // Trigger lives in "child.toml", which was loaded by base world.
    // Condition `name = "parent:armed"` must match an event whose
    // origin_layer is the trigger's loader (base = None).
    let mut state = TriggerState {
        trigger: Trigger {
            condition: TriggerCondition::OnFlagSet {
                name: "parent:armed".into(),
            },
            when: None,
            id: None,
            repeat: false,
            cooldown_secs: None,
        },
        fired: false,
        origin_layer: Some("child.toml".into()),
        seen_destroyed: HashSet::new(),
        last_fired_elapsed: None,
    };
    let layer_chain = vec![Some("child.toml".into()), None];
    let events = vec![WorldEvent::FlagSet {
        name: "armed".into(),
        origin_layer: None,
    }];
    let name_to_uuid = HashMap::new();
    let fired = evaluate_single_trigger(
        &mut state,
        &events,
        &name_to_uuid,
        &[],
        &layer_chain,
        &HashMap::new(),
        0.0,
    );
    assert!(
        fired.is_some(),
        "parent:armed must match a base-layer FlagSet"
    );
}

#[test]
fn on_flag_set_does_not_match_same_name_in_different_layer() {
    // Per-layer scoping: same flag name in another sub-world must not
    // fire a trigger watching `armed` in its own layer.
    let mut state = TriggerState {
        trigger: Trigger {
            condition: TriggerCondition::OnFlagSet {
                name: "armed".into(),
            },
            when: None,
            id: None,
            repeat: false,
            cooldown_secs: None,
        },
        fired: false,
        origin_layer: Some("child.toml".into()),
        seen_destroyed: HashSet::new(),
        last_fired_elapsed: None,
    };
    let layer_chain = vec![Some("child.toml".into()), None];
    // Event originated in a DIFFERENT layer (the base / None layer).
    let events = vec![WorldEvent::FlagSet {
        name: "armed".into(),
        origin_layer: None,
    }];
    let name_to_uuid = HashMap::new();
    let fired = evaluate_single_trigger(
        &mut state,
        &events,
        &name_to_uuid,
        &[],
        &layer_chain,
        &HashMap::new(),
        0.0,
    );
    assert!(
        fired.is_none(),
        "same-named flag in another layer must not cross-fire"
    );
}

#[test]
fn on_flag_set_parent_walk_past_root_does_not_match() {
    // Trigger in base world (origin_layer = None) requests `parent:armed`
    // — walks past root. Must never match.
    let mut state = TriggerState {
        trigger: Trigger {
            condition: TriggerCondition::OnFlagSet {
                name: "parent:armed".into(),
            },
            when: None,
            id: None,
            repeat: false,
            cooldown_secs: None,
        },
        fired: false,
        origin_layer: None,
        seen_destroyed: HashSet::new(),
        last_fired_elapsed: None,
    };
    let layer_chain = vec![None];
    let events = vec![WorldEvent::FlagSet {
        name: "armed".into(),
        origin_layer: None,
    }];
    let name_to_uuid = HashMap::new();
    let fired = evaluate_single_trigger(
        &mut state,
        &events,
        &name_to_uuid,
        &[],
        &layer_chain,
        &HashMap::new(),
        0.0,
    );
    assert!(fired.is_none(), "parent: from base must resolve past root");
}

#[test]
fn resolve_layer_prefix_returns_innermost_for_bare_name() {
    let chain = vec![Some("child.toml".into()), None];
    let r = resolve_layer_prefix("armed", &chain);
    assert_eq!(r, Some(("armed".into(), Some("child.toml".into()))));
}

#[test]
fn resolve_layer_prefix_walks_one_level() {
    let chain = vec![Some("child.toml".into()), None];
    let r = resolve_layer_prefix("parent:armed", &chain);
    assert_eq!(r, Some(("armed".into(), None)));
}

#[test]
fn resolve_layer_prefix_walks_past_root_returns_none() {
    let chain = vec![None];
    assert_eq!(resolve_layer_prefix("parent:armed", &chain), None);
    assert_eq!(resolve_layer_prefix("parent:parent:armed", &chain), None);
}

// ── combat_test's authored wave clock (#892, re-authored for #960) ─────
//
// The eight-wave schedule is content, not code: every wave hangs off its own
// `on_timer`, each wave's objective and its `mission_threat_remaining`
// decrement hang off `on_all_destroyed` over that wave's group, and victory
// is one `on_all_destroyed` over `hostiles` guarded by
// `counter(waves_spawned) >= 8`. That composition is only as good as the
// trigger pipeline's actual semantics, and nothing else tests it end to end
// — `tests/headless_runner` boots the real sim but flies the player on AI
// backfill, which does not clear the whole raid at any seed sampled, so a
// run there never reaches the end of the schedule.
//
// This drives the world's REAL triggers through the REAL evaluator with a
// scripted player, and models exactly the runtime behaviours the authoring
// leans on: `OnTimer` fires once its `after_secs` has elapsed, group
// membership accumulates on spawn and is never removed, and a deferred
// effect dispatches later than the trigger that queued it.
//
// (#984) Those triggers are Rhai registrations now, so the harness mirrors
// `tick_trigger_pipeline` one step further than it used to: it evaluates
// per-INDEX through `evaluate_single_trigger` (a scripted trigger fires with
// an EMPTY action list, so the index is what finds its handler), then RUNS
// that handler on the production runtime host and applies what it buffered.
// Nothing about the evaluator changed; what changed is where the effects
// come from, and the whole point of this test is that the answer is the same.

/// Outcome of one scripted run of `combat_test.toml`'s trigger set.
struct ChainRun {
    /// Wave-group spawn order, as `(group, elapsed_secs)`.
    spawns: Vec<(String, f32)>,
    /// Elapsed seconds at which the victory `game_over` was dispatched.
    victory_at: Option<f32>,
    /// `waves_spawned` at the moment victory dispatched.
    waves_at_victory: i64,
    /// Per-step `(mission_threat_remaining, waves_spawned, wave groups
    /// fully destroyed, wave groups with a living member)`.
    trace: Vec<(i64, i64, usize, usize)>,
}

/// Run `combat_test.toml`'s triggers against a scripted player at the given
/// `ship_power` tier. `kill_after_secs` is how long that player takes to
/// clear a ship after it arrives — `0.0` is the perfect player that meets
/// every wave at its spawn point, and a value larger than the wave interval
/// is a player falling behind the clock, which is a state the death-gated
/// chain could not produce at all.
fn run_combat_test_chain(ship_power: i64, kill_after_secs: f32) -> ChainRun {
    use crate::world::script::effects::BufferedEffect;
    use crate::world::script::fixture::ScriptedWorld;
    use crate::world::script::schedule::SchedClock;

    const WORLD: &str = "assets/worlds/combat_test.toml";
    let toml = include_str!("../../assets/worlds/combat_test.toml");
    crate::world::config::parse_world(toml).expect("combat_test.toml must parse");
    let world = ScriptedWorld::compile(WORLD, toml);

    let mut states: Vec<TriggerState> = world
        .triggers
        .iter()
        .map(|st| TriggerState {
            trigger: st.trigger.clone(),
            fired: false,
            origin_layer: None,
            seen_destroyed: HashSet::new(),
            last_fired_elapsed: None,
        })
        .collect();

    let mut flags = crate::world::flags::FlagStore::new();
    flags.set_flag_value("ship_power", ship_power);

    let mut name_to_uuid: HashMap<String, String> = HashMap::new();
    let mut entity_groups: HashMap<String, HashSet<String>> = HashMap::new();
    // Live ships, as `name -> the elapsed time it may be killed at`.
    let mut alive: HashMap<String, f32> = HashMap::new();
    let mut pending: Vec<(f32, TriggerAction)> = Vec::new();
    let mut run = ChainRun {
        spawns: Vec::new(),
        victory_at: None,
        waves_at_victory: 0,
        trace: Vec::new(),
    };

    const STEP: f32 = 0.5;
    let mut elapsed = 0.0f32;
    for _ in 0..4000 {
        // 1. Dispatch delayed actions whose time has come.
        let (due, rest): (Vec<_>, Vec<_>) = pending.into_iter().partition(|(at, _)| *at <= elapsed);
        pending = rest;
        for (_, action) in due {
            apply_action(
                &action,
                elapsed,
                kill_after_secs,
                &mut name_to_uuid,
                &mut entity_groups,
                &mut alive,
                &mut flags,
                &mut run,
            );
        }

        // 2. The scripted player: destroy every hostile whose kill time has
        //    come. At `kill_after_secs = 0.0` that is every wave on arrival,
        //    which spends most of the run with every registered hostile dead
        //    and more waves still to come — exactly the window the victory
        //    guard has to close.
        let mut targets: Vec<String> = alive
            .iter()
            .filter(|(_, kill_at)| **kill_at <= elapsed)
            .map(|(n, _)| n.clone())
            .collect();
        targets.sort();
        let mut events: Vec<WorldEvent> = targets
            .iter()
            .map(|n| WorldEvent::Destroyed {
                uuid: name_to_uuid[n].clone(),
            })
            .collect();
        // The clock the pipeline feeds in every tick, plus the one-shot
        // load event the faction flip and the threat count hang off.
        events.push(WorldEvent::TimerElapsed {
            elapsed_secs: elapsed,
        });
        if elapsed == 0.0 {
            events.push(WorldEvent::WorldLoaded);
        }
        for n in &targets {
            alive.remove(n);
        }

        // 3. Evaluate, chaining within the step exactly as the runtime's
        //    per-tick chaining loop does. Per-INDEX, because a scripted
        //    trigger's effects come from the handler the index names.
        let mut round = events;
        for _ in 0..8 {
            let mut fired: Vec<usize> = Vec::new();
            for (idx, state) in states.iter_mut().enumerate() {
                let flag_chain = [&flags];
                if evaluate_single_trigger(
                    state,
                    &round,
                    &name_to_uuid,
                    &flag_chain,
                    &[None],
                    &entity_groups,
                    elapsed,
                )
                .is_some()
                {
                    fired.push(idx);
                }
            }
            if fired.is_empty() {
                break;
            }
            round = Vec::new();
            let clock = SchedClock {
                tick: 0,
                elapsed_secs: elapsed,
                tick_hz: 60.0,
            };
            for idx in fired {
                let effects = world.fire(idx, &flags, &clock);
                // Immediate effects, in authored order. A name-resolving
                // `Action` is what the applier re-dispatches; a `Cmd` is
                // already resolved, and the only ones this world emits that
                // the model cares about are its flag writes.
                for effect in effects.commands {
                    match effect {
                        BufferedEffect::Action(action) => apply_action(
                            &action,
                            elapsed,
                            kill_after_secs,
                            &mut name_to_uuid,
                            &mut entity_groups,
                            &mut alive,
                            &mut flags,
                            &mut run,
                        ),
                        BufferedEffect::Cmd(cmd) => {
                            apply_cmd(&cmd, &mut flags);
                        }
                    }
                }
                // Deferred effects join the same queue a `delay_secs` action
                // did; the clock above makes `fire_at_elapsed` absolute.
                for delayed in effects.delayed {
                    pending.push((delayed.fire_at_elapsed, delayed.action));
                }
            }
        }

        // Sample the two counters against the ground truth they claim to
        // describe: how many wave groups are wholly dead, and how many
        // still have a living member.
        let (destroyed, living) = entity_groups
            .iter()
            .filter(|(g, _)| g.starts_with("wave_"))
            .fold((0usize, 0usize), |(d, l), (_, members)| {
                if members.iter().any(|m| alive.contains_key(m)) {
                    (d, l + 1)
                } else {
                    (d + 1, l)
                }
            });
        run.trace.push((
            flags.counter("mission_threat_remaining"),
            flags.counter("waves_spawned"),
            destroyed,
            living,
        ));
        if run.victory_at.is_some() {
            break;
        }
        elapsed += STEP;
    }
    run
}

/// Apply one dispatched action to the scripted world.
#[allow(clippy::too_many_arguments)]
fn apply_action(
    action: &TriggerAction,
    elapsed: f32,
    kill_after_secs: f32,
    name_to_uuid: &mut HashMap<String, String>,
    entity_groups: &mut HashMap<String, HashSet<String>>,
    alive: &mut HashMap<String, f32>,
    flags: &mut crate::world::flags::FlagStore,
    run: &mut ChainRun,
) {
    match action {
        TriggerAction::SpawnEntity { name, groups, .. } => {
            name_to_uuid.insert(name.clone(), format!("uuid-{name}"));
            alive.insert(name.clone(), elapsed + kill_after_secs);
            for g in groups {
                // Membership accumulates and is NEVER removed on death —
                // the property every `on_all_destroyed` in this world rests
                // on.
                entity_groups
                    .entry(g.clone())
                    .or_default()
                    .insert(name.clone());
                if g.starts_with("wave_") && !run.spawns.iter().any(|(s, _)| s == g) {
                    run.spawns.push((g.clone(), elapsed));
                }
            }
        }
        TriggerAction::IncrementWorldFlag { name, by } => {
            flags.increment_flag(name, *by);
        }
        TriggerAction::SetWorldFlagValue { name, value } => {
            flags.set_flag_value(name, *value);
        }
        TriggerAction::GameOver {
            outcome: Some(crate::core::balance::Outcome::Victory),
            ..
        } if run.victory_at.is_none() => {
            run.victory_at = Some(elapsed);
            run.waves_at_victory = flags.counter("waves_spawned");
        }
        _ => {}
    }
}

/// Apply one RESOLVED command effect to the scripted world.
///
/// A scripted handler's flag writes arrive as `MutateFlag` rather than as
/// the `IncrementWorldFlag` / `SetWorldFlagValue` actions the declarative
/// front-end dispatched, so the model applies them through the same two
/// `FlagStore` verbs `world::server`'s own `MutateFlag` arm uses. Everything
/// else this world emits as a command — objective completions, the defeat
/// `game_over` — the model does not track.
fn apply_cmd(cmd: &crate::world::dispatch::ActionCmd, flags: &mut crate::world::flags::FlagStore) {
    use crate::world::dispatch::{ActionCmd, FlagMutation};
    if let ActionCmd::MutateFlag { name, mutation, .. } = cmd {
        match mutation {
            FlagMutation::Increment(by) => {
                flags.increment_flag(name, *by);
            }
            FlagMutation::SetValue(v) => {
                flags.set_flag_value(name, *v);
            }
            _ => {}
        }
    }
}

#[test]
fn combat_test_wave_clock_releases_eight_waves_on_schedule_then_victory() {
    // Destroyer tier (power_rating 70) — the DEMO loadout, below both bonus
    // gates, so this is exactly the eight-wave table the issue specifies.
    let run = run_combat_test_chain(70, 0.0);

    let order: Vec<&str> = run.spawns.iter().map(|(g, _)| g.as_str()).collect();
    assert_eq!(
        order,
        (1..=8).map(|n| format!("wave_{n}")).collect::<Vec<_>>(),
        "all eight waves must be released, in order"
    );

    // The CLOCK, not a chain: each wave lands at its own authored
    // `after_secs` regardless of how fast the player clears the last one.
    // This scripted player clears every wave the step it arrives, so under
    // the old death-gated chain the eighth wave would land at roughly
    // 8 x 10 = 80 s. Pinning the absolute times is what makes that
    // difference visible — a chain re-introduced by accident would still
    // produce the right ORDER and the right count.
    let times: Vec<f32> = run.spawns.iter().map(|(_, t)| *t).collect();
    assert_eq!(
        times,
        vec![0.0, 45.0, 90.0, 135.0, 180.0, 225.0, 270.0, 315.0],
        "each wave must arrive on its authored clock, not a breather after \
         the previous one died"
    );

    assert!(
        run.victory_at.is_some(),
        "clearing every wave must reach victory"
    );
    assert_eq!(
        run.waves_at_victory, 8,
        "victory must not be reachable before all eight waves have been \
         released — under a clock the counter guard is what closes the long \
         windows in which every registered hostile is dead and more waves \
         are still to come"
    );
    let last_spawn = run.spawns.last().expect("wave 8 spawned").1;
    assert!(
        run.victory_at.unwrap() > last_spawn,
        "victory must land after wave 8, not in a gap before it"
    );
    // Those windows are not hypothetical: with a player this fast, most of
    // the run has nothing alive at all and several waves still to come. If
    // the guard were dropped, victory would fire in the first of them.
    assert!(
        run.trace
            .iter()
            .any(|(_, spawned, _, living)| *living == 0 && *spawned < 8),
        "the scripted run must actually enter the empty-field window the \
         victory guard exists to close"
    );
}

#[test]
fn combat_test_remaining_threat_counts_waves_still_to_be_fought() {
    // Issue #943 under #960's clock. `mission_threat_remaining` is the
    // DENOMINATOR the torpedo-conservation doctrine divides its rounds by,
    // so what it MEANS decides how a magazine paces itself across the run.
    // It counts waves NOT YET DESTROYED — which under the death-gated chain
    // was indistinguishable from "waves not yet spawned", and under a clock
    // is not.
    //
    // Flown twice: a player that meets each wave on arrival, and one that
    // takes 60 s per wave and so falls behind a 45 s schedule. The second is
    // the state the chain could never produce, and the one that separates
    // the two readings.
    for kill_after in [0.0, 60.0] {
        let run = run_combat_test_chain(70, kill_after);

        // It is published before anything can read it, at the number of
        // waves.
        assert_eq!(
            run.trace.first().map(|(threat, ..)| *threat),
            Some(8),
            "the world must publish its full threat on load, or a magazine \
             paced against it divides by an unbounded ratio and paces \
             nothing (kill_after={kill_after})"
        );

        // THE INVARIANT, and the whole reason the decrement stayed on each
        // wave's own death rather than moving to its spawn: the published
        // threat is exactly the waves that are not yet dead. A wave already
        // on the field is threat the ship still has to survive, so it is
        // still counted; a wave that is dead is not, whatever order it died
        // in.
        for (threat, spawned, destroyed, living) in &run.trace {
            assert_eq!(
                *threat,
                8 - *destroyed as i64,
                "remaining threat must equal the waves not yet destroyed \
                 (spawned={spawned}, destroyed={destroyed}, living={living}, \
                 kill_after={kill_after})"
            );
        }

        // It only ever falls, and it reaches zero. A one-way latch that
        // never reaches 0 strands the fleet's last reserve of rounds; one
        // that rose would let a hull that had already stopped firing
        // conclude it had more mission left than it does.
        for pair in run.trace.windows(2) {
            assert!(
                pair[1].0 <= pair[0].0,
                "remaining threat must never rise, got {:?} -> {:?} \
                 (kill_after={kill_after})",
                pair[0],
                pair[1]
            );
        }
        assert_eq!(
            run.trace.last().map(|(threat, ..)| *threat),
            Some(0),
            "every wave pays its own single decrement, so a run that clears \
             all eight must end at zero (kill_after={kill_after})"
        );
    }

    // …and the two counters are genuinely independent under a clock. A
    // player 60 s per wave behind a 45 s schedule has waves stacked on the
    // field: `waves_spawned` has run ahead while the threat count has not
    // fallen, so `8 - waves_spawned` would UNDER-report the threat left and
    // a magazine paced on it would spend its reserve early. This is why
    // this stayed a second counter rather than arithmetic on the first.
    let behind = run_combat_test_chain(70, 60.0);
    assert!(
        behind.trace.iter().any(|(_, _, _, living)| *living >= 2),
        "a player 60s per wave behind a 45s clock must end up with two \
         waves alive at once"
    );
    assert!(
        behind
            .trace
            .iter()
            .any(|(threat, spawned, _, _)| *threat != 8 - *spawned),
        "under a clock the remaining-threat count must diverge from \
         `8 - waves_spawned`; if it never does, the two are redundant and \
         one of them is wrong"
    );
}

#[test]
fn combat_test_wave_clock_waits_for_the_power_tier_bonus_ships_too() {
    // Battleship tier: every wave carries a bonus hull in its own group, so
    // there is strictly more to kill per wave. It must still complete — and
    // the WAVES must still land on the same clock, which is what would break
    // if a bonus ship were spawning outside its wave's group and holding
    // that wave's objective (and so its threat decrement) open.
    let demo = run_combat_test_chain(70, 0.0);
    let full = run_combat_test_chain(120, 0.0);

    let order: Vec<&str> = full.spawns.iter().map(|(g, _)| g.as_str()).collect();
    assert_eq!(
        order,
        (1..=8).map(|n| format!("wave_{n}")).collect::<Vec<_>>(),
        "the top tier runs the same eight-wave schedule"
    );
    assert_eq!(
        full.spawns.iter().map(|(_, t)| *t).collect::<Vec<_>>(),
        demo.spawns.iter().map(|(_, t)| *t).collect::<Vec<_>>(),
        "a clock does not care how much there is to kill — both tiers must \
         see the waves at the same authored times"
    );
    assert!(
        full.victory_at.is_some(),
        "the top tier must be winnable too — the old per-tier name lists were not"
    );
    assert!(
        full.victory_at.unwrap() >= demo.victory_at.unwrap(),
        "a tier with bonus ships riding the wave groups cannot finish sooner"
    );
    assert_eq!(
        full.trace.last().map(|(threat, ..)| *threat),
        Some(0),
        "the bonus hulls ride their wave's group, so the wave's single \
         decrement still fires once every ship in it is dead"
    );
}
