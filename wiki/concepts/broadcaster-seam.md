# Broadcaster Seam

A declarative registration layer that sits between simulation/lobby state and the outbound message bus.

## Purpose

Before this seam, every periodic broadcast was a hand-written Bevy system that duplicated the same boilerplate:
- gate on `GamePhase`,
- tick the shared `SimBroadcastTimer`,
- resolve the audience from `Sessions`,
- write an `OutboundMessage`.

The broadcaster seam replaces this boilerplate with a `register(audience, cadence, producer)` API. Each producer is a function `Fn(&mut World) -> Vec<ServerMessage>` that knows nothing about routing or timing. Exclusive world access (`&mut World`) lets producers drain mutable resources (e.g. event queues) without a separate system.

## Plugins

| Plugin | Active phase | Location |
|---|---|---|
| `SimBroadcaster` | `InProgress` | `src/core/broadcast/sim.rs:32` (`pub type SimBroadcaster = Broadcaster<Sim>`; the `Sim` marker is at `sim.rs:12`) |
| `LobbyBroadcaster` | `Lobby` | `src/core/broadcast/lobby.rs:39` (`pub type LobbyBroadcaster = Broadcaster<Lobby>`; the `Lobby` marker is at `lobby.rs:12`) |

Both plugins expose a builder-style `register()` method and implement `bevy::Plugin`. The dispatch system is an exclusive world system (takes `&mut World`) that:

1. Gates on the correct `GamePhase`.
2. Ticks per-registration cadence timers.
3. Resolves each `Audience` against `Sessions` to a `Target` (skips if `None`).
4. Calls the producer and writes the result via `world.write_message(OutboundMessage { ... })`.

## Key types

### Audience (`src/core/broadcast/audience.rs:8`)

```rust
pub enum Audience {
    All,                              // → Target::All
    Holding(StationId),               // → Target::Token(holder) or None if vacant
    Token(String),                    // → Target::Token(specific_player)
    AllExcept(String),                // → Target::AllExcept(specific_player)
}
```

- `All` — broadcast to every connected player. Used for `SimState`, `ModifierAdded`/`Removed`.
- `Holding(StationId("power".into()))` — target the current holder of a specific station. Returns `None` (skip) when the station is vacant. Used for per-station state pushes (`PowerState`, `WeaponsUpdate`, `RepairState`). Post issue #618 the variant carries a lowercase `StationId` directly (previously `Console`); `resolve()` calls `Sessions::holder_for_station(&StationId)` and no longer needs `ShipConfig`.
- `Token(...)` — direct a message to one specific player regardless of station assignment.
- `AllExcept(...)` — broadcast to everyone except one player (e.g. for `PlayerJoined` or `PlayerLeft`).

### Cadence (`src/core/broadcast/cadence.rs:3`)

```rust
pub enum Cadence {
    Hz(f32),             // periodic at the given frequency
    Period(Duration),    // periodic at the given interval
    OnEvent,             // every frame; producer returns Vec (empty = nothing to send)
    Once,                // fires on the first tick only, then never again
}
```

Semantic notes:

- **`Hz`**: `1.0/hz` seconds per tick, repeating. If `hz <= 0.0` the timer is `None` (never fires).
- **`Period`**: explicit `Duration` between ticks, repeating. Always creates a valid timer.
- **`OnEvent`**: the producer is called *every* frame. It returns a `Vec<ServerMessage>` — when the `Vec` is empty no `OutboundMessage` is written. The producer drains a mutable resource (outbox, event queue) so each event is broadcast exactly once. **When audience resolves to `None`** the dispatch skips the producer for that tick — the onus is on the producer to not lose events. The `LobbyOutbox` / `SimOutbox` pattern (see below) avoids this by pushing `(target, msg)` pairs with pre-resolved targets, and the `audience = All` is a no-op placeholder.
- **`Once`**: fires on the very first frame the system runs (zero-duration `Timer::Once`). Used for one-shot setup broadcasts (currently `broadcast_world_setup_on_start` still exists as a hand-written system but is a candidate for migration).

