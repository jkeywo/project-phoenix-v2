---
title: Server HTML Lobby UI
type: concept
tags: [lobby, server, html, ui, bridge, responsive]
sources: [server.html, gui/host-lobby-view.js, src/server/viewscreen_border.rs, src/console_bridge.rs, src/server/bridge.rs]
updated: 2026-08-27
---

# Server HTML Lobby UI

The lobby UI on the **server** (viewscreen) page is rendered entirely as HTML/CSS/JS in `server.html`. The global `window.__updateLobby(json)` callback applies each `LobbyStatePayload` snapshot pushed by the Bevy server.

The host page is the only consumer of this push channel: each viewer of `server.html` sees the same lobby state because the data originates from a single authoritative Bevy world.

## Push path (Rust → DOM)

1. **Bevy producer.** `push_lobby_state` in `src/server/viewscreen_border.rs` builds a `LobbyStatePayload`, encodes it via `core::codec::encode_lobby_state`, and writes a `LobbyStateChanged` event. Its station roster contains claimable (`auxiliary = false`) seats only; auxiliary mounted Stations never become cards or affect counts.
2. **WASM bridge drain.** `flush_host_channels` in `src/server/bridge.rs` drains those events on every tick and invokes the single registered host-channel callback with `("lobby", json)` (#818).
3. **JS callback registration.** `set_host_channel_callback(window.__hostChannel)` is invoked once in `server.html` when WASM is ready; the dispatcher's handlers table routes `"lobby"` payloads to `window.__updateLobby`.
4. **View model and DOM mutation.** `gui/host-lobby-view.js` derives the render model; `window.__updateLobby` in `server.html` applies it to the lobby DOM.

This is a **one-way state-push channel** that runs in parallel to the regular [Message Flow](./message-flow.md) (which targets specific peers via PeerJS). Lobby state is broadcast-equivalent: only the host's own DOM consumes it.

## Payload shape

`LobbyStatePayload` (`src/core/messages.rs`):

| Field | Type | Notes |
|---|---|---|
| `phase` | `GamePhase` | Only `Lobby` makes the panel visible. |
| `scenario_title` | `String` | Header big text. |
| `scenario_body` | `String` | Header subtitle. |
| `crew_count` | `u32` | Currently filled claimable Stations. |
| `max_players` | `u32` | Number of claimable seats on the active ship. |
| `stations` | `Vec<StationPayload>` | One entry per claimable, non-auxiliary station. |
| `spectators` | `Vec<String>` | Names holding the explicit Spectator role. |
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

The lobby cards, rail, QR area, and responsive layout are DOM owned by
`server.html` and `gui/host-lobby-view.js`; Bevy publishes data but does not
build a lobby UI tree.

## Tests

Smoke coverage in `tests/smoke/lobby-responsive.spec.js`:

- Portrait viewport (480×900): rail below the claimable-station grid, no horizontal body scroll, and spectator pills rendered.
- Landscape viewport (1280×720): rail right of the claimable-station grid with multiple card columns.

Protocol-level lobby coverage stays in `tests/smoke/lobby.spec.js` (station
selection, readiness, assignment broadcasts, and invalid claims). Those tests
do not assert responsive DOM layout.

## Sources

- `server.html`
- `src/server/viewscreen_border.rs` — `push_lobby_state`
- `src/server/bridge.rs` — `set_host_channel_callback`, `flush_host_channels` (named Host Channel table, #818)
- `src/console_bridge.rs` — `LobbyStateChanged` event
- `src/core/messages.rs` — `LobbyStatePayload` / `StationPayload`
- [Message Flow](./message-flow.md), [Codec Seam](./codec-seam.md)
