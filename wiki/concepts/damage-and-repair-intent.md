---
title: Damage and Repair Information
type: concept
tags: [damage, repair, engineering, station, information]
sources: [crates/phoenix-simulation/src/snapshot/application.rs, crates/phoenix-model/src/emp.rs, crates/phoenix-simulation/src/ship/emp.rs, crates/phoenix-simulation/src/snapshot/emp.rs, crates/phoenix-simulation/src/server_app/system_status.rs, crates/phoenix-sim-gameplay/src/ship/compartment_damage.rs, crates/phoenix-sim-gameplay/src/ship/physical_damage.rs, crates/phoenix-sim-gameplay/src/jobs/repair_physical.rs, crates/phoenix-simulation/src/core/broadcast/reconnect.rs, crates/phoenix-sim-gameplay/src/ship/impulse_boost_systems.rs, crates/phoenix-sim-gameplay/src/ship/physics_systems.rs, crates/phoenix-sim-gameplay/src/modifiers/coordination.rs, crates/phoenix-sim-gameplay/src/modifiers/cache.rs, crates/phoenix-sim-gameplay/src/ship/control_source.rs, crates/phoenix-simulation/src/gm/actions/action.rs, crates/phoenix-simulation/src/gm/observation/projection.rs, crates/phoenix-simulation/src/snapshot/mod.rs, crates/phoenix-simulation/src/sim_digest.rs, gui/gm-system-panel.js, pasm/spec/architecture/engineering-damage.yaml, crates/phoenix-simulation/src/ship/damage_sync.rs, crates/phoenix-simulation/src/ship/coordination_systems.rs, crates/phoenix-simulation/src/console/repair/server.rs, crates/phoenix-simulation/src/console/repair/visibility.rs, crates/phoenix-sim-gameplay/src/modifiers/repair_teams.rs, gui/console-state.js, gui/components/ph-repair-teams.js]
updated: 2026-10-16
---

# Damage and Repair Information

Equipped ships have one Gameplay `ShipDamage` owner: compartment structural HP
and component conditions. Components have no separate HP pool or precise internal
position. Planar damage traces cross the selected weighted deck; actual materials,
penetration and deposited payload determine structural loss and condition dose.
Legacy System HP remains a separate owner for unequipped props and fixtures.
Exact detail is projected by Console.

Optional EMP follows the same frozen physical impact and positive compartment
deposits. `phoenix-model/src/emp.rs` defines the separate temporary failure and
resistance receipt on the actual `ComponentState`; it does not change physical
condition, dose or repair remedies. `ship/emp.rs` supplies the derived fixed-clock
context before operation. The receipt carries absolute tick expiries, so the
existing fitting and intact-recovery transfers preserve it while stowed hardware
continues to age. `snapshot/emp.rs` checks installed and portable receipts against
the complete candidate header before writes. `server_app/system_status.rs`
reports temporary EMP separately from physical destruction.
Each receipt includes its originating resistance policy, so a different or absent
policy on the receiving hull cannot reset it. A nonempty resistance episode
retains those parameters; the current receiver controls eligibility for a new
deposit and supplies a new episode's parameters after the old stacks expire.

`HullVisibility` is the single policy used by both live Repair publication and
reconnect resync:

- every recipient receives the ship-wide structural and structurally unavailable
  fractions;
- each Console sees exact damage for its owned Systems, independent of hosting;
- the Repair Console sees true `Core` and other Systems only while a team is
  actually `Repairing` in its compartment; explicit Repair feeds carry that same view;
- detached Console equipment is owned, so it is not classified as Core;
- a `Travelling` or recalled/returning team reveals nothing yet;
- `system_hull` and `queue_depth` are filtered by that policy, while the list
  of legal dispatch targets remains whole so Engineering can send a team into
  an area it cannot inspect remotely.

`RepairPlugin` owns the token-keyed `LastBroadcastHull` cache beside that live
publisher and registers its reset and reconnect projector under the stable
`hull` lifecycle key. Reconnect reads the same `HullVisibility` projection
without mutating the cache, so another session's return cannot perturb an
existing recipient's next live delta. The reconnect owner now exposes typed,
read-only inputs through `register_reconnect_projection`; generic delivery
retains each request occurrence (including duplicate tokens), then lexical
owner order. One safe `PipeSystem` sequences the targeted Welcome collector,
coherent owner capture and transport sink under the original
`refresh_caches_on_midgame_reconnect` logical name. Its complete access union
prevents an ECS writer from splitting request collection, Blackboard and Hull
observations, or delivery. Request scratch is separate from the live Hull and
Console-publication caches. Normal plugin finalization seals registration.
The existing Repair/Identify acceptance fixture covers a two-tick command
delay, team travel and actual repair with projected visibility. Its
SDK-enabled native evidence predates the explicit Console migration and must
be renewed for that migration. See [Typed reconnect projection proof](../../docs/reconnect-projection-proof.md)
for source binding, diagnostic-test status and remaining validation limits.

## Requests, dispatch, and repair

A damage-tier crossing creates a typed `RepairRequest` through the ordinary
coordination queue. The generic lag router resolves the live Repair recipient
and applies `HullVisibility` before a human delivery crosses the typed delivery
seam, so an ineligible popup carries its tier but no exact deficit. Repair's
own receiver applies the first sub-Disabled / every Disabled-or-Destroyed alert
latch and emits accepted popups to the shared enqueue-ordered flush. AI delivery
retains its source observation and available deficit and the same receiver merges it into
`RepairRequestQueue`; that value is
a ranking input rather than player knowledge. Requests remain advisory:
Engineering may dispatch a scarce team without one. A later observation of the same
identified System supersedes older symptoms across permitted reporters; equal-tick
readings retain the worst severity, while unrelated Systems remain unresolved. Human controls and Backfill
issue the same admitted dispatch and priority commands. The canonical job/team
owner executes those assignments; the report queue remains separate received
knowledge. Permitted job rows expose assignment/outcome only, without expanding
onsite diagnosis or giving a feed recipient machinery authority.

Travel time is therefore also an information gate. Once on site, a team diagnoses its physical compartment and selects structural
or component work. Typed component priorities select inspected work there.
Leaving the compartment removes that local detail immediately. Reports retain
their typed `RepairGroup`: home Station, detached Console or true Core. Queue keys, reports, dispatch, sweeps and alert latches
share this grouping; snapshots and digests retain the typed identities. `gui/console-state.js` assembles the
projected state for the shared `ph-repair-teams` control.

## GM System availability

`ControlSourceResolver` carries a separate GM-disabled System set (#1312). Its
normal availability policy blocks human commands, AI and operation while the
latch is set. Damage synchronisation and rating changes cannot clear it, and
Restore removes only the latch: physical structure, component conditions and repair eligibility are
unchanged. `snapshot/application.rs` restores this state before continuation admission;
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
