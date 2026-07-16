---
title: Weapons Intent
type: concept
tags: [weapons, tactical, phaser, blaster, torpedo]
sources: [src/console/weapons/mod.rs, src/weapons/blaster.rs, src/weapons/torpedo.rs, src/entities/config.rs, gui/components/ph-phasers-controls.js, gui/components/ph-blasters-controls.js, gui/components/ph-torpedo-controls.js]
updated: 2026-07-14
---

# Weapons Intent

Tactical controls three independent, ship-authored weapon families through admitted `ControlSystem` commands.

- **Phasers:** sustained, target-locked beams. A ready bank must have its target inside its arc and range; damage accumulates during the beam, then that bank cools down.
- **Blasters:** straight, non-homing projectile volleys. Each bank predicts target motion at firing time, may require holding to charge, then fires its configured volley; bolts have their own speed, range-derived lifetime, collision radius, damage, and shield pierce.
- **Torpedoes:** guided, proximity-detonating projectiles. Tubes have arcs and load state, share a magazine, and snapshot damage/pierce at launch.

All three are ship-specific TOML capabilities. Tactical receives configured arcs plus live bank/tube state; the current weapons target is shared by player controls and the Tactical view.

## Improvement Opportunities

- Weapon targeting is one shared lock rather than per-bank or per-weapon targets, which limits mixed engagements.
- Blaster prediction is fixed at launch with no guidance; the UI does not explain likely miss conditions.
- Phaser, blaster, and torpedo readiness are exposed differently, making cross-family tactical comparison harder.
- Arc-bearing requests help Helm turn toward phaser arcs; equivalent intent for blasters and torpedoes is unclear.
- Torpedoes are currently planar while planned ship vertical movement is 3D.

## Accepted Future Work

- A phaser attack captures its target at start and keeps it while that target remains in arc.
- Blaster and torpedo banks gain multiple barrel markers and authored timed patterns. Each pattern step is an offset plus one or more barrel indices, allowing alternating or simultaneous salvos.
- Tactical receives a uniform readiness, range, arc, and blocking-reason contract across all weapon families.
- Torpedoes become full 3D projectiles with vertical ship movement.
- Arc-bearing coordination becomes weapon-family-aware: Tactical may request Helm alignment for usable phasers, blasters, or torpedoes.
