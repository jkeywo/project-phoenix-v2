---
title: Client Architecture
type: concept
tags: [client, javascript, iframe, console, state, vitest]
sources: [client.html, gui/console-registry.js, gui/sim-state.js, gui/console-state.js, gui/action-map.js, gui/iframe-bridge.js, tests/client/]
updated: 2026-07-03
---

## Summary

The client (`client.html`) is **pure HTML/CSS/JS — no WASM, no Bevy, no `client` Cargo feature** (the Bevy client was deleted in PRD #438 / issues #442, #463). It connects to the host over PeerJS, folds `ServerMessage`s into a plain JS state object, and renders each console as a standalone HTML iframe. All logic lives in pure, Vitest-tested modules under `gui/`; `client.html` itself is thin wiring.

## Data flow

```
PeerJS message (JSON)
  → client.html handleMessage()
  → gui/sim-state.js apply(msg)          # folds ServerMessage into sim-state object
      gui/lobby-state.js                  # lobby-phase state
      gui/comms-state.js                  # comms inbox/contacts state
  → gui/console-state.js build*(state)    # pure per-console view-model → JSON string
  → gui/iframe-bridge.js push()           # __updateConsole(name, json) into the iframe
```

Outbound: each console iframe posts `console_action` messages; `gui/action-map.js` is the table-driven dispatcher mapping `action.action` values to `ClientMessage`s (mostly `ControlSystem { target, payload }`) via `send(type, data?)`.

## Module inventory (`gui/`)

| Module | Owns |
|---|---|
| `console-registry.js` | **Single source of truth** for HTML-panel consoles: lowercase station id → section id + iframe id |
| `sim-state.js` | JS port of the old Rust `ClientSimState`: `apply(msg)`, per-console radar configs, message builders |
| `lobby-state.js` | Lobby view-model (stations, players, ready states) |
| `comms-state.js` | Comms inbox/contact view-model |
| `console-state.js` | Pure `build*(state)` view-model builders, one per console iframe |
| `action-map.js` | Table-driven `console_action` → `ClientMessage` dispatch |
| `iframe-bridge.js` | `push()` / `wireLoad()` state-push into console iframes (ADR-0001 §2) |
| `content-switcher.js` + `tab-bar.js` | Console tab bar + section visibility |
| `active-console.js` | Pure next-active-console selection logic |
| `phase-toggle.js` | Lobby vs in-game section visibility (`GameOver` counts as in-game) |
| `phone-bezel.js` | Diegetic phone bezel chrome |
| `radar-math.js` | Client-side radar blip projection |
| `console-ui.js` | Shared iframe UI primitives (`reconcileRows`, `setBtn`, `setBar`, `setAutoState`, `setText`, keyed rebuild) |
| `console-core.js`, `device-orientation.js`, `help-panel.js` | Iframe boot glue, orientation handling, help overlay |

Each console UI is one HTML file: `gui/captain-console.html`, `gui/helm-console.html`, `gui/comms-console.html`, `gui/navigation-console.html`, etc., loaded as an iframe and listed in `console-registry.js`. See [Console UI Authoring Library](./console-ui-library.md) for the authoring pattern.

## Build & test

- `node scripts/build-client.mjs` — file copy → `dist/client/` (no compile step).
- `npx vitest run` — `tests/client/*.test.js` cover every pure module above.

## History

The Rust/Bevy client and its panel plugins (`src/client/`, `src/*_panel.rs`, `ShipView`, `PhoneBorderPlugin`) were removed in the #438 slice series (#439 bezel, #440 lobby, #441 tab bar, #442 Bevy cleanup) and #463 (client WASM removal). The wiki pages describing those deleted panels (CaptainPanel, HelmPanel, WeaponsPanel, RepairPanel, PowerPanel, SciencePanel, ShipView) were deleted in the 2026-07-03 docs audit — this page is their replacement.
