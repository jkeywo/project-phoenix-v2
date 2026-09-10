---
title: Client Architecture
type: concept
tags: [client, javascript, iframe, console, console-family, state, accessibility, keyboard, gamepad, feedback, gm, host-channel, vitest]
sources: [gui/gamepad-presentation.js, gui/gm-workspace.js, gui/native-gm-workspace.js, src/native_host/panes/operator.rs, src/gm_objective.rs, gui/gm-objective-panel.js, gui/gm-mission-panel.js, gui/gm-direct-effect-panel.js, gui/gm-effect-scope.js, gui/gm-spawn-panel.js, gui/gm-knowledge-compare.js, gui/gm-role-presets.js, client.html, server.html, gui/mount-plan.js, gui/hero-bar.js, gui/reducer-result.js, gui/lobby-state.js, gui/sim-state.js, gui/comms-state.js, gui/console-state.js, gui/console-families.js, gui/console-payload.js, gui/dirty-consoles.js, gui/gm-local-projection.js, gui/gm-activity-feed.js, gui/entity-inspector.js, gui/semantic-action-registry.js, gui/action-feedback.js, gui/semantic-controls-remapper.js, gui/host-actions.js, gui/gm-session-actions.js, gui/gm-session-controls.js, gui/server-settings.js, gui/gamepad-input.js, gui/client-semantic-actions.js, gui/composite-action-routing.js, gui/stations/captain-actions.js, gui/stations/helm-actions.js, gui/stations/tactical-actions.js, gui/stations/comms-actions.js, gui/stations/sensors-actions.js, gui/stations/navigation-actions.js, gui/stations/navigation-action-control.js, gui/stations/engineering-actions.js, gui/stations/engineering-action-control.js, gui/components/ph-navigation-map.js, gui/components/ph-civilian-traffic.js, gui/operator-profile.js, gui/action-map.js, gui/console-core.js, gui/console-latency.js, gui/iframe-bridge.js, gui/client-router.js, gui/coordination-popup.js, gui/accessibility-profile.js, gui/roving-tabindex.js, gui/focus-trap.js, gui/tokens.css, src/core/messages.rs, src/command_admission/mod.rs, src/gm_action.rs, src/gm_projection.rs, src/gm_activity.rs, src/server/bridge.rs, src/console/captain/server.rs, src/console/navigation/server.rs, src/console/repair/dispatch.rs, src/console/repair/external_server.rs, src/civilian/server.rs, src/science/server.rs, src/ship/helm_admission.rs, src/ship/sensors.rs, src/ship/shields.rs, src/ship/power.rs, src/tractor/server.rs, src/umbilical/server.rs, src/ship/system_registry.rs, src/dock/server.rs, src/entities/spawner.rs, src/lobby/server.rs, tests/client/]
updated: 2026-09-07
---

## Summary

The client (`client.html`) is **pure HTML/CSS/JS — no WASM or Bevy**. It connects to the host over the Phoenix transport (a typed join code resolved through the rendezvous service, then two WebRTC DataChannels), folds `ServerMessage`s into a plain JS state object, and renders each console as a standalone HTML iframe. All logic lives in pure, Vitest-tested modules under `gui/`; `client.html` itself is thin wiring.

## Data flow

The active crew lobby reserves space for the connection diagnostics beneath it.
`client.html` observes `#conn-diag-row` with `ResizeObserver` and uses its height
in the lobby's bottom inset, keeping the copy control clear of READY even when
the readout wraps on a phone. The diagnostics stay outside the connection-gated
console container so they remain usable during a disconnect.

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
iframes through `__updateSemanticActionBindings` on load and remap. The Captain
family has three stable adapters: `captain.red-alert`, `captain.view`, and
`captain.objective-priority`. Fire restraint is an Engineering Power order.
Sensors and Science add five:
`sensors.target-selection`, `sensors.scan`, `sensors.viewscreen`,
`sensors.cancel-impulse`, and `science.shield-focus`. Every visible control in
those families and its default/remapped binding invokes the same adapter.
Boolean actions derive an explicit assignment from the latest authoritative
view; target, scan, camera, objective and shield controls pass only identities
already present in the projected view, while their generic bindings cycle only
through choices in that same view. Each console iframe then posts its
`console_action`, and
`gui/action-map.js` remains the table-driven dispatcher mapping
`action.action` values to `ClientMessage`s (mostly
`ControlSystem { target, payload }`) via `send(type, data?)`.

