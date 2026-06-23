---
title: Issue #529 - Make Captain expose a real viewscreen-owned system
type: source
tags: [prd-517, captain, viewscreen, rating, auto-badge]
source_url: https://github.com/jkeywo/project-phoenix-v2/issues/529
status: shipped
updated: 2026-06-23
---

# Issue #529 - Make Captain expose a real viewscreen-owned system (PRD #517 A3)

## Status

Shipped. Parent: PRD #517 (slice A3). Blocked-by: #505 — now closed.

## Audit finding closed

The `viewscreen` system was previously declared `ai_only = true` with no `station` field, making it a ghosted ownerless system. The Captain station's rating table only referenced `red-alert`. `apply_rating` / `backfill` on the captain station would silently ignore viewscreen. This is the "inert Captain-kind audit finding" from PRD #517 A3.

## What was done

### `src/core/messages.rs`

- Added `viewscreen_system_id: SystemId` field to `CaptainConsoleState` (with `serde(default = "default_viewscreen_system_id")`).
- Added `viewscreen_auto: bool` field to `CaptainConsoleState` (with `serde(default)`).
- Added `default_viewscreen_system_id()` helper.
- Updated `Default` impl to include both new fields.

### `src/console/captain/server.rs`

- Updated `spawn_captain_console_state_entity` to include `viewscreen_system_id` and `viewscreen_auto` in the spawned initial state.
- Added `viewscreen_auto` computation in `recompute_captain_console_state`:
  `cs.0.source_for(&viewscreen_system_id()) == ControlSource::Ai`.
- Updated `next` struct literal to include both new fields.
- Updated two explicit `CaptainConsoleState { ... }` constructions in tests.
- Added 2 new tests:
  - `recompute_marks_ai_controlled_viewscreen_auto`
  - `recompute_viewscreen_auto_is_false_by_default`

### Embedded TOML in `src/ship_plugin.rs`

- Changed `viewscreen` system from `ai_only = true` (no station) to `station = "captain"`.
- Added `"viewscreen"` to the captain `"Assisted"` rating's `automated_systems`.
- Updated `set_station_rating_backfill_automates_all_station_systems` test to also assert `viewscreen → Ai`.

### Test TOMLs in `src/ship/rating.rs` and `src/ship/config.rs`

- Both test TOML fixtures updated (same change: `station = "captain"` for viewscreen, `"viewscreen"` in Assisted rating).
- `rating.rs`: replaced `returns_ai_only_systems` test with `returns_no_ai_only_systems_after_viewscreen_moved_to_captain`; added 3 new captain rating tests.
- `config.rs`: updated `rejects_ownerless_without_ai_only` test to use an inline orphan system (since viewscreen is no longer ownerless/ai_only).

### `src/core/codec.rs`

- Updated `encode_console_state_round_trips_captain` to include `viewscreen_system_id` and `viewscreen_auto` fields; added assertions for new field names in JSON.

## Post-change state

`cargo test` passes (1971 tests, up from 1966). `viewscreen` is now a first-class member of the captain station's system roster and participates in the rating/AUTO path.

## Cross-references

- [PRD #517 - Consistency cleanup](./prd-517-consistency-cleanup.md)
- [Coarse-system migration concept](../concepts/coarse-system-migration.md)
