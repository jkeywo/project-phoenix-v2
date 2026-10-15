---
title: Weapons Intent
type: concept
tags: [weapons, tactical, phaser, blaster, torpedo, replication, reconnect]
sources: [crates/phoenix-simulation/src/console/weapons/mod.rs, crates/phoenix-simulation/src/console/weapons/server.rs, crates/phoenix-simulation/src/console/weapons/blackboard.rs, crates/phoenix-simulation/src/console/weapons/beam.rs, crates/phoenix-simulation/src/console/weapons/blaster.rs, crates/phoenix-simulation/src/console/weapons/torpedo.rs, crates/phoenix-simulation/src/core/broadcast/lifecycle.rs, crates/phoenix-sim-gameplay/src/weapons/blaster.rs, crates/phoenix-sim-gameplay/src/weapons/torpedo.rs, crates/phoenix-sim-gameplay/src/entities/config.rs, gui/components/ph-phasers-controls.js, gui/components/ph-blasters-controls.js, gui/components/ph-torpedo-controls.js, crates/phoenix-sim-gameplay/src/weapons/mount.rs, crates/phoenix-sim-gameplay/src/weapons/pattern.rs, crates/phoenix-sim-gameplay/src/entities/config/weapon_definition.rs, gui/components/weapon-target-controls.js, gui/stations/weapon-target-actions.js, editor/workshop-weapon-groups.js, gui/workshop-weapon-groups-panel.js]
updated: 2026-10-15
---

# Weapons Intent

Tactical controls three independent, ship-authored weapon families through admitted `ControlSystem` commands.

Shipped armament is authored as `[[weapon]]` entries with a common
`weapon.mount`, a typed `weapon.delivery`, markers/patterns and its ordinary AI
declaration. `strategy_id` preserves the existing bank/tube identity where the
composition entry uses a unique name. Entity loading validates and compiles
these definitions once into the beam, bolt or guided strategy parameters;
there is no second runtime weapon store. Family-specific authored blocks remain
supported parameter shorthand and cannot duplicate the same strategy identity.

Gameplay `weapons/mount.rs` owns the common planar geometry and impact payload
contract. The existing `weapons/pattern.rs` resolves schedules and barrel
sequences in stable authored order. Bolts use timed firing steps; torpedoes use
the sequence for origins while loaded rounds and burst cadence retain authority
over ammunition. These shared mechanisms introduce no new combat controller.

- **Phasers:** sustained, target-locked beams. A ready bank must have its target inside its arc and range; damage accumulates during the beam, then that bank cools down.
- **Blasters:** straight, non-homing projectile volleys. Each bank predicts target motion at firing time, may require holding to charge, then fires its configured volley; bolts have their own speed, range-derived lifetime, collision radius, damage, and shield pierce.
- **Torpedoes:** guided, proximity-detonating projectiles. Tubes have arcs and load state, share a magazine, and snapshot damage/pierce at launch.

Preparation and sustained beams spend affordable continuous local energy; an
instant firing step pays every simultaneous barrel/projectile before ammo,
cooldown or effects change. Pending torpedo burst rounds remain physical onboard
commitments until emitted. Zero intake does not silence a charged weapon.
Explicit Tactical restraint and scenario hold/release orders share one canonical
intent; AI reads it through its permitted Console publication.

Torpedo ammunition now uses [shared resource stocks](resource-stores.md): loading claims a round, completion consumes it into the tube, and cancellation releases it. The existing readout remains available magazine quantity. Configured docked replenishment honours tube-held space.

All three are ship-specific TOML capabilities. Tactical receives configured arcs plus live bank/tube state; the current weapons target is shared by player controls, Backfill, the Tactical blackboard, and the viewscreen projection. Current limitations and future work belong in PASM and GitHub rather than this navigation page.

## Client projection lifecycle

`weapons_update_broadcaster` computes the current `WeaponsUpdate` at 10 Hz and sends it only to the holder of the ship's authored Weapons Station (`Audience::HoldingWeapons`). `WeaponsPlugin` owns both the corresponding `LastWeaponsUpdate` delta cache and the `WeaponsUpdateFirstTick` first-tick suppression state, classifies both as cache state, and registers their per-run reset plus the reconnect projector under the stable `weapons` lifecycle key.

A mid-round reconnect receives that same live computation only when its session currently holds the Weapons Station. The targeted projection does not read or mutate either publisher cache, so it cannot force a resend or change the next delta seen by another connected player.

## Tactical operator actions

The collapsed weapon-assignment controls preserve the simple primary-target
journey. Each owned Human bank or tube can hold fire, choose a current permitted
contact, follow its prepared group, or return to the primary target. A group
choice sends one correlated command per currently controllable member; it keeps
an individual override while updating the underlying group choice. Delegated
and foreign machinery remains read-only. The browser preserves focused pickers
and drafts across snapshots and never rewrites received assignment state.
`gui/components/weapon-target-controls.js` presents this view;
`gui/stations/weapon-target-actions.js` checks the exact owner and contact before
using the ordinary action map.

Workshop's prepared-group form edits `weapons_console.group` through the accepted
source document and history. `editor/workshop-weapon-groups.js` validates actual
weapon membership, materialises inherited edits explicitly, and writes an empty
replacement when clearing inherited groups. Ordinary System edits refuse
dangling members. This authors groups for a subsequent Preview/Test or load;
it does not regroup installed machinery during play.

The four hull variants share one context-scoped Tactical action catalogue. Target choice, phaser mode/fire, blaster charge/fire/cancel, and torpedo volley/fire controls send the pre-existing console actions through the normal action map and admitted command path. A visible blaster press sends exactly one `ChargeBlasterStart` semantic action (instant banks fire from it; charged banks complete on the host); release sends no legacy alias action. Parameter choices are checked against the live Tactical projection, and unqualified keyboard/gamepad shortcuts choose the same ready bank or tube a visible control would, while bank/tube readiness, ammunition, cooldown, and Combat Lock remain server facts. Correlated Tactical commands receive their terminal feedback only from the owning consumer; target selection never patches the browser state optimistically.
