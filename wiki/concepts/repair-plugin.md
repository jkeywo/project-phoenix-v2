---
title: Repair Runtime
type: concept
tags: [repair, damage, teams, blackboard, ai, external-repair, semantic-actions, feedback]
sources: [src/console/repair/server.rs, src/console/repair/dispatch.rs, src/console/repair/external_server.rs, src/console/repair/visibility.rs, src/ship/coordination_systems.rs, src/ship/damage_sync.rs, src/ship/components.rs, src/modifiers/repair_teams.rs, src/core/messages.rs, src/command_admission/mod.rs, gui/components/ph-repair-teams.js, gui/stations/engineering-actions.js, gui/action-map.js]
updated: 2026-08-31
---

# Repair Runtime

`RepairPlugin` owns the server adapter for internal repair teams, Repair Backfill, blackboard publication, and external-repair registration. The deterministic team state machine lives in the Bevy-free `src/modifiers/repair_teams.rs`.

## Internal repairs

The Repair console and `operate_repair_ai` emit the same admitted payloads. The dispatch adapters in `src/console/repair/dispatch.rs` apply team dispatch and priority changes. `tick_repair_teams` advances each ship's own teams and repairs its own `EntitySystemHull`.

An on-site team sweeps repairable systems at its station worst-first. `SetRepairTargetPriority` can pin one system as the next job without changing the standing deterministic order. Team slots carry `SystemId` and display text so the client never reconstructs target identity from an obsolete console enum.

`repair.dispatch-team` preserves the exact authored Repair owner, selected integer
team slot and Station/core target, while `repair.prioritise-system` preserves that
owner plus the named damaged `SystemId`. Their
parameter-free bindings choose only an idle visible team/target or a damaged
row included in the owner's exact `priority_targets` projection. That list is
derived from the same sweep-group candidate predicate that applies a named
priority, then visibility-filtered, so neither the adapter nor the component
guesses reachability from partial hull rows. The owning dispatch router
returns `Applied` only for an existing, non-committed slot with a resolvable
target; named priority returns it only when an on-site sweep accepts the pin.
All other correlated outcomes are `Refused`, with no optimistic team movement.

`RepairRequestQueue` is the per-ship AI request queue. It deduplicates by
Station and retains the worst tier, largest exact deficit, and deterministic
Station-id tie-break. `RepairHumanAlerted` is the separate per-ship human alert
latch and is cleared when the reported damage group returns to Operational.

## External repairs

`src/console/repair/external_server.rs` owns dispatch to a nearby ally or structure. It shares the ship's team pool, consumes ordinary admitted commands, and applies progress to the target's authoritative condition track. Backfill uses the same command seam.

`repair.external-dispatch` is the shared dispatch/recall semantic action on
Repair and Engineering variants. Dispatch feedback completes after the live
lock, range and free-team verdict; explicit recall is an idempotent applied
assignment. A team remains committed, travels and repairs at the authored rates
owned by the existing server state machines. A correlated external command on a
hull without the optional dispatch capability terminates once as `Refused` rather
than entering a queue with no owner.

## Publication and visibility

`publish_repair_blackboard` writes the per-ship Repair blackboard. `src/console/repair/visibility.rs` projects recipient-visible damage before the state reaches a player. `repair_state_broadcaster` sends LocalShip state at 10 Hz to the holder of the authored `repair` system.

## Coordination receive path

`process_coordination_lag` retains the generic concerns: delay, live Station
control, Popup/Consume/Suppress selection, session-token resolution, and
per-recipient Repair visibility. It emits `DeliveredCoordination` with an `Ai`
or `HumanPopup` delivery outcome while preserving the producer-owned
presentation envelope.

`receive_repair_coordination` validates the ship's authored Repair address and
owns behavior after that seam. AI requests merge into `RepairRequestQueue`.
Human requests apply the first sub-Disabled report / every Disabled-or-Destroyed
rule before emitting the already-projected popup to the shared ordered flush;
they never mutate the AI queue. The router-assigned sequence keeps accepted
Repair popups in global enqueue order with same-tick generic Station and Ship
fan-out, while the generic lag router has no query for either Repair state type.

## Related

- [Damage and Repair Intent](./damage-and-repair-intent.md)
- [Modifier Coordination](./modifier-coordination.md)
- [Broadcaster Seam](./broadcaster-seam.md)
