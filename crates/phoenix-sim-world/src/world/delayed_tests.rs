use super::*;

fn delayed(name: &str, fire_at: f32) -> DelayedAction {
    DelayedAction {
        action: TriggerAction::SetWorldFlag {
            name: name.to_string(),
        },
        origin_layer: None,
        entity_name: None,
        fire_at_elapsed: fire_at,
    }
}

fn flag_name(pda: &DelayedAction) -> &str {
    match &pda.action {
        TriggerAction::SetWorldFlag { name } => name,
        other => panic!("test fixture only queues SetWorldFlag, got {other:?}"),
    }
}

fn delayed_in_layer(name: &str, fire_at: f32, layer: Option<&str>) -> DelayedAction {
    DelayedAction {
        action: TriggerAction::SetWorldFlag {
            name: name.to_string(),
        },
        origin_layer: layer.map(str::to_string),
        entity_name: None,
        fire_at_elapsed: fire_at,
    }
}

#[test]
fn unload_cancel_drops_only_the_layers_actions() {
    let queue = vec![
        delayed_in_layer("base", 5.0, None),
        delayed_in_layer("sub_a", 8.0, Some("sub.toml")),
        delayed_in_layer("other", 9.0, Some("other.toml")),
        delayed_in_layer("sub_b", 3.0, Some("sub.toml")),
    ];
    let out = partition_delayed_actions_on_unload(queue, "sub.toml", false);
    let names: Vec<&str> = out.iter().map(flag_name).collect();
    assert_eq!(
        names,
        vec!["base", "other"],
        "cancel drops exactly the unloaded layer's actions, preserving order"
    );
}

#[test]
fn unload_resolve_keeps_layer_actions_and_pulls_fire_time_to_zero() {
    let queue = vec![
        delayed_in_layer("base", 5.0, None),
        delayed_in_layer("sub_a", 8.0, Some("sub.toml")),
        delayed_in_layer("sub_b", 3.0, Some("sub.toml")),
    ];
    let out = partition_delayed_actions_on_unload(queue, "sub.toml", true);
    let names: Vec<&str> = out.iter().map(flag_name).collect();
    assert_eq!(names, vec!["base", "sub_a", "sub_b"]);
    // The layer's actions are now immediately ready; the base action is
    // untouched.
    for pda in &out {
        if pda.origin_layer.as_deref() == Some("sub.toml") {
            assert_eq!(
                pda.fire_at_elapsed, 0.0,
                "resolved actions fire immediately"
            );
        } else {
            assert_eq!(pda.fire_at_elapsed, 5.0, "other layers untouched");
        }
    }
}

#[test]
fn unload_unknown_path_is_a_noop() {
    let queue = vec![
        delayed_in_layer("base", 5.0, None),
        delayed_in_layer("sub_a", 8.0, Some("sub.toml")),
    ];
    let out = partition_delayed_actions_on_unload(queue, "ghost.toml", false);
    assert_eq!(out.len(), 2, "unloading an unrelated path changes nothing");
}

#[test]
fn empty_queue_yields_empty_schedule() {
    let schedule = partition_delayed_actions(Vec::new(), 10.0);
    assert!(schedule.ready.is_empty());
    assert!(schedule.still_pending.is_empty());
}

#[test]
fn all_ready_when_every_fire_time_has_elapsed() {
    let schedule = partition_delayed_actions(vec![delayed("a", 1.0), delayed("b", 2.0)], 5.0);
    assert_eq!(schedule.ready.len(), 2);
    assert!(schedule.still_pending.is_empty());
}

#[test]
fn none_ready_when_every_fire_time_is_in_the_future() {
    let schedule = partition_delayed_actions(vec![delayed("a", 6.0), delayed("b", 7.5)], 5.0);
    assert!(schedule.ready.is_empty());
    assert_eq!(schedule.still_pending.len(), 2);
}

#[test]
fn boundary_fire_at_equal_to_elapsed_is_ready() {
    // `elapsed >= fire_at_elapsed` — the boundary fires, matching the
    // inline drain loop this partition replaced.
    let schedule = partition_delayed_actions(vec![delayed("edge", 5.0)], 5.0);
    assert_eq!(schedule.ready.len(), 1);
    assert!(schedule.still_pending.is_empty());
}

#[test]
fn mixed_queue_splits_by_fire_time() {
    let schedule = partition_delayed_actions(
        vec![
            delayed("due", 1.0),
            delayed("later", 9.0),
            delayed("now", 5.0),
        ],
        5.0,
    );
    assert_eq!(schedule.ready.len(), 2);
    assert_eq!(schedule.still_pending.len(), 1);
    assert_eq!(flag_name(&schedule.still_pending[0]), "later");
}

#[test]
fn both_partitions_preserve_original_queue_order() {
    let schedule = partition_delayed_actions(
        vec![
            delayed("r1", 0.0),
            delayed("p1", 8.0),
            delayed("r2", 2.0),
            delayed("p2", 6.0),
            delayed("r3", 1.0),
        ],
        5.0,
    );
    let ready: Vec<&str> = schedule.ready.iter().map(flag_name).collect();
    let pending: Vec<&str> = schedule.still_pending.iter().map(flag_name).collect();
    assert_eq!(ready, vec!["r1", "r2", "r3"]);
    assert_eq!(pending, vec!["p1", "p2"]);
}
