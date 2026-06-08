---
title: Issue #439 — HTML Phone Bezel Frame (incl. red alert swap)
type: source
tags: [issue, client, hud, bezel, phone, html, shipped]
source_url: https://github.com/jkeywo/project-phoenix-v2/issues/439
status: shipped
updated: 2026-06-08
---

# Issue #439 — HTML Phone Bezel Frame (incl. red alert swap)

First vertical slice of the [HTML/JS Client GUI Shell PRD #438](./prd-438-html-client-gui-shell.md) (parent issue). Replaces the Rust/Bevy `PhoneBorderPlugin` bezel with HTML/CSS/JS in `client.html`.

## Status

Shipped 2026-06-08. The Rust `PhoneBorderPlugin` (PRD #187) is still loaded — it will be removed in [issue #442](./issue-442-bevy-cleanup.md). Both implementations co-exist during the transitional window.

## Problem

`client.html`'s bezel was rendered by the Bevy WASM canvas (`src/client/phone_border/framing.rs`). Every visual change required a Rust recompile and WASM deploy. The bezel needed to migrate to the DOM so iteration could happen without a build cycle.

## Solution

- **`gui/phone-bezel.js`** — ES module exporting the pure `bezelSrc(slot, alert) → URL` function and a `BEZEL_SLOTS` constant. Also attaches both to `window` for use by the inline non-module script in `client.html`.
- **`client.html`** —
  - CSS adds `#phone-bezel` (z-index 15, fixed, full-viewport, `pointer-events: none`, hidden by default) with 4 corner `<img>` (120 × 72) and 4 edge `<div>` (22 px) children.
  - `.alert-on` class on the bezel root drives CSS `background-image` swaps for the 4 edges; JS re-sets the `src` attribute on each corner `<img>` via `bezelSrc()` for the alert variants.
  - `applyBezelAlert(alert)` helper handles both. Logs once if `window.bezelSrc` is missing.
  - `SimState` handler reads `snap.red_alert` into `state.redAlert`; on change, calls `applyBezelAlert`.
  - `render()` toggles bezel visibility based on `state.phase === 'InProgress'` so the lobby remains borderless.
  - `#status` z-index raised from 5 → 25 so connection status isn't obscured by the bottom-left corner.
  - `#weapons-ui.active` gets `padding: var(--bezel-inset)` so the tactical iframe sits inside the bezel safe zone.
- **`vitest.config.js`** — `include` extended to pick up `tests/client/**/*.test.js`.
- **`tests/client/phone-bezel.test.js`** — 19 Vitest unit tests covering all 8 slot × 2 alert combinations plus truthy/falsy edge cases for the `alert` parameter.

## Schema additions

- New module: `gui/phone-bezel.js` (ES module, shared with Vitest tests).
- New CSS variable: `--bezel-inset: 22px` on `:root`.
- New DOM root: `#phone-bezel` with 4 `.bezel-corner` `<img>` + 4 `.bezel-edge` `<div>` children.
- No new wire messages — `red_alert` is already on `SimSnapshot` (`src/core/messages.rs:489`).

## Key decisions

- **Corners as `<img>`, edges as `<div>`** — corners need precise non-repeating placement, edges need to tile. JS swaps corner `src`; CSS swaps edge `background-image`. Both swaps fire off the same `.alert-on` class flip on the root.
- **Pure function via ES module** — the renderer is testable in isolation by Vitest without DOM or image loading. The browser consumer reads it through `window.bezelSrc` for compatibility with the existing non-module inline script.
- **Bezel hidden during lobby** — the lobby is borderless (per PRD #438). Visibility flips in `render()` on phase change.

## Out of scope

- Tab bar (issue #441).
- Lobby HTML integration (issue #440; partially landed in commit `7d5f0d0`).
- Bevy `PhoneBorderPlugin` removal (issue #442).
- Per-corner-art transparency QA — assumed adequate.

## Cross-references

- Parent: [PRD #438 — HTML/JS Client GUI Shell](./prd-438-html-client-gui-shell.md)
- Supersedes (incrementally): [PRD #187 — Phone Console HUD: Diegetic Bezel Frame](./prd-187-phone-console-hud.md)
- Test file: `tests/client/phone-bezel.test.js`
- Module: `gui/phone-bezel.js`
- Markup + CSS + handler: `client.html` (see lines around `#phone-bezel`, `applyBezelAlert`)
