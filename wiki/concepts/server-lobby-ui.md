---
title: Server HTML Lobby UI
type: concept
tags: [lobby, server, html, ui, bridge, responsive, accessibility, reduced-motion, native, gm]
sources: [assets/audio/room-ducking.toml, gui/audio-ducking.js, scripts/room-ducking.mjs, tests/client/audio-ducking.test.js, tests/smoke/audio-ducking.spec.js, server.html, client.html, gui/host-audio.js, gui/browser-audio-provider.js, gui/audio-preferences.js, gui/audio-settings-panel.js, gui/audio-live-equivalents.js, gui/private-audio.js, gui/private-audio-preferences.js, gui/private-request-feedback.js, gui/private-alerts.js, src/native_host/native_gm/boot.js, src/server_app/broadcast_publish.rs, gui/operator-profile.js, gui/gm-workspace.js, src/native_host/panes/operator.rs, assets/audio/private-feedback.json, src/audio_lifecycle.rs, src/server/audio_lifecycle.rs, gui/host-lobby-view.js, gui/host-lobby-render.js, gui/host-lobby.css, gui/host-qr.js, gui/host-qr.css, gui/join-url.js, gui/lobby-view.js, gui/client-lobby-render.js, gui/lobby-state.js, gui/fleet-session.js, src/server/viewscreen_border.rs, src/console_bridge.rs, src/server/bridge.rs, src/native_host/host_lobby/mod.rs, src/native_host/host_lobby/join.rs, src/lockstep/mod.rs, src/core/messages.rs, src/gm_roster.rs, src/lobby/start_policy.rs]
updated: 2026-09-21
---

# Server HTML Lobby UI

The lobby UI on the **server** (viewscreen) page is rendered as HTML/CSS/JS. The global `window.__updateLobby(json)` callback applies each `LobbyStatePayload` snapshot pushed by the Bevy server.

Since issue #1325 the render itself is **not** in `server.html`: the decisions are `gui/host-lobby-view.js` (#1229), the DOM writes are `gui/host-lobby-render.js`, and the panel's rules are `gui/host-lobby.css`. Issue #1329 did the same for the join panel that floats over the lobby — `gui/host-qr.js` and `gui/host-qr.css`. `server.html` links all five and keeps only what is its own: the audio graph, the fleet freeze and mesh pump, the asset-loading and game-over overlays, and what a click on the QR does in a desktop browser (it opens a client in its own window).

There is no `qrVisible` flag any more. The join panel's visibility **is** `#overlay`, read and written only through `gui/host-qr.js`: two documents in two engines cannot share a module-level boolean, and four handlers keeping one in step is how it came to disagree with the screen.

The browser Viewscreen's Audio tab uses `gui/audio-settings-panel.js` over
`gui/host-audio.js` and `gui/browser-audio-provider.js`. Every authored room
sound, including overlapping shots and all four computer-message severities,
passes through category and Master gain nodes. Music, Ambience/engines,
Effects/weapons and Alerts are available when configured; Interface feedback
is explained as unavailable here. Deliberate enable/retry and a bounded Music
output test report blocked, failed and unavailable output separately from mute.
`gui/audio-preferences.js` migrates the old Master key into the audio section of
this endpoint's existing Viewscreen preference record. Audio and visual writes
merge their own sections, and each reset preserves the other section.
The shared Mono output checkbox averages allowed left/right samples after
spatial positioning and before authored/category/Master gains. The browser
provider keeps an explicit stereo endpoint, so the result reaches both ears.
It changes active voices in place, preserving loop position; visual direction
and information do not change. Stereo is the initial and audio-reset default.

Optional alert ducking in the same panel lowers only Music and Ambience; it is
off by default. `gui/audio-ducking.js` schedules smooth sample-clock attack and
recovery through separate provider gain nodes. Red Alert onset and configured
warning/critical computer tones enter that path only when they actually play.
Repeated alerts extend one bounded hold and cannot prevent its eventual release.
`assets/audio/room-ducking.toml` owns the envelope; `scripts/room-ducking.mjs`
generates the browser JSON projection with a drift test. Private panels omit it.

`src/audio_lifecycle.rs` derives a local audio continuation boundary from actual
mission and restore/recovery state. Shared GM and Station projections can read
it without compiling room playback. The optional `src/server/audio_lifecycle.rs`
adapter rebuilds current room/HUD baselines before the shared projection ordering
boundary; boot classifies the optional state without installing playback.
The browser flush sends that boundary
before current config/HUD and discards old one-shots; `gui/audio-live-equivalents.js`
shows only live weapons bearing, ship impact and current beam firing. Existing
Red Alert and computer-message readouts retain their meaning when muted.
There is no audio history, catch-up or replay. Private GM pages remain silent
for this shared soundtrack; native room playback is described in
[Native Host](./native-host.md).

Private Station and GM feedback uses one parent-document owner in
`gui/private-audio.js`. The current Station iframe forwards real semantic
lifecycle transitions to that owner; replaced or inactive iframe sources are
ignored. Handled actions produce quiet clicks, refusal and timeout cues by
default; Pending and Applied tones are opt-in. Continuous and held controls
remain silent. Older typed GM families use bounded in-flight correlations in
`gui/private-request-feedback.js` against their existing attributed journal
results, including M8 presentation actions. Reading old results never creates
new feedback.

