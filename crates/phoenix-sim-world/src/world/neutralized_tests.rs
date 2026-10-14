use super::*;
use crate::world::config::scripted_trigger;
use crate::world::trigger_registry::{TriggerRuntimeState, WorldTriggerRegistry};

fn group_trigger(after_secs: f32) -> TriggerState {
    let mut trigger = scripted_trigger(TriggerCondition::OnAllNeutralized {
        group: "wave".into(),
        after_secs,
    });
    trigger.id = Some("defeat".into());
    TriggerState {
        trigger,
        fired: false,
        origin_layer: None,
        seen_destroyed: HashSet::new(),
        seen_disabled: HashSet::new(),
        last_fired_elapsed: None,
    }
}
fn names() -> HashMap<String, String> {
    [("a".into(), "uuid-a".into()), ("b".into(), "uuid-b".into())].into()
}
fn groups() -> HashMap<String, HashSet<String>> {
    [("wave".into(), ["a".into(), "b".into()].into())].into()
}
fn disabled(name: &str, value: bool) -> WorldEvent {
    WorldEvent::Disabled {
        uuid: format!("uuid-{name}"),
        disabled: value,
    }
}
fn evaluate(states: &mut Vec<TriggerState>, events: &[WorldEvent], time: f32) -> usize {
    evaluate_triggers_with_flags(states, events, &names(), &[], &groups(), time).len()
}

#[test]
fn recovery_before_the_time_gate_is_not_irreversible_destruction() {
    let mut states = vec![group_trigger(10.0)];
    assert_eq!(
        evaluate(
            &mut states,
            &[disabled("a", true), disabled("b", true)],
            1.0
        ),
        0
    );
    assert_eq!(evaluate(&mut states, &[disabled("a", false)], 2.0), 0);
    assert_eq!(evaluate(&mut states, &[], 10.0), 0);
    assert_eq!(evaluate(&mut states, &[disabled("a", true)], 11.0), 1);
    assert!(states[0].seen_destroyed.is_empty());
}

#[test]
fn mixed_destroyed_and_disabled_members_fire_and_reset_preserves_current_truth() {
    let mut states = vec![group_trigger(0.0)];
    let destroyed = WorldEvent::Destroyed {
        uuid: "uuid-a".into(),
    };
    assert_eq!(
        evaluate(&mut states, &[destroyed, disabled("b", true)], 1.0),
        1
    );
    assert_eq!(evaluate(&mut states, &[disabled("b", false)], 2.0), 0);
    assert_eq!(reset_triggers_by_id(&mut states, "defeat"), 1);
    assert_eq!(
        evaluate(&mut states, &[], 3.0),
        0,
        "spent trigger still observes recovery"
    );
    assert_eq!(evaluate(&mut states, &[disabled("b", true)], 4.0), 1);
    assert!(states[0].seen_destroyed.contains("uuid-a"));
    assert!(!states[0].seen_destroyed.contains("uuid-b"));
}

#[test]
fn unknown_empty_and_new_group_members_cannot_be_counted_as_defeated() {
    let mut states = vec![group_trigger(0.0)];
    let events = [disabled("unknown", true), disabled("a", true)];
    assert_eq!(evaluate(&mut states, &events, 1.0), 0);
    let mut membership = groups();
    membership
        .get_mut("wave")
        .unwrap()
        .insert("unspawned".into());
    assert!(evaluate_triggers_with_flags(
        &mut states,
        &[disabled("b", true)],
        &names(),
        &[],
        &membership,
        2.0
    )
    .is_empty());
    assert!(
        evaluate_triggers_with_flags(&mut states, &[], &names(), &[], &HashMap::new(), 3.0)
            .is_empty()
    );
    assert!(evaluate_triggers_with_flags(
        &mut states,
        &[],
        &names(),
        &[],
        &[("wave".into(), HashSet::new())].into(),
        3.0
    )
    .is_empty());
}

