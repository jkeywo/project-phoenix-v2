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
| `PhaserCooldown` | Post-beam cooldown (duration sourced from `PhaserCombatConfigResource`) |
| `CurrentPhaserMode` | Auto or Manual phaser mode |
| `PhaserRenderConfig` | Beam colour and max render range, populated from ship TOML during world setup |
| `PhaserCombatConfigResource` | Player phaser tuning (beam duration, cooldown, damage/sec, range); sourced from `[weapons_console]` |
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

## Torpedo configuration (TOML-driven)

The `TorpedoSystemResource` is initialised by `WeaponsPlugin::build` with
`TorpedoConfig::default()` so test apps that never load a ship TOML still
get a working torpedo system. The live game path overrides this resource
during `spawn_game_start_entities` (`src/server_app.rs`) when the spawned
ship's `EntityConfig` carries a `[torpedoes]` block:

```toml
[torpedoes]
count = 10
damage_hull = 50
damage_shields = 5
speed = 30.0
turn_rate_deg_per_sec = 45.0   # converted to radians at the boundary
lifespan = 20.0
load_time = 10.0
```

All fields use `serde(default)` and fall back to the same values as
`TorpedoConfig::default()` (`src/weapons/torpedo.rs:49`). Designer-facing
`turn_rate_deg_per_sec` is converted to radians in
`TorpedoesConfig::to_runtime()` (`src/entities/config.rs`).

NPC ships that want a different loadout simply add their own `[torpedoes]`
block to their entity TOML; NPC ships that omit the block keep the defaults.

A drift-guard test
(`player_ship_toml_torpedoes_block_matches_runtime_default_values` in
`src/entities/config.rs`) fails if the player ship's `[torpedoes]` values
diverge from `TorpedoConfig::default()`.

## Phaser combat configuration (TOML-driven)

Player phaser tuning (beam duration, cooldown, damage-per-second, range)
is sourced from the existing `[weapons_console]` block in
`assets/entities/player_ship.toml`:

```toml
[weapons_console]
beam_range = 40.0
beam_damage_per_sec = 5.0
beam_duration_secs = 6.0
cooldown_secs = 6.0
```

`PhaserCombatConfig::from_weapons_console` (`src/entities/config.rs`)
maps these into a `PhaserCombatConfig` (using "zero means absent"
fallback to defaults, mirroring the NPC weapons path). The value is
inserted as `PhaserCombatConfigResource` during
`spawn_game_start_entities` (`src/server_app.rs`). `WeaponsPlugin::build`
seeds the resource with `PhaserCombatConfig::default()` so test apps
that never load a ship TOML still get a working phaser system.

`handle_fire_phaser`, `tick_active_beam`, and the
`weapons_update_broadcaster` all read this resource instead of the
legacy module-private `_LEGACY_BEAM_DURATION_SECS` /
`_LEGACY_BEAM_COOLDOWN_SECS` constants. The public
`BEAM_DAMAGE_PER_SEC` constant is retained because integration tests in
`src/server_app.rs` use it as a baseline assertion value.

Deliberately **not** TOML-driven (engineering invariants): the forward
firing arc (180° hemisphere) and `radar::PHASER_RANGE` (only referenced
by `radar.rs`'s own unit tests now). The `PhaserConfig` /
`PhaserSystem` types in `src/weapons/phaser.rs` are dead code in the
live game and were intentionally left untouched this slice.

Drift guards in `src/entities/config.rs`:
- `player_ship_toml_weapons_console_phaser_combat_matches_runtime_defaults`
- `player_ship_toml_shields_base_block_matches_runtime_default_values`

End-to-end "TOML flows to live state" tests:
- `phaser_combat_config_resource_reflects_player_ship_toml_weapons_console`
  (`src/console/weapons/server.rs`)
- `shield_system_reflects_player_ship_toml_shields_console_base_block`
  (`src/weapons/shield.rs`)

## Shield base configuration (TOML-driven)

Shield base tuning (facing count, max HP, regen rate, offline duration)
lives in a nested sub-block under `[shields_console]`:

```toml
[shields_console.base]
num_facings = 4         # UI assumes 4; changing this will break the Shields panel
max_hp = 100
regen_per_sec = 5.0
offline_duration = 10.0
```

The nested block mirrors the `[sensors_console.long_range_radar]`
precedent. `ShieldsBaseConfig::to_runtime()` produces a
`ShieldConfig`; `spawn_game_start_entities` constructs the live
`ShieldSystem` via
`ShieldSystem::new(&sc.base.map(to_runtime).unwrap_or_default())` and
then overlays the focus configuration. `Option<ShieldsBaseConfig>` was
used so the 22 existing `EntityConfig {...}` test literals scattered
across the codebase did not need to be touched.

## Sources

- `src/weapons_plugin.rs`
- `src/simulation.rs` (pub use re-exports and integration tests)
- Issue [#245](https://github.com/jkeywo/project-phoenix-v2/issues/245)
- [Console Plugin Pattern](./console-plugin-pattern.md)
- [Broadcaster Seam](./broadcaster-seam.md)
