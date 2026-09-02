---
title: Server HTML Lobby UI
type: concept
tags: [lobby, server, html, ui, bridge, responsive, accessibility, reduced-motion, native, gm]
sources: [server.html, client.html, gui/host-lobby-view.js, gui/host-lobby-render.js, gui/host-lobby.css, gui/host-qr.js, gui/host-qr.css, gui/join-url.js, gui/lobby-view.js, gui/lobby-state.js, gui/fleet-session.js, src/server/viewscreen_border.rs, src/console_bridge.rs, src/server/bridge.rs, src/native_host/host_lobby/mod.rs, src/native_host/host_lobby/join.rs, src/lockstep/mod.rs, src/core/messages.rs, src/gm_roster.rs, src/lobby/start_policy.rs]
updated: 2026-09-01
---

# Server HTML Lobby UI

The lobby UI on the **server** (viewscreen) page is rendered as HTML/CSS/JS. The global `window.__updateLobby(json)` callback applies each `LobbyStatePayload` snapshot pushed by the Bevy server.

Since issue #1325 the render itself is **not** in `server.html`: the decisions are `gui/host-lobby-view.js` (#1229), the DOM writes are `gui/host-lobby-render.js`, and the panel's rules are `gui/host-lobby.css`. Issue #1329 did the same for the join panel that floats over the lobby — `gui/host-qr.js` and `gui/host-qr.css`. `server.html` links all five and keeps only what is its own: the audio graph, the fleet freeze and mesh pump, the asset-loading and game-over overlays, and what a click on the QR does in a desktop browser (it opens a client in its own window).

There is no `qrVisible` flag any more. The join panel's visibility **is** `#overlay`, read and written only through `gui/host-qr.js`: two documents in two engines cannot share a module-level boolean, and four handlers keeping one in step is how it came to disagree with the screen.

The split exists because there are now **two** surfaces rendering this lobby from the same payload: the host page, and the native host's viewscreen surface (see [Native Host](./native-host.md#the-host-lobby-on-the-viewscreen-issue-1325)), whose document is built from this page's own `#lobby-panel` markup. Both call the same `renderHostLobby`. A second implementation of these element ids would drift the first time either was touched, so there is not one.

Each viewer of `server.html` sees the same lobby state because the data originates from a single authoritative Bevy world.

## Push path (Rust → DOM)

