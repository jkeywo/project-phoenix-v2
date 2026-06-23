---
title: Issue #526 - Route Repair through coarse ControlSystem dispatch
type: source
tags: [prd-517, repair, control-system, coarse-system]
source_url: https://github.com/jkeywo/project-phoenix-v2/issues/526
status: shipped
updated: 2026-06-23
---

# Issue #526 - Route Repair through coarse ControlSystem dispatch (PRD #517 A7)

## Status

Shipped. Parent: PRD #517 (slice A7).

## What was done

### `src/ship/system_registry.rs`

- Registered `repair` kind in `SystemKindRegistry::with_core_systems()` with `repair_ai` controller.
- Added `core_registry_has_repair_ai_controller` test.
- Removed the "deferred to #526" doc-comment from `REPAIR_SYSTEM_ID`.

### `src/console/repair/server.rs`

- Added imports: `RepairTarget`, `SystemControlPayload`, `repair_system_id`, `REPAIR_SYSTEM_ID`, `ShipSystemControlSources`.
- Updated `handle_dispatch_repair_team`:
  - Added `control_sources: Res<ShipSystemControlSources>` parameter.
  - Resolves `policy = control_sources.0.policy_for(&repair_system_id())` before the event loop.
  - Added `ClientMessage::ControlSystem { target, payload } if target.0 == REPAIR_SYSTEM_ID` arm.
  - Maps `RepairTarget::Station(StationId(s))` → `Console::from_console_id(&s)` (drops if unknown).
  - `RepairTarget::Core` → no-op (per PRD C note, out of scope for this slice).
  - Added `if !policy.accept_human_input { continue; }` guard before the holder check.
- Initialized `ShipSystemControlSources` in `test_app()`.
- Added 5 new tests:
  - `control_system_dispatch_authorized_sends_team_to_travelling`
  - `control_system_dispatch_unauthorized_sender_is_rejected`
  - `control_system_dispatch_rejected_when_ai_controlled`
  - `control_system_dispatch_repair_target_core_is_noop`
  - `legacy_dispatch_still_works_after_control_system_migration`

## Notes

`RepairTarget::Core` is wire-legal (accepted by the `ControlSystem` match arm) but produces no runtime effect. Per-core repair targeting is tracked by PRD C slice C7. This is documented in both the handler doc-comment and the test.

## Post-change state

`cargo test` passes (1956 tests, up from 1950).

## Cross-references

- [PRD #517 - Consistency cleanup](./prd-517-consistency-cleanup.md)
- [Coarse-system migration concept](../concepts/coarse-system-migration.md)
