---
title: Issue #527 - Route Navigation through coarse ControlSystem dispatch
type: source
tags: [prd-517, navigation, control-system, coarse-system]
source_url: https://github.com/jkeywo/project-phoenix-v2/issues/527
status: shipped
updated: 2026-06-23
---

# Issue #527 - Route Navigation through coarse ControlSystem dispatch (PRD #517 A8)

## Status

Shipped. Parent: PRD #517 (slice A8).

## What was done

### `src/console/navigation/mod.rs`

- Added imports: `SystemControlPayload`, `navigation_system_id`, `NAVIGATION_SYSTEM_ID`, `ControlTickPolicy`.
- Updated `handle_navigation_waypoint`:
  - Added `control_sources: Option<Res<ShipSystemControlSources>>` parameter.
  - Computes policy via `control_sources.as_deref().map(|cs| cs.0.policy_for(&navigation_system_id()))` with a safe default.
  - Early-returns if `!policy.accept_human_input`.
  - Refactored to a pre-loop `navigation_authorized()` guard per event.
  - Added `ClientMessage::ControlSystem { target, payload } if target.0 == NAVIGATION_SYSTEM_ID` arm with `SetNavigationWaypoint` and `ClearNavigationWaypoint` sub-arms.
  - Extracted `make_waypoint_mode()` helper to eliminate duplicated anchor-vs-free logic.
- Added 6 new tests:
  - `control_system_navigation_holder_can_set_and_clear_waypoint`
  - `control_system_unauthorized_sender_rejected`
  - `control_system_rejected_when_ai_controlled`
  - `control_system_anchored_waypoint_tracks_entity`
  - `legacy_set_navigation_waypoint_still_works`
  - `legacy_clear_navigation_waypoint_still_works`

### `src/core/messages.rs`

- Updated `ui_action_to_client_message` for `UiAction::SetNavigationWaypoint` and `UiAction::ClearNavigationWaypoint` to emit `ClientMessage::ControlSystem { target: navigation_system_id(), payload: ... }` instead of the legacy variants.
- Updated 3 existing `ui_action_tests` to assert the new `ControlSystem` output.

### `src/core/codec.rs`

- Added 3 codec round-trip tests:
  - `control_system_set_navigation_waypoint_round_trips`
  - `control_system_set_navigation_waypoint_with_anchor_round_trips`
  - `control_system_clear_navigation_waypoint_round_trips`

## Post-change state

`cargo test` passes (1965 tests, up from 1956). Navigation is now fully integrated into the coarse-system dispatch path. 9/9 consoles converted.

## Cross-references

- [PRD #517 - Consistency cleanup](./prd-517-consistency-cleanup.md)
- [Coarse-system migration concept](../concepts/coarse-system-migration.md)
