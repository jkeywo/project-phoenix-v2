---
title: Repair Runtime
type: concept
tags: [repair, damage, teams, blackboard, ai, external-repair, semantic-actions, feedback]
sources: [crates/phoenix-simulation/src/console/repair/reports.rs, crates/phoenix-simulation/src/console/repair/server.rs, crates/phoenix-simulation/src/console/repair/dispatch.rs, crates/phoenix-simulation/src/console/repair/external_server.rs, crates/phoenix-simulation/src/console/repair/visibility.rs, crates/phoenix-simulation/src/ship/coordination_systems.rs, crates/phoenix-simulation/src/ship/damage_sync.rs, crates/phoenix-sim-gameplay/src/ship/components.rs, crates/phoenix-sim-gameplay/src/modifiers/repair_teams.rs, crates/phoenix-model/src/messages.rs, crates/phoenix-simulation/src/command_admission/mod.rs, gui/components/ph-repair-teams.js, gui/stations/engineering-actions.js, gui/action-map.js]
updated: 2026-10-14
---

# Repair Runtime

`RepairPlugin` owns the server adapter for internal repair teams, Repair Backfill, blackboard publication, and external-repair registration. The deterministic team state machine lives in the Bevy-free `crates/phoenix-sim-gameplay/src/modifiers/repair_teams.rs`.

## Internal repairs

The Repair console and `operate_repair_ai` emit the same admitted payloads. The dispatch adapters in `crates/phoenix-simulation/src/console/repair/dispatch.rs` apply team dispatch and priority changes. `tick_repair_teams` advances each ship's own teams and repairs its own `EntitySystemHull`.

An on-site team sweeps repairable Systems in its assigned repair group worst-first. `SetRepairTargetPriority` can pin one system as the next job without changing the standing deterministic order. Team slots carry `SystemId` and display text so the client never reconstructs target identity from an obsolete console enum.

`repair.dispatch-team` preserves the exact authored Repair owner, selected integer
team slot and Station/Console/Core target, while `repair.prioritise-system` preserves that
owner plus the named damaged `SystemId`. Their
parameter-free bindings choose only an idle visible team/target or a damaged
row included in the owner's exact `priority_targets` projection. That list is
derived from the same sweep-group candidate predicate that applies a named
priority, then visibility-filtered, so neither the adapter nor the component
guesses reachability from partial hull rows. The owning dispatch router
returns `Applied` only for an existing, non-committed slot with a resolvable
target; named priority returns it only when an on-site sweep accepts the pin.
All other correlated outcomes are `Refused`, with no optimistic team movement.

`RepairRequestQueue` records outstanding requests for human and AI Repair
operators. It deduplicates by typed `RepairGroup` (home Station, detached Console,
or Core), retaining the worst tier and largest deficit. `reports.rs` preserves
separate contributor observations and rejects older samples from the same source.
Recovery watermarks prevent strictly older reports from reopening a resolved group;
equal-tick damage remains admissible because diagnosis precedes damage sampling.

`prune_repair_request_queue` supplies the existing operational-group diagnostic as
an explicit recovery observation. A named System reaching full HP during actual
team work contributes a completion observation; recall or Returning alone does not.
That observation does not resolve other outstanding damage in the group.
`RepairHumanAlerted` remains the separate popup latch. Queue observations and
recovery watermarks participate in capture, restore and deterministic digest.

## External repairs

`crates/phoenix-simulation/src/console/repair/external_server.rs` owns dispatch to a nearby ally or structure. It shares the ship's team pool, consumes ordinary admitted commands, and applies progress to the target's authoritative condition track. Backfill uses the same command seam.

`repair.external-dispatch` is the shared dispatch/recall semantic action on
Repair and Engineering variants. Dispatch feedback completes after the live
lock, range and free-team verdict; explicit recall is an idempotent applied
assignment. A team remains committed, travels and repairs at the authored rates
owned by the existing server state machines. A correlated external command on a
hull without the optional dispatch capability terminates once as `Refused` rather
than entering a queue with no owner.

## Publication and visibility

`publish_repair_blackboard` writes the per-ship Repair blackboard. `crates/phoenix-simulation/src/console/repair/visibility.rs` projects recipient-visible damage before the state reaches a player. `repair_state_broadcaster` sends LocalShip state at 10 Hz to the holder of the authored `repair` system.

## Coordination receive path

`process_coordination_lag` retains the generic concerns: delay, live Station
control, Popup/Consume/Suppress selection, session-token resolution, and
per-recipient Repair visibility. It emits `DeliveredCoordination` with an `Ai`
or `HumanRouted`/`HumanPopup` delivery outcome while preserving the producer-owned
presentation envelope.

`receive_repair_coordination` validates the ship's authored Repair address and
owns behavior after that seam. Both AI and `HumanRouted` deliveries merge into
`RepairRequestQueue`; the peer-identical routing outcome owns this update rather
than the presenting peer's popup. Human popups retain their existing severity/latch
rules and ordered flush. Human-supplied observation metadata is stripped at enqueue,
so a client cannot impersonate an equipment report or assert a future sample.

Repair publication filters contributor subject and source-System metadata through
the same detail permissions as the queue row. An explicit Repair feed receives the
producer Console's projection, without granting repair-team control.

## Related

- [Recorded Observations](./recorded-observations.md)

- [Damage and Repair Intent](./damage-and-repair-intent.md)
- [Modifier Coordination](./modifier-coordination.md)
- [Broadcaster Seam](./broadcaster-seam.md)