Captain and Sensors/Science actions use authoritative action feedback. Registry activation
mints one bounded opaque correlation and records the same-device epoch timestamp
at `Pressed`, then moves presentation to `Pending` only after its adapter handles
the activation. The action map sends a `ControlSystemCorrelated` envelope; the
host accepts only the exact correlated target/payload pairs, keeps
the correlation out of the payload, command log, mesh and simulation state, and
replies reliably to the originating session with `ActionFeedback::Applied` only
after the matching owning consumer runs, or `Refused` when admission or the
owner rejects it. Science Target, scan availability and protected scan details
remain authoritative projections; Pending never mutates them optimistically.
`client.html` owns the bounded correlation-to-iframe timeout router. It
settles latency and forwards the terminal result only for that exact
correlation; ordinary blackboard pushes cannot acknowledge it. Every iframe has
one generic accessible final-status presenter, while specialised controls may
also expose busy state. Pending never changes Red Alert, camera, or objective
state: only the normal authoritative blackboard does.

Navigation uses the same registry and correlation lifecycle for chart display,
free placement, selected-contact anchoring, clear, contact selection and
civilian order. Pointer controls, native buttons, keyboard and gamepad converge
on those six adapters; placement also supplies a visible arrow-key cursor and
Enter/Space commit path for operators who cannot drag or point. Composite
actions keep the parent runtime's real `captain` or `comms` context because the
parent routes a gamepad event to the active Station iframe, then require a
focused, open or visible map inside that iframe. Select, Start and the two stick
buttons keep their map defaults separate from Comms D-pad controls. Contact
selection completes locally. Waypoint and civilian commands stay Pending until
the owning Navigation or civilian consumer returns Applied or Refused, and
published blackboards—not the input adapter—remain the only authoritative
visual state.

Power allocation, internal team dispatch, named-System repair priority, Tractor,
Umbilical and external-repair controls use the same contract. Pointer controls
pass their exact owner SystemId plus group, team slot, dispatch target or named
System identity as
activation detail; keyboard/gamepad activations choose the first operable row
from the latest authoritative family projection. Seven shared actions cover the
dedicated battleship Power/Repair consoles and the cruiser/destroyer Engineering
composites. Courier Captain layers Captain, Comms, Power and Repair registries in
one document. That iframe exposes its currently open semantic subcontext to the
parent gamepad reader, while `composite-action-routing.js` routes the activation
back to the active Captain iframe instead of looking for a nonexistent family
iframe. A missing/loading iframe capability is distinct from an empty catalogue:
the parent keeps its neutral gate closed and re-arms it when the live catalogue
changes, so a held control cannot fire as an overlay finishes loading. Neither
path mutates gameplay state locally. Correlated Power/Repair
mutations settle only at their owning server validator; Tractor/Umbilical starts
settle after their same-tick live hold/flow verdict.

The host page uses one page-scoped semantic registry and feedback lifecycle
without pretending its chrome is a console. The registry contains the
`host.qr-code`, `gm.pause`, and `gm.resume` definitions, each with the same two
keyboard-or-standard-gamepad binding slots and shared remapper/conflict path.
QR and GM contexts overlap, so the registry detects their conflicts together.
Its document-level keyboard dispatcher and explicitly selected
`mapping === "standard"` gamepad runtime are the only remapped input paths;
`gm-session-controls` installs no competing private listener.

The visible QR button, native Enter/Space activation, and remapped input all
invoke the one adapter in `gui/host-actions.js`; that adapter calls only
server.html's existing `__hostToggleQrCode` seam and completes local feedback
synchronously through Pressed → Pending → Applied. `gui/server-settings.js`
keeps the final visual and aria-live feedback outside the modal so a closed
Settings panel cannot hide it, but paints persistent QR visibility and
`aria-pressed` only from `__hostIsQrVisible`, since lobby and phone routes can
also change that state. These host-page bindings remain in page memory and add
no Rust message or simulation authority.

