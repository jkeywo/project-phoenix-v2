---
title: Issue #490 - System registry + Red Alert system
type: source
tags: [issue, systems, registry, red-alert, ai, html-gui]
source_url: https://github.com/jkeywo/project-phoenix-v2/issues/490
status: open
updated: 2026-06-22
---

# Issue #490 - System registry + Red Alert system

## Status

Open implementation slice under PRD #487. Builds on the #488 control-message
contract and the #489 ship-config verifier.

## Problem

The new station/system model needs a registry for system kinds before runtime
systems can be addressed by `SystemId`. Red Alert is the first coarse system to
exercise that path while preserving the current captain behavior and visual
border tint.

## Solution

Add a pure Rust system-kind registry in `src/ship/system_registry.rs` and a
matching HTML GUI registry in `gui/system-registry.js`.

Red Alert now has a stable system id, `red-alert`. The captain UI sends
`ClientMessage::ControlSystem { target: SystemId("red-alert"), payload:
ToggleRedAlert }`, and the existing captain server system accepts both that new
message and the legacy `ToggleRedAlert` variant during migration.

## Key decisions

- Registering a Rust system kind requires an explicit AI controller
  registration.
- The built-in Red Alert kind is registered with a `red_alert_ai` controller
  placeholder so future rating automation has a concrete controller id.
- `CaptainConsoleState` carries `red_alert_system_id` and `red_alert_auto` so
  HTML consoles can render human-operated and AI-operated Red Alert states.
- AI-operated Red Alert renders disabled/read-only with an AUTO badge in
  `gui/captain-console.html`.
- Existing viewscreen and phone border tint still derive from the existing
  `red_alert` snapshot/state fields; the visual contract is unchanged.

## Open user stories

Future slices use the registry from ship config loading, rating selection, and
the per-instance control-source resolver. Legacy `ToggleRedAlert` can be
removed only after all senders have migrated to `ControlSystem`.

## Cross-references

- [PRD #487 - Station / Console / System architecture redesign](./prd-487-station-console-system-redesign.md)
- [Issue #488 - Station/System ADR](./issue-488-station-system-adr.md)
- [Issue #489 - Ship config loader + verifier](./issue-489-ship-config-loader.md)
- [Captain Console](../entities/captain-console.md)
