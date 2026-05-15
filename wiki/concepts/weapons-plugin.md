---
title: WeaponsPlugin
---

# WeaponsPlugin

Extracted from `simulation.rs` as part of the simulation split series (issue [#245](https://github.com/jkeywo/project-phoenix-v2/issues/245)).

## Ownership

`WeaponsPlugin` owns all phaser, torpedo, and beam-render state and message handling for the Tactical console.

### Systems

| System | Responsibility |
|---|---|
| `handle_set_target` | Processes `SetTarget` from the Tactical holder; locks target if in radar range |
| `handle_fire_phaser` | Processes `FirePhaser`; starts a beam if target is in arc and phaser is ready |
| `handle_set_phaser_mode` | Processes `SetPhaserMode { Auto \| Manual }` from Tactical holder |
| `handle_set_phaser_frequency` | Processes `SetPhaserFrequency` from Tactical or Sensors (complexity-gated) |
| `handle_fire_torpedo` | Processes `FireTorpedo { tube, target_uuid }`; launches if tube is loaded |
| `tick_active_beam` | Advances the active phaser beam: damage accumulation, sever-on-range, natural end, cooldown start |
| `tick_torpedo_system` | Advances all in-flight torpedoes; fires `TorpedoDestroyed` for expired ones |

### Resources

| Resource | Purpose |
|---|---|
| `WeaponsTarget` | Currently locked target UUID (`None` if no lock) |
| `ActiveBeam` | Active phaser beam: target UUID, remaining seconds, damage accumulator, bank |
| `PhaserCooldown` | Post-beam cooldown (6 s lockout after every beam end) |
| `CurrentPhaserMode` | Auto or Manual phaser mode |
| `PhaserRenderConfig` | Beam colour and max range, populated from ship TOML during world setup |
| `TorpedoSystemResource` | Wraps the pure-Rust `TorpedoSystem` state machine |

### Message type

| Type | Registered by |
|---|---|
| `AsteroidDestroyedVfx` | `WeaponsPlugin` via `add_message::<AsteroidDestroyedVfx>()` |

### Public constants

| Constant | Value | Used by |
|---|---|---|
| `BEAM_DAMAGE_PER_SEC` | `5.0` | `weapons_plugin.rs` (tick), `simulation.rs` (tests) |

## Registration

```rust
.add_plugins(crate::weapons_plugin::WeaponsPlugin)
```

Registered as a sub-plugin of `SimulationPlugin` in `src/simulation.rs`. The module is declared in `src/lib.rs`.

The `weapons_update_broadcaster()` function (a `SimBroadcaster` producing `WeaponsUpdate` at 10 Hz to the Tactical holder) is also defined in `weapons_plugin.rs` and registered by `SimulationPlugin`.

## Broadcaster

`weapons_update_broadcaster()` reads:
- `ShipState` — for ship position and yaw (fire-ready arc check)
- `WorldResource` — for target entity position
- `WeaponsTarget`, `PhaserCooldown`, `ActiveBeam`
- `TorpedoSystemResource` — for per-tube reload state and magazine count
- `ShipModifiers` — for `RadarRange` multiplier (effective weapons range)

Produces `ServerMessage::WeaponsUpdate` sent to the Tactical console holder at 10 Hz.

## Tests

Tests live in `src/weapons_plugin.rs` under `#[cfg(test)] mod tests`.

| Test | Behaviour verified |
|---|---|
| `fire_phaser_on_valid_target_broadcasts_beam_started` | Beam starts when target is in range and arc |
| `fire_phaser_rejected_when_target_behind_ship` | Beam rejected if target not in forward arc |
| `fire_phaser_rejected_during_cooldown` | Beam rejected while cooldown is active |
| `weapons_update_fire_ready_true_when_target_in_range_and_arc` | `WeaponsUpdate.fire_ready` true for valid target |
| `weapons_update_fire_ready_false_when_target_out_of_phaser_range` | `WeaponsUpdate.fire_ready` false for out-of-range target |
| `phaser_damage_modifier_doubles_kill_rate` | `PhaserDamage` modifier at +1 doubles effective DPS |

Integration tests (test-app exercises `WeaponsPlugin` as a complete plugin) are in `src/simulation.rs::tests`.

## Sources

- `src/weapons_plugin.rs`
- `src/simulation.rs` (pub use re-exports and integration tests)
- Issue [#245](https://github.com/jkeywo/project-phoenix-v2/issues/245)
- [Console Plugin Pattern](./console-plugin-pattern.md)
- [Broadcaster Seam](./broadcaster-seam.md)
