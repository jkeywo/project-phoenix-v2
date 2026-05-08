---
title: Console Plugin Pattern
type: concept
tags: [bevy, plugin, console, modularity]
sources: [src/client/captain_plugin.rs, src/client/helm_plugin.rs, CONTEXT.md]
updated: 2026-05-08
---

# Console Plugin Pattern

Each client-side console is a **single Bevy plugin** owning everything for that console:

- UI nodes (Bevy `Node` hierarchy)
- Marker components (e.g. `RedAlertButton`, `ThrustSlider`)
- Setup systems (build the UI on `OnEnter(GamePhase::InProgress)`)
- Event handlers (button clicks → `OutboundMessage` writers)
- Teardown systems (despawn on phase change)

## Current plugins

- `CaptainConsolePlugin` — `src/client/captain_plugin.rs`
- `HelmConsolePlugin` — `src/client/helm_plugin.rs`

A plugin is registered in `src/client/app.rs` only when the player is at that console. Multiple consoles per player is supported by the data model (`Player.consoles: Vec<Console>`) — adding two plugins side by side just works.

## Why this shape

- **Adding a console = adding a plugin.** No surgery on a god-object UI module.
- **Removal is safe.** If a console is dropped, deleting its plugin file removes everything: state, UI, handlers, markers.
- **Test isolation.** A plugin can be tested with a minimal Bevy app harness containing just it.
- **No cross-console coupling.** Helm doesn't know about Captain, and vice versa.

## Adding `WeaponsConsolePlugin` (PRD #66)

The pattern translates 1:1:

1. New file `src/client/weapons_plugin.rs`.
2. UI: radar canvas + Fire button + lock indicator.
3. Marker components for radar dots + Fire button.
4. Systems: tap-to-lock → `SetTarget`, Fire press → `FirePhaser`, react to `TargetLock` updates.
5. Register in `app.rs` when `Player.consoles` includes `Console::Weapons`.

## Related

- [Console](../entities/console.md) · [Bridge Crew Stations (planned)](../entities/bridge-crew-stations-planned.md)
- [View-Model Pattern](./view-model-pattern.md) — what plugins read from
