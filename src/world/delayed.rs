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
//   from `Time::elapsed_secs()` and `WorldContentRuntime::world_loaded_at_secs`
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
