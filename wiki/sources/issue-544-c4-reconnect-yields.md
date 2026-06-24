---
title: Issue #544 C4 - Reconnect yields to live station claims
type: source
tags: [prd-519, c4, reconnect, station, spectator]
source_url: https://github.com/jkeywo/project-phoenix-v2/issues/544
status: shipped
updated: 2026-06-24
---

# Issue #544 C4 - Reconnect Yields To Live Station Claims

## Status

Shipped. Parent: [PRD #519](./prd-519-player-station-ai-backfill.md).

## What Changed

Reconnect now checks the returning player's previous `StationId`. If no other connected player has claimed it, the server restores the station and reapplies `last_rating`. If another player has claimed it, the returning player lands without a station/spectator assignment instead of taking the seat back.

Key code references:

- `src/lobby/handler.rs:86` - reconnect restore/yield logic in `Identify`.
- `src/lobby/handler.rs:139` - restored `StationAssigned` broadcast.
- `src/lobby/handler.rs:148` - restored `RatingChanged` broadcast.

## Post-Change Contract

The old all-or-nothing console-set restore path is gone. Reconnect is based on station identity and current connected occupancy.