## Producer-registration recipe

A producer is registered by chaining `.register(audience, cadence, producer)` on a broadcaster instance. Here is the canonical pattern — taken from the `PowerState` tracer that was the very first migration:

```rust
pub fn power_state_broadcaster() -> SimBroadcaster {
    SimBroadcaster::new().register(
        Audience::Holding(StationId("power".into())),
        Cadence::Hz(10.0),
        |world: &mut World| {
            let power = world.resource::<ShipPowerSystem>();
            vec![ServerMessage::PowerState {
                helm: power.0.helm,
                weapons: power.0.weapons,
                sensors: power.0.sensors,
                battery_charge: power.0.battery_charge,
                locked: power.0.locked,
            }]
        },
    )
}
```

Steps:
1. Create a new `SimBroadcaster::new()` (or `LobbyBroadcaster::new()`).
2. Call `.register(audience, cadence, |world: &mut World| -> Vec<ServerMessage> { ... })`.
3. Add the result as a Bevy sub-plugin: `app.add_plugins(power_state_broadcaster())`.

The broadcaster implements `bevy::Plugin` with `is_unique() -> false`, so you can register multiple instances. Duplicate registrations (same audience + cadence) are merged into the shared `SimBroadcastRegistry` / `LobbyBroadcastRegistry` resource.

## Complete catalogue of registered producers

All `ServerMessage` broadcasts are listed below. **Every** `OutboundMessage` write happens through a broadcaster-registered producer — either directly via the dispatch loop's `world.write_message(OutboundMessage { ... })` (`src/core/broadcast/sim.rs:189`, `src/core/broadcast/lobby.rs:188`) or inside a producer closure that is exclusively called by that dispatch.

### SimBroadcaster (game phase = `InProgress`)

| Producer | Message(s) | Audience | Cadence | Registered in | Delivers as |
|---|---|---|---|---|---|
| `power_state_broadcaster` | `PowerState` | `Holding(StationId("power"))` | `Hz(10.0)` | `src/ship/power.rs:216` | Snapshot (via `sim_outbox_broadcaster`) |
| `weapons_update_broadcaster` | `WeaponsUpdate` | `Holding(StationId("tactical"))` | `Hz(10.0)` | `src/console/weapons/mod.rs` | Snapshot (via `sim_outbox_broadcaster`) |
| `repair_state_broadcaster` | `RepairState` | `Holding(StationId("repair"))` | `Hz(10.0)` | `src/console/repair/server.rs:147` | Snapshot (via `sim_outbox_broadcaster`) |
| `sim_state_broadcaster` | `SimState` + `SystemHullUpdate` | `All` | `Hz(10.0)` | `src/server_app.rs:381` | Snapshot |
| `modifier_events_broadcaster` | `ModifierAdded` / `ModifierRemoved` | `All` | `OnEvent` | `src/server_app.rs:622` | Reliable |
| `sim_outbox_broadcaster` | Drain of `SimOutbox` (see below) | `All` (placeholder) | `OnEvent` | `src/server_app.rs:661` | Per-message via `delivery_class_for_msg()` |

The `SimOutbox` producer (`sim_outbox_broadcaster`, `src/server_app.rs:661`) is a special forwarding producer that drains `SimOutbox` — a `Vec<(Target, ServerMessage)>` resource written by simulation systems that need to emit arbitrary-target messages (entity spawn/despawn, torpedo events, shield status, world setup, comms state, objective summaries). It is the only producer that sets `delivery` per message via `delivery_class_for_msg()` (`src/server_app.rs:683`), which classifies high-frequency state pushes as `Snapshot` and everything else as `Reliable`. The producer writes each entry directly via `world.write_message(OutboundMessage { ... })` and returns an empty `Vec`. Messages currently routed through `SimOutbox`:

