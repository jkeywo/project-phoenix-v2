use super::*;
use crate::world::config::{scripted_trigger, TriggerCondition};

fn scripted(path: &str) -> ScriptTrigger {
    ScriptTrigger {
        trigger: scripted_trigger(TriggerCondition::OnWorldLoaded),
        source_path: path.into(),
        handler: "fire".into(),
    }
}

fn declarative() -> TriggerState {
    TriggerState {
        trigger: scripted_trigger(TriggerCondition::OnWorldLoaded),
        fired: false,
        origin_layer: None,
        seen_destroyed: HashSet::new(),
        last_fired_elapsed: None,
    }
}

#[test]
fn mixed_entries_evaluate_in_order_with_their_own_optional_handler() {
    let mut registry = WorldTriggerRegistry::default();
    registry.push(declarative());
    registry.append_scripted([scripted("a.rhai")], Some("a.toml"));
    registry.push(declarative());
    registry.append_scripted([scripted("c.rhai")], Some("c.toml"));
    let flags = FlagStore::default();
    let fired = registry.evaluate(
        &[WorldEvent::WorldLoaded],
        &HashMap::new(),
        &HashMap::new(),
        3.0,
        |origin| (vec![&flags], vec![origin.map(str::to_owned)]),
    );
    assert_eq!(fired.len(), 4);
    assert_eq!(
        fired
            .iter()
            .map(|fire| fire.handler.as_ref().map(|h| h.script_path.as_str()))
            .collect::<Vec<_>>(),
        vec![None, Some("a.rhai"), None, Some("c.rhai")]
    );
    assert_eq!(
        fired
            .iter()
            .map(|fire| fire.context.origin_layer.as_deref())
            .collect::<Vec<_>>(),
        vec![None, Some("a.toml"), None, Some("c.toml")]
    );
    assert!(registry
        .iter()
        .all(|state| state.fired && state.last_fired_elapsed == Some(3.0)));
    assert!(registry
        .evaluate(
            &[WorldEvent::WorldLoaded],
            &HashMap::new(),
            &HashMap::new(),
            4.0,
            |origin| (vec![&flags], vec![origin.map(str::to_owned)])
        )
        .is_empty());
}

#[test]
fn same_length_middle_removal_and_append_changes_generation_and_keeps_order() {
    let mut registry = WorldTriggerRegistry::default();
    for owner in ["a", "b", "c"] {
        registry.append_scripted([scripted(owner)], Some(owner));
    }
    let original = registry.generation();
    assert_eq!(registry.remove_layer("b"), 1);
    registry.append_scripted([scripted("b")], Some("b"));
    assert_eq!(registry.len(), 3);
    assert_ne!(registry.generation(), original);
    assert_eq!(
        registry
            .iter()
            .map(|state| state.origin_layer.as_deref())
            .collect::<Vec<_>>(),
        vec![Some("a"), Some("c"), Some("b")]
    );
    assert_eq!(registry.handler(1).unwrap().script_path, "c");
    let unchanged = registry.generation();
    assert_eq!(registry.remove_layer("missing"), 0);
    registry.append_scripted([], None);
    assert_eq!(registry.generation(), unchanged);
}

#[test]
fn continuation_restore_keeps_handlers_and_replaces_every_latch() {
    let mut registry = WorldTriggerRegistry::default();
    registry.append_scripted([scripted("a"), scripted("b")], Some("layer"));
    let original_generation = registry.generation();
    let rows = vec![
        TriggerRuntimeState {
            index: 1,
            fired: true,
            seen_destroyed: vec!["z".into(), "a".into()],
            last_fired_elapsed: Some(8.0),
        },
        TriggerRuntimeState {
            index: 0,
            ..Default::default()
        },
    ];
    registry.restore(&rows).unwrap();
    assert_ne!(registry.generation(), original_generation);
    assert_eq!(registry.handler(0).unwrap().script_path, "a");
    assert_eq!(registry.handler(1).unwrap().script_path, "b");
    assert_eq!(registry[1].origin_layer.as_deref(), Some("layer"));
    let captured = registry.capture();
    assert_eq!(captured[0], rows[1]);
    assert_eq!(captured[1].seen_destroyed, ["a", "z"]);
    assert_eq!(captured[1].last_fired_elapsed, Some(8.0));
    let flags = FlagStore::default();
    let fired = registry.evaluate(
        &[WorldEvent::WorldLoaded],
        &HashMap::new(),
        &HashMap::new(),
        9.0,
        |origin| (vec![&flags], vec![origin.map(str::to_owned)]),
    );
    assert_eq!(
        fired.len(),
        1,
        "the restored spent trigger must not run again"
    );
    assert_eq!(fired[0].handler.as_ref().unwrap().script_path, "a");
}

#[test]
fn incompatible_continuation_is_refused_before_any_row_or_generation_changes() {
    let mut registry = WorldTriggerRegistry::default();
    registry.append_scripted([scripted("a"), scripted("b")], None);
    let original = registry.capture();
    let generation = registry.generation();
    for rows in [
        vec![TriggerRuntimeState {
            fired: true,
            ..Default::default()
        }],
        vec![
            TriggerRuntimeState {
                fired: true,
                ..Default::default()
            };
            2
        ],
        vec![
            TriggerRuntimeState {
                fired: true,
                ..Default::default()
            },
            TriggerRuntimeState {
                index: u32::MAX,
                ..Default::default()
            },
        ],
    ] {
        assert!(registry.restore(&rows).is_err());
        assert_eq!(
            registry.capture(),
            original,
            "even an earlier valid row must stay untouched"
        );
        assert_eq!(registry.generation(), generation);
        assert_eq!(registry.handler(1).unwrap().script_path, "b");
    }
}

