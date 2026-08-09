// Pure delayed-action scheduling for the world engine (issue #821).
//
// Pure Rust module — no Bevy. `tick_trigger_pipeline` queues actions whose
// authored `action_delays` entry is positive as `DelayedAction`s; the
// `tick_delayed_actions` applier in `world::server` reads the clock, calls
// `partition_delayed_actions` to decide which are due, and dispatches the
// ready ones through the shared `world::dispatch` table.
//
// # Purity boundaries
//
// * **Time is injected.** The elapsed-seconds value is derived by the applier
//   from `Time::elapsed_secs()` and `WorldContentRuntime::mission_clock_anchor_secs`
//   (a Bevy resource read); this module only compares plain floats.
// * **Ordering is preserved.** Both partitions keep the queue's original
//   relative order, so two actions authored with the same delay dispatch in
//   authoring order — exactly what the old inline drain loop did.

use crate::world::config::TriggerAction;

/// An action queued for deferred dispatch via the `action_delays` trigger field.
#[derive(Clone, Debug)]
pub struct DelayedAction {
    pub action: TriggerAction,
    pub origin_layer: Option<String>,
    pub entity_name: Option<String>,
    pub fire_at_elapsed: f32,
}

/// Outcome of partitioning the pending delayed-action queue against the
/// current elapsed time.
#[derive(Debug, Default)]
pub struct DelayedActionSchedule {
    /// Actions whose `fire_at_elapsed` has been reached (`elapsed >= fire_at`),
    /// in original queue order. The applier dispatches these this tick.
    pub ready: Vec<DelayedAction>,
    /// Actions still in the future, in original queue order. The applier
    /// writes these back to `pending_delayed_actions`.
    pub still_pending: Vec<DelayedAction>,
}

/// Partition `actions` into ready / still-pending against `elapsed` seconds
/// since world load. An action fires when `elapsed >= fire_at_elapsed`
/// (boundary inclusive), matching the drain loop this replaces.
pub fn partition_delayed_actions(
    actions: Vec<DelayedAction>,
    elapsed: f32,
) -> DelayedActionSchedule {
    let mut schedule = DelayedActionSchedule::default();
    for pda in actions {
        if elapsed >= pda.fire_at_elapsed {
            schedule.ready.push(pda);
        } else {
            schedule.still_pending.push(pda);
        }
    }
    schedule
}

/// Rewrite the pending delayed-action queue when the layer at `path` unloads
/// (issue #751).
///
/// Actions owned by other layers (or the base world) are kept untouched, in
/// original order. Actions whose `origin_layer` equals `Some(path)` are
/// handled by the authored `resolve` policy:
///
/// * `resolve == false` (Cancel) — dropped from the queue (cancelled).
/// * `resolve == true` (Resolve) — kept, with `fire_at_elapsed` pulled to
///   `0.0` so the next delayed-action tick dispatches them immediately rather
///   than waiting for their original scheduled time.
///
/// Pure: no clock, no dispatch — the applier feeds the result back into
/// `pending_delayed_actions`.
pub fn partition_delayed_actions_on_unload(
    actions: Vec<DelayedAction>,
    path: &str,
    resolve: bool,
) -> Vec<DelayedAction> {
    actions
        .into_iter()
        .filter_map(|mut pda| {
            if pda.origin_layer.as_deref() == Some(path) {
                if resolve {
                    pda.fire_at_elapsed = 0.0;
                    Some(pda)
                } else {
                    None
                }
            } else {
                Some(pda)
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
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
}