| Source system | Message(s) | Target | Location | Delivers as |
|---|---|---|---|---|
| `broadcast_shield_status` | `ShieldStatus` | `All` | `src/server_app.rs:1094` | Snapshot |
| `broadcast_world_setup_on_start` | `WorldSetup` | `All` | `src/server_app.rs:1443` | Reliable |
| `broadcast_comms_state` | `CommsState` | `Holding(StationId("comms"))` | `src/comms/server.rs:599` | Reliable |
| `broadcast_objective_summary` | `ObjectiveSummary` | `Holding(StationId("captain"))` | `src/world/server.rs:644` | Reliable |
| Torpedo systems | `TorpedoLaunched` | `All` via `SimOutbox` | `src/console/weapons/mod.rs` | Reliable |
| `asteroid_spawn` / `update_asteroid_window` | `EntitySpawned` / `EntityDespawned` | `All` via `SimOutbox` | `asteroids/lifecycle.rs` | Reliable |
| Phaser / damage systems | `PhaserFired`, `AsteroidDestroyed`, etc. | `All` via `SimOutbox` | `src/console/weapons/mod.rs` | Reliable |

### LobbyBroadcaster (game phase = `Lobby`)

| Producer | Message(s) | Audience | Cadence | Registered in |
|---|---|---|---|---|
| `lobby_outbox_broadcaster` | Drain of `LobbyOutbox` (see below) | `All` (placeholder) | `OnEvent` | `lobby/server.rs:712` |

The `LobbyOutbox` producer (`lobby_outbox_broadcaster`, `lobby/server.rs:712`) parallels the `SimOutbox` pattern. It drains `LobbyOutbox` — written by `lobby_handler::process_message()` and `handle_disconnect` systems. Messages routed through `LobbyOutbox`:

| ServerMessage | Target | Notes |
|---|---|---|
| `Welcome` | single token | Includes `GameState` + `ShipStations` + `WorldData` |
| `PlayerJoined` | `All` | New player connected |
| `PlayerLeft` | `All` | Player disconnected |
| `StationAssigned` | `All` | Station + console list changed |
| `GameStarted` | `All` | Phase transition to `InProgress` |
| `ComplexityChanged` | `All` | Complexity preset changed for a console |

## Contract: OutboundMessage is written ONLY through the broadcaster

`OutboundMessage` (`src/lobby/server.rs:96`) is the Bevy `Message` that carries a `(Target, ServerMessage, DeliveryClass)` triple to the JS bridge:

```rust
pub struct OutboundMessage {
    pub target: Target,
    pub msg: ServerMessage,
    pub delivery: DeliveryClass,   // Reliable or Snapshot
}
```

The rule is:

> **No Bevy system outside `src/core/broadcast/` may write `world.write_message(OutboundMessage { ... })` directly.**

All outbound traffic flows through one of two paths:

1. **Broadcaster dispatch loop** — `src/core/broadcast/sim.rs:189` and `src/core/broadcast/lobby.rs:188` write messages returned by registered producers. Each producer closure is called inside a pre-resolved audience context; the dispatch loop writes the `OutboundMessage` with `delivery: DeliveryClass::Reliable` (the producers themselves don't set delivery).
2. **Forwarding producer closures** — `src/server_app.rs:661` (`sim_outbox_broadcaster`) calls `delivery_class_for_msg()` (`src/server_app.rs:683`) to set per-message delivery, and `src/lobby/server.rs:696` (`drain_lobby_outbox`) hardcodes `DeliveryClass::Reliable` for all lobby messages.

This means a codebase grep for `write_message.*OutboundMessage` should only hit files under `src/core/broadcast/`, `src/server_app.rs`, and `src/lobby/server.rs`.

## Remaining non-migrated systems

The following Bevy systems still write to `SimOutbox` (not `OutboundMessage` directly — the contract holds) but use the old hand-written pattern (own gating + timer + audience resolution). They are candidates for future migration to direct `SimBroadcaster` registrations:

- `broadcast_shield_status` (`src/server_app.rs:1094`)
- `broadcast_world_setup_on_start` (`src/server_app.rs:1443`)
- `broadcast_comms_state` (`src/comms/server.rs:599`)
- `broadcast_objective_summary` (`src/world/server.rs:644`)

Each could be replaced by a `SimBroadcaster` registration that produces the same `ServerMessage` at the right cadence, eliminating the hand-written gating and timer.

## Broadcast delta-cache registry (`cache_registry.rs`, issue #613)

Six broadcast producers avoid redundant sends by diffing each tick's computed value against a "last broadcast" cache (`Resource`). Before issue #613, each cache was reset ad-hoc from two call sites, and nothing ever pruned per-UUID entries for despawned entities.

The registry at `src/core/broadcast/cache_registry.rs` is the single place that knows about all six caches and exposes three operations:

| Operation | Effect | Used by |
|---|---|---|
| `reset_all` | Zero every cache | `OnEnter(GamePhase::InProgress)` — multi-game restart safety |
| `resync_for_token` | Push full-state snapshot to one session token without touching shared caches (so no other client gets force-resent full state) | Mid-game `Welcome` (reconnect), replacing the #599 quick-fix `refresh_caches_on_midgame_reconnect` that reset *every* cache globally |
| `prune` | Remove despawned entity UUIDs from the two UUID-keyed caches (`LastBroadcastEntityPositions`, `LastBroadcastEntityHealth`) | Asteroid destruction, asteroid window-eviction, runtime-entity reconciliation |

The five cache resources are defined in `cache_registry.rs` and re-exported from `server_app.rs`:
- `LastBroadcastEntityPositions` — per-UUID `(Vec3, f32)` for NPCs/stations
- `LastBroadcastEntityHealth` — per-UUID `(hull_fraction, shield_fraction)`
- `LastBroadcastHull` — per-system `SystemHullStatus` Vec
- `LastBroadcastShields` — per-facing `ShieldFacingStatus` Vec
- `LastBroadcastBlackboards` — per-system `SystemBlackboard`

A sixth cache, `LastWeaponsUpdate` (`src/console/weapons/mod.rs`), stays defined in its natural home but is also covered by `reset_all` — so the registry's interface covers all six even though one struct lives elsewhere.

## Cross-links to relevant PRDs

- **PRD #117** — Modifier System: `ModifierAdded` / `ModifierRemoved` messages broadcast via `OnEvent` producer.
- **PRD #118** — Repair + Power Consoles: `PowerState` and `RepairState` broadcasts to per-console holders.
- **PRD #120** — Station-Based Lobby: `StationAssigned` via `LobbyOutbox`; `Welcome` includes `ShipStations`.
- **PRD #153** — Region Entities: `EntitySpawned` / `EntityDespawned` via `SimOutbox`; `SimSnapshot.entity_states` included in `SimState` broadcast.
- **PRD #154** — Console Complexity: `ComplexityChanged` via `LobbyOutbox`.
- **PRD #180** — Viewscreen Frame: `SimState` carries `red_alert` for vignette + HUD drive.
- **PRD #187** — Phone Console HUD: All periodic broadcasts (`SimState`, `WeaponsUpdate`, `RepairState`, `PowerState`) drive phone bezel chrome.

## Registration points

- `src/bridge.rs` & `src/server/bridge.rs` — `LobbyBroadcaster` / `LobbyOutboxPlugin` registered alongside `LobbyPlugin`.
- `src/server_app.rs:346`–`:351` — `add_simulation_plugins` registers `weapons_update_broadcaster`, `sim_state_broadcaster`, `modifier_events_broadcaster`, and `sim_outbox_broadcaster`; `power_state_broadcaster` and `repair_state_broadcaster` register inside their own plugins (`src/ship/power.rs:201`, `src/console/repair/server.rs:134`).
- `src/lobby/server.rs:690` — `LobbyOutboxPlugin::build` registers `drain_lobby_outbox` after `process_lobby`.