The GM pause surface uses the same registry but not the QR action's synchronous
local settlement. `gui/gm-session-actions.js` creates a durable
operator-scoped action id and submits an absolute `SetSessionPaused` request;
`gui/gm-session-controls.js` shows Pending until the `gm_session` Host Channel
projects the authoritative Applied, No-op, or Refused result. Every projection
is an absolute replacement of `paused` and its ordered terminal results: it
settles the exact `(operator_id, correlation)` pending row and clears obsolete
local terminal rows. Persistent Pause and `aria-pressed` always come from that
projection, never from the click, so equal peers and retries cannot invent
local state while lockstep decides the canonical order.

Each binding slot is a union: `KeyboardEvent.code` plus all four modifiers, or
a portable control from the browser's standard gamepad mapping. Gamepad
bindings name logical controls (face, shoulder and trigger buttons, D-pad
directions, Select, Start, stick buttons, an axis direction and threshold, or an
undirected continuous standard axis), never
`Gamepad.id`, vendor data or a connection. Continuous action definitions own
their output range, neutral and dispatch cadence; client-local tuning owns
deadzone and inversion separately from binding identity. Two directed bindings
on the same axis and direction collide even when their trigger thresholds
differ, because both can fire from one deflection. The registry exposes
serialisable binding and tuning profiles; `operator-profile.js` persists both
for the parent browser client.
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
currently active console iframe. The v1 profile retains the selected device
descriptor (`id` and mapping), not authority over an enumeration slot. An
unambiguous available match can reconnect automatically; identical matches need
explicit selection. Native host leases identify devices assigned to other
consoles and filter their input before publication. No selected, connected,
leased standard controller means no gamepad input.
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
by greatest deflection, then binding-slot order. Discrete hold actions likewise
emit one press and one release; console changes, disconnects, deselection and
visibility interruption release accepted holds before re-arming the neutral
gate, so boost cannot remain latched after its physical input disappears.
Keyboard dispatch remains the independent iframe path throughout, with
left/right modifier keys treated as the same portable binding family while the
actual pressed side is retained for release.

The complete Helm semantic family is `helm.thrust`, `helm.steering`,
`helm.lateral-thrust`, `helm.impulse`, `helm.boost`, `helm.viewscreen` and
`helm.dock`. The parent samples the selected standard pad's authored axes and
buttons; each iframe control and its retained keyboard/pointer behavior activate
the same identities. `gui/action-map.js` maps those identities onto the existing
fine-system commands without changing the 100 ms continuous cadence or
normalising an analogue value a second time. Cruiser, Destroyer and the Courier's
composite Tactical console mount the lateral action's visible control, while only
Destroyer mounts contextual Dock;
the capability checks keep those variants unavailable on other hulls. Impulse,
boost, viewscreen and dock use correlated authoritative feedback, completed by
their owning consumers after application or refusal. No Helm component polls
the Gamepad API or bypasses the semantic registry with a direct command.

`gui/operator-profile.js` is the one current-version private browser profile.
It whitelists Accessibility effects and assistance, exactly two bindings for
each known semantic action, preferred controller descriptor, continuous tuning,
the default-on hide-touch-controls choice, feedback preferences and GM
confirmation choices. It contains no player/session credentials, Station
authority or save data and has no simulation message builder. Native panes use
the same schema through the host-local `panes::operator` store; browser profiles
remain private to their browser. Legacy slot-only values require fresh device
selection instead of acquiring a possibly different controller. The parent snapshots it after each supported
setting change and exports/imports the same JSON from Controls. Import first
normalises bounded private preferences and validates every supplied binding
against the registry's reserved-chord and overlapping-context conflict rules.
Actions absent from an older profile then receive conflict-free authored
defaults in registry order; a default that would displace an imported remap is
left empty and reported through the accessible normalized-import status. Only
the resulting valid candidate is stored and atomically applied. The old
`phoenix-accessibility-v1` value migrates only when no current profile exists;
a corrupt current record yields authored defaults rather than silently
resurrecting the old value.

`gui/help-panel.js` derives controller help from the semantic registry
and current remaps. Settings and the claimed-station lobby guide share that
content and refresh it for the active console context. `gui/gamepad-presentation.js`
evaluates Helm joystick and lateral thrust visibility independently against connected hardware and
usable bindings; disconnect or missing bindings restores the corresponding
control. Ordinary clients use Change station through release confirmation;
host-assigned native screens retain their console tabs without release controls.

## Module inventory (`gui/`)

