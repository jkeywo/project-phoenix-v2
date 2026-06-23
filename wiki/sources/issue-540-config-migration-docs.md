---
title: Issue #540 - Config migration docs (B1–B6)
type: source
tags: [issue, ship-config, stations, systems, wire-protocol, ClientMessage, ComplexityUI]
source_url: https://github.com/jkeywo/project-phoenix-v2/issues/540
status: shipped
updated: 2026-06-23
---

# Issue #540 - Config migration docs (B1–B6)

## Status

Shipped. Documents the B1–B6 migration that landed in issues #531–#539.

## What was done

### B1 + B2 — Load ShipConfig from disk, migrate player_ship.toml (#531 #532)

Added `src/ship/config.rs` (from issue #489) as the live Bevy resource path:

- `ShipConfigResource(ShipConfig)` in `src/ship_plugin.rs` — loaded at startup
  from `assets/entities/player_ship.toml` by `load_ship_config_from_disk()`.
- `ActiveStationRatings` in `src/ship_plugin.rs` — tracks the current rating
  per station (used by `ControlSourceResolver` and AI tuning queries).
- Migrated `player_ship.toml` from the legacy `[stations]` per-player-count
  tables to `[[station]]` + `[[system]]` array-of-tables.
  - 9 stations: captain, helm, tactical, repair, sensors, shields, navigation,
    power, comms.
  - ~15 systems declared with `id`, `kind`, `station`, `power_group`, and
    optional `[system.config]` blocks.
  - `[[station.rating]]` entries replace the old complexity-toml references.

### B3 — Retire per-player-count roster machinery (#533)

Deleted `lobby/stations_config.rs` per-player-count tables and the
`stations_policy.rs` promotion-link machinery. The lobby now uses the fixed
roster from `ShipConfigResource`.

### B4 — Retire ConsoleComplexity system (#534)

Deleted `src/console_ai/complexity.rs`, `src/console_ai/delegation.rs`, and
all 5 `assets/complexity/*.toml` files. Removed the `complexity_toml` field
from all `EntityConfig` console structs and deleted the GUI modules
`gui/complexity-store.js`, `gui/complexity-ui.js`, `gui/hideable-elements.js`.

### B5a–d — Retire 18 legacy ClientMessage variants (#535–#538)

Removed 18 variants from `ClientMessage` that were superseded by the
`ControlSystem { target: SystemId, payload: SystemControlPayload }` envelope:

| Group | Removed variants |
|---|---|
| Helm (B5a) | `HelmInput`, `StartImpulseCharge`, `CancelImpulse`, `ToggleBoost`, `SetBoost` |
| Tactical (B5b) | `SetTarget`, `SetPhaserMode` |
| Science/Nav (B5c) | `SetScienceTarget`, `SetSensorsTarget`, `SetNavigationWaypoint`, `ClearNavigationWaypoint` |
| Comms (B5d) | `Hail`, `SelectCommsMessage`, `RespondToMessage`, `ClearComms`, `ShowOnScreen` |

All handler functions now match only the `ControlSystem` envelope. The
`ui_action_to_client_message` map in `core/messages.rs` was updated first so
JS clients continued to work throughout. 27 codec round-trip tests for the
removed variants were deleted; 1855 tests pass.

### B6 — Delete ShipConfigResource inline stub (#539)

Replaced the 90-line hardcoded TOML stub inside `impl Default for
ShipConfigResource` with a delegation to `load_ship_config_from_disk()`.
Fallback paths that previously returned the stub now panic — the server
cannot start without a valid `player_ship.toml`.

## Key architectural notes

### player_ship.toml new schema

```toml
# Station roster (fixed, not per-player-count)
[[station]]
id = "captain"          # StationId — referenced by [[system]]
name = "Captain"
console = "captain"     # Console enum id
description = "..."
rank = "Cpt."
short_code = "CPT"

[[station.rating]]
name = "Std"
automated_systems = []  # SystemIds automated at this rating

# System declarations
[[system]]
id = "red-alert"        # SystemId — stable wire address
kind = "red_alert"      # SystemKind — determines handler
station = "captain"     # owning station
power_group = "ops"     # power allocation bucket

[system.config]         # optional: kind-specific config (opaque toml::Value)
```

### Wire protocol after B5

All console actions use the `ControlSystem` envelope:

```json
{ "type": "ControlSystem",
  "data": { "target": "helm",
             "payload": { "type": "HelmInput", "data": { "thrust": 1.0, "steering": 0.0 } } } }
```

The `target` field is a `SystemId` string (lowercase kebab).
`SystemControlPayload` variants are typed per-system-kind.

### ControlSourceResolver

`ShipSystemControlSources` maps each `SystemId` → `ControlSource` (Human/Ai).
Each game tick, handler systems call `policy_for(&system_id)` and check
`accept_human_input` / `operate_ai` before processing messages or running AI.
`ActiveStationRatings` drives which systems are automated for each station.

## Cross-references

- [Issue #489 - Ship config loader](./issue-489-ship-config-loader.md)
- [Issue #488 - Station/System ADR](./issue-488-station-system-adr.md)
- [PRD #487 - Station/Console/System redesign](./prd-487-station-console-system-redesign.md)
- [player_ship.toml](./player_ship_toml.md)
- [ShipPlugin concept](../concepts/ship-plugin.md)
- [Message flow concept](../concepts/message-flow.md)
