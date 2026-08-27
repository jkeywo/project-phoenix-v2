---
title: Weapons Intent
type: concept
tags: [weapons, tactical, phaser, blaster, torpedo, replication, reconnect]
sources: [src/console/weapons/mod.rs, src/console/weapons/server.rs, src/console/weapons/blackboard.rs, src/console/weapons/beam.rs, src/console/weapons/blaster.rs, src/console/weapons/torpedo.rs, src/core/broadcast/lifecycle.rs, src/weapons/blaster.rs, src/weapons/torpedo.rs, src/entities/config.rs, gui/components/ph-phasers-controls.js, gui/components/ph-blasters-controls.js, gui/components/ph-torpedo-controls.js]
updated: 2026-08-28
---

# Weapons Intent

Tactical controls three independent, ship-authored weapon families through admitted `ControlSystem` commands.

- **Phasers:** sustained, target-locked beams. A ready bank must have its target inside its arc and range; damage accumulates during the beam, then that bank cools down.
- **Blasters:** straight, non-homing projectile volleys. Each bank predicts target motion at firing time, may require holding to charge, then fires its configured volley; bolts have their own speed, range-derived lifetime, collision radius, damage, and shield pierce.
- **Torpedoes:** guided, proximity-detonating projectiles. Tubes have arcs and load state, share a magazine, and snapshot damage/pierce at launch.

All three are ship-specific TOML capabilities. Tactical receives configured arcs plus live bank/tube state; the current weapons target is shared by player controls, Backfill, the Tactical blackboard, and the viewscreen projection. Current limitations and future work belong in PASM and GitHub rather than this navigation page.

## Client projection lifecycle

`weapons_update_broadcaster` computes the current `WeaponsUpdate` at 10 Hz and sends it only to the holder of the ship's authored Weapons Station (`Audience::HoldingWeapons`). `WeaponsPlugin` owns both the corresponding `LastWeaponsUpdate` delta cache and the `WeaponsUpdateFirstTick` first-tick suppression state, classifies both as cache state, and registers their per-run reset plus the reconnect projector under the stable `weapons` lifecycle key.

A mid-round reconnect receives that same live computation only when its session currently holds the Weapons Station. The targeted projection does not read or mutate either publisher cache, so it cannot force a resend or change the next delta seen by another connected player.
