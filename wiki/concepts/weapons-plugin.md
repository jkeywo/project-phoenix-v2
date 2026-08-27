---
title: WeaponsPlugin
type: concept
tags: [weapons, tactical, phaser, torpedo, blaster, targeting, ai]
sources: [src/console/weapons/mod.rs, src/console/weapons/server.rs, src/console/weapons/beam.rs, src/console/weapons/torpedo.rs, src/console/weapons/blaster.rs, src/console/weapons/blackboard.rs, src/console/weapons/shared.rs, src/weapons/, src/server_app/registration.rs, src/server_app/world_setup.rs]
updated: 2026-08-27
---

# WeaponsPlugin

The Tactical/weapons module follows the pure-root plus server-adapter convention. `src/console/weapons/mod.rs` declares and re-exports the module tree; `src/console/weapons/server.rs` owns `WeaponsPlugin`, shared resources, registration, and Tactical target selection.

## Module layout

| File | Responsibility |
|---|---|
| `src/console/weapons/server.rs` | Plugin assembly, shared state, admitted-consumer registration, Tactical AI target selection, and the Tactical-owned receiver for delivered frequency hints. |
| `src/console/weapons/beam.rs` | Phaser banks, target-lock application, auto-fire, beam lifecycle, and damage. |
| `src/console/weapons/torpedo.rs` | Tube load/fire commands, target snapshots, torpedo lifecycle, and detonation. |
| `src/console/weapons/blaster.rs` | NPC blaster charge/fire and hit application. |
| `src/console/weapons/blackboard.rs` | Weapons/Tactical radar blackboards, `WeaponsUpdate`, and its broadcaster. |
| `src/console/weapons/shared.rs` | Cross-family helpers and one-tick handoff resources. |
| `src/console/weapons/server_tests.rs` | The large plugin integration suite included from `src/console/weapons/server.rs`. |

Pure weapon state machines and geometry used by these adapters live under `src/weapons/`.

## Command symmetry

Each authored bank, tube, and Tactical control is a fine `SystemId`. Human controls and AI hosts emit identical admitted payloads. In particular, `ai_target_selection` emits `SetTarget`; `handle_set_target` is the sole writer of the authoritative Tactical selection. Auto-fire reads that selection but does not bypass target admission.

The Tactical selector ranks the ship's visible/scored objective surface deterministically. A named positive operate/destroy directive can nominate its exact live target; ordinary acquisition remains bounded by the authored radar reach and hostility rules.

## Coordination

Weapons resolves its fine Tactical/Helm Systems to explicit owning-Station
addresses before enqueue. An out-of-arc target produces an
`ArcBearingRequest` addressed to Helm. A delayed `FrequencyHint` addressed to
Tactical reaches `receive_tactical_coordination`; that Tactical-owned receiver
rechecks the live control policy before latching the unchanged frequency for
the existing next-tick applier. The payload never selects or widens its own
recipient.

## Damage pipelines

Beam processing is phased so read-only shooter/LOS preparation completes before damage and lifetime updates. Torpedo processing similarly builds one target snapshot before advancing every ship's torpedoes. Both paths route shields and hull damage through the shared damage model, publish lifecycle events through `SimOutbox`, and leave the LocalShip present when defeat is latched.

Weapon reach, arcs, cooldowns, load times, payloads, colours, and AI policy parameters come from entity TOML. `src/server_app/world_setup.rs` projects the selected hull's authored configuration into runtime components/resources; renderer-only beam appearance stays separate from hit authority.

## Publication

Publish systems build per-system blackboards plus the Tactical radar aggregate. `weapons_update_broadcaster` sends the LocalShip summary at 10 Hz to the projected weapons holder. Client panels render the authored bank/tube ids and arcs from `ShipClientConfig`; they do not own firing state.

## Related

- [Weapons Runtime](./weapons-intent.md)
- [Radar Projection](./radar-projection.md)
- [Broadcaster Seam](./broadcaster-seam.md)
- [Information-Parity Audit](./information-parity-audit.md)
