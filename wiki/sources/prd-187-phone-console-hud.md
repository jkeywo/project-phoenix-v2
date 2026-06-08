---
title: PRD #187 — Phone Console HUD: Diegetic Bezel Frame
type: source
tags: [prd, client, hud, bezel, phone, shipped, superseded-by-438]
source_url: https://github.com/jkeywo/project-phoenix-v2/issues/187
status: shipped
updated: 2026-06-08
---

# PRD #187 — Phone Console HUD: Diegetic Bezel Frame

A bezel frame around every phone console panel, plus full Helm and Captain chrome (compass strip, throttle/yaw indicators, alert vignette). Lives in the `phone_border/` plugin module on the client side.

## Status

Shipped (2026-05-12). Companion to PRD #180 (the viewscreen frame on the server side).

**Being superseded** by [PRD #438 — HTML/JS Client GUI Shell](./prd-438-html-client-gui-shell.md): the bezel is migrating to HTML/CSS in `client.html`. Issue #439 (shipped 2026-06-08) added the HTML bezel as a transitional overlay; issue #442 will remove the Bevy `PhoneBorderPlugin` once the HTML shell fully replaces it.

## Problem

Phone consoles rendered as plain Bevy UI panels with no framing — they felt like web forms, not a starship console. There was no visual feedback for red alert on the phone itself, and Helm in particular had no compass / throttle / yaw indicators.

## Solution

- **`phone_border/` plugin** (`mod.rs`, `framing.rs`, `helm.rs`, `captain.rs`). Wraps every console panel with a diegetic bezel. Loaded from the client `client_app.rs`.
- **Reusable framing** for all consoles (Helm, Tactical, Repair, Power, Science, Captain).
- **Helm chrome:** compass strip showing current heading, throttle indicator, yaw rate indicator.
- **Captain chrome:** alert button styled as a physical toggle, view-mode selector.
- **Red-alert vignette** on the phone itself, mirroring the server-side viewscreen vignette from PRD #180.

## Schema additions

- New module: `src/phone_border/` (client-only, gated by the `client` Cargo feature).
- No new wire messages — purely client-side rendering of state already delivered by `SimSnapshot`, `WeaponsUpdate`, etc.

## Out of scope

- Per-console artwork variants beyond the shared bezel.
- Tablet / landscape layouts.
- Audio cues.

## Cross-references

- Companion to [PRD #180 — Viewscreen Frame](./prd-180-viewscreen-frame.md) (server side)
- [Console](../entities/console.md)
- [Roadmap Overview](../roadmap/overview.md)
