---
title: Issue #528 - Route Shields advisories through CoordinationEnqueue
type: source
tags: [prd-517, shields, coordination, channel-3]
source_url: https://github.com/jkeywo/project-phoenix-v2/issues/528
status: shipped
updated: 2026-06-23
---

# Issue #528 - Route Shields advisories through CoordinationEnqueue (PRD #517 A1)

## Status

Shipped. Parent: PRD #517 (slice A1).

## Channel decision (#505 context)

Shield-facing advisories (`ShieldFacingDown`, `ShieldFacingRestored`) are channel-3 (delayed advisory) traffic directed at Helm. They are not authoritative immediate state changes (channel 2). The `CoordinationEnqueue` path was already established and is the correct surface. This matches the PRD #517 / issue #493 decision: all channel-3 traffic routes via `CoordinationEnqueue`, no direct `SimOutbox` pushes of `CoordinationPopup` outside `process_coordination_lag`.

## What was done

### `src/ship/shields.rs`

- Added `use crate::ship_plugin::CoordinationEnqueue;` import.
- Removed `use crate::simulation::SimOutbox;` (no longer needed in this file's production paths).
- Added `app.add_message::<CoordinationEnqueue>()` to `ShipShieldsPlugin::build`.
- Rewrote `emit_shields_coordination`:
  - Removed `mut outbox: ResMut<SimOutbox>` and `sessions: Res<crate::lobby::Sessions>` parameters.
  - Added `mut writer: MessageWriter<CoordinationEnqueue>` parameter.
  - Both `ShieldFacingDown` and `ShieldFacingRestored` payloads now call `writer.write(CoordinationEnqueue { sender_origin, target: helm_system_id(), payload, sender_label: "Shields" })`.
  - The `sender_label` is now always `"Shields"` (AI/Human routing is handled by the delivery-time routing matrix in `process_coordination_lag`, not the label).
- Deleted the private `enqueue_coordination` helper entirely.
- Updated 4 test assertions (4 coordination tests) to collect `CoordinationEnqueue` events via a new `CoordEnqueueBox` resource + `collect_coord` system, replacing the old `CoordinationPopup`-in-SimOutbox checks.
- Updated imports in the test module.

## Post-change state

- `rg "CoordinationPopup" src/ship/shields.rs` → only doc-comment mention (line 170 in the doc-block for `emit_shields_coordination`). No direct push remains.
- `cargo test` passes (1965 tests).

## Cross-references

- [PRD #517 - Consistency cleanup](./prd-517-consistency-cleanup.md)
- [Issue #493 - Coordination-lag scope](./issue-493-coordination-lag-scope.md)
- [Coarse-system migration concept](../concepts/coarse-system-migration.md)
