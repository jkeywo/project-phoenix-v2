---
title: Game Phases
type: concept
tags: [phases, lobby, loading, in-progress, game-over, reconnect]
sources: [src/core/messages.rs, src/lobby/handler.rs, src/lobby/server.rs, src/server_app/registration.rs, src/server_app/broadcast_publish.rs]
updated: 2026-08-27
---

# Game Phases

`GamePhase` is authoritative server state with four values: `Lobby`, `Loading`,
`InProgress`, and `GameOver`. Systems and command admission use that state to
decide which messages and simulation work are valid.

## Lobby

Players identify, set their names, claim a direct station or Spectator role,
choose an authored station rating, and set readiness. When every connected
non-spectator participant is ready, the server enters `Loading` if render
assets still need preloading or begins `InProgress` immediately if the preload
gate is already complete.

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
