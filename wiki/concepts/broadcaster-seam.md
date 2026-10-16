---
title: Broadcaster Seam
type: concept
tags: [broadcast, messages, networking, audience, cadence, delivery-class, snapshot, reliable]
sources: [crates/phoenix-simulation/src/ship/console_publication.rs, crates/phoenix-simulation/src/ship/coordination_systems.rs, crates/phoenix-simulation/src/core/broadcast/, crates/phoenix-simulation/src/server_app/components.rs, crates/phoenix-simulation/src/server_app/broadcast.rs, crates/phoenix-simulation/src/server_app/broadcast_publish.rs, crates/phoenix-simulation/src/console/weapons/blackboard.rs, crates/phoenix-simulation/src/console/weapons/server.rs, crates/phoenix-simulation/src/ship/power.rs, crates/phoenix-simulation/src/ship/shields.rs, crates/phoenix-simulation/src/console/repair/server.rs, crates/phoenix-simulation/src/console/repair/visibility.rs, crates/phoenix-simulation/src/lobby/server.rs, crates/phoenix-simulation/src/debug_overlay.rs]
updated: 2026-10-16
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
- `HoldingSystem(SystemId)` for the live holder of an authored System’s current Console host;
- `HoldingSystemKind(String)` for a capability whose authored instances share one current Console host;
- `HoldingWeapons` for the hull's projected Tactical/weapons audience.

The System-derived variants require `ShipConfig` and current `ConsoleHosts`. Missing or split hosting fails closed; neither a matching string nor a home Station grants authority.

`Cadence` supports periodic `Hz`/`Period`, drain-on-every-tick `OnEvent`, and one-shot `Once` producers.

Production simulation factories name a typed `SimProducer` owner through
`SimBroadcaster::for_producer`. Its explicit order is Repair, Power, Shields,
Weapons, SimState, Modifier and Outbox, preserving canonical wire delivery even
when the owning plugins register differently. The registry inserts each entry
and its cadence timer together and refuses duplicate owners. Generic
`Broadcaster::new().register(...)` callbacks keep insertion order; when combined
with typed owners they follow those owners without reordering one another.

Dispatch resolves every ready audience through borrowed ship topology before
calling any producer. The scheduled Blackboard publisher retains a world-local
query, projects each granted board once, and reuses unchanged local cache rows.
Changed packets still carry their complete sorted scope for unordered snapshot
delivery. Repair visibility is built only when a connected viewer may need a
Repair projection; the raw reserved local route is unchanged. Coordination
similarly resolves visibility lazily for actual human RepairRequest popups,
sharing that projection within one ship's delivery pass.

## Simulation publishers

| Producer | Authoritative output | Audience/cadence | Home |
|---|---|---|---|
| `sim_state_broadcaster` | `SimState` and hull snapshots | all, 10 Hz | `crates/phoenix-simulation/src/server_app/broadcast.rs` |
| `weapons_update_broadcaster` | Tactical weapons state | weapons holder, 10 Hz | `crates/phoenix-simulation/src/console/weapons/blackboard.rs` |
| `power_state_broadcaster` | reactor/power state | holder of `power-reactor`, 10 Hz | `crates/phoenix-simulation/src/ship/power.rs` |
| `repair_state_broadcaster` | repair state | holder of `repair`, 10 Hz | `crates/phoenix-simulation/src/console/repair/server.rs` |
| `shields_state_broadcaster` | shield facings and frequency | holder of the authored `shields` System kind, 10 Hz | `crates/phoenix-simulation/src/ship/shields.rs` |
| `modifier_events_broadcaster` | modifier add/remove edges | all, on event | `crates/phoenix-simulation/src/server_app/broadcast_publish.rs` |
| `sim_outbox_broadcaster` | arbitrary-target `SimOutbox` entries | target and class already carried by each entry | `crates/phoenix-simulation/src/server_app/broadcast_publish.rs` |

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

## Stable-keyed replication lifecycle

`crates/phoenix-simulation/src/core/broadcast/lifecycle.rs` lets each replication owner register a
`ReplicationLifecycleAdapter` under a stable semantic key with a reset callback,
a reconnect projection callback, or both. The registry is a `BTreeMap`, so
reset and reconnect runners invoke owners in lexical key order regardless of
plugin insertion order. Reconnect callbacks return current permitted
`ServerMessage` projections; the generic routing seam targets them only to the
reconnecting token as Snapshot traffic and knows no message variant.

The complete registry has five owners in lexical order:

- `LastConsolePublications` caches complete permitted publication scopes per recipient. Live and reconnect projections use current Console hosts and the producing Console’s Repair visibility; reconnect does not mutate another recipient’s cache.
- `LastBroadcastEntityPositions` and `LastBroadcastEntityHealth` live beside `sim_state_broadcaster` in `crates/phoenix-simulation/src/server_app/broadcast.rs` under the reset-only `entity-state` key. `Welcome` already carries current world state, so this owner has no reconnect projector. Its explicit UUID-pruning function is called only by asteroid/runtime despawn paths; unrelated caches have no synthetic pruning hook.
- `RepairPlugin` registers token-keyed `LastBroadcastHull` beside `push_hull_updates` under the stable `hull` key. Reconnect uses the same `HullVisibility` projection as live publication, including on-site detail, without writing the cache or perturbing another recipient's next delta.
- `ShipShieldsPlugin` registers a cache-free reconnect projector under `shields`. It uses the same current Console-host audience as live publication, including fail-closed split hosting.
- `WeaponsPlugin` registers `LastWeaponsUpdate`, `WeaponsUpdateFirstTick` and its audience cache under `weapons`. A host change or unhosted interval rearms publication even when machinery values are unchanged. Reconnect uses the same current-host resolver without mutating publication caches.

## Adding a producer

1. Keep authoritative state in its owning domain.
2. Build a registered producer or use `SimOutbox` for arbitrary per-message targets.
3. Select the narrowest typed `Audience`, suitable `Cadence`, and explicit delivery class at the owning seam.
4. Register reset/reconnect behavior beside any replicated-state owner through a stable lifecycle key.
5. Register the broadcaster from the owning plugin or `crates/phoenix-simulation/src/server_app/registration.rs`.
6. Add an observable routing/cadence test; do not write directly to the bridge from the domain system.

## Related

- [Message Flow](./message-flow.md)
- [Networking](./networking.md)
- [Server App Composition](./server-app.md)
