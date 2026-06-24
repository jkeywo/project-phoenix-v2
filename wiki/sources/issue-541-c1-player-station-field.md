---
title: Issue #541 C1 - Add Player.station field
type: source
tags: [prd-519, c1, player, station, wire]
source_url: https://github.com/jkeywo/project-phoenix-v2/issues/541
status: shipped
updated: 2026-06-24
---

# Issue #541 C1 - Add Player.station Field

## Status

Shipped. Parent: [PRD #519](./prd-519-player-station-ai-backfill.md).

## What Changed

`Player` gained `station: Option<StationId>` as the stable ownership field, alongside `ready` and `last_rating`. During the transitional slice, read paths preferred `Player.station` and fell back to the old console list where needed.

Key code references:

- `src/core/messages.rs:452` - `Player` wire shape.
- `src/lobby/session.rs:68` - `station_for_token`.
- `src/lobby/session.rs:76` - `set_station`.

## Post-Change Contract

Code should ask which station a token holds, then derive console/system access from `ShipConfig`. It should not infer player ownership by scanning a mutable console vector.
