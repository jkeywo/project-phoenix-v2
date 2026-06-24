---
title: PRD #519 - Lobby migration to Player.station + AI backfill
type: source
tags: [prd-519, player, station, ai-backfill, reconnect]
source_url: https://github.com/jkeywo/project-phoenix-v2/issues/519
status: open
updated: 2026-06-24
---

# PRD #519 - Lobby Migration To Player.station + AI Backfill

## Status

Open parent PRD for phase 4 of the station/console/system redesign. Slices C1-C7 have been implemented across issues #541-#545; issue #546 is the documentation slice.

## Problem

The lobby still treated console ownership as `Player.consoles: Vec<Console>` and used all-or-nothing console restoration on reconnect. That model did not line up with fixed stations, per-station ratings, or AI backfill.

## Target State

- `Player.station: Option<StationId>` is the ownership field.
- Console access is derived from `Player.station` plus `ShipConfig`.
- Disconnect applies the station `Backfill` rating so AI operates station systems while the player is away.
- Reconnect restores the prior station and `last_rating` only if another connected player has not claimed the station.
- `SetReady` is the only start path; `ClientMessage::StartGame` is removed.
- Repair supports `RepairTarget::Station(_)` and `RepairTarget::Core`.

## Slice Map

- C1: [Issue #541 C1](./issue-541-c1-player-station-field.md)
- C2: [Issue #541 C2](./issue-541-c2-selectstation-writes-station.md)
- C3: [Issue #544 C3](./issue-544-c3-ai-backfill-disconnect.md)
- C4: [Issue #544 C4](./issue-544-c4-reconnect-yields.md)
- C5: [Issue #545 C5](./issue-545-c5-retire-player-consoles.md)
- C6: [Issue #542 C6](./issue-542-c6-delete-startgame.md)
- C7: [Issue #543 C7](./issue-543-c7-repair-target-core.md)
- D: [Issue #546 D](./issue-546-d-player-station-docs.md)

## Cross-References

- [PRD #487 - Station/Console/System redesign](./prd-487-station-console-system-redesign.md)
- [Player](../entities/player.md)
- [Game phases](../concepts/game-phases.md)
