---
title: Game Loop
type: concept
tags: [loop, ticks, simulation, rates, determinism, lockstep, fleet]
sources: [src/server_app/registration.rs, src/sim_tick.rs, src/ai/cadence.rs, src/command_admission/log.rs, src/lockstep/mod.rs, src/lockstep/session.rs, src/ship/physics.rs, src/server/bridge.rs, gui/host-actions.js, gui/server-settings.js, AGENTS.md]
updated: 2026-08-29
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
   exactly once per tick, before `SimSet::Input`. The same pass stamps the
   application tick (`src/command_admission/log.rs`): an accepted command is
   stamped for the tick it applies on (`SimTick` + `CommandDelay`) and queued
   for that tick in `PendingCommands`, ordered by `CommandOrder` — `(origin
   fleet slot, that slot's own sequence)`. When the tick comes round the queue
   drains into the routed ship and writes the run's `CommandLog` in one step,
   so the record and the apply order cannot drift. The log records what crossed
   the *network boundary* only — AI decisions emitted in-process by
   `emit_ai_command` are absent, because a replay re-derives them from the seed.
   Both halves of the seam are registered by one call
   (`register_admission_seam`), and the log is cleared at the run boundary in
   `OnEnter(GamePhase::InProgress)` so a second round starts fresh.

   `CommandDelay` is `0` for a lone host, so the queue drains inside the same
   pass that filled it and a command applies the tick it was admitted on. A host
   in a **fleet** (issue #1116) runs at the mission's authored `[global]
   command_delay_ticks` instead: a crew's command applies that many ticks later,
   on the same tick on every host, which is what gives each host time to receive
   every peer's input for a tick before it simulates it. A host that has not
   received it withholds the tick — `Time<Virtual>` paused, so the tick never
   begins — rather than speculating. See `src/lockstep/`.

   **When a ship host vanishes (issue #1119)** the fleet keeps running: its ship
   is not removed and not replaced by a simplified sim — it keeps its complete
   authoritative state, and only its control *source* flips to ordinary Backfill,
   through the same `ship::rating::apply_rating` a single-host disconnect uses.
   The *when* is a tick-stamped mesh event (`MeshFrame::HostLoss`) applied at one
   agreed tick on every survivor: `host_loss::agreed_loss_tick`, the first tick
   past the lost host's own last watermark, derived identically on every survivor
   from reliable-delivered frames rather than from who noticed the close first —
   so the flip lands on the same tick everywhere and the digest stays equal.
   `FleetRoster::depart_slot` empties the lost slot's frozen crewing so
   `resolve_human_seeking_hosts` re-seeks its Comms/Nav to AI the same tick.
   Reordered, duplicate and delayed reports converge on one transition (the
   `PendingHostLoss` max-merge plus a departed-slot guard in the barrier). See
   `src/lockstep/host_loss.rs` and `tests/lockstep_backfill.rs`.
3. **The `SimSet` chain** — Input → Physics → Damage → Modifiers → Publish →
   PublishAggregate → Broadcast, gated on `GamePhase::InProgress`.
4. **Phase transitions** — Bevy's `StateTransition` schedule is inserted into
   the `FixedMainScheduleOrder` after `FixedUpdate` (`sim_tick.rs`), so a
   `NextState<GamePhase>` written by the lobby countdown or a game-over setter
   applies on the tick that wrote it, and `OnEnter` spawns land on a tick
   boundary. Since issue #1121's fix round every *production* phase writer —
   the lobby countdown, the JS bridge's force-start, the asset preloader, and
   headless' and the native host's auto-start — writes from `FixedUpdate` this
   way. The frame-level `StateTransition` site (registered once per rendered
   frame by `StatesPlugin` itself) still runs too, but only bare-`App`
   fixtures and test drivers that write the phase from a frame schedule land
   on it now (e.g. `tests/headless_runner.rs` setting `NextState<GamePhase>`
   directly rather than through a fixed system).
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
available in the demo build even though the Debug/Cheat tab is absent. That
pause button is the only host pause driver; pause has no keyboard binding.

The same Gameplay tab exposes the viewscreen join-QR toggle. Phones carry the
matching control in their Gameplay settings. The host's `host.qr-code` semantic
action has two host-local remappable slots (KeyQ plus an empty slot by default),
and its visible button, native button keyboard activation and remapped key all
call the existing page toggle through one adapter. Its shared lifecycle
completes locally as Pressed → Pending → Applied; no tick, Rust message or
simulation state is involved. Persistent QR visibility and the host button's
`aria-pressed` still read the page's shared QR state because lobby and phone
routes can also change it.

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
