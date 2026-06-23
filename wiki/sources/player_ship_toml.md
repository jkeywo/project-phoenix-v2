---
title: assets/entities/player_ship.toml
---

# `assets/entities/player_ship.toml`

The single source of truth for the player ship's stats: hull, repair, weapons,
torpedoes, sensors, helm, navigation, comms, shields, power, and the per-player-
count `[stations]` layout. Loaded at startup by `entities/config_cache.rs`,
parsed into `EntityConfig` (`src/entities/config.rs`), and consumed by:

- `lobby/server.rs::update_session_with_config` — builds `ShipClientConfig`
  for the `Welcome` message (`helm_radar_range`, `repair_*`,
  `impulse_charge_duration`, `phaser_banks`, `torpedo_tubes`,
  `phaser_beam_color`, `torpedo_arc_color`).
- `server_app.rs::spawn_game_start_entities` — inserts
  `PhaserCombatConfigResource`, `TorpedoSystemResource`, `ShieldConfig`,
  `ImpulseConfigResource`, `HullIntegrity` etc. into the live world.
- `entities/spawner.rs` — applies `[collider]` / `[mesh]` / `[hull]` / `[hull
  .console_hull]` to the spawned ship entity.

## Key blocks

| Block | Consumed by | Notes |
|---|---|---|
| `[hull]` + `[[hull.console_hull]]` | `ship/damage.rs::HullIntegrity`, `modifiers/repair_teams.rs` | Per-console HP slots; total HP = sum of slots. |
| `[repair]` | `RepairPlugin`, broadcast via `ShipClientConfig` | Travel duration + repair rate; clients derive bar timings. |
| `[weapons_console]` | `WeaponsPlugin`, `PhaserCombatConfigResource` | `beam_range`, `beam_damage_per_sec`, `beam_duration_secs`, `cooldown_secs`, `beam_color`, `torpedo_arc_color`. |
| `[[weapons_console.phaser_banks]]` | `validate_phaser_banks`, `PhaserSystem`, client radar arcs | `id`, `facing_deg`, `fire_arc_deg`, `auto_arc_deg` (≤ `fire_arc_deg`), optional `beam_range` override. |
| `[torpedoes]` | `TorpedoConfig` | Shared `count` pool, damage, speed, turn rate, lifespan, load_time. |
| `[[torpedoes.tubes]]` | `validate_torpedo_tubes`, `TorpedoSystem`, client radar arcs | `id`, `facing_deg`, `fire_arc_deg`. Ammo is shared, not per-tube. |
| `[sensors_console]` + `[sensors_console.long_range_radar]` | `SciencePlugin`, Sensors panel | Detection range + filter. |
| `[shields_console]` + `[shields_console.base]` | `ShieldSystem` | `num_facings = 4` (UI assumption). |
| `[power]` | `PowerPlugin` | `capacity`, per-level `rates`, `emergency_threshold`. |
| `[helm_console]` + `[helm_console.boost]` | `HelmPlugin`, `ImpulseConfigResource`, `BoostConfigResource` | Speed/accel/yaw + `impulse_*`; boost speed/accel, steering-rate multiplier, battery timings. |
| `[navigation_console.system_chart]` | Navigation panel | Range + filter. |
| `[comms]` | `CommsPlugin` | `range` for the ship's own hailing radius. |
| `[[station]]` | `ship/config.rs`, `ShipConfigResource` | Fixed roster: one entry per station (9 stations). Each carries `[[station.rating]]` sub-tables. |
| `[[system]]` | `ship/config.rs`, `ShipConfigResource` | System declarations: `id`, `kind`, `station`, `power_group`, optional `[system.config]`. |
| `[radar_appearance]` | Renderer | Colour + radius on radar. |

## Station / system schema (2026-06, B1+B2)

As of issues #531–#532, the legacy `[stations]` per-player-count tables have been
replaced with:

- `[[station]]` — fixed roster (9 stations: captain, helm, tactical, repair,
  sensors, shields, navigation, power, comms). Each carries `[[station.rating]]`
  sub-tables listing `automated_systems` and optional `[station.rating.ai_tuning]`.
- `[[system]]` — one entry per system instance (`id`, `kind`, `station`,
  `power_group`). Parsed into `ShipConfig` by `src/ship/config.rs` and loaded
  into `ShipConfigResource` at startup.

See [Issue #540 - Config migration docs](./issue-540-config-migration-docs.md)
for the full migration log.

## Per-bank phasers and per-tube torpedoes (2026-05)

See [weapons-plugin → Per-bank phasers and per-tube torpedoes](../concepts/weapons-plugin.md#per-bank-phasers-and-per-tube-torpedoes-2026-05)
for the full TOML schema, wire shape, client UI, and drift guards. The
canonical ship layout ships two phaser banks (`fore`, `aft`) and three
torpedo tubes (`fore_port`, `fore_starboard`, `aft`), but everything is
data-driven — NPC ships and player variants can declare any layout the
validators accept.

## Drift guards

- `player_ship_toml_torpedoes_block_matches_runtime_default_values`
  (`src/entities/config.rs`).
- `player_ship_toml_weapons_console_phaser_combat_matches_runtime_defaults`.
- `player_ship_toml_shields_base_block_matches_runtime_default_values`.
- `validate_phaser_banks`, `validate_torpedo_tubes` — invoked when the ship
  config is loaded; reject empty lists, duplicate ids, out-of-range arcs.

## Sources

- [`assets/entities/player_ship.toml`](../../assets/entities/player_ship.toml)
- [`src/entities/config.rs`](../../src/entities/config.rs)
- [`src/lobby/server.rs`](../../src/lobby/server.rs)
- [WeaponsPlugin concept](../concepts/weapons-plugin.md)
