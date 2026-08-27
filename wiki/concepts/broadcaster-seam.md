---
title: Broadcaster Seam
type: concept
tags: [broadcast, messages, audience, cadence, delivery]
sources: [src/core/broadcast/, src/server_app/broadcast.rs, src/server_app/broadcast_publish.rs, src/console/weapons/blackboard.rs, src/ship/power.rs, src/console/repair/server.rs, src/lobby/server.rs]
updated: 2026-08-27
---

# Broadcaster Seam

The broadcaster seam owns periodic and event-driven publication from authoritative simulation/lobby state to `OutboundMessage`. Producers return messages; the shared dispatcher owns cadence, audience resolution, and delivery to the bridge.

## Types

`SimBroadcaster` is the production generic broadcaster and runs while the game
is in progress. `LobbyBroadcaster` is the lobby-phase specialisation of the
same generic type, retained as a tested extension seam; production lobby
messages take the direct `LobbyOutboxPlugin` path described below.

`Audience` supports:

- `All`, `Token`, and `AllExcept` for direct routing;
- `Holding(StationId)` for an explicitly addressed station;
- `HoldingSystem(SystemId)` for the live holder of the station that owns an authored system;
- `HoldingWeapons` for the hull's projected Tactical/weapons audience.

The system-derived variants require the current `ShipConfig`; they do not infer station identity from a matching string.

`Cadence` supports periodic `Hz`/`Period`, drain-on-every-tick `OnEvent`, and one-shot `Once` producers.

## Simulation publishers

| Producer | Authoritative output | Audience/cadence | Home |
|---|---|---|---|
| `sim_state_broadcaster` | `SimState` and hull snapshots | all, 10 Hz | `src/server_app/broadcast.rs` |
| `weapons_update_broadcaster` | Tactical weapons state | weapons holder, 10 Hz | `src/console/weapons/blackboard.rs` |
| `power_state_broadcaster` | reactor/power state | holder of `power-reactor`, 10 Hz | `src/ship/power.rs` |
| `repair_state_broadcaster` | repair state | holder of `repair`, 10 Hz | `src/console/repair/server.rs` |
| `modifier_events_broadcaster` | modifier add/remove edges | all, on event | `src/server_app/broadcast_publish.rs` |
| `sim_outbox_broadcaster` | arbitrary-target `SimOutbox` entries | target already carried by each entry | `src/server_app/broadcast_publish.rs` |

The lobby deliberately does **not** route its outbox through the phase-gated
generic broadcaster. `LobbyOutboxPlugin` drains `LobbyOutbox` directly in
`FixedUpdate`, after `tick_countdown`, and marks every entry `Reliable`.
That phase-agnostic drain is what lets `GameStarted` escape on the same tick
that the countdown transitions from `Lobby` to `InProgress`.

## Delivery classes

`sim_outbox_broadcaster` classifies each `ServerMessage` as `Reliable` or `Snapshot`. High-frequency replaceable state uses the snapshot channel; lifecycle, commands, setup, and one-shot notifications remain reliable. `flush_outbound` in `src/server/bridge.rs` is the Rust-to-JavaScript boundary that preserves the class.

## Cache resets

Snapshot delta caches are registered in `src/core/broadcast/cache_registry.rs`. `reset_all` clears them on entry to `InProgress`, forcing a complete first publication for a new run. Domain caches such as `LastWeaponsUpdate` remain defined beside their producers but participate in the same reset contract.

## Adding a producer

1. Keep authoritative state in its owning domain.
2. Build a producer that reads that state and returns `ServerMessage` values.
3. Select the narrowest typed `Audience` and a suitable `Cadence`.
4. Register the broadcaster from the owning plugin or `src/server_app/registration.rs`.
5. Add an observable routing/cadence test; do not write directly to the bridge from the domain system.

## Related

- [Message Flow](./message-flow.md)
- [Networking](./networking.md)
- [Server App Composition](./server-app.md)
