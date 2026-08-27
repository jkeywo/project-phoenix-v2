---
title: Damage and Repair Information
type: concept
tags: [damage, repair, engineering, station, information]
sources: [pasm/spec/architecture/engineering-damage.yaml, src/ship/damage_sync.rs, src/ship/coordination_systems.rs, src/console/repair/server.rs, src/console/repair/visibility.rs, src/modifiers/repair_teams.rs, gui/console-state.js, gui/components/ph-repair-teams.js]
updated: 2026-08-27
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

## Related

- [Repair Plugin](./repair-plugin.md)
- [Modifier Coordination](./modifier-coordination.md)
- [Information Parity](./information-parity-audit.md)
