---
title: Issue #541 C2 - SelectStation writes Player.station
type: source
tags: [prd-519, c2, lobby, station, ship-config]
source_url: https://github.com/jkeywo/project-phoenix-v2/issues/541
status: shipped
updated: 2026-06-24
---

# Issue #541 C2 - SelectStation Writes Player.station

## Status

Shipped. Parent: [PRD #519](./prd-519-player-station-ai-backfill.md).

## What Changed

`ClientMessage::SelectStation { station }` writes `Player.station` through `SessionManager::set_station`. The station roster is derived from `ShipConfig`, and `StationAssigned` carries both the display station name and stable `station_id`.

Key code references:

- `src/lobby/handler.rs:214` - `SelectStation` branch.
- `src/lobby/stations_config.rs:36` - fixed roster derived from `ShipConfig`.
- `src/core/messages.rs:1110` - `ServerMessage::StationAssigned`.

## Post-Change Contract

Station selection is station-level. Console tabs are a client display consequence of the assigned station, not the authoritative ownership record.
