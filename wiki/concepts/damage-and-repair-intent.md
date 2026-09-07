---
title: Damage and Repair Information
type: concept
tags: [damage, repair, engineering, station, information]
sources: [src/ship/impulse_boost_systems.rs, src/ship/physics_systems.rs, src/modifiers/coordination.rs, src/modifiers/cache.rs, src/ship/control_source.rs, src/gm_action.rs, src/gm_projection.rs, src/snapshot.rs, src/sim_digest.rs, gui/gm-system-panel.js, pasm/spec/architecture/engineering-damage.yaml, src/ship/damage_sync.rs, src/ship/coordination_systems.rs, src/console/repair/server.rs, src/console/repair/visibility.rs, src/modifiers/repair_teams.rs, gui/console-state.js, gui/components/ph-repair-teams.js]
updated: 2026-09-07
---

# Damage and Repair Information

Damage is authoritative per fine system, but exact detail is projected by role.
`HullVisibility` is the single policy used by both live Repair publication and
reconnect resync:

- every recipient receives the ship-wide aggregate and destroyed-hull
  fractions;
- a station holder sees exact damage for that station's systems;
- Engineering always sees the `Core` bucket, and sees a non-Core station only
  while one of its teams is actually `Repairing` there;
- a `Travelling` or recalled/returning team reveals nothing yet;
- `system_hull` and `queue_depth` are filtered by that policy, while the list
  of legal dispatch targets remains whole so Engineering can send a team into
  an area it cannot inspect remotely.

`RepairPlugin` owns the token-keyed `LastBroadcastHull` cache beside that live
publisher and registers its reset and reconnect projector under the stable
`hull` lifecycle key. Reconnect reads the same `HullVisibility` projection
without mutating the cache, so another session's return cannot perturb an
existing recipient's next live delta.

## Requests, dispatch, and repair

A damage-tier crossing creates a typed `RepairRequest` through the ordinary
coordination queue. The generic lag router resolves the live Repair recipient
and applies `HullVisibility` before a human delivery crosses the typed delivery
seam, so an ineligible popup carries its tier but no exact deficit. Repair's
own receiver applies the first sub-Disabled / every Disabled-or-Destroyed alert
latch and emits accepted popups to the shared enqueue-ordered flush. AI delivery
retains the exact host-internal deficit and the same receiver merges it into
`RepairRequestQueue`; that value is
a ranking input rather than player knowledge. Requests remain advisory:
Engineering may dispatch a scarce team without one. Human controls and Backfill
issue the same admitted dispatch and priority commands.

Travel time is therefore also an information gate. Once on site, a team sweeps
the damaged fine systems owned by that station; its standing ordinal priority
and optional pinned target choose among the eligible rows. Leaving the station
removes that local detail immediately. `gui/console-state.js` assembles the
projected state for the shared `ph-repair-teams` control.

## GM System availability

`ControlSourceResolver` carries a separate GM-disabled System set (#1312). Its
normal availability policy blocks human commands, AI and operation while the
latch is set. Damage synchronisation and rating changes cannot clear it, and
Restore removes only the latch: HP, damage tiers and repair eligibility are
unchanged. `snapshot.rs` restores this state before continuation admission;
`sim_digest.rs` folds the sorted ship/System identities.

Ongoing impulse and boost transitions enforce availability too: Disable cancels
the drive, and Restore permits a new command without restarting the cancelled
operation. Passive radar uses a distinct `SystemDisabled` modifier contribution
that suppresses its range slot to zero for the authored System instance IDs.
Damage contributions remain intact, so Restore recovers the current
damage-limited range rather than healing the radar. Snapshot restore rebuilds
these derived ranges before the first Input consumer runs.

The map inspector uses `gui/gm-system-panel.js` with the configured System rows
and structured results in `gm_entity`. Confirmation is injected under the
`system.disable` and `system.restore` categories; waiting for confirmation has
not yet submitted a command.

## Related

- [Repair Plugin](./repair-plugin.md)
- [Modifier Coordination](./modifier-coordination.md)
- [Information Parity](./information-parity-audit.md)
