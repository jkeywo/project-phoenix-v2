---
title: Captain Console
type: entity
tags: [console, captain, red-alert, view-mode, authority]
sources: [src/client/captain_plugin.rs, src/server/simulation.rs, PRD-001, PRD-036]
updated: 2026-05-08
---

# Captain Console

The "Captain's Chair." The authority seat: only the captain can start the game, toggle Red Alert, and change the viewscreen camera.

## Controls

| Control | Effect | Source |
|---|---|---|
| Engage | `StartGame` → phase transitions Lobby → InProgress | PRD #1 |
| Red Alert toggle | Flips `ShipState.red_alert`; renders red border on viewscreen and all consoles | PRD #1 |
| View selector (Fore/Aft/Port/Starboard) | `SetView { mode: Camera(direction) }` → repositions hull camera | PRD #36 |

## Server-side guards

All captain messages are guarded in the simulation/lobby handlers by checking `sender_token == SessionManager::captain_token()`. Non-captains sending these messages get silently dropped — no error, no broadcast.

## Layout

3×3 CSS grid above the Red Alert button (PRD #36):

```
   ▲          fore (top-centre)
◄ View ►     port · label · starboard
   ▼          aft (bottom-centre)
```

The active view direction is highlighted. State is restored from `SimSnapshot.view_mode` on reconnect, so refreshing the captain's phone preserves the current camera.

## What the captain does *not* control

- Ship movement → that's [Helm Console](./helm-console.md).
- Weapons / Engineering / Science → see [Bridge Crew Stations (planned)](./bridge-crew-stations-planned.md).

## Related

- [Console](./console.md) · [View Modes](../concepts/view-modes.md)
- [PRD #36 — Captain View Selector](../sources/prd-036-captain-view-selector.md)
