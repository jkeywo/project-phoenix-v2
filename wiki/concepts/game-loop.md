---
title: Game Loop
type: concept
tags: [loop, ticks, simulation, rates]
sources: [src/server/simulation.rs, src/server/ship_physics.rs, AGENTS.md]
updated: 2026-05-08
---

# Game Loop

Bevy runs at the browser's `requestAnimationFrame` rate (typically 60 Hz). On top of that, the project layers two explicit 10 Hz channels.

## Per-frame work (every tick)

1. **Drain inbound messages** from JS bridge into Bevy `MessageReader<InboundMessage>`.
2. **Lobby/sim handlers** consume them, mutate `SessionManager` and `ShipState`, write `MessageWriter<OutboundMessage>`.
3. **Rapier physics step** — apply current ship velocity, integrate positions, detect collisions.
4. **Collision handler** — on ship/asteroid contact, zero ship velocity.
5. **Renderer** — update camera transform from `ShipState.view_mode`, update Red Alert overlay visibility.
6. **Flush outbound** to JS callback for routing.

## 10 Hz channels

| Channel | Direction | Trigger |
|---|---|---|
| `HelmInput` | client → server | Joystick active on the helm console |
| `SimState { snapshot }` | server → all clients | Bevy timer system, every 100 ms |

`SimState` carries `red_alert`, `view_mode`, `ship_x`, `ship_z`, `ship_yaw`. Clients render their UI from this. There's no client-side prediction — the server is fully authoritative.

## Why 10 Hz specifically

- Phone-to-host bandwidth is fine at 10 Hz of small JSON; cheap on battery.
- WebRTC RTT in a room is low; 100 ms staleness is barely perceptible for a relaxing tabletop sim.
- If a client misses one tick, the next one is the full ground truth — no diffing complexity.

PRD #66 keeps the 10 Hz rate but adds **per-console payloads** routed `Target::One(token)` so Weapons/Engineering only see what they need.

## Bevy frame caveat on WASM

`App::run()` returns immediately on the WASM target — Bevy installs itself onto `requestAnimationFrame` rather than blocking. Code after `wasm_init()`'s `app.run()` call won't execute on WASM. See `bridge.rs` and `AGENTS.md`'s "WASM ≠ Native" note.

## Related

- [Ship Physics](./ship-physics.md) — what runs each helm tick
- [Message Flow](./message-flow.md)
