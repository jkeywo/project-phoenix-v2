---
title: SciencePlugin
---

# SciencePlugin

Extracted from `simulation.rs` as part of the simulation split series (issue [#258](https://github.com/jkeywo/project-phoenix-v2/issues/258)).

## Ownership

`SciencePlugin` owns the Science/Sensors console advisory hand-off logic. It handles `SetScienceTarget`, which lets the Sensors console holder suggest a target to the Weapons console holder. No resources are owned by this plugin — all state lives in the message-passing layer.

### Systems

| System | Responsibility |
|---|---|
| `handle_set_science_target` | Processes `SetScienceTarget` from the Sensors console holder; validates sender is the Sensors holder, then routes a `ScienceTargetSuggestion` to the Tactical (Weapons) console holder via `SimOutbox` |

### Resources

`SciencePlugin` introduces no new resources. It reads:

| Resource | Source |
|---|---|
| `CurrentPhase` | `LobbyPlugin` — gate: only active during `InProgress` |
| `Sessions` | `LobbyPlugin` — console-holder lookup |
| `SimOutbox` | `SimulationPlugin` — targeted message routing |

## Registration

```rust
.add_plugins(crate::science_plugin::SciencePlugin)
```

Registered as a sub-plugin of `SimulationPlugin` in `src/simulation.rs`. The module is declared in `src/lib.rs`.

## Message Flow

```
Sensors console holder
  → ClientMessage::SetScienceTarget { uuid }
    → handle_set_science_target
      → SimOutbox (Target::Token(weapons_token))
        → ServerMessage::ScienceTargetSuggestion { uuid }
          → Tactical (Weapons) console holder only
```

The Tactical player is free to act on or ignore the suggestion. No game state is mutated by the suggestion itself.

## Tests

Tests live in `src/science_plugin.rs` under `#[cfg(test)] mod tests`.

| Test | Behaviour verified |
|---|---|
| `sensors_set_science_target_broadcasts_suggestion_to_weapons` | Sensors holder sending `SetScienceTarget` produces `ScienceTargetSuggestion` routed only to Weapons token |
| `non_sensors_player_cannot_send_science_target` | Non-Sensors sender is silently ignored |
| `set_science_target_ignored_in_lobby` | Message is dropped outside `InProgress` phase |

## Sources

- `src/science_plugin.rs`
- `src/simulation.rs` (aggregator registration)
- Issue [#258](https://github.com/jkeywo/project-phoenix-v2/issues/258)
- [Console Plugin Pattern](./console-plugin-pattern.md)
- [Broadcaster Seam](./broadcaster-seam.md)
