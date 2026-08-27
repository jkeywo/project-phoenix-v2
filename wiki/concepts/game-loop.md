---
title: Game Loop
type: concept
tags: [loop, ticks, simulation, rates, determinism]
sources: [src/server_app/registration.rs, src/sim_tick.rs, src/ai/cadence.rs, src/command_admission/log.rs, src/ship/physics.rs, src/server/bridge.rs, AGENTS.md]
updated: 2026-08-27
---

# Game Loop

Bevy's frame loop runs at the browser's `requestAnimationFrame` rate, but the
**simulation advances on a fixed logical tick**, not on the rendered frame:
the whole `SimSet` chain is configured in
Bevy's `FixedUpdate`, stepping zero or more whole ticks per frame at the
TOML-authored `[global] sim_tick_hz` (serde default 60 Hz). `SimTick`
(`src/sim_tick.rs`) counts the steps.

## Per-frame work (frame-rate–driven)

1. **Drain inbound messages** from the JS bridge (`PreUpdate`) — Bevy buffers
   them until at least one fixed step has observed them, so a frame with zero
   steps loses nothing.
2. **Renderer / PFX / audio / HUD pushes** (`Update`/`PostUpdate`) — read the
   latest stepped sim state; the fixed loop always completes before `Update`
   in a frame, so no cross-schedule ordering edges are needed.
3. **Flush outbound** to the JS callback (`PostUpdate`).

## Per-tick work (fixed logical tick, `FixedUpdate`)

1. **Lobby handlers** (`LobbySystemSet`) consume inbound messages, mutate
   `SessionManager`, drive the countdown on tick time.
2. **Command admission** — clears and refills every ship's `AdmittedCommands`
   exactly once per tick, before `SimSet::Input`. The same pass records the
   application tick
   (`src/command_admission/log.rs`): an accepted command is stamped for the
   tick it applies on (`SimTick` + `CommandDelay`), queued for that tick in
   `PendingCommands`, and recorded in the run's `CommandLog` in one step, so
   the record and the apply order cannot drift. `CommandDelay` is `0` on a
   local host, so the queue drains inside the same pass that filled it. The log
   records what crossed the *network boundary*
   only — AI decisions emitted in-process by `emit_ai_command` are absent,
   because a replay re-derives them from the seed. Both halves of the seam
   are registered by one call (`register_admission_seam`), and the log is
   cleared at the run boundary in `OnEnter(GamePhase::InProgress)` so a
   second round starts fresh.
3. **The `SimSet` chain** — Input → Physics → Damage → Modifiers → Publish →
   PublishAggregate → Broadcast, gated on `GamePhase::InProgress`.
4. **Phase transitions** — Bevy's `StateTransition` schedule is inserted into
   the `FixedMainScheduleOrder` after `FixedUpdate` (`sim_tick.rs`), so a
   `NextState<GamePhase>` written by the lobby countdown or a game-over setter
   applies on the tick that wrote it, and `OnEnter` spawns land on a tick
   boundary. It still runs once per frame as well, for the frame-driven writers
   (JS bridge force-start, asset preloader, headless auto-start).
5. **AI cadence derivation** (`FixedLast`, `src/ai/cadence.rs`) — the AI
   decision tick is every `sim_tick_hz / ai_tick_hz`-th logical tick, and the
   snapshot tick every `ai_tick_hz / ai_snapshot_hz`-th of those; both ratios
   are validated as integers at world load. No wall clock anywhere.

Rapier steps on the logical tick too. Its `PhysicsSet` chain is
registered in `FixedUpdate` with `TimestepMode::Fixed` at the authored
`sim_tick_hz`, and is ordered explicitly against the chain above:
`PhysicsSet::SyncBackend` after `SimSet::Physics` (so it reads the transforms
`sync_ship_position` just wrote) and `PhysicsSet::Writeback` before
`SimSet::Damage` (so `handle_collisions` reads this tick's contacts). See
`register_physics` in `src/server_app/registration.rs`.

## 10 Hz channels

| Channel | Direction | Trigger |
|---|---|---|
| Helm joystick (`HelmInput` UI action → two `ControlSystem` messages: `SetThrust` → `helm-thrust`, `SetSteering` → `helm-steering`) | client → server | Joystick active on the helm console |
| `SimState { snapshot }` | server → all clients | Bevy timer system, every 100 ms of sim time |

`SimState` carries the shared authoritative snapshot. System-specific
blackboards and state messages use the same snapshot cadence and audience
projection. Clients render from those publications; there is no client-side
prediction.

## Why 10 Hz specifically

- Phone-to-host bandwidth is fine at 10 Hz of small JSON; cheap on battery.
- WebRTC RTT in a room is low; 100 ms staleness is barely perceptible for a relaxing tabletop sim.
- If a client misses one tick, the next one is the full ground truth — no diffing complexity.

## Headless

`phoenix-headless` drives frames with `TimeUpdateStrategy::ManualDuration`
(`--hz` is the FRAME rate); the sim still steps at the world's `sim_tick_hz`
inside the fixed loop, so any frame rate covers the same logical ticks per
sim-second. The browser exposes the counter as `wasm_sim_tick()` for the
smoke tests (`tests/smoke/sim-tick.spec.js`).

## Simulation pause (settings cog)

`wasm_toggle_pause()` (`drain_host_controls` in `src/server/bridge.rs`)
pauses `Time<Virtual>`, which starves the fixed accumulator — `FixedUpdate`
stops running altogether while paused, not just the `SimSet` chain inside it.

The host settings cog exposes pause on its **Gameplay** tab. It remains
available in the demo build even though the Debug/Cheat tab is absent. The cog
is the only host driver; there is no keyboard binding.

The same Gameplay tab exposes the viewscreen join-QR toggle. Phones carry the
matching control in their Gameplay settings. Both routes change the host page's
shared QR overlay state.

Lobby countdown/readiness, command admission, and the `SimSet` chain all run in
`FixedUpdate`. Pausing therefore freezes the lobby and stops admitting commands
as well as stopping simulation work: everything keyed to the one virtual clock
shares its pause state.

One consequence worth knowing: `ReturnToLobby` is lobby handling, so it is not
read while paused. `hostReturnToLobby()` in `server.html` — the single funnel
for both the Game Over button and the cog's **Exit to Lobby** — therefore
unpauses before it sends.

`tests/smoke/sim-tick.spec.js` asserts this total-pause contract by driving
rendered frames and checking that `wasm_sim_tick()` does not advance.

## Bevy frame caveat on WASM

`App::run()` returns immediately on the WASM target — Bevy installs itself onto `requestAnimationFrame` rather than blocking. Code after `wasm_init()`'s `app.run()` call will not execute on WASM. See `src/server/bridge.rs` and `AGENTS.md`.

## Related

- [Ship Physics](./ship-physics.md) — what runs each helm tick
- [Message Flow](./message-flow.md)
