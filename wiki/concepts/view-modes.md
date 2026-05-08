---
title: View Modes
type: concept
tags: [view, camera, captain, viewscreen, radar]
sources: [src/shared/messages.rs, src/server/renderer.rs, PRD-036]
updated: 2026-05-08
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

## Captain authority

`SetView { mode }` is captain-only and InProgress-only. Server checks:

```
sender_token == captain_token()  &&  phase == InProgress
```

Ignored otherwise — silently, no error.

## Reconnect persistence

`SimSnapshot.view_mode` is included in the 10 Hz broadcast, so a captain who refreshes their phone immediately sees the correct button highlighted. Same mechanism keeps the client UI in sync after a brief disconnect.

## Related

- [Captain Console](../entities/captain-console.md) · [Radar Projection](./radar-projection.md)
- [PRD #36](../sources/prd-036-captain-view-selector.md)
