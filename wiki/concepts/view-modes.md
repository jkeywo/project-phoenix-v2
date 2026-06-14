---
title: View Modes
type: concept
tags: [view, camera, captain, viewscreen, radar]
sources: [src/core/messages.rs, src/server/renderer.rs, PRD-036]
updated: 2026-06-14
---

# View Modes

What the **viewscreen** (server display) is showing. Captain-controlled.

## Wire shape

```rust
pub enum ViewMode {
    Camera(ViewDirection),   // Fore | Aft | Port | Starboard
    Radar,                   // top-down asteroid map
}
```

Default: `Camera(Fore)`.

## Camera modes (PRD #36)

For each direction, the viewscreen camera is parented to the ship and offset by **6.0 units** (the capsule radius) along the ship-local axis, looking outward parallel to the ground:

| Direction | Offset | Looking |
|---|---|---|
| `Fore` | +forward | along ship heading |
| `Aft` | −forward | opposite heading |
| `Port` | −right | left |
| `Starboard` | +right | right |

A top-centre text label on the viewscreen (`FORE` / `AFT` / `PORT` / `STARBOARD`) shows the current direction so the room knows which way they're looking.

## Radar mode

The viewscreen renders the asteroid field as a top-down map using [`radar_dots()`](./radar-projection.md). Useful as an alternative tactical view.

## Captain panel synchronisation

The Captain console state push reports `view_direction` only for camera modes. When another console takes the viewscreen (`Radar`, `SensorsRadar`, `SystemChart`, `NavigationChart`, or `Comms`), `CaptainConsoleState.view_direction` is an empty string; `gui/captain-console.html` treats that as no selected direction so the Fore/Port/Starboard/Aft buttons lose their active highlight.

## Captain authority

`SetView { mode }` is captain-only and InProgress-only. Server checks:

```
sender_token == captain_token()  &&  phase == InProgress
```

Ignored otherwise — silently, no error.

## Reconnect persistence

`SimSnapshot.view_mode` is included in the 10 Hz broadcast, so a captain who refreshes their phone immediately sees the correct button highlighted. Same mechanism keeps the client UI in sync after a brief disconnect.

## Viewscreen chrome (PRD #180)

The 3D camera output is now framed by the viewscreen border (`ViewscreenBorderPlugin`, `src/viewscreen_border.rs`): a ten-sprite tiled pixel-art frame with a designation label (`AEV-074 · PHOENIX`) centred on the top cap and a three-column `HEADING / HULL / CONDITION` HUD strip on the bottom cap. The frame is gated to `GameState::InProgress` and is independent of the current `ViewMode` — both `Camera(_)` and `Radar` modes render inside the same chrome. On Red Alert the border sprites swap to their alert variants, the designation and HUD values turn alert-red (labels stay neutral), and a [`UiMaterial`](./ui-materials.md) vignette pulses behind the frame.

## Related

- [Captain Console](../entities/captain-console.md) · [Radar Projection](./radar-projection.md)
- [PRD #36](../sources/prd-036-captain-view-selector.md) · [PRD #180](../sources/prd-180-viewscreen-frame.md)
- [`UiMaterial` shader pattern](./ui-materials.md)
