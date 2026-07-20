---
title: Station
type: entity
tags: [station, lobby, roster, rating, ai]
sources: [src/ship/config.rs, src/lobby/stations_config.rs, src/ship/components.rs, src/ship/rating_systems.rs, assets/entities/alliance_battleship.toml]
updated: 2026-07-16
---

# Station

A fixed bridge seat a player can claim in the lobby. Stations replaced the
old per-player-count console bundles in B1–B3 (issue #518).

Each hull carries an authored roster of stations. A station owns one or more
[Systems](./system.md); a console is chosen by the station's authored
`console` URL and can lay out any combination of its owned fine systems. For
example, the Courier has Captain and Tactical stations.

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

Defined in `src/lobby/stations_config.rs:StationDef`. Its optional `console`
URL is passed from TOML to the client, while `gui/mount-plan.js` derives
stable DOM ids from the station id (`${id}-ui` / `${id}-iframe`, one
tactical → weapons alias).

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

The optional `console = "..."` field selects the station's iframe page.

Parsed into `StationConfig` by `src/ship/config.rs`, validated and loaded into
`ShipConfigResource` at startup by `load_ship_config_from_disk()`.

## Ratings

Each station has one or more named ratings (e.g. `Std`, `Assisted`). The active
rating for each station is tracked in `ActiveStationRatings` (`src/ship/components.rs`)
and drives `ShipSystemControlSources` — the per-system Human/AI gate that every
handler checks before processing input or running AI controllers.

`automated_systems` lists the `SystemId`s that become AI-controlled when this
rating is active. `[station.rating.ai_tuning]` carries optional rule parameters
consumed by the per-kind AI controller (e.g. fire delay, engagement threshold).

The current station holder may change ratings at runtime, including during an
active round, via `SetStationRating`. The host validates that the sender holds
the station and that the requested name is authored for it, applies the new
automated-system set immediately, then broadcasts `RatingChanged` to every
client. This is shared game authority, not a local settings preference.

## Lobby flow

1. `stations_from_ship_config()` builds a `ShipStations { stations: Vec<StationDef> }`
   from `ShipConfigResource` at plugin startup (`src/lobby/stations_config.rs`).
2. `Welcome` carries the flat `stations` list to every joining client.
3. `SelectStation { station }` from a client matches against `ShipStations.stations`
   by display name or id and writes `Player.station = Some(StationId)`.
4. `StationAssigned { station_id }` is broadcast to all peers; each client
   derives the console panel from the id via `gui/mount-plan.js`.

## Related

- [Console](./console.md) — GUI panel keyed on the station id
- [System](./system.md) — fine-grained capability instance owned by a station
- player_ship.toml — TOML source
- Issue #518 — B1–B6 migration
- PRD #487 — architecture context
- Issue [#619](https://github.com/jkeywo/project-phoenix-v2/issues/619) — Console enum + `StationDef.consoles` + `StationConfig.console` deleted
