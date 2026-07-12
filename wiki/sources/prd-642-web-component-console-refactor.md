---
title: "PRD #642 — Web Component Console Refactor"
type: source
tags: [console, gui, web-components, refactor, destroyer, cruiser, battleship]
source_url: https://github.com/jkeywo/project-phoenix-v2/issues/642
status: open
updated: 2026-07-13
---

## Summary

Replaces all monolithic per-console HTML files with two layers: reusable `<ph-*>` Web Components (`gui/components/`) and thin per-station layout HTML files (`gui/{ship}/{station}.html`). All old flat `gui/*-console.html` files are deleted once replaced.

## Status

Open. 27 child issues (#643–#669).

## Problem

Console UIs are monolithic per-file blobs. Render logic, layout, and styling are all mixed in one file per console. The "destroyer" multi-console pattern copy-pastes `renderNavigation`, `renderComms`, `renderPower`, etc. across 3–4 files. Adding a new ship type requires duplicating entire console files. `ph-damage-bar` and `ph-damage-detail` were written but never wired up — there was no natural place to plug them in.

## Solution

Two layers:
1. **`gui/components/ph-*.js`** — each component owns one UI concern, uses Shadow DOM, exposes `set state(val)` and an injected `sendAction` property.
2. **`gui/{ship}/{station}.html`** — ~30–60 lines of CSS grid layout + `el.state = s.field` wiring. No render logic.

## Key decisions

- `sendAction` injected as a property by the console HTML (not imported inside components).
- Each component handles its own portrait/landscape reflow via `@media` in its Shadow DOM `<style>`.
- Shadow-DOM internals should avoid reusing page-level `id` selectors exposed by the surrounding console HTML, and decorative SVG layers should opt out of pointer events so Playwright and touch input hit the intended interactive path.
- Console HTML assigns position/size only; component fills that slot and adapts internally.
- Paths: `gui/{ship}/{station}.html` (subdirectory per ship class).
- Old flat files deleted in the final slice (#669), not before.
- Supersedes the per-console HTML + `console-ui.js` decision from issue #509.

## Component inventory

### Existing — moved to `gui/components/` in #645
- `ph-sensor-panel`, `ph-shield-panel`, `ph-damage-bar`, `ph-damage-detail`

### New
| Issue | Component | Category |
|---|---|---|
| #646 | `ph-camera-select` | Captain |
| #647 | `ph-red-alert` | Captain |
| #648 | `ph-objective-list` | Captain |
| #649 | `ph-hull-integrity` | Repair |
| #650 | `ph-battery-bar` | Power |
| #651 | `ph-comms-hail-list` | Comms |
| #652 | `ph-comms-contact-list` | Comms |
| #653 | `ph-comms-current-message` | Comms |
| #654 | `ph-helm-joystick` | Helm |
| #655 | `ph-impulse-btn`, `ph-boost-btn` | Helm |
| #656 | `ph-phasers-controls` | Tactical |
| #657 | `ph-blasters-controls` | Tactical |
| #658 | `ph-torpedo-controls` | Tactical |
| #659 | `ph-shield-facings` | Shields |
| #660 | `ph-repair-teams` | Repair |
| #661 | `ph-power-controls` | Power |
| #644 | `ph-radar` (base) | Shared |
| #662 | `ph-helm-radar` | Helm |
| #663 | `ph-tactical-radar` | Tactical |
| #664 | `ph-sensor-radar` | Sensors |
| #665 | `ph-navigation-map` | Navigation |

## Ship console files

| Issue | Ship | Station count | Files |
|---|---|---|---|
| #666 | Destroyer | 4 | `gui/destroyer/{captain,helm,tactical,engineering}.html` |
| #667 | Cruiser | 6 | `gui/cruiser/{captain,helm,tactical,science,engineering,comms}.html` |
| #668 | Battleship | 9 | `gui/battleship/{captain,helm,tactical,repair,sensors,shields,navigation,power,comms}.html` |

## Open user stories

See PRD #642 body for full list. Key ones:
- Developer adds a new ship class by composing components, writing layout CSS only.
- Bug in torpedo tube UI fixed once; applies to all ships.
- Each component independently Vitest-testable.
- Console HTML is ~30–50 lines, readable at a glance.

## Cross-references

- [Client Architecture](../concepts/client-architecture.md)
- [Console UI Authoring Library](../concepts/console-ui-library.md) — superseded by this PRD for the component layer
- Issue #509 — previous decision against declarative layout engine (this PRD supersedes it)
- `gui/science-console.html` — the one existing console already using Web Components; the pattern to follow