The same Audio tab shows only private Master, Alerts and Interface controls,
cue toggles and a bounded Interface output test. Portable choices live in the
operator profile; legacy client Master migration preserves the previous level
and saves once. Native profile storage preserves these bounded fields without
hardware output IDs or occurrences. Native private playback requires an
explicit provider; a missing adapter stays visibly unavailable, even when an
embedded engine exposes browser-shaped audio APIs. The authored inventory and
levels are `assets/audio/private-feedback.json`; the three new brief non-speech
tones have their generator and provenance under `scripts/` and `assets/audio/`.
Private mono follows the same portable profile and shared control. It affects
only this endpoint's allowed Interface/Alerts output. Missing mono capability
disables the control with an explanation while retaining the saved choice.
`gui/private-alerts.js` consumes only current permitted Comms, GM attention and
GM health projections through this owner's Alerts bus. Comms belongs to the
live human host of the actual System, including visiting Captain/Tactical
hosting; latest live unread Urgent and read-or-unread Critical messages qualify.
Both shared Comms components display explicit urgency words. GM attention is
limited to urgent pending Comms, eligible beats and idle NPCs, respecting the
existing hold/filter/snooze controls. The four technical health failure kinds
use the unfilterable banner path instead.

The existing runtime audio lifecycle stamps `presentation_generation` on
BlackboardUpdate and GM attention/health envelopes, outside canonical boards.
The client stores the generation beside each received board; native sparse
queues never merge boards across it. Mount, reconnect, restore, entitlement and
document return establish silent baselines. Muted, held and unavailable cues
are consumed without storing history or offering catch-up. Native GM boot
forwards the same attention/health/workload channels to the shared workspace.

The split exists because there are now **two** surfaces rendering this lobby from the same payload: the host page, and the native host's viewscreen surface (see [Native Host](./native-host.md#the-host-lobby-on-the-viewscreen-issue-1325)), whose document is built from this page's own `#lobby-panel` markup. Both call the same `renderHostLobby`. A second implementation of these element ids would drift the first time either was touched, so there is not one.

Each viewer of `server.html` sees the same lobby state because the data originates from a single authoritative Bevy world.

Mission selection may now expose authored player-ship slots. Worlds with only
the legacy `[[available_ships]]` list still present their existing single hull
stage: runtime configuration synthesises one `player` slot, so they need no
content migration. Explicit slots carry independent allowed/default hulls and
use the shared `ship_slots` reservation law; reservation is pre-start state,
released immediately on Back or disconnect rather than entering post-start
fleet recovery.

Launch freezes claimed hulls and each unclaimed policy in authored slot order.
`backfill` (the default) materialises the slot's default hull with no human
crew, so its ordinary Station control-source resolver selects AI. `absent`
omits that slot's own `GameStart` ship row entirely; an empty audience never
broadens to every ship. The frozen roster is snapshot/digest state and remains
fixed when a host disconnects after launch, at which point only its Station
control sources fall back to AI. An authored multi-slot world is rejected if it
does not provide one resolvable, ship-tagged `GameStart` row per slot.

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
| `station_ratings` | `Vec<(StationId, String)>` | Connected non-Spectator holders' selected lobby Ratings, sorted by Station id; no participant identity or session token. |
| `presentation_ready` | `bool` | The renderer's required assets have reached a terminal preload state; rendererless/headless peers report ready. |
| `stations` | `Vec<StationPayload>` | One entry per claimable, non-auxiliary station. |
| `spectators` | `Vec<String>` | Names holding the explicit Spectator role. |
| `gms` | `Vec<GmOperator>` | Equal GM rows (`id`, `name`, `connected`, `ready`), separate from every crew and Station count. |
| `all_stations_filled` | `bool` | Flips ready badge to `READY TO LAUNCH`. |

`StationPayload`: `name`, `short_code`, `rank`, `holder_name?`, `is_mine`, `preset_names`.

`SessionManager::lobby_station_ratings` selects each held Station's pending
lobby Rating, then its first authored Rating, then `Std`. `server.html`
publishes these pairs alongside readiness through `gui/fleet-session.js`,
including when only a Rating changes. Mission start freezes them as
`FleetShip.crew`, so every host boots the same per-hull control sources.

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
  with a final game-over push on phase entry — that push alone carries
  `game_over_message`, `game_over_outcome`, `scenario_title` and
  `game_over_report`, which `server.html` and the native
  `gui/viewscreen-hud.html` frame through the phone's own
  `gui/game-over-view.js` and draw with the shared `gui/game-over.css`;
- `RedAlertVignetteMaterial`, shield flash, hull shake, camera shake, and the
  reduced-motion preference remain renderer-side presentation state.

The lobby cards, rail and responsive layout are DOM owned by
`gui/host-lobby-render.js` + `gui/host-lobby.css` over `gui/host-lobby-view.js`;
the join panel is `gui/host-qr.js` + `gui/host-qr.css` over the same view
model's `qrOverlayAction` (issue #1329). The Game Master group is `server.html`'s
own projection over that view model, rendered as a sibling region rather than
folded into the Station list; the crew page receives the same public GM
projection in `Welcome` and `GmRosterChanged`, which `gui/lobby-view.js` folds
into the client lobby's view model and `gui/client-lobby-render.js` (issue
#1369) writes out likewise — the same view-model-then-renderer split this
panel uses. Bevy publishes data but does not build a lobby UI tree — on either
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
`ViewscreenMotion.shake_intensity` now carries the separate camera-shake choice
from `gui/viewscreen-presentation.js` (#1428). Flash/pulse and decorative motion
have independent settings too; Viewscreen choices persist per endpoint through
the browser/native presentation adapters, separately from private profiles.

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
