---
title: RepairPlugin
---

# RepairPlugin

Extracted from `simulation.rs` as part of the simulation split series (issue [#250](https://github.com/jkeywo/project-phoenix-v2/issues/250)).

## Ownership

`RepairPlugin` owns all breakdown-queue and repair-team state and message handling for the Repair console.

### Systems

| System | Responsibility |
|---|---|
| `handle_repair` | Processes `Repair { shape }` from the Repair console holder; dispatches team on match, penalises team on mismatch or empty queue |
| `tick_repair_teams` | Advances team progress each frame; restores hull HP for each completed team |
| `broadcast_repair_icons` | Sends `ShowRepairIcon` / `ClearRepairIcon` deltas to console holders; picks one decoy from undamaged consoles |

### Resources

| Resource | Purpose |
|---|---|
| `ShipRepairTeams` | Wraps the pure-Rust `RepairTeams` state machine (three slots: Idle / Repairing / Cooldown) |
| `BreakdownQueueResource` | Breakdown queue, cumulative-damage counter, and per-session RNG |
| `RepairIconState` | Delta-tracking map from console to last-sent shape, plus decoy RNG |

### Constants

| Constant | Value | Used by |
|---|---|---|
| `REPAIR_TEAM_HP` | `10.0` | `tick_repair_teams` (HP restored per completed team) |

## Registration

```rust
.add_plugins(crate::repair_plugin::RepairPlugin)
```

Registered as a sub-plugin of `SimulationPlugin` in `src/simulation.rs`. The module is declared in `src/lib.rs`.

## Broadcaster

`repair_state_broadcaster()` (defined in `repair_plugin.rs` and registered by `RepairPlugin`) reads:
- `ShipRepairTeams` — for slot states (Repairing / Cooldown / Idle)
- `BreakdownQueueResource` — for the current front-of-queue breakdown shape

Produces `ServerMessage::RepairState` sent to the Repair console holder at 10 Hz.

## Tests

Tests live in `src/repair_plugin.rs` under `#[cfg(test)] mod tests`.

| Test | Behaviour verified |
|---|---|
| `non_repair_sender_is_ignored` | Non-Repair holder pressing a shape is a no-op |
| `correct_shape_dispatches_team_and_pops_queue` | Matching shape dispatches team 0 and empties the queue |
| `wrong_shape_penalises_team_and_leaves_queue` | Wrong shape puts team 0 on cooldown, queue unchanged |
| `all_busy_teams_ignore_further_presses` | When all three teams are occupied, presses are silently dropped |
| `empty_queue_press_penalises_team` | Pressing when queue is empty penalises the lowest free team |
| `repair_team_completion_restores_hp` | Completed team tick restores `REPAIR_TEAM_HP` hull points |
| `repair_state_shows_in_progress` | `RepairState { in_progress: true }` broadcast after dispatch |
| `repair_state_shows_penalty` | `RepairState { penalty: true }` broadcast after wrong-shape press |
| `push_assigns_real_icon_to_damaged_console` | Damaged console holder receives `ShowRepairIcon` with correct shape |
| `push_assigns_decoy_to_undamaged_console` | At least one undamaged console holder receives a decoy `ShowRepairIcon` |
| `pop_clears_real_icon` | Queue pop triggers `ClearRepairIcon` to the previously damaged console holder |
| `old_decoy_cleared_before_new_decoy_assigned` | Decoy replacement sends `ClearRepairIcon` to the old decoy holder |
| `empty_queue_clears_all_icons` | Emptying the queue clears all icons and sends no `ShowRepairIcon` |
| `no_undamaged_consoles_shows_no_decoy` | When all consoles are damaged, no extra decoy is added |

## Sources

- `src/repair_plugin.rs`
- `src/simulation.rs` (pub use re-exports)
- Issue [#250](https://github.com/jkeywo/project-phoenix-v2/issues/250)
- [Console Plugin Pattern](./console-plugin-pattern.md)
- [Broadcaster Seam](./broadcaster-seam.md)
