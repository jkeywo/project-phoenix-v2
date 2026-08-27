---
title: View Modes
type: concept
tags: [view, camera, captain, viewscreen, radar]
sources: [src/core/messages.rs, src/ship/viewscreen.rs, src/ship/state.rs, src/console/captain/server.rs, src/server/renderer.rs, src/server/radar.rs, src/server/viewscreen_border.rs, gui/console-state.js]
updated: 2026-08-27
---

# View Modes

`ViewMode` is the authoritative description of what the shared host viewscreen
is presenting. It is stored on the local ship's `ShipViewMode` component and
published in `SimSnapshot`, so reconnecting clients and every mounted console
derive their controls from the same state.

## Modes

- `Camera(CameraView)` selects a named `camera_*` marker from the ship's model
  rig. `camera_fore` is the default. The renderer resolves the marker's world
  position and direction; a missing marker falls back to the ship centre and
  forward direction.
- `Cinematic` uses the authored cinematic camera section to follow the local
  ship and nearby entities.
- `Radar`, `ScienceRadar`, and `SensorsRadar` select the helm or science radar
  presentation.
- `SystemChart` and `NavigationChart` select the navigation chart family.
- `Comms` presents the Comms surface.

Overlay modes keep the game camera rendering behind their radar, chart, or
Comms layer. The viewscreen border and HUD are independent presentation state
and remain around the active mode while a game is in progress.

## Requests and arbitration

Every valid request goes through `ViewscreenArbiter` on the ship:

- the latest valid command wins, with a monotonically increasing sequence and
  no source-priority ranking;
- a camera or cinematic request returns control to Captain presentation;
- repeating the exact active overlay from the same requester dismisses it and
  restores the remembered Captain camera;
- requests admitted in the same tick have an explicit deterministic order:
  `SetView` is applied before Comms `ShowOnScreen`.

The arbiter is part of `ShipViewMode`, so its remembered Captain view and
sequence survive client reconnects. A reconnect alone cannot replace a newer
view; only a newly admitted request can do that.

## Authority

Admission maps each mode to its owning system before applying it:

| Mode family | Source system |
|---|---|
| `Camera(_)`, `Cinematic` | Captain |
| `Radar` | Helm radar |
| `ScienceRadar`, `SensorsRadar` | Sensors |
| `SystemChart`, `NavigationChart` | Navigation |
| `Comms` | Comms |

Station ownership, system control source, and game phase are checked at the
command-admission boundary. Downstream arbitration therefore compares only
already-valid requests.

## Related

- [Captain Console](../entities/captain-console.md)
- [Radar Projection](./radar-projection.md)
- [System](../entities/system.md)
- [UI Materials](./ui-materials.md)
