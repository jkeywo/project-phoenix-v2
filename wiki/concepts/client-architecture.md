---
title: Client Architecture
type: concept
tags: [client, javascript, iframe, console, console-family, state, accessibility, keyboard, gamepad, feedback, vitest]
sources: [client.html, server.html, gui/mount-plan.js, gui/hero-bar.js, gui/reducer-result.js, gui/lobby-state.js, gui/sim-state.js, gui/comms-state.js, gui/console-state.js, gui/console-families.js, gui/console-payload.js, gui/dirty-consoles.js, gui/semantic-action-registry.js, gui/action-feedback.js, gui/semantic-controls-remapper.js, gui/host-actions.js, gui/server-settings.js, gui/gamepad-input.js, gui/client-semantic-actions.js, gui/stations/captain-actions.js, gui/stations/helm-actions.js, gui/action-map.js, gui/console-core.js, gui/console-latency.js, gui/iframe-bridge.js, gui/client-router.js, gui/coordination-popup.js, gui/accessibility-profile.js, gui/roving-tabindex.js, gui/focus-trap.js, gui/tokens.css, src/core/messages.rs, src/command_admission/mod.rs, src/console/captain/server.rs, src/ship/system_registry.rs, src/dock/server.rs, src/entities/spawner.rs, src/lobby/server.rs, tests/client/]
updated: 2026-08-31
---

## Summary

The client (`client.html`) is **pure HTML/CSS/JS — no WASM or Bevy**. It connects to the host over the Phoenix transport (a typed five-letter join code resolved through the rendezvous service, then two WebRTC DataChannels), folds `ServerMessage`s into a plain JS state object, and renders each console as a standalone HTML iframe. All logic lives in pure, Vitest-tested modules under `gui/`; `client.html` itself is thin wiring.

## Data flow

```
DataChannel message (JSON, reliable or lossy)
  → gui/rendezvous-transport.js decodes + localiseTree()
  → client.html handleMessage()
  → gui/lobby-state.js apply(msg)         # folds lobby state and reports semantic changes
  → gui/sim-state.js apply(msg)           # folds simulation state and reports semantic changes
  → gui/comms-state.js apply(msg)         # folds Comms state and reports semantic changes
  → gui/reducer-result.js mergeReducerResults(...results)  # sets + ordered effects
  → gui/dirty-consoles.js dirtyConsolesFor(changes, stationSystems,
                                               systemConsoleFamilies,
                                               blackboardConsoleFamilies)
                                                       # which consoles changed
  → gui/console-state.js buildConsoleState(name, simState)        # rebuild ONLY the dirty consoles → JSON string
  → gui/iframe-bridge.js push()           # __updateConsole(name, json) into the iframe
  → gui/client-router.js routeReducerResult(changes)       # lobby mirror + render guards
  → client.html applySideEffect(effect)   # executes ordered presentation effects
```

`simState` (`gui/sim-state.js`) is the single simulation store. The three state
reducers interpret each inbound message, `dirty-consoles.js` narrows iframe
publication to the affected consoles, and `client-router.js` consumes only the
merged reducer result.

`systemConsoleFamilies` is the host-projected presentation classification for
actual System instance ids. `blackboardConsoleFamilies` separately classifies
reserved and aggregate channels. Neither duplicates Station ownership, and a
channel does not become a commandable System.

The simulation, lobby and Comms reducers return fresh semantic results through
`gui/reducer-result.js`: mergeable sets of changed domains, Systems and
blackboards, plus an ordered effect array. The sets coalesce duplicate keys;
the effect array deliberately does not, so two equal damage or Coordination
events still produce two pieces of feedback. Only valid Blackboard entries that
were actually folded become changed keys, and human-seeking host changes report
the broader `station-hosting` domain. Dirty routing consumes only the sets plus
authoritative topology metadata after state reduction. Once those iframe
snapshots have been published, `client-router.js` consumes the effect array,
mirrors the already-reduced lobby store and applies local render/bezel guards.
Neither router receives or inspects the original `ServerMessage`.

Outbound: context-scoped operator input first resolves through
`gui/semantic-action-registry.js`, whose stable identities, two local binding
slots and presentation metadata sit above transport and carry no Station or
session authority. Each console iframe owns an isolated registry instance;
`client.html` owns the current in-memory binding choices and copies them into
iframes through `__updateSemanticActionBindings` on load and remap. The two real
Captain adapters are `captain.red-alert` and `captain.weapons-hold`: each
visible control and its default/remapped keyboard bindings invoke the same
adapter, derive an explicit boolean from the latest authoritative Captain view,
and emit the existing `set_red_alert` or `set_weapons_hold` action. Each console
iframe then posts its `console_action`, and `gui/action-map.js` remains the
table-driven dispatcher mapping `action.action` values to `ClientMessage`s
(mostly `ControlSystem { target, payload }`) via `send(type, data?)`.

