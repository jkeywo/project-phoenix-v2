---
title: Server HTML Lobby UI
type: concept
tags: [lobby, server, html, ui, bridge, responsive]
sources: [server.html, src/server/viewscreen_border.rs, src/console_bridge.rs, src/server/bridge.rs]
updated: 2026-06-08
---

# Server HTML Lobby UI

The lobby UI on the **server** (viewscreen) page is rendered entirely as HTML/CSS/JS in `server.html`. It is mutated by the global `window.__updateLobby(json)` callback whenever the Bevy server pushes a new `LobbyStatePayload` snapshot. The Bevy `LobbyScreenRoot` tree that previously rendered this UI was deleted in 2026-06-08 as part of issue [#436](https://github.com/jkeywo/project-phoenix-v2/issues/436).

The host page is the only consumer of this push channel: each viewer of `server.html` sees the same lobby state because the data originates from a single authoritative Bevy world.

## Push path (Rust → DOM)

1. **Bevy producer.** `push_lobby_state` in `src/server/viewscreen_border.rs:330` builds a `LobbyStatePayload`, encodes it via `core::codec::encode_lobby_state`, and writes a `LobbyStateChanged` event whenever the lobby state has actually changed (it dedupes by hashing the payload).
2. **WASM bridge drain.** `flush_lobby_state` in `src/server/bridge.rs` drains those events on every tick and invokes the registered JS callback with the JSON body.
3. **JS callback registration.** `set_lobby_state_callback(window.__updateLobby)` is invoked in `server.html` once WASM is ready (search for `set_lobby_state_callback` in the boot block).
4. **DOM mutation.** `window.__updateLobby` (`server.html:756` onward) parses the JSON and rewrites `#lobby-title`, `#lobby-subtitle`, `#lobby-crew-count`, `#lobby-crew-dots`, `#lobby-spectator-tag`, `#lobby-ready-badge`, `#station-grid`, `#reserved-aggregate`, `#lobby-spectator-list`, and `#lobby-status-hint`.

This is a **one-way state-push channel** that runs in parallel to the regular [Message Flow](./message-flow.md) (which targets specific peers via PeerJS). Lobby state is broadcast-equivalent: only the host's own DOM consumes it.

## Payload shape

`LobbyStatePayload` (`src/core/messages.rs`):

| Field | Type | Notes |
|---|---|---|
| `phase` | `GamePhase` | Only `Lobby` makes the panel visible. |
| `scenario_title` | `String` | Header big text. |
| `scenario_body` | `String` | Header subtitle. |
| `crew_count` | `usize` | Currently filled stations. |
| `max_players` | `usize` | Maximum supported by the active ship/station preset. |
| `stations` | `Vec<StationPayload>` | One entry per active station (1–6). |
| `spectators` | `Vec<String>` | Names waiting in queue. |
| `all_stations_filled` | `bool` | Flips ready badge to `READY TO LAUNCH`. |

`StationPayload`: `name`, `short_code`, `rank`, `holder_name?`, `consoles: Vec<String>`, `preset_names: Vec<String>`.

The JS hard-codes `MAX_SLOTS = 6`. When `stations.length < MAX_SLOTS`, the JS still emits one `.station-card.empty.per-slot` per missing slot (so that wide layouts retain a 6-cell visual) and additionally activates the aggregate chip (so compact layouts can collapse to a single line).

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
        │   │   ├── .station-card[.claimed]     /* 0–6 active station cards      */
        │   │   └── .station-card.empty.per-slot /* 0–N reserved placeholders    */
        │   └── #reserved-aggregate.reserved-aggregate[.active]
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
- `.lobby-grid .station-card.empty` is hidden (`display: none`).
- `.reserved-aggregate.active` is shown (`display: block`).
- The aggregate chip text is `↻ N station slot(s) reserved (max 6)`.

In wide mode the aggregate chip is `display: none` and the per-slot empties are visible — preserving the 3×2 (or up to 6×1 ultrawide) look.

A scroll fallback (`overflow-y: auto` on `#station-grid`) handles extreme cases where 6 cards × min-height won't fit even after reflow.

## Rust — what stays in `viewscreen_border.rs`

After the #436 sweep:

- `ViewscreenBorderPlugin` and its plugin registration scaffold
- `RedAlertVignetteMaterial` (a `UiMaterial` for the red-alert tint)
- `push_lobby_state` (the producer this page is about)
- `push_hud_state` (analogous channel for `__updateHud`)
- `process_shield_flash`, `process_hull_shake`, `apply_camera_shake`
- `compute_hud_state`, `load_viewscreen_assets`, `spawn_border_on_startup`, `spawn_hud_state_entity`

The Bevy lobby UI tree (`LobbyScreenRoot`, `LobbyGridRoot`, `LobbyStationCard`, `LobbyCrewDisplay`, `LobbyReadyVal`, plus `spawn_lobby_screen`, `rebuild_lobby_station_grid`, `update_lobby_header_values`, `toggle_lobby_screen_visibility`, `spawn_station_card`, `spawn_station_placeholder`, `ready_status`, `complexity_label`, and their tests) was removed in the same PR (~870 lines).

## Tests

Smoke coverage in `tests/smoke/lobby-responsive.spec.ts`:

- Portrait viewport (480×900): aggregate chip visible, per-slot empties hidden, rail below grid, no horizontal body scroll, spectator pills rendered.
- Landscape viewport (1280×720): per-slot empties visible, aggregate hidden, rail right of grid, multi-column grid.

Protocol-level lobby coverage stays in `tests/smoke/lobby.spec.ts` (`SelectStation`, `StartGame`, captain authority, etc.). Those tests do not touch the DOM.

## Sources

- `server.html`
- `src/server/viewscreen_border.rs:330` — `push_lobby_state`
- `src/server/bridge.rs` — `set_lobby_state_callback`, `flush_lobby_state`
- `src/console_bridge.rs` — `LobbyStateChanged` event
- `src/core/messages.rs` — `LobbyStatePayload` / `StationPayload`
- Issue [#436](https://github.com/jkeywo/project-phoenix-v2/issues/436) — original HTML rebuild
- [Message Flow](./message-flow.md), [Codec Seam](./codec-seam.md)
- [PRD #120 — Station-Based Lobby](../sources/prd-120-station-based-lobby.md)
