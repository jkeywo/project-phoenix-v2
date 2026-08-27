---
title: Repair Runtime
type: concept
tags: [repair, damage, teams, blackboard, ai, external-repair]
sources: [src/console/repair/server.rs, src/console/repair/dispatch.rs, src/console/repair/external_server.rs, src/console/repair/visibility.rs, src/modifiers/repair_teams.rs, src/core/messages.rs, gui/components/ph-repair-teams.js]
updated: 2026-08-27
---

# Repair Runtime

`RepairPlugin` owns the server adapter for internal repair teams, Repair Backfill, blackboard publication, and external-repair registration. The deterministic team state machine lives in the Bevy-free `src/modifiers/repair_teams.rs`.

## Internal repairs

The Repair console and `operate_repair_ai` emit the same admitted payloads. The dispatch adapters in `src/console/repair/dispatch.rs` apply team dispatch and priority changes. `tick_repair_teams` advances each ship's own teams and repairs its own `EntitySystemHull`.

An on-site team sweeps repairable systems at its station worst-first. `SetRepairTargetPriority` can pin one system as the next job without changing the standing deterministic order. Team slots carry `SystemId` and display text so the client never reconstructs target identity from an obsolete console enum.

## External repairs

`src/console/repair/external_server.rs` owns dispatch to a nearby ally or structure. It shares the ship's team pool, consumes ordinary admitted commands, and applies progress to the target's authoritative condition track. Backfill uses the same command seam.

## Publication and visibility

`publish_repair_blackboard` writes the per-ship Repair blackboard. `src/console/repair/visibility.rs` projects recipient-visible damage before the state reaches a player. `repair_state_broadcaster` sends LocalShip state at 10 Hz to the holder of the authored `repair` system.

## Related

- [Damage and Repair Intent](./damage-and-repair-intent.md)
- [Modifier Coordination](./modifier-coordination.md)
- [Broadcaster Seam](./broadcaster-seam.md)