#[test]
fn disabled_entity_trigger_matches_entry_only_and_never_destruction_or_recovery() {
    let condition = TriggerCondition::OnDisabled {
        entity_name: "a".into(),
    };
    assert!(condition_matches(
        &condition,
        &disabled("a", true),
        &names(),
        &[]
    ));
    assert!(!condition_matches(
        &condition,
        &disabled("a", false),
        &names(),
        &[]
    ));
    assert!(!condition_matches(
        &condition,
        &WorldEvent::Destroyed {
            uuid: "uuid-a".into()
        },
        &names(),
        &[]
    ));
    assert!(!condition_matches(
        &TriggerCondition::OnDestroyed {
            entity_name: "a".into()
        },
        &disabled("a", true),
        &names(),
        &[]
    ));
}

#[test]
fn paused_registry_observes_recovery_and_json_continuation_keeps_current_membership() {
    let mut registry = WorldTriggerRegistry::from(vec![group_trigger(10.0)]);
    registry.evaluate_filtered(
        &[disabled("a", true), disabled("b", true)],
        &names(),
        &groups(),
        1.0,
        |_| (vec![], vec![None]),
        (|_| false, |_| true),
    );
    let json = serde_json::to_string(&registry.capture()).unwrap();
    let rows: Vec<TriggerRuntimeState> = serde_json::from_str(&json).unwrap();
    assert_eq!(rows[0].seen_disabled, vec!["uuid-a", "uuid-b"]);
    let mut restored = WorldTriggerRegistry::from(vec![group_trigger(10.0)]);
    restored.restore(&rows).unwrap();
    restored.evaluate_filtered(
        &[disabled("a", false)],
        &names(),
        &groups(),
        2.0,
        |_| (vec![], vec![None]),
        (|_| false, |_| true),
    );
    assert!(restored
        .evaluate(&[], &names(), &groups(), 10.0, |_| (vec![], vec![None]))
        .is_empty());
    assert_eq!(
        restored
            .evaluate(&[disabled("a", true)], &names(), &groups(), 11.0, |_| (
                vec![],
                vec![None]
            ))
            .len(),
        1
    );
}

#[test]
fn a_healthy_replacement_never_inherits_a_previous_actors_defeat() {
    let mut states = vec![group_trigger(10.0)];
    assert_eq!(
        evaluate(
            &mut states,
            &[disabled("a", true), disabled("b", true)],
            1.0
        ),
        0
    );
    let mut current = names();
    current.insert("a".into(), "replacement-a".into());
    assert!(
        evaluate_triggers_with_flags(&mut states, &[], &current, &[], &groups(), 10.0).is_empty()
    );
    assert_eq!(
        evaluate_triggers_with_flags(
            &mut states,
            &[WorldEvent::Disabled {
                uuid: "replacement-a".into(),
                disabled: true
            }],
            &current,
            &[],
            &groups(),
            11.0
        )
        .len(),
        1
    );
    // Even irreversible destruction belongs to a particular actor instance.
    let mut states = vec![group_trigger(10.0)];
    evaluate(
        &mut states,
        &[
            WorldEvent::Destroyed {
                uuid: "uuid-a".into(),
            },
            disabled("b", true),
        ],
        1.0,
    );
    assert!(
        evaluate_triggers_with_flags(&mut states, &[], &current, &[], &groups(), 10.0).is_empty()
    );
}

#[test]
fn single_neutralized_trigger_accepts_either_cause_but_spends_only_once() {
    for events in [
        vec![disabled("a", true)],
        vec![WorldEvent::Destroyed {
            uuid: "uuid-a".into(),
        }],
    ] {
        let mut states = vec![group_trigger(0.0)];
        states[0].trigger.condition = TriggerCondition::OnNeutralized {
            entity_name: "a".into(),
        };
        assert_eq!(evaluate(&mut states, &events, 1.0), 1);
        assert_eq!(
            evaluate(
                &mut states,
                &[WorldEvent::Destroyed {
                    uuid: "uuid-a".into()
                }],
                2.0
            ),
            0
        );
    }
    let condition = TriggerCondition::OnNeutralized {
        entity_name: "a".into(),
    };
    assert!(!condition_matches(
        &condition,
        &disabled("a", false),
        &names(),
        &[]
    ));
    assert!(!condition_matches(
        &condition,
        &disabled("b", true),
        &names(),
        &[]
    ));
}