Red Alert is the first action to opt into authoritative action feedback. Its
registry activation mints one bounded opaque correlation and records the
same-device epoch timestamp at `Pressed`, then moves presentation to `Pending`
only after its adapter handles the activation. The action map sends a
`ControlSystemCorrelated` envelope; the host keeps the correlation out of the
payload, command log, mesh and simulation state, and replies reliably to the
originating session with `ActionFeedback::Applied` only after the due Captain
consumer runs, or `Refused` when admission rejects it. `client.html` owns the
bounded correlation-to-iframe timeout router. It settles latency and forwards
the terminal result only for that exact correlation; ordinary blackboard pushes
cannot acknowledge it. The iframe's one lifecycle transition supplies the
visible/live status, semantic cue and optional vibration intent. Pending never
changes Red Alert's active styling: only the normal authoritative Captain
blackboard does. Weapons Hold remains on the uncorrelated legacy envelope.

The host page uses the same registry and lifecycle without pretending its local
chrome is a console or a network command. `host.qr-code` is scoped to the
`host` context with the same two binding slots (KeyQ plus an empty slot by
default). The visible Gameplay button, its native Enter/Space activation and
the document-level remapped keyboard route all invoke the one adapter in
`gui/host-actions.js`; that adapter calls only server.html's existing
`__hostToggleQrCode` seam and completes local feedback synchronously through
Pressed → Pending → Applied. `gui/server-settings.js` keeps the final visual and
aria-live feedback outside the modal so a closed Settings panel cannot hide it,
but paints persistent QR visibility and `aria-pressed` only from
`__hostIsQrVisible`, since lobby and phone routes can also change that state.
The binding profile remains host-page memory: it adds no persistence, gamepad,
Rust message or simulation authority.

Each binding slot is a union: `KeyboardEvent.code` plus all four modifiers, or
a portable control from the browser's standard gamepad mapping. Gamepad
bindings name logical controls (face-bottom, D-pad directions, a left-stick
axis direction and threshold, or an undirected continuous standard axis), never
`Gamepad.id`, vendor data or a connection. Continuous action definitions own
their output range, neutral and dispatch cadence; client-local tuning owns
deadzone and inversion separately from binding identity. The registry exposes
serialisable binding and tuning profiles in memory, but persists neither.
Registry conflicts exist only where action context arrays intersect, including
two slots on the same action. Settings asks the parent registry to propose a
change; conflicting proposals remain presentation-only until the player chooses
Replace, which atomically clears every overlapping assignment, while Cancel and
reserved browser/OS chords leave the profile untouched. Authored two-slot
defaults are retained separately from current bindings. Per-action reset
restores both slots and clears collisions with those defaults; Reset All
restores the complete conflict-free authored profile. These contexts and local
binding choices add no Station or command authority.

`gui/gamepad-input.js` is the only Gamepad API reader. The parent client samples
one snapshot per animation frame and activates semantic actions only in the
currently active console iframe. A player must explicitly choose a connected
`mapping === "standard"` browser slot; no selected slot means no gamepad input.
Ownership also carries an ephemeral connection generation, so disconnect clears
edge state and raises a persistent client-level accessible warning outside
Settings, and a new device reusing the same index cannot inherit control. The
Settings mirror updates its selector/status nodes in place so a polling status
change cannot detach a focused binding-capture control. Selection, reconnection,
console-context changes, remapping, tuning and binding capture emit one
immediate neutral for an active continuous action and then require every bound
continuous axis to fall within its configured deadzone before input can resume.
Nonzero values dispatch immediately and then at the action's authored cadence;
release or disconnect emits neutral exactly once. Multiple bound axes resolve
by greatest deflection, then binding-slot order. Keyboard dispatch remains the
independent iframe path throughout.

`helm.steering` is the first continuous semantic action. The parent samples the
selected standard pad's left-stick X axis, while the Helm iframe adapter emits
only `set_helm_steering`; `gui/action-map.js` maps that to the existing
`SetSteering` command on `helm-steering`. The visual joystick no longer reads
the Gamepad API or chooses a device, and its pointer/WASD `set_helm` path stays
unchanged.

## Module inventory (`gui/`)