| Module | Owns |
|---|---|
| `mount-plan.js` | **Single home** of the station-id → DOM-id naming scheme (`${id}-ui`/`${id}-iframe`, one tactical → weapons alias) and `planMounts(shipStations)` — the manifest is the server-supplied `ship_stations` |
| `hero-bar.js` | Shared complete-Station tab model over `SimSnapshot.station_hosts`: direct Station pinned first, visiting Stations in hull order, selected identity/rating/ownership, and roving keyboard focus |
| `sim-state.js` | JS port of the old Rust `ClientSimState`: `apply(msg)`, per-console radar configs, message builders, typed blackboard discriminants, both read-only Console Family replicas from `Welcome`, and the latest explicitly Station- or Ship-addressed Coordination popup with producer-authored presentation retained beside its typed payload |
| `reducer-result.js` | Fresh reducer results: mergeable semantic change sets plus an ordered, repeatable lifecycle/presentation effect sequence |
| `lobby-state.js` | Lobby view-model (stations, players, equal public GM presence, ready states) and lobby-domain reducer results; GM roster updates are full replacements and never become player or Station rows |
| `comms-state.js` | Comms inbox/contact view-model and Comms-domain reducer results |
| `console-state.js` | Pure view-model builders. One family registry contains all builders, including Command, Tractor and Umbilical; flat and composed consoles carry actual owned `SystemId`s and projected families, while typed blackboard discriminants select semantic data independently of id spelling. |
| `console-payload.js` | Metadata-driven flat/keyed normalization plus `familyView`: mirrors flat views only under actual projected ids and selects composite views by Console Family, with no inverse id census. |
| `action-map.js` | Table-driven `console_action` → `ClientMessage` dispatch |
| `action-feedback.js` | Pure bounded Pressed → Pending → Applied/Refused/TimedOut presentation lifecycle, exact parent-to-originating-iframe router, and the shared live-status/semantic-cue/vibration transition |
| `gm-local-projection.js`, `entity-inspector.js`, `components/ph-navigation-map.js` inspect mode | Strict local Host Channel GM map DTO; semantic point/Region partition with authored geometry and radar appearance; stable UUID selection across absolute refresh/removal; point-first, UUID-stable touch/keyboard selection; identity/status/target inspection with live hull totals and authored System/Station ownership for directed effects; and shared pan/zoom presentation without a peer state stream or any change to Navigation commands |
| `gm-mission-panel.js` | Authored event Fire, Pause/Resume and Skip-next controls from the absolute local `gm_mission` projection, with attributed results from the shared GM action journal |
| `gm-objective-panel.js` | Authored Objective activation and active-record completion/failure from `gm_mission`; immutable recipient scope, record-specific preview, exact attributed feedback, and absolute replacement on reconnect |
| `gm-direct-effect-panel.js`, `gm-effect-scope.js` | Damage/healing on the selected Entity, Station or System using projected hull totals and ownership for scope, overflow and lethality preview |
| `gm-spawn-panel.js`, `components/ph-navigation-map.js` placement mode | The local `gm_spawn` authored palette and closed variants, with pointer/keyboard position and heading resolved into one typed spawn action |
| `gm-knowledge-compare.js` | Selected-ship Truth/Crew Knowledge/Difference view using `gm_entity`, the `gm_station` fold and ordinary Sensors/Comms builders; contacts can differ through range/tag filters, both Objective columns use that ship's recipient-filtered list, and Comms still uses shared inputs |
| `gm-role-presets.js` | Live-switchable scenario-authored personal presets with All fallback and private identity persistence; current map/inspector/activity and Pause/Resume visibility filtering changes no action authority |
| `gm-activity-feed.js` | Strict raw local Host Channel DTO for the bounded Damage, Destruction, Objective, Trigger, Red Alert, Connection, and GM Action history; one repeat-preserving `{tick, category, ships, links, detail}` reduction whose fixed facts retain their pre-advance tick and whose PostUpdate operational sources retain the producing tick; strict category + semantic-ship AND filtering with global rows only under All ships, render-site localisation, and availability-checked links into the GM map's stable UUID selection |
| `semantic-action-registry.js`, `semantic-controls-remapper.js`, `gamepad-input.js`, `client-semantic-actions.js`, `composite-action-routing.js`, `stations/{captain,helm,tactical,comms,sensors,navigation,engineering}-actions.js`, `host-actions.js`, `gm-session-actions.js`, `gm-session-controls.js`, `server-settings.js`, `operator-profile.js` | Non-authoritative context/input identity, exactly two keyboard-or-standard-gamepad slots, continuous metadata and axis tuning, the shared keyboard capture/conflict/reset presenter used by phone and host/GM Settings, explicit selected-standard-gamepad ownership with neutral-gated discrete edges and continuous values, composite-family subcontext routing back to the owning Station iframe, atomic validation plus private browser persistence/export/import, reserved-chord policy, overlap-only conflict replacement and reset operations, plus the real Captain, Helm, Tactical, Comms, Sensors/Science, Navigation, Engineering/Power/Repair, host QR, and attributed GM Pause/Resume adapters above `action-map.js` |
| `iframe-bridge.js` | `push()` / `wireLoad()` state-push into console iframes (ADR-0001 §2) |
| `content-switcher.js` | Section visibility over the ship's mounted stations; one human directly holds one station |
| `station-roster.js` | Pure fold: players + station defs → lobby roster rows + aggregates |
| `client-router.js` | Pure reducer-result driver: mirrors accepted LobbyState, applies local render/bezel guards, and emits the ordered named side-effect plan without receiving a ServerMessage; Coordination effects carry the authoritative address through to presentation |
| `dirty-consoles.js` | Merged semantic domains, changed Systems and changed blackboards → Console Families → actual owning Stations. It has no `ServerMessage` input or message-variant census; unknown domains/ids and pre-`Welcome` metadata route nowhere rather than guessing. |
| `lobby-view.js` | Lobby view model (row classes, ready-button state, and the `{ id, params }` string-id pair for every string the lobby says — it decides, the renderer resolves) |
| `client-lobby-render.js` | The client lobby's DOM writes over that model (issue #1369), on the host's `gui/host-lobby-render.js` conventions: document first, `t` injected, every write guarded on its element, and the six controls that close over page-local mutable state (`releaseArmed`, `pendingMidGameClaim`, `lobbyConsole`) arriving as an injected handlers object |
| `coordination-popup.js` | Generic producer-owned Coordination presentation resolver plus the phone's two-content-line and Viewscreen's one-line DOM builders; it knows no semantic payload variants and owns each surface's single bracket pair |
| `phase-toggle.js` | Lobby vs in-game section visibility (`GameOver` counts as in-game) |
| `game-over-view.js`, `game-over.css` | The ending, decided once and drawn once: the pure frame (scenario, headline by declared outcome, closing prose, normalised post-mission report rows — a reported ending wears no verdict) and the one token-only sheet (chamfered, scanlined viewscreen frame sized by its content, so a six-row report never scrolls) shared by `client.html`'s phone overlay, `server.html`'s Viewscreen overlay and the native `viewscreen-hud.html` |
| `phone-bezel.js` | Diegetic phone bezel chrome |
| `console-ui.js` | Shared iframe UI primitives (`reconcileRows`, `setBtn`, `setBar`, `setAutoState`, `setText`, keyed rebuild) |
| `accessibility-profile.js` | Private per-player presentation profile: OS defaults plus explicit overrides, resolved to `data-contrast` and `data-reduced-motion` on the shell and console roots |
| `roving-tabindex.js` | Shared one-Tab-stop keyboard navigation for composite controls; arrows move inside the composite while actions continue through `action-map.js` |
| `focus-trap.js` | Shared modal contract: move and trap focus, close on Escape, inert the background, then restore the invoking control |
| `tokens.css`, `components/ph-console-styles.js` | Shared high-contrast, reduced-motion and visible-focus presentation consumed on both sides of shadow roots |
| `console-core.js`, `help-panel.js`, `manual-panel.js`, `settings-panel.js`, `server-settings.js` | Iframe boot glue, the phone Settings menu (including current-station help and the ship manual), and the host Settings shell/readback presenter. Orientation is not among them: portrait vs landscape is a `@media (orientation: …)` query and no script computes it (issue #1370) |

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
- `tests/client/client-lobby-render.test.js` drives the lobby renderer in jsdom
  against `client.html`'s own `#lobby-ui` subtree — lifted with `DOMParser`, so
  a renamed element fails there rather than rendering nothing with a clean log
  — including the deliberately half-mounted document and the empty one, which
  is what the guard on every write is for.
