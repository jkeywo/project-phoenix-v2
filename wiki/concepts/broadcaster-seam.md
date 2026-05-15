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
| `SimBroadcaster` | `InProgress` | `src/core/broadcast/sim.rs` (struct at line 75) |
| `LobbyBroadcaster` | `Lobby` | `src/core/broadcast/lobby.rs` (struct at line 71) |

Both plugins expose a builder-style `register()` method and implement `bevy::Plugin`. The dispatch system is an exclusive world system (takes `&mut World`) that:

1. Gates on the correct `GamePhase`.
2. Ticks per-registration cadence timers.
3. Resolves each `Audience` against `Sessions` to a `Target` (skips if `None`).
4. Calls the producer and writes the result via `world.write_message(OutboundMessage { ... })`.

## Key types

### Audience (`src/core/broadcast/audience.rs:7`)

```rust
pub enum Audience {
    All,                              // → Target::All
    Holding(Console),                 // → Target::Token(holder) or None if vacant
    Token(String),                    // → Target::Token(specific_player)
    AllExcept(String),                // → Target::AllExcept(specific_player)
}
```

- `All` — broadcast to every connected player. Used for `SimState`, `ModifierAdded`/`Removed`.
- `Holding(Console::Power)` — target the current holder of a specific console. Returns `None` (skip) when the console is vacant. Used for per-console state pushes (`PowerState`, `WeaponsUpdate`, `RepairState`).
- `Token(...)` — direct a message to one specific player regardless of console assignment.
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
        Audience::Holding(Console::Power),
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

All `ServerMessage` broadcasts are listed below. **Every** `OutboundMessage` write happens through a broadcaster-registered producer — either directly via the dispatch loop's `world.write_message(OutboundMessage { ... })` (`src/core/broadcast/sim.rs:183`, `src/core/broadcast/lobby.rs:174`) or inside a producer closure that is exclusively called by that dispatch.

### SimBroadcaster (game phase = `InProgress`)

| Producer | Message(s) | Audience | Cadence | Registered in |
|---|---|---|---|---|
| `power_state_broadcaster` | `PowerState` | `Holding(Console::Power)` | `Hz(10.0)` | `src/simulation.rs:384` |
| `weapons_update_broadcaster` | `WeaponsUpdate` | `Holding(Console::Tactical)` | `Hz(10.0)` | `src/simulation.rs:405` |
| `repair_state_broadcaster` | `RepairState` | `Holding(Console::Repair)` | `Hz(10.0)` | `src/simulation.rs:450` |
| `sim_state_broadcaster` | `SimState` | `All` | `Hz(10.0)` | `src/simulation.rs:485` |
| `modifier_events_broadcaster` | `ModifierAdded` / `ModifierRemoved` | `All` | `OnEvent` | `src/simulation.rs:564` |
| `sim_outbox_broadcaster` | Drain of `SimOutbox` (see below) | `All` (placeholder) | `OnEvent` | `src/simulation.rs:596` |

The `SimOutbox` producer (`sim_outbox_broadcaster`, `src/simulation.rs:596`) is a special forwarding producer that drains `SimOutbox` — a `Vec<(Target, ServerMessage)>` resource written by simulation systems that need to emit arbitrary-target messages (entity spawn/despawn, torpedo events, shield status, world setup, repair icons, comms state, objective summaries). The producer writes each entry directly via `world.write_message(OutboundMessage { ... })` and returns an empty `Vec`. Messages currently routed through `SimOutbox`:

| Source system | Message(s) | Target | Location |
|---|---|---|---|
| `broadcast_shield_status` | `ShieldStatus` | `All` | `src/simulation.rs:1427` |
| `broadcast_world_setup_on_start` | `WorldSetup` | `All` | `src/simulation.rs:1454` |
| `broadcast_repair_icons` | `ShowRepairIcon` / `ClearRepairIcon` | holder of each damaged console | `src/simulation.rs:1469` |
| `broadcast_comms_state` | `CommsState` | `Holding(Console::Comms)` | `src/world/server.rs:506` |
| `broadcast_objective_summary` | `ObjectiveSummary` | `Holding(Console::CaptainChair)` | `src/world/server.rs:544` |
| `handle_torpedo_launch` | `TorpedoLaunched` | `All` via `SimOutbox` | `simulation.rs` |
| `asteroid_spawn` / `update_asteroid_window` | `EntitySpawned` / `EntityDespawned` | `All` via `SimOutbox` | `asteroids/lifecycle.rs` |
| Phaser / damage systems | `PhaserFired`, `AsteroidDestroyed`, etc. | `All` via `SimOutbox` | `simulation.rs`, `weapons/phaser.rs` |

