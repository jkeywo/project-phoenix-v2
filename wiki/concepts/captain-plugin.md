---
title: CaptainPlugin
type: concept
tags: [captain, red-alert, viewscreen, objectives, authority, ai]
sources: [src/console/captain/server.rs, src/command_admission/mod.rs, src/server_app/registration.rs, src/core/messages.rs, src/ship/viewscreen.rs, gui/stations/captain-actions.js, gui/action-map.js]
updated: 2026-08-31
---

# CaptainPlugin

`CaptainPlugin` is the server adapter for Captain-owned decisions and Captain blackboard publication.

## Responsibilities

- applies admitted `SetRedAlert`, weapons-hold, viewscreen `SetView`, and objective-priority commands;
- completes correlated Captain actions with reliable `Applied` feedback after the matching authoritative mutation, including idempotent assignments;
- mirrors scripted weapons-hold flags into the same authoritative state;
- runs Captain Backfill policy on the shared deterministic AI cadence and emits ordinary admitted commands;
- publishes the Captain blackboard from authoritative ship, objective, and combat state.

Captain authority is resolved at admission from the fine system named by the command. Correlated requests are admitted only for the exact Captain target/payload combinations whose consumer owns terminal feedback; invalid pairs are refused before they can enter the command stream. `SetView` is authorized from the selected view mode's source system; it is not a blanket client-side camera permission. `SetRedAlert { active }` is an idempotent assignment, not a toggle.

Views are applied in deterministic order with scripted `ShowOnScreen` requests so simultaneous inputs produce one stable result. The renderer consumes the selected authoritative view; the console does not control a camera locally.

`src/server_app/registration.rs` installs the plugin in the fixed `SimSet` chain. Tests live in `src/console/captain/server_tests.rs`.

## Related

- [Captain Console](../entities/captain-console.md)
- [Red Alert Runtime](./red-alert-intent.md)
- [View Modes](./view-modes.md)
- [Objectives](./objectives.md)
