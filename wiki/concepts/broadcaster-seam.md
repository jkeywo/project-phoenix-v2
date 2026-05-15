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
| `SimBroadcaster` | `InProgress` | `src/core/broadcast/sim.rs` |
| `LobbyBroadcaster` | `Lobby` | `src/core/broadcast/lobby.rs` |

Both plugins expose a builder-style `register()` method and implement `bevy::Plugin`. The dispatch system is an exclusive world system (takes `&mut World`) that:

1. Gates on the correct `GamePhase`.
2. Ticks per-registration cadence timers.
3. Resolves each `Audience` against `Sessions` to a `Target` (skips if `None`).
4. Calls the producer and writes the result via `world.write_message`.

## Key types

```
Audience::All                  → Target::All
Audience::Holding(Console::X)  → Target::Token(holder_token)  (or None)
Audience::Token(t)             → Target::Token(t)
Audience::AllExcept(t)         → Target::AllExcept(t)

Cadence::Hz(f32)               periodic at the given frequency
Cadence::Period(Duration)      periodic at the given interval
Cadence::OnEvent               every frame; producer returns Vec (empty = nothing to send)
Cadence::Once                  fires on the first tick only
```

## Migrated producers

| Producer | Audience | Cadence | Registered in |
|---|---|---|---|
| `PowerState` | `Holding(Console::Power)` | `Hz(10.0)` | `simulation::power_state_broadcaster()` |
| `WeaponsUpdate` | `Holding(Console::Tactical)` | `Hz(10.0)` | `simulation::weapons_update_broadcaster()` |
| `RepairState` | `Holding(Console::Repair)` | `Hz(10.0)` | `simulation::repair_state_broadcaster()` |
| `SimState` | `All` | `Hz(10.0)` | `simulation::sim_state_broadcaster()` |
| `ModifierAdded` / `ModifierRemoved` | `All` | `OnEvent` | `simulation::modifier_events_broadcaster()` |
| `LobbyOutbox` (drains `Welcome`, `PlayerJoined`, `PlayerLeft`, `StationAssigned`, `GameStarted`, `ComplexityChanged`, etc.) | `All` | `OnEvent` | `lobby::lobby_outbox_broadcaster()` |

## Registration points

- `SimulationPlugin::build()` calls all five broadcaster functions above and adds each as a sub-plugin.
- `lobby_outbox_broadcaster()` is called in `src/bridge.rs` / `src/server/bridge.rs` alongside `LobbyPlugin`.

## Subsequent slices

Future issues will migrate the remaining `broadcast_*` systems in `simulation.rs` to this seam, one system per issue.
