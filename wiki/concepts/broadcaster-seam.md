---
title: Broadcaster Seam
type: concept
tags: [broadcast, messages, audience, cadence, delivery-class, snapshot, reliable]
sources: [src/core/broadcast/, src/server_app/components.rs, src/server_app/broadcast.rs, src/server_app/broadcast_publish.rs, src/console/weapons/blackboard.rs, src/ship/power.rs, src/ship/shields.rs, src/console/repair/server.rs, src/lobby/server.rs, src/debug_overlay.rs]
updated: 2026-08-27
---

# Broadcaster Seam

The broadcaster seam owns periodic and event-driven publication from authoritative simulation/lobby state to `OutboundMessage`. Producers return messages; the shared dispatcher owns cadence, audience resolution, and delivery to the bridge. A registered phase supplies its delivery class, while arbitrary-target simulation producers record their choice in `SimOutbox` at insertion time.

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
| `shields_state_broadcaster` | shield facings and frequency | holder of `shields-system`, 10 Hz | `src/ship/shields.rs` |
| `modifier_events_broadcaster` | modifier add/remove edges | all, on event | `src/server_app/broadcast_publish.rs` |
| `sim_outbox_broadcaster` | arbitrary-target `SimOutbox` entries | target and class already carried by each entry | `src/server_app/broadcast_publish.rs` |

The lobby deliberately does **not** route its outbox through the phase-gated
generic broadcaster. `LobbyOutboxPlugin` drains `LobbyOutbox` directly in
`FixedUpdate`, after `tick_countdown`, and marks every entry `Reliable`.
That phase-agnostic drain is what lets `GameStarted` escape on the same tick
that the countdown transitions from `Lobby` to `InProgress`.

## Delivery classes

Registered `SimBroadcaster` producers use `Snapshot`. `SimOutbox` has no public
raw insertion path: arbitrary-target producers must call `push_snapshot`,
`push_reliable`, `extend_snapshot`, or `extend_reliable`, storing the selected
class beside the target and payload. `sim_outbox_broadcaster` drains those
entries without reclassifying their `ServerMessage` variant. High-frequency
replaceable state uses the snapshot channel; lifecycle, setup, and one-shot
notifications remain reliable. The debug-state publisher is an intentional
direct Reliable exception because it must still publish while the fixed
simulation loop is paused. `flush_outbound` in `src/server/bridge.rs` preserves
the class at the Rust-to-JavaScript boundary.

## Cache resets

Snapshot delta caches are registered in `src/core/broadcast/cache_registry.rs`. `reset_all` clears them on entry to `InProgress`, forcing a complete first publication for a new run. Domain caches such as `LastWeaponsUpdate` remain defined beside their producers but participate in the same reset contract. Shields has no delta cache: its sole periodic publisher reads the current authoritative component and targets only the live holder of the authored Shields System.

## Adding a producer

1. Keep authoritative state in its owning domain.
2. Build a registered producer or use `SimOutbox` for arbitrary per-message targets.
3. Select the narrowest typed `Audience`, suitable `Cadence`, and explicit delivery class at the owning seam.
4. Register the broadcaster from the owning plugin or `src/server_app/registration.rs`.
5. Add an observable routing/cadence test; do not write directly to the bridge from the domain system.

## Related

- [Message Flow](./message-flow.md)
- [Networking](./networking.md)
- [Server App Composition](./server-app.md)
