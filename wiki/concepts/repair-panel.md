---
title: Repair Console — Client Panel
---

# Repair Console — Client Panel

The Repair console client UI is `gui/repair-console.html`, a pure-HTML/JS panel. There is no client-side WASM component for the Repair console (the `client` Cargo feature was removed in #463).

## Overview

The panel receives `RepairBlackboard` data from the server and renders:

- A list of damageable consoles with their current HP/tier status (`console_hull` field).
- Team slots showing Idle / Travelling / Repairing / Cooldown state with a progress bar.
- Dispatch buttons: for each team, a set of repair-target buttons (station names + Core).

Clicking a dispatch button sends a `dispatch_repair_team` action via `gui/action-map.js:136`:

```js
// action-map.js
dispatchRepairTeam({ team_idx, target })
// target is { Station: { id: "helm" } } or "Core"
```

This is wrapped into a `ControlSystem { target: repair, payload: DispatchRepairTeam { team_idx, target } }` wire message before transmission. The legacy `ClientMessage::DispatchRepairTeam` path is also retained.

## Data flow (client side)

```
Server broadcasts RepairBlackboard (SystemBlackboard::Repair)
  → client.html JS: handleMessage
  → repair-console.html: onRepairBlackboard(bb)
    → render team slots (bb.teams)
    → render console hull bars (bb.console_hull)
    → populate dispatch target buttons (bb.damageable_consoles)
```

`damageable_consoles` drives which targets appear as buttons. Once Core is declared in `[[hull.console_hull]]` in `player_ship.toml`, `Console::Core` appears in `damageable_consoles` and a "Core" dispatch button is rendered.

## RepairBlackboard fields used by the panel

| Field | Purpose |
|---|---|
| `teams: Vec<TeamSlot>` | Team slot states — rendered as rows with progress bars |
| `console_hull: Vec<ConsoleHullStatus>` | Per-console HP and tier — rendered as damage bars |
| `travel_duration_secs: f32` | Used to scale progress bar animation duration |
| `damageable_consoles: Vec<Console>` | Determines which dispatch target buttons to show |

## No shape-matching minigame

The old `RepairBreakdownLabel` / `RepairShapeButton(Shape)` UI existed in a previous WASM-based client architecture. Both the shape-button press handler and the breakdown queue display were removed when the shape-matching minigame was retired (PRD #272-era). The current UI has no shape buttons.

The previous wiki entry described `RepairBreakdownLabel`, `RepairShapeButton(Shape)`, `RepairIconState`, and `BreakdownQueueResource` — these no longer exist.

## Visibility

The repair panel is shown when:

1. The game phase is `InProgress`.
2. The local player holds the `Repair` station (or `Core` as a repair target does not change panel ownership — the Repair console is still owned by the `repair` station holder).

## Sources

- `gui/repair-console.html`
- `gui/action-map.js` (dispatch_repair_team action, line 136)
- `client.html` (message routing, handleMessage)
- `src/core/messages.rs` (RepairBlackboard, RepairTarget)
- `src/console/repair/server.rs` (server-side publish)
- Issue [#508](https://github.com/jkeywo/project-phoenix-v2/issues/508)
- [Repair Console — Server Plugin](./repair-plugin.md)