1. **Bevy producer.** `push_lobby_state` in `src/server/viewscreen_border.rs` builds a `LobbyStatePayload`, encodes it via `core::codec::encode_lobby_state`, and writes a `LobbyStateChanged` event. Its station roster contains claimable (`auxiliary = false`) seats only; auxiliary mounted Stations never become cards or affect counts.
2. **WASM bridge drain.** `flush_host_channels` in `src/server/bridge.rs` drains those events on every tick and invokes the single registered host-channel callback with `("lobby", json)` (#818).
3. **JS callback registration.** `set_host_channel_callback(window.__hostChannel)` is invoked once in `server.html` when WASM is ready; the dispatcher's handlers table routes `"lobby"` payloads to `window.__updateLobby`.
4. **View model and DOM mutation.** `gui/host-lobby-view.js` derives the render model; `window.__updateLobby` in `server.html` performs the page's own side effects and hands the model to `gui/host-lobby-render.js`, which writes the lobby DOM.

The native host reaches the same last step by a different route: it reads the same `LobbyStateChanged` message directly (there is no WASM bridge in that process) and pushes the same bytes over `native_host::host_lobby`'s bridge into an embedded view, whose module island calls the same `localiseHostPayload` → `hostLobbyViewModel` → `renderHostLobby` chain.

This is a **one-way state-push channel** that runs in parallel to the regular [Message Flow](./message-flow.md) (which targets specific peers over their own DataChannels). Lobby state is broadcast-equivalent: only the host's own DOM consumes it.

## Payload shape

`LobbyStatePayload` (`src/core/messages.rs`):

| Field | Type | Notes |
|---|---|---|
| `phase` | `GamePhase` | Only `Lobby` makes the panel visible. |
| `scenario_title` | `String` | Header big text. |
| `scenario_body` | `String` | Header subtitle. |
| `crew_count` | `u32` | Currently filled claimable Stations. |
| `max_players` | `u32` | Number of claimable seats on the active ship. |
| `readiness` | `ReadinessTally` | Connected non-Spectator crew and its ready subset; Station choice is irrelevant. |
| `presentation_ready` | `bool` | The renderer's required assets have reached a terminal preload state; rendererless/headless peers report ready. |
| `stations` | `Vec<StationPayload>` | One entry per claimable, non-auxiliary station. |
| `spectators` | `Vec<String>` | Names holding the explicit Spectator role. |
| `gms` | `Vec<GmOperator>` | Equal GM rows (`id`, `name`, `connected`, `ready`), separate from every crew and Station count. |
| `all_stations_filled` | `bool` | Flips ready badge to `READY TO LAUNCH`. |

`StationPayload`: `name`, `short_code`, `rank`, `holder_name?`, `is_mine`, `preset_names`.

The grid is sized directly from the claimable roster. It creates no padding or reserved placeholder cards.

## DOM contract

```
#lobby-panel.lobby-panel
├── .lobby-bg                                   /* solid black backdrop          */
└── .lobby-panel-wrap                           /* flex column, scaled padding   */
    ├── .lobby-header                           /* wraps on narrow viewports     */
    │   ├── .lobby-title-block
    │   │   ├── #lobby-title
    │   │   └── #lobby-subtitle
    │   └── .lobby-status-block
    │       ├── .lobby-crew-info                /* CREW + count + dots + tag     */
    │       └── #lobby-ready-badge
    └── .lobby-body                             /* row in landscape, col compact */
        ├── .lobby-grid-column
        │   ├── #station-grid.lobby-grid        /* auto-fit minmax(220, 360)     */
        │   │   └── .station-card[.claimed]     /* one per claimable Station     */
        └── .lobby-rail                          /* aside; rail right or below   */
            ├── #lobby-gm-group[role=region]
            │   └── #lobby-gm-list              /* equal GM presence            */
            ├── .lobby-rail-label
            ├── #lobby-spectator-list.lobby-rail-section
            │   └── .spectator-pill[.waiting]   /* one per connected/waiting     */
            ├── .lobby-rail-spacer
            └── #lobby-status-hint
```

## Responsive layout

Grid uses `grid-template-columns: repeat(auto-fit, minmax(220px, 360px))`, so column count adapts from 1 to 6 with viewport width. Card font sizes use `clamp(...)` so typography scales between TVs and phones.

A single media query
```css
@media (orientation: portrait), (max-width: 720px) { … }
```
toggles **compact mode**, in which:

- `.lobby-body` switches from `flex-direction: row` to `flex-direction: column` so `.lobby-rail` flows below `#station-grid`.
- `.lobby-rail`'s left border becomes a top border; padding and direction flip; `#lobby-spectator-list` becomes a flex-wrap of pills.
- Claimable Station cards reflow without adding placeholders.

Wide mode uses the same claimable cards in an adaptive multi-column grid.

A scroll fallback (`overflow-y: auto` on `#station-grid`) handles rosters that do not fit after reflow.

## Rust ownership

`ViewscreenBorderPlugin` owns the host-overlay producers and presentation effects:

- `push_lobby_state` emits `LobbyStateChanged` when the authoritative lobby
  projection changes;
- `recompute_hud_state` and `push_hud_state` emit the in-game HUD projection,
  with a final game-over push on phase entry;
- `RedAlertVignetteMaterial`, shield flash, hull shake, camera shake, and the
  reduced-motion preference remain renderer-side presentation state.

The lobby cards, rail and responsive layout are DOM owned by
`gui/host-lobby-render.js` + `gui/host-lobby.css` over `gui/host-lobby-view.js`;
the join panel is `gui/host-qr.js` + `gui/host-qr.css` over the same view
model's `qrOverlayAction` (issue #1329). The Game Master group is `server.html`'s
own projection over that view model, rendered as a sibling region rather than
folded into the Station list; the crew page receives the same public GM
projection in `Welcome` and `GmRosterChanged`, and `gui/lobby-view.js` renders
it likewise. Bevy publishes data but does not build a lobby UI tree — on either
surface.

## The join panel

One panel, one draw site, two surfaces. `gui/host-qr.js` owns the `#qr-panel`
nest and the visibility of the `#overlay` that carries it:

| when | the join panel |
|---|---|
| the Lobby phase | shown |
| Loading, GameOver | hidden |
| InProgress | left exactly as it was |

That last row is why a QR opened mid-mission for a late arrival stays open: the
view model returns `null` for the action, and nothing else touches the panel.
Toggles reach the same `toggleQr` from three places — the host page's settings
cog (`__hostToggleQrCode`), a phone's `ClientMessage::ToggleQrCode`, and the
native surface's own control.

The encoder is **vendored** (`gui/vendor/qrcode.js`, `qrcode@1.5.1`'s browser
build, MIT) and served by whichever process serves the page. It used to be a
`cdn.jsdelivr.net` `<script>`, which made the join code depend on the room
having internet — untenable for a native host on a bridge machine.

## Game Master presence and fleet start

An admitted GM also receives a labelled Ready/Unready control and a separate
Force Start control. Both readiness and refusal states use text, not colour
alone. Force Start reports its attributed applied/no-op/refused result and
cannot replace a failed content-validation message with a generic launch
success. The browser publishes validation only after the exact selected hull's
Station map, the complete pre-`wasm_init` boot/compatibility path, and the local
renderer's terminal presentation-preload state succeed. The mesh owner includes
that final presentation gate before emitting the immutable grant; managed peers
then enter `InProgress` directly instead of re-reading frame-paced preload state
after grant delivery.

The browser-to-Rust roster seam is asynchronous. `wasm_join_fleet` returns a
generation, and the page does not publish the frozen topology, release a grant,
or flush its bounded pre-adoption frame FIFO until
`wasm_fleet_join_status` accepts that exact generation. Managed/validation
projections retry through a bounded queue, coalescing only consecutive absolute
samples. Leave uses the same acknowledgement: the settings control disables
while pending, but the fleet transport and roster remain live until Rust accepts
the fresh-Lobby teardown. A late refusal leaves the live fleet on screen and
reports the machine reason instead of silently turning it into a solo host.

## Viewscreen reduced motion

The viewscreen's motion preference is presentation state and never enters the
authoritative digest. In the browser host, `server.html` observes
`prefers-reduced-motion` and calls `wasm_set_reduced_motion`; the bridge latch
is drained into `ViewscreenMotion` by `sync_reduced_motion`. The native path
seeds the same resource from `PHOENIX_REDUCED_MOTION` at startup. Consumers
therefore share one decision: reduced motion zeroes hull-damage camera/page
shake and caps the shield flash, while the host CSS disables the Red Alert
vignette pulse. The default keeps the normal effects, and
`ViewscreenMotion.shake_intensity` is the future comfort-slider seam.

## Tests

Smoke coverage in `tests/smoke/lobby-responsive.spec.js`:

- Portrait viewport (480×900): rail below the claimable-station grid, no horizontal body scroll, and spectator pills rendered.
- Landscape viewport (1280×720): rail right of the claimable-station grid with multiple card columns.

Protocol-level lobby coverage stays in `tests/smoke/lobby.spec.js` (station
selection, readiness, assignment broadcasts, and invalid claims). Those tests
do not assert responsive DOM layout.

`tests/client/host-lobby-render.test.js` drives the extracted renderer in jsdom
against `server.html`'s own `#lobby-panel` subtree — lifted with `DOMParser`, so
a renamed element fails there rather than rendering nothing with a clean log —
including the document with fewer elements in it, which is the native lobby's.
`tests/client/host-lobby-view.test.js` covers the pure model beneath it.

`tests/smoke/viewscreen-reduced-motion.render.spec.js` exercises the live WASM
motion latch and asserts zero page translation plus a disabled vignette pulse;
the pure Rust tests beside `ViewscreenMotion` cover shake scaling and the
reduced-motion flash cap.

## Sources

- `server.html`
- `src/server/viewscreen_border.rs` — `push_lobby_state`
- `src/server/bridge.rs` — `set_host_channel_callback`, `flush_host_channels` (named Host Channel table, #818)
- `src/console_bridge.rs` — `LobbyStateChanged` event
- `src/core/messages.rs` — `LobbyStatePayload` / `StationPayload`
- `gui/host-lobby-render.js`, `gui/host-lobby.css` — the shared renderer and stylesheet (#1325)
- `src/native_host/host_lobby/` — the native host's surface over the same pair
- [Native Host](./native-host.md), [Message Flow](./message-flow.md), [Codec Seam](./codec-seam.md)
