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
#[path = "delayed_tests.rs"]
mod tests;
