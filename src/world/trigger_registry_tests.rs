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