### LobbyBroadcaster (game phase = `Lobby`)

| Producer | Message(s) | Audience | Cadence | Registered in |
|---|---|---|---|---|
| `lobby_outbox_broadcaster` | Drain of `LobbyOutbox` (see below) | `All` (placeholder) | `OnEvent` | `lobby/server.rs:179` |

The `LobbyOutbox` producer (`lobby_outbox_broadcaster`, `lobby/server.rs:179`) parallels the `SimOutbox` pattern. It drains `LobbyOutbox` — written by `lobby_handler::process_message()` and `handle_disconnect` systems. Messages routed through `LobbyOutbox`:

| ServerMessage | Target | Notes |
|---|---|---|
| `Welcome` | single token | Includes `GameState` + `ShipStations` + `WorldData` |
| `PlayerJoined` | `All` | New player connected |
| `PlayerLeft` | `All` | Player disconnected |
| `StationAssigned` | `All` | Station + console list changed |
| `GameStarted` | `All` | Phase transition to `InProgress` |
| `ComplexityChanged` | `All` | Complexity preset changed for a console |

## Contract: OutboundMessage is written ONLY through the broadcaster

`OutboundMessage` (`lobby/server.rs:51`) is the Bevy `Message` that carries a `(Target, ServerMessage)` pair to the JS bridge. The rule is:

> **No Bevy system outside `src/core/broadcast/` may write `world.write_message(OutboundMessage { ... })` directly.**

All outbound traffic flows through one of two paths:

1. **Broadcaster dispatch loop** — `src/core/broadcast/sim.rs:183` and `src/core/broadcast/lobby.rs:174` write messages returned by registered producers.
2. **Forwarding producer closures** — `simulation.rs:604` (`sim_outbox_broadcaster`) and `lobby/server.rs:188` (`lobby_outbox_broadcaster`) write messages directly inside their producer closure, which is exclusively called from path 1.

This means a codebase grep for `write_message.*OutboundMessage` should only hit files under `src/core/broadcast/` and the two outbox-drain producer closures.

## Remaining non-migrated systems

The following Bevy systems still write to `SimOutbox` (not `OutboundMessage` directly — the contract holds) but use the old hand-written pattern (own gating + timer + audience resolution). They are candidates for future migration to direct `SimBroadcaster` registrations:

- `broadcast_shield_status` (`src/simulation.rs:1427`)
- `broadcast_world_setup_on_start` (`src/simulation.rs:1454`)
- `broadcast_repair_icons` (`src/simulation.rs:1469`)
- `broadcast_comms_state` (`src/world/server.rs:506`)
- `broadcast_objective_summary` (`src/world/server.rs:544`)

Each could be replaced by a `SimBroadcaster` registration that produces the same `ServerMessage` at the right cadence, eliminating the hand-written gating and timer.

## Cross-links to relevant PRDs

- **PRD #117** — Modifier System: `ModifierAdded` / `ModifierRemoved` messages broadcast via `OnEvent` producer.
- **PRD #118** — Repair + Power Consoles: `PowerState` and `RepairState` broadcasts to per-console holders.
- **PRD #120** — Station-Based Lobby: `StationAssigned` via `LobbyOutbox`; `Welcome` includes `ShipStations`.
- **PRD #153** — Region Entities: `EntitySpawned` / `EntityDespawned` via `SimOutbox`; `SimSnapshot.entity_states` included in `SimState` broadcast.
- **PRD #154** — Console Complexity: `ComplexityChanged` via `LobbyOutbox`.
- **PRD #180** — Viewscreen Frame: `SimState` carries `red_alert` for vignette + HUD drive.
- **PRD #187** — Phone Console HUD: All periodic broadcasts (`SimState`, `WeaponsUpdate`, `RepairState`, `PowerState`) drive phone bezel chrome.

## Registration points

- `src/bridge.rs:96` — `LobbyBroadcaster` (the `lobby_outbox_broadcaster`) registered alongside `LobbyPlugin`.
- `src/server/bridge.rs:96` — same for native server build.
- `SimulationPlugin::build()` — registers all six `SimBroadcaster` producers (`src/simulation.rs:370-375`).
- `src/lobby/server.rs:214` — test harness registers `lobby_outbox_broadcaster`.
