---
title: Issue #548 — E2 Promote singleton Resources to per-entity Ship Components
type: source
tags: [ai, ship, ecs, components, resources, prd-520, shipped]
source_url: https://github.com/jkeywo/project-phoenix-v2/issues/548
status: shipped
updated: 2026-06-25
---

# Issue #548 — E2 Promote singleton Resources to per-entity Ship Components

PRD #520 slice E2. Converts four singleton `Resource`s into per-entity `Component`s on `Ship` entities. Enables multiple ships to each carry independent state.

## Changes

- `ShipConfigResource` → `ShipConfigComponent` (`Component` instead of `Resource`)
- `CoordinationQueueResource` → `CoordinationQueue`
- `ShipSystemControlSources` and `ActiveStationRatings` — renamed to Components
- `HelmInputResources` SystemParam dissolved — fields inlined into callers
- Player ship spawn in `server_app.rs` inserts all four Components alongside the `Ship` marker
- Lobby handlers changed from `single_mut()` to `iter_mut().next()`

## Files

- `src/ship_plugin.rs`, `src/server_app.rs`, `src/lobby/server.rs`

## Cross-references

- [PRD #520](./prd-520-ai-ship-unification.md) — parent
- [AI Ship Unification](../concepts/ai-ship-unification.md)