| Module | Owns |
|---|---|
| `mount-plan.js` | **Single home** of the station-id → DOM-id naming scheme (`${id}-ui`/`${id}-iframe`, one tactical → weapons alias) and `planMounts(shipStations)` — the manifest is the server-supplied `ship_stations` |
| `hero-bar.js` | Shared complete-Station tab model over `SimSnapshot.station_hosts`: direct Station pinned first, visiting Stations in hull order, selected identity/rating/ownership, and roving keyboard focus |
| `sim-state.js` | JS port of the old Rust `ClientSimState`: `apply(msg)`, per-console radar configs, message builders, typed blackboard discriminants, both read-only Console Family replicas from `Welcome`, and the latest explicitly Station- or Ship-addressed Coordination popup with producer-authored presentation retained beside its typed payload |
| `reducer-result.js` | Fresh reducer results: mergeable semantic change sets plus an ordered, repeatable lifecycle/presentation effect sequence |
| `lobby-state.js` | Lobby view-model (stations, players, ready states) and lobby-domain reducer results |
| `comms-state.js` | Comms inbox/contact view-model and Comms-domain reducer results |
| `console-state.js` | Pure view-model builders. One family registry contains all builders, including Command, Tractor and Umbilical; flat and composed consoles carry actual owned `SystemId`s and projected families, while typed blackboard discriminants select semantic data independently of id spelling. |
| `console-payload.js` | Metadata-driven flat/keyed normalization plus `familyView`: mirrors flat views only under actual projected ids and selects composite views by Console Family, with no inverse id census. |
| `action-map.js` | Table-driven `console_action` → `ClientMessage` dispatch |
| `action-feedback.js` | Pure bounded Pressed → Pending → Applied/Refused/TimedOut presentation lifecycle, exact parent-to-originating-iframe router, and the shared live-status/semantic-cue/vibration transition |
| `semantic-action-registry.js`, `semantic-controls-remapper.js`, `gamepad-input.js`, `client-semantic-actions.js`, `stations/{captain,helm}-actions.js`, `host-actions.js` | Non-authoritative context/input identity, exactly two keyboard-or-standard-gamepad slots, continuous metadata and in-memory axis tuning, the shared keyboard capture/conflict/reset presenter used by phone and host Settings, explicit one-connection phone gamepad ownership with neutral-gated discrete edges and continuous values, reserved-chord policy, overlap-only conflict replacement and reset operations, plus the real Captain, Helm and host QR adapters above `action-map.js` |
| `iframe-bridge.js` | `push()` / `wireLoad()` state-push into console iframes (ADR-0001 §2) |
| `content-switcher.js` | Section visibility over the ship's mounted stations; one human directly holds one station |
| `station-roster.js` | Pure fold: players + station defs → lobby roster rows + aggregates |
| `client-router.js` | Pure reducer-result driver: mirrors accepted LobbyState, applies local render/bezel guards, and emits the ordered named side-effect plan without receiving a ServerMessage; Coordination effects carry the authoritative address through to presentation |
| `dirty-consoles.js` | Merged semantic domains, changed Systems and changed blackboards → Console Families → actual owning Stations. It has no `ServerMessage` input or message-variant census; unknown domains/ids and pre-`Welcome` metadata route nowhere rather than guessing. |
| `lobby-view.js` | Lobby view model (row classes, ready-button state, status-line string-id selection) |
| `coordination-popup.js` | Generic producer-owned Coordination presentation resolver plus the phone's two-content-line and Viewscreen's one-line DOM builders; it knows no semantic payload variants and owns each surface's single bracket pair |
| `phase-toggle.js` | Lobby vs in-game section visibility (`GameOver` counts as in-game) |
| `phone-bezel.js` | Diegetic phone bezel chrome |
| `console-ui.js` | Shared iframe UI primitives (`reconcileRows`, `setBtn`, `setBar`, `setAutoState`, `setText`, keyed rebuild) |
| `accessibility-profile.js` | Private per-player presentation profile: OS defaults plus explicit overrides, resolved to `data-contrast` and `data-reduced-motion` on the shell and console roots |
| `roving-tabindex.js` | Shared one-Tab-stop keyboard navigation for composite controls; arrows move inside the composite while actions continue through `action-map.js` |
| `focus-trap.js` | Shared modal contract: move and trap focus, close on Escape, inert the background, then restore the invoking control |
| `tokens.css`, `components/ph-console-styles.js` | Shared high-contrast, reduced-motion and visible-focus presentation consumed on both sides of shadow roots |
| `console-core.js`, `device-orientation.js`, `help-panel.js`, `manual-panel.js`, `settings-panel.js`, `server-settings.js` | Iframe boot glue, orientation handling, the phone Settings menu (including current-station help and the ship manual), and the host Settings shell/readback presenter |

Each console UI is one HTML file per ship class (`gui/battleship/helm.html`, `gui/cruiser/science.html`, …) loaded as an iframe; the URL comes from the station's TOML `console` field via `gui/console-resolver.js`, and the section/iframe DOM ids from `gui/mount-plan.js`. See [Console UI Authoring Library](./console-ui-library.md) for the authoring pattern.

## Console Family routing

Every shipped `SystemKindDescriptor` requires a Console Family, and `Welcome`
projects that metadata onto every actual authored System id in
`ShipClientConfig.system_console_families`. Reserved and aggregate channels
(`helm`, `tactical`, `power`, `shields`, `dossiers`, and `scan`) use
`blackboard_console_families`; this explicit second namespace lets dirty routing
identify the affected family without pretending a channel is commandable or
damageable.

The client uses those maps and the Station's actual `station_systems` topology
for builder selection, dirty routing, payload keys, flat normalization and
visiting-System composition. Semantic blackboard selection uses the wire's
`SystemBlackboard.kind`, retained beside each unwrapped payload. Multiple
same-kind blackboards use authored Station order when available and lexical
order for cross-read-only fallback, keeping selection deterministic.

There is no exact/prefix family matcher, client-maintained family-to-System
list, iframe `family` hint, or Station-name boot switch. Before `Welcome` there
is no authoritative topology with which to choose a builder, so the payload is
empty. Command, Tractor and Umbilical use the same builder registry as every
other family. Dock keeps its resolved actual id for its control adapter; the
broader action-map adapter remains a separate concern.

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
