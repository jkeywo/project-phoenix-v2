---
title: Station
type: entity
tags: [station, lobby, roster, rating, ai]
sources: [src/ship/config.rs, src/lobby/stations_config.rs, src/ship_plugin.rs, assets/entities/player_ship.toml]
updated: 2026-07-03
---

# Station

A fixed bridge seat a player can claim in the lobby. Stations replaced the
old per-player-count console bundles in B1–B3 (issue #518).

The ship carries a fixed roster of **9 stations**: captain, helm, tactical,
repair, sensors, shields, navigation, power, comms. Each maps 1-to-1 to a
[Console](./console.md) panel and owns one or more [Systems](./system.md).

## Wire shape (`StationDef`)

Sent to clients inside `Welcome.ship_stations.stations`:

```json
{
  "id": "helm",
  "name": "Helm",
  "description": "Drive the ship and manage impulse.",
  "rank": "Lt.",
  "short_code": "HLM"
}
```

Defined in `src/lobby/stations_config.rs:StationDef`. The `consoles` field
was deleted in issue #619 — clients derive the console panel from the
`station_id` string directly via `gui/console-registry.js`.

## TOML schema (`[[station]]`)

```toml
[[station]]
id = "helm"           # StationId — stable snake-case wire address
name = "Helm"
description = "Drive the ship and manage impulse."
rank = "Lt."
short_code = "HLM"

[[station.rating]]
name = "Std"
automated_systems = []      # system IDs automated at this rating

[[station.rating]]
name = "Assisted"
automated_systems = []      # system IDs automated at this rating

[station.rating.ai_tuning]  # optional; arbitrary TOML consumed by the AI rule
key = { ... }
```

The `console = "..."` field that appeared here pre-#619 was deleted with the
Console enum; the station id itself is now the canonical routing key.

Parsed into `StationConfig` by `src/ship/config.rs`, validated and loaded into
`ShipConfigResource` at startup by `load_ship_config_from_disk()`.

## Ratings

Each station has one or more named ratings (e.g. `Std`, `Assisted`). The active
rating for each station is tracked in `ActiveStationRatings` (`src/ship_plugin.rs`)
and drives `ShipSystemControlSources` — the per-system Human/AI gate that every
handler checks before processing input or running AI controllers.

`automated_systems` lists the `SystemId`s that become AI-controlled when this
rating is active. `[station.rating.ai_tuning]` carries optional rule parameters
consumed by the per-kind AI controller (e.g. fire delay, engagement threshold).

Ratings change at runtime via the `RatingChanged` server broadcast (PRD #517 A2).

## Lobby flow

1. `stations_from_ship_config()` builds a `ShipStations { stations: Vec<StationDef> }`
   from `ShipConfigResource` at plugin startup (`src/lobby/stations_config.rs`).
2. `Welcome` carries the flat `stations` list to every joining client.
3. `SelectStation { station }` from a client matches against `ShipStations.stations`
   by display name or id and writes `Player.station = Some(StationId)`.
4. `StationAssigned { station_id }` is broadcast to all peers; each client
   derives the console panel from the id via `gui/console-registry.js`.

## Related

- [Console](./console.md) — GUI panel keyed on the station id
- [System](./system.md) — fine-grained capability instance owned by a station
- [player_ship.toml](../sources/player_ship_toml.md) — TOML source
- [Issue #518](../sources/issue-540-config-migration-docs.md) — B1–B6 migration
- [PRD #487](../sources/prd-487-station-console-system-redesign.md) — architecture context
- Issue [#619](https://github.com/jkeywo/project-phoenix-v2/issues/619) — Console enum + `StationDef.consoles` + `StationConfig.console` deleted
