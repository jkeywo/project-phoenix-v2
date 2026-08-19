---
title: Client Architecture
type: concept
tags: [client, javascript, iframe, console, state, vitest]
sources: [client.html, gui/mount-plan.js, gui/hero-bar.js, gui/sim-state.js, gui/console-state.js, gui/action-map.js, gui/iframe-bridge.js, tests/client/]
updated: 2026-08-19
---

## Summary

The client (`client.html`) is **pure HTML/CSS/JS — no WASM, no Bevy, no `client` Cargo feature** (the Bevy client was deleted in PRD #438 / issues #442, #463). It connects to the host over PeerJS, folds `ServerMessage`s into a plain JS state object, and renders each console as a standalone HTML iframe. All logic lives in pure, Vitest-tested modules under `gui/`; `client.html` itself is thin wiring.

## Data flow

```
PeerJS message (JSON)
  → client.html handleMessage()
  → gui/client-router.js route(msg)       # pure per-message driver: uiState mutation + side-effect plan
  → gui/sim-state.js apply(msg)           # folds ServerMessage into the single simState store
      gui/lobby-state.js                  # lobby-phase state
      gui/comms-state.js                  # comms inbox/contacts state
  → gui/dirty-consoles.js dirtyConsolesFor(msg, stationSystems)   # which consoles this message dirtied
  → gui/console-state.js buildConsoleState(name, simState)        # rebuild ONLY the dirty consoles → JSON string
  → gui/iframe-bridge.js push()           # __updateConsole(name, json) into the iframe
```

`simState` (`gui/sim-state.js`) is the single client store; there is no
separate `client.html` state mirror (removed in #819–#823). `client-router.js`
drives each inbound message and `dirty-consoles.js` narrows the rebuild to just
the consoles a given message affects, rather than rebuilding every console every
tick.

Outbound: each console iframe posts `console_action` messages; `gui/action-map.js` is the table-driven dispatcher mapping `action.action` values to `ClientMessage`s (mostly `ControlSystem { target, payload }`) via `send(type, data?)`.

## Module inventory (`gui/`)

| Module | Owns |
|---|---|
| `mount-plan.js` | **Single home** of the station-id → DOM-id naming scheme (`${id}-ui`/`${id}-iframe`, one tactical → weapons alias) and `planMounts(shipStations)` — the manifest is the server-supplied `ship_stations` |
| `hero-bar.js` | Shared complete-Station tab model over `SimSnapshot.station_hosts`: direct Station pinned first, visiting Stations in hull order, selected identity/rating/ownership, and roving keyboard focus |
| `sim-state.js` | JS port of the old Rust `ClientSimState`: `apply(msg)`, per-console radar configs, message builders |
| `lobby-state.js` | Lobby view-model (stations, players, ready states) |
| `comms-state.js` | Comms inbox/contact view-model |
| `console-state.js` | Pure view-model builders. System-composed consoles use the station's TOML-authored fine `SystemId`s and receive views keyed by those ids. |
| `action-map.js` | Table-driven `console_action` → `ClientMessage` dispatch |
| `iframe-bridge.js` | `push()` / `wireLoad()` state-push into console iframes (ADR-0001 §2) |
| `content-switcher.js` | Section visibility over the ship's mounted stations (one human = one station — the tab bar was deleted in #827) |
| `station-roster.js` | Pure fold: players + station defs → lobby roster rows + aggregates |
| `client-router.js` | Pure per-message driver: uiState mutations + named side-effect plan for the client.html glue |
| `dirty-consoles.js` | Declarative `ServerMessage` → dirty-console mapping (`dirtyConsolesFor(msg, stationSystems)`, #823); narrows each tick's rebuild to only the consoles a message affects |
| `lobby-view.js` | Lobby view model (row classes, ready-button state, status-line string-id selection) |
| `coordination-popup.js` | CoordinationPopup payload → `{ sender, title, body }` normaliser |
| `phase-toggle.js` | Lobby vs in-game section visibility (`GameOver` counts as in-game) |
| `phone-bezel.js` | Diegetic phone bezel chrome |
| `console-ui.js` | Shared iframe UI primitives (`reconcileRows`, `setBtn`, `setBar`, `setAutoState`, `setText`, keyed rebuild) |
| `console-core.js`, `device-orientation.js`, `help-panel.js`, `manual-panel.js`, `settings-panel.js` | Iframe boot glue, orientation handling, and the phone Settings menu, including current-station help and the ship manual |

Each console UI is one HTML file per ship class (`gui/battleship/helm.html`, `gui/cruiser/science.html`, …) loaded as an iframe; the URL comes from the station's TOML `console` field via `gui/console-resolver.js`, and the section/iframe DOM ids from `gui/mount-plan.js`. See [Console UI Authoring Library](./console-ui-library.md) for the authoring pattern.

`client.html` owns one shared Hero Bar above those iframes. It switches whole
mounted Station surfaces, so visiting Navigation uses the same normal
Navigation iframe as a direct holder. The bar states Direct/Visiting ownership
in text as well as styling and exposes ARIA tabs with Arrow/Home/End focus
movement; a departed visitor returns selection to the direct Station.

## Build & test

- `node scripts/build-client.mjs` — file copy → `dist/client/` (no compile step).
- `npx vitest run` — `tests/client/*.test.js` cover every pure module above.

## History

The Rust/Bevy client and its panel plugins (`src/client/`, `src/*_panel.rs`, `ShipView`, `PhoneBorderPlugin`) were removed in the #438 slice series (#439 bezel, #440 lobby, #441 tab bar, #442 Bevy cleanup) and #463 (client WASM removal). The wiki pages describing those deleted panels (CaptainPanel, HelmPanel, WeaponsPanel, RepairPanel, PowerPanel, SciencePanel, ShipView) were deleted in the 2026-07-03 docs audit — this page is their replacement.
