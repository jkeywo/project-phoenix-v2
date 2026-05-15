# Broadcaster Seam

A declarative registration layer that sits between simulation/lobby state and the outbound message bus.

## Purpose

Before this seam, every periodic broadcast was a hand-written Bevy system that duplicated the same boilerplate:
- gate on `GamePhase`,
- tick the shared `SimBroadcastTimer`,
- resolve the audience from `Sessions`,
- write an `OutboundMessage`.

The broadcaster seam replaces this boilerplate with a `register(audience, cadence, producer)` API. Each producer is a pure function `Fn(&World) -> Option<ServerMessage>` that knows nothing about routing or timing.

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
Cadence::OnEvent               every frame; producer decides via Option
Cadence::Once                  fires on the first tick only
```

## Migrated producers

| Producer | Audience | Cadence | Registered in |
|---|---|---|---|
| `PowerState` | `Holding(Console::Power)` | `Hz(10.0)` | `simulation::power_state_broadcaster()` |

## Registration points

- `SimulationPlugin::build()` calls `power_state_broadcaster()` and adds it as a sub-plugin.
- `LobbyBroadcaster::new()` (no producers yet) is added in `src/bridge.rs` / `src/server/bridge.rs`.

## Subsequent slices

Future issues will migrate the remaining `broadcast_*` systems in `simulation.rs` to this seam, one system per issue.
