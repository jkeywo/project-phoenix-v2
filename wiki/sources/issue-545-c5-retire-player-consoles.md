---
title: Issue #545 C5 - Retire Player.consoles
type: source
tags: [prd-519, c5, player, station, console-ownership]
source_url: https://github.com/jkeywo/project-phoenix-v2/issues/545
status: shipped
updated: 2026-06-24
---

# Issue #545 C5 - Retire Player.consoles

## Status

Shipped in commit `9d03142`. Parent: [PRD #519](./prd-519-player-station-ai-backfill.md).

## What Changed

`Player.consoles: Vec<Console>` was deleted. Session, lobby, broadcast, and console authorization code now derives console access from `Player.station` plus the loaded `ShipConfig`.

Key code references:

- `src/core/messages.rs:452` - `Player` no longer contains `consoles`.
- `src/lobby/session.rs:91` - available consoles derived from unclaimed stations.
- `src/lobby/session.rs:127` - `console_holder` resolves station ownership via `ShipConfig`.
- `src/lobby/session.rs:140` - `player_has_console` delegates to `station_has_console`.
- `src/lobby/handler.rs:252` - `ReleaseStation` clears station ownership.
- `gui/lobby-state.js` - client-side display derives held consoles from `player.station` and `ship_stations`.

## Post-Change Contract

The only authoritative player ownership field is `Player.station`. Any `consoles` list in `StationAssigned` or client view state is derived presentation data.
