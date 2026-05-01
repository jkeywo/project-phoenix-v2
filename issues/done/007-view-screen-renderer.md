---
type: AFK
priority: feature-tracer
---

# 007 — View Screen Renderer (Bevy)

Bevy rendering systems for the host canvas.

## What to build

`src/renderer.rs`:

- `RendererPlugin` Bevy plugin
- Lobby view: text list of connected players and their console assignments; QR code placeholder (2D quad)
- In-game view: rotating 3D cube that changes colour when `RedAlertState.active` is true (red when alert, normal otherwise)
- Driven entirely by game state resources — no direct PeerJS knowledge

## Acceptance Criteria

- [ ] `cargo check` passes
- [ ] Visual output is manual-test only (no automated assertions)
