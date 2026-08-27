---
title: Client Architecture
type: concept
tags: [client, javascript, iframe, console, state, accessibility, keyboard, vitest]
sources: [client.html, gui/mount-plan.js, gui/hero-bar.js, gui/sim-state.js, gui/console-state.js, gui/action-map.js, gui/iframe-bridge.js, gui/accessibility-profile.js, gui/roving-tabindex.js, gui/focus-trap.js, gui/tokens.css, tests/client/]
updated: 2026-08-27
---

## Summary

The client (`client.html`) is **pure HTML/CSS/JS — no WASM or Bevy**. It connects to the host over PeerJS, folds `ServerMessage`s into a plain JS state object, and renders each console as a standalone HTML iframe. All logic lives in pure, Vitest-tested modules under `gui/`; `client.html` itself is thin wiring.

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

`simState` (`gui/sim-state.js`) is the single client store. `client-router.js`
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
| `content-switcher.js` | Section visibility over the ship's mounted stations; one human directly holds one station |
| `station-roster.js` | Pure fold: players + station defs → lobby roster rows + aggregates |
| `client-router.js` | Pure per-message driver: uiState mutations + named side-effect plan for the client.html glue |
| `dirty-consoles.js` | Declarative `ServerMessage` → dirty-console mapping (`dirtyConsolesFor(msg, stationSystems)`, #823); narrows each tick's rebuild to only the consoles a message affects |
| `lobby-view.js` | Lobby view model (row classes, ready-button state, status-line string-id selection) |
| `coordination-popup.js` | CoordinationPopup payload → `{ sender, title, body }` normaliser |
| `phase-toggle.js` | Lobby vs in-game section visibility (`GameOver` counts as in-game) |
| `phone-bezel.js` | Diegetic phone bezel chrome |
| `console-ui.js` | Shared iframe UI primitives (`reconcileRows`, `setBtn`, `setBar`, `setAutoState`, `setText`, keyed rebuild) |
| `accessibility-profile.js` | Private per-player presentation profile: OS defaults plus explicit overrides, resolved to `data-contrast` and `data-reduced-motion` on the shell and console roots |
| `roving-tabindex.js` | Shared one-Tab-stop keyboard navigation for composite controls; arrows move inside the composite while actions continue through `action-map.js` |
| `focus-trap.js` | Shared modal contract: move and trap focus, close on Escape, inert the background, then restore the invoking control |
| `tokens.css`, `components/ph-console-styles.js` | Shared high-contrast, reduced-motion and visible-focus presentation consumed on both sides of shadow roots |
| `console-core.js`, `device-orientation.js`, `help-panel.js`, `manual-panel.js`, `settings-panel.js` | Iframe boot glue, orientation handling, and the phone Settings menu, including current-station help and the ship manual |

Each console UI is one HTML file per ship class (`gui/battleship/helm.html`, `gui/cruiser/science.html`, …) loaded as an iframe; the URL comes from the station's TOML `console` field via `gui/console-resolver.js`, and the section/iframe DOM ids from `gui/mount-plan.js`. See [Console UI Authoring Library](./console-ui-library.md) for the authoring pattern.

`client.html` owns one shared Hero Bar above those iframes. It switches whole
mounted Station surfaces, so visiting Navigation or Comms uses the same normal
Navigation/Comms iframe as a direct holder. The shell does not expose placement
as Direct/Visiting text: it shows the selected Station and rating, AI-only
outcomes, a thin authoritative health bar on every tab, and separate importance
cues. In landscape it becomes a vertical strip on the left, with upright tab
buttons and sideways selected-Station metadata. It exposes ARIA tabs with
Arrow/Home/End focus movement; a departed visitor returns selection to the
direct Station.
Navigation and Comms visiting surfaces use this shared mounted-station path.
Tactical's unrelated Intel toggle uses the generic overlay pattern in
`gui/console-overlays.js`.

## Interaction and presentation accessibility

The Accessibility profile is local presentation state, not simulation state.
`gui/accessibility-profile.js` resolves the OS contrast and motion preferences
unless the player has explicitly overridden them, persists that private choice,
and stamps the resolved attributes onto the shell and every console root.
`gui/tokens.css` consumes those attributes once: the high-contrast palette and
focus-ring pair are shared tokens, while the reduced-motion layer suppresses
looping and decorative animation across document and shadow-root boundaries.

The structural floor in `tests/client/interaction-floors.test.js` scans every
console surface and rejects coarse regressions such as a surface with nothing
focusable, an unnamed glyph, or a composite with no role. It deliberately does
not claim per-control coverage: delegated child controls that a source scan can
miss are exercised by the mounted jsdom family tests and keyboard smoke cases.
Custom composites with discrete options present one named, roled Tab stop and
use `gui/roving-tabindex.js` for arrow movement inside the component. Continuous
Helm controls retain the one document-level key-relay path, and passive scopes
stay named and roled without inventing a selection interaction. Keyboard
activation dispatches the same named `action-map.js` actions as the pointer
path. The migration debt registry is empty after the #1176–#1178 family sweeps.
Modal surfaces share `gui/focus-trap.js`; they cannot each invent their own Tab,
Escape or focus-restoration behaviour.

This is name/role and keyboard hygiene, not full screen-reader narration.
Structured text alternatives for canvas scopes remain planned rather than
being inferred from the focusable canvas wrappers.

## Build & test

- `node scripts/build-client.mjs` — file copy → `dist/client/` (no compile step).
- `npx vitest run` — `tests/client/*.test.js` cover every pure module above.
- `tests/client/interaction-floors.test.js` and the keyboard-family tests keep
  focusability, naming, roles and keyboard reachability under source-level and
  jsdom regression coverage; `tests/smoke/*keyboard*.spec.js` exercise the real
  console documents without pointer events.