#[test]
fn continuation_row_preserves_the_existing_ron_shape() {
    let stored = r#"(index:7,fired:true,seen_destroyed:["a","z"],last_fired_elapsed:Some(3.5))"#;
    let row: TriggerRuntimeState = ron::from_str(stored).unwrap();
    assert_eq!(ron::to_string(&row).unwrap(), stored);
    assert_eq!(
        ron::to_string(&TriggerRuntimeState::default()).unwrap(),
        "(index:0,fired:false)"
    );
}

#[test]
fn filtered_evaluation_pauses_before_state_and_skips_after_latching() {
    let staged = ["paused", "skipped", "ordinary"].map(|id| {
        let mut row = scripted(id);
        row.trigger.id = Some(id.into());
        row
    });
    let mut registry = WorldTriggerRegistry::default();
    registry.append_scripted(staged, None);
    let flags = FlagStore::default();
    let mut skipped = false;
    let fired = registry.evaluate_filtered(
        &[WorldEvent::WorldLoaded],
        &HashMap::new(),
        &HashMap::new(),
        3.0,
        |_| (vec![&flags], vec![None]),
        (
            |state| state.trigger.id.as_deref() != Some("paused"),
            |state| {
                if state.trigger.id.as_deref() == Some("skipped") {
                    skipped = true;
                    false
                } else {
                    true
                }
            },
        ),
    );
    assert!(skipped);
    assert!(!registry[0].fired);
    assert_eq!(registry[0].last_fired_elapsed, None);
    assert!(registry[1].fired);
    assert_eq!(registry[1].last_fired_elapsed, Some(3.0));
    assert_eq!(fired.len(), 1);
    assert_eq!(fired[0].handler.as_ref().unwrap().script_path, "ordinary");
    assert_eq!(fired[0].trigger_id.as_deref(), Some("ordinary"));
}

#[test]
fn manual_after_middle_layer_replacement_keeps_owned_handlers_and_authored_order() {
    let mut registry = WorldTriggerRegistry::default();
    for (owner, function) in [("a", "alpha"), ("b", "old_beta"), ("c", "gamma")] {
        let mut row = scripted("shared.rhai");
        row.handler = function.into();
        row.trigger.id = Some(owner.into());
        registry.append_scripted([row], Some(owner));
    }
    registry.remove_layer("b");
    let mut replacement = scripted("shared.rhai");
    replacement.handler = "new_beta".into();
    replacement.trigger.id = Some("b".into());
    registry.append_scripted([replacement], Some("b"));
    let flags = FlagStore::default();
    let mut pending = ["b", "a", "c", "missing"]
        .map(str::to_owned)
        .into_iter()
        .collect();
    let fired = registry.fire_manual(
        &mut pending,
        5.0,
        |state| state.trigger.id.clone(),
        |_| vec![&flags],
    );
    assert!(pending.is_empty());
    assert_eq!(
        fired
            .iter()
            .map(|row| (
                row.trigger_id.as_deref().unwrap(),
                row.handler.as_ref().unwrap().fn_name.as_str(),
                row.context.origin_layer.as_deref().unwrap()
            ))
            .collect::<Vec<_>>(),
        vec![
            ("a", "alpha", "a"),
            ("c", "gamma", "c"),
            ("b", "new_beta", "b")
        ]
    );
    // Removing the live table cannot change the already-selected handler pair.
    registry.clear();
    assert_eq!(fired[2].handler.as_ref().unwrap().fn_name, "new_beta");
}

#[test]
fn manual_cooldown_retains_arm_and_restore_resumes_the_same_paired_handler() {
    let mut row = scripted("repeat.rhai");
    row.trigger.id = Some("repeat".into());
    row.trigger.repeat = true;
    row.trigger.cooldown_secs = Some(10.0);
    let mut registry = WorldTriggerRegistry::default();
    registry.append_scripted([row.clone()], Some("layer"));
    let flags = FlagStore::default();
    let mut pending = ["repeat".to_string()].into_iter().collect();
    assert_eq!(
        registry
            .fire_manual(
                &mut pending,
                5.0,
                |state| state.trigger.id.clone(),
                |_| vec![&flags]
            )
            .len(),
        1
    );
    let saved = registry.capture();
    let mut restored = WorldTriggerRegistry::default();
    restored.append_scripted([row], Some("layer"));
    restored.restore(&saved).unwrap();
    assert_eq!(restored.capture(), saved);
    // The same elapsed stamp cannot create a second occurrence in this tick.
    pending.insert("repeat".into());
    let first = registry.fire_manual(
        &mut pending.clone(),
        5.0,
        |state| state.trigger.id.clone(),
        |_| vec![&flags],
    );
    let resumed = restored.fire_manual(
        &mut pending,
        5.0,
        |state| state.trigger.id.clone(),
        |_| vec![&flags],
    );
    assert!(first.is_empty());
    assert!(resumed.is_empty());
    assert!(pending.contains("repeat"));
    assert_eq!(restored.capture(), registry.capture());
    let resumed = restored.fire_manual(
        &mut pending,
        15.0,
        |state| state.trigger.id.clone(),
        |_| vec![&flags],
    );
    assert_eq!(resumed.len(), 1);
    assert_eq!(
        resumed[0].handler.as_ref().unwrap().script_path,
        "repeat.rhai"
    );
    assert!(pending.is_empty());
}
