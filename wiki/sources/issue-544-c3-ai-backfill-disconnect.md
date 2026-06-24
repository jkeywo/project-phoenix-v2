---
title: Issue #544 C3 - AI backfill on disconnect
type: source
tags: [prd-519, c3, disconnect, backfill, rating]
source_url: https://github.com/jkeywo/project-phoenix-v2/issues/544
status: shipped
updated: 2026-06-24
---

# Issue #544 C3 - AI Backfill On Disconnect

## Status

Shipped. Parent: [PRD #519](./prd-519-player-station-ai-backfill.md).

## What Changed

When a station holder disconnects, the server records the station's current rating in `Player.last_rating`, flips `connected` to false, applies the station's `Backfill` rating, and broadcasts `RatingChanged`.

Key code references:

- `src/lobby/handler.rs:337` - `process_disconnect_with_stations`.
- `src/lobby/session.rs:83` - `set_last_rating`.
- `src/ship/rating.rs` - rating application helpers.

## Post-Change Contract

Disconnect is not a reshuffle. The station remains associated with the disconnected player record, but its systems are AI-operated until the player returns or another connected player claims the station.
