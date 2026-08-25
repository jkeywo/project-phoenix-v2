---
title: Station
type: entity
tags: [station, lobby, roster, rating, ai, human-seeking]
sources: [src/ship/config.rs, src/ship/coordination.rs, src/ship/coordination_systems.rs, src/lobby/stations_config.rs, src/ship/components.rs, src/ship/rating_systems.rs, assets/entities/alliance_destroyer.toml]
updated: 2026-08-25
---

# Station

A Station is an authored operable surface with a stable identity, complete
console, rating, and owned Systems. A primary Station is a lobby-claimable
bridge seat; an auxiliary Station is mounted but not offered as a separate
seat.

A fixed bridge seat a player can claim in the lobby. Stations replaced the
old per-player-count console bundles in B1–B3 (issue #518).

The Alliance Cruiser's Navigation is an auxiliary, human-seeking station. It
is hosted first by Comms and retains its own Navigation system ownership when
opened as a visiting console, without adding a claimable lobby seat.

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

A complete human-seeking Station adds `human_seeking = true`, a finite
`host_order`, and an authored `visiting_rating`. Its own active direct holder
wins; otherwise `resolve_visiting_station` walks only compatible directly held
Stations and then selects Backfill AI. A human-seeking Station may itself host
while it has an active direct holder; a Station that is currently visiting has
no direct holder and is therefore ineligible, preventing nested tabs and
transitive authority. A world's top-level `scenario_detail_floor` selectors
name Station ids (console families) or System kinds; `write_scenario_detail_floor`
resolves them against the selected hull into the live `ScenarioDetailFloor`
component before placement, which publishes the resulting effective visiting
rating in `VisitingStationHosts`; `SimSnapshot.station_hosts` projects those
station-level placements generically to the client shell. The Alliance Destroyer
authors two: Navigation (the first shipped user) and Comms, added in #1098 —
each System stays owned by its own complete Station wherever the surface is
presented. Comms carries only one authored rating on every hull that declares
it, so its `visiting_rating` is the same full interface a direct holder gets
rather than a reduced one; Navigation's `Simplified` visiting tier is the
exception, not the pattern.

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

Static world entities may also be configured as AI-only combatants without
declaring any player station. Axiom Station uses this shape: its Tactical radar
and 360-degree phaser are ownerless AI-only systems, providing local point
defence while leaving the station absent from the lobby roster.

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
