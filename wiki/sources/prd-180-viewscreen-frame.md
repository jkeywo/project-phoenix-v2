---
title: PRD #180 — Viewscreen frame: Bevy UI border, alert vignette shader, HUD readouts
type: source
tags: [prd, viewscreen, ui, red-alert, hud, shader, ui-material]
source_url: https://github.com/jkeywo/project-phoenix-v2/issues/180
sources: [src/viewscreen_border.rs, assets/viewscreen/, assets/fonts/, assets/shaders/red_alert_vignette.wgsl]
updated: 2026-05-12
---

# PRD #180 — Viewscreen Frame

## Status

**Shipped.** Delivered across three child slices:

- **#181** — asset pipeline (Trunk `copy-dir` + WGSL shader scaffolding)
- **#182** — static border frame (ten normal-state sprites, gated to `InProgress`)
- **#183** — red-alert visual (alert texture swap, `RedAlertVignetteMaterial`, removal of CSS overlay)
- **#184** — designation text + HEADING / HULL / CONDITION HUD readouts

## Problem

The server viewscreen was a full-bleed 3D render with a single CSS vignette for Red Alert. There was no visual chrome — no frame, no ship designation, no at-a-glance status. The bridge could not tell heading, hull state, or condition at a glance. The viewscreen read as a debug 3D viewport rather than a starship instrument.

## Solution

Port the supplied demo's viewscreen frame into Bevy UI as a single self-contained plugin (`ViewscreenBorderPlugin` in `src/viewscreen_border.rs`):

1. **Tiled pixel-art border** — ten `ImageNode`s (four corners, four edges, two caps). Corners and caps fixed pixel size; edges use `NodeImageMode::Tiled`.
2. **Custom `UiMaterial`** — `RedAlertVignetteMaterial` with a single `intensity` uniform, fragment shader at `assets/shaders/red_alert_vignette.wgsl`. Full-bleed `MaterialNode` spawned first so the border sprites occlude the outer ring (the "glow leaks from behind the frame" effect).
3. **Designation** — `"AEV-074 · PHOENIX"` centred on the top cap, Chakra Petch.
4. **3-column status strip** on the bottom cap: `HEADING` (000–359 from `yaw_to_compass_bearing`), `HULL` (integer percentage from `ShipHullIntegrity`), `CONDITION` (`NOMINAL` / `ALERT`). Labels Chakra Petch (neutral); values JetBrains Mono (signal-cyan / alert-red).
5. **Texture swap on alert** — each border `ImageNode` carries a `BorderSlot` marker; `swap_border_textures` rewrites the handle when `ShipState.red_alert` flips.
6. **Pulse curve** — pure `pulse_intensity(time, red_alert, prev, dt) -> f32` combines a 0.25 s on/off ease with a 1.3 s sine pulse between 0.55 and 1.0; driven per-frame by `drive_vignette_intensity`.
7. **CSS overlay removed** — the old `#red-alert-overlay` div, its CSS, and the `SimState` red-alert handler in `server.html`'s `routeOutbound` are all gone. Bevy owns the alert visual end-to-end.

## Key decisions

- **Pure-Bevy ownership of alert visual.** `ShipState.red_alert` is the single source of truth, read directly each frame.
- **Spawn-order Z layering.** Vignette → border → text. No explicit `ZIndex`.
- **Texture swap, not crossfade.** Matches the demo's pop; the pulsing vignette carries the temporal energy.
- **Fixed pixel sizes** for corners and caps. Sub-1024 px viewports are explicitly unsupported.
- **Per-frame HUD update with no change-detection.** Three `format!` calls + a few component writes per frame is negligible.
- **Pure helpers extracted and unit-tested.** `yaw_to_compass_bearing` (16 tests cover the 0/90/180/270/360 cardinal points, negatives, multi-turn, and the 359.5° rounding boundary) and `pulse_intensity` (idle, ease in/out monotonicity, steady-state band, sine phase points). The framework plumbing is manual-verify only, matching the precedent from `renderer.rs`/`beam_render.rs`.

## Schema changes

None. No wire-protocol changes. The plugin is a pure consumer of existing server-side state (`ShipState`, `ShipHullIntegrity`, `CurrentPhase`).

## Cross-references

- [Viewscreen border module](../../src/viewscreen_border.rs)
- [`UiMaterial` shader pattern](../concepts/ui-materials.md)
- [View Modes](../concepts/view-modes.md)
- Child issues: #181, #182, #183, #184
