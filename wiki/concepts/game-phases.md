---
title: Game Phases
type: concept
tags: [phases, lobby, loading, in-progress, game-over, reconnect]
sources: [src/delivery/payload.rs, tests/native_host_catalogue.rs, tests/client/scenario-catalogue-wire.test.js, src/core/messages.rs, src/lobby/start_policy.rs, src/lobby/handler.rs, src/lobby/server.rs, src/lockstep/mod.rs, src/server/bridge.rs, src/server_app/registration.rs, src/server_app/broadcast_publish.rs]
updated: 2026-09-07
---

# Game Phases

`GamePhase` is authoritative server state with four values: `Lobby`, `Loading`,
`InProgress`, and `GameOver`. Systems and command admission use that state to
decide which messages and simulation work are valid.

## Lobby

Before world load, both hosts publish the same typed catalogue snapshot:
scenario provenance, curated hulls, active-pack metadata and first-valid-wins
locks. The native surface uses that snapshot too. After both locks are set,
the phone closes its picker while keeping the pack roster; a later Identify
receives the locked snapshot on either host that offered a catalogue. The
shared wire fixture is tested through the actual phone reducer and native
pack-install path.

Players identify, set their names, claim a direct station or Spectator role,
choose an authored station rating, and set readiness. When every connected
non-spectator participant is ready, the server enters `Loading` if render
assets still need preloading or begins `InProgress` immediately if the preload
gate is already complete.

In a managed fleet, the browser host mesh suppresses each ship's independent
countdown and computes one collective policy from every ship's connected/ready
crew tally plus every connected GM's ready flag. Spectators and disconnected
rows are excluded; a GM-only roster follows the same non-empty all-ready rule.
An authenticated GM force-start may skip readiness only. Successful content,
compatibility, peer, and terminal presentation-preload validation remain
mandatory. The browser proposes `start-N` with `apply_tick = 0`; owner-side Rust
assigns a safe exact tick and seals the grant into its authenticated `TickFrame`.
Members accept only that canonical value and every simulation peer enters
`InProgress` directly at its tick. Validation is frozen when the owner seals the
grant, so a late withdrawal, GM disconnect, or frame-paced local preload
transition cannot split the fleet at application.

The multi-participant fleet itself is accepted only from a fresh `Lobby`.
Adoption rebases otherwise unequal browser boot state to activation tick 1: the
fixed clock has one timestep elapsed and zero overstep, the simulation RNG is
reseeded from the authored world seed, and the world-id mint begins tick 1.
Each render frame may then commit at most one complete fixed tick before the
transport flushes; only whole catch-up debt is discarded, while fractional
interpolation time remains. Returned-to-Lobby, already-starting, and
resume-staged worlds are refused rather than partially rebased.

A disconnect clears readiness and triggers the same all-ready re-evaluation for
the remaining crew. Claimable seats come from the selected hull's non-auxiliary
station roster; there is no queue or fixed console list.

## Loading

The server publishes `LoadingProgress` while required render assets reach a
terminal load state. Reconnect and station restoration remain available. Once
the preload completes, `GameStarted` is broadcast and the fixed simulation
enters `InProgress`.

## InProgress

Admitted console and AI commands drive the fixed-tick simulation. The ordered
sets are `Input -> Physics -> Damage -> Modifiers -> Publish ->
PublishAggregate -> Broadcast`. World scripts, objectives, AI, regions,
weapons, engineering, and other gameplay plugins run under their registered
phase and cadence conditions.

Late joiners and reconnecting players receive `Welcome` plus current world and
station state. A disconnect preserves the remembered station, stores its
rating, and applies Backfill AI; a successful reconnect restores the claim and
rating if no connected player took the seat.

## GameOver

Player-ship destruction or a scripted `game_over` action records an optional
reason and outcome, enters `GameOver`, and broadcasts the terminal message.
The game-over UI can issue `ReturnToLobby`; the server resets round readiness
and returns to the lobby through the authoritative lobby handler.

## Related

- [Game Loop](./game-loop.md)
- [Session](../entities/session.md)
- [Asset Preload](./asset-preload.md)
