---
title: Native Host
type: concept
tags: [native, viewscreen, boot-profile, wgpu, winit, transport, delivery, ultralight, panes, displays, monitors, bridge-profile]
sources: [src/native_host/mod.rs, src/native_host/app.rs, src/native_host/transport.rs, src/native_host/bridge_profile.rs, src/native_host/bridge_display.rs, src/native_host/panes/mod.rs, src/native_host/panes/identity.rs, src/native_host/panes/routing.rs, src/native_host/panes/document.rs, src/native_host/panes/surface.rs, src/native_host/panes/ultralight.rs, src/native_host/panes/recovery.rs, src/delivery/serve.rs, src/boot/mod.rs, src/bin/phoenix_host.rs, src/entities/template_preload.rs]
updated: 2026-08-29
---

# Native Host

One Windows executable that runs the ordinary authoritative simulation and
draws the shared viewscreen through native Bevy/wgpu, instead of a browser tab
running the same WASM. Issue #1121.

```bash
cargo build --release --features host --bin phoenix-host
./target/release/phoenix-host --world assets/worlds/combat_test.toml --solo
```

It is the **same binary** PRD #855 shipped for delivery. `--world` adds the
simulation; with no `--world` the process is byte-for-byte the delivery host it
always was, so the bundle serving, the `--manifest` catalogue restriction and
the startup version pin are shared rather than forked.

## Where the code is

| Piece | File |
|---|---|
| App builder | `src/native_host/app.rs` |
| Transport seam | `src/native_host/transport.rs` |
| Content-root pin | `src/native_host/mod.rs` (`pin_content_root`) |
| Boot profile + render surface | `src/boot/mod.rs` (`BootProfile::NativeHost`, `NativeRenderSurface`) |
| Process, argv, threading | `src/bin/phoenix_host.rs` |
| Template preload | `src/entities/template_preload.rs` |

## The boot profile

`BootProfile::NativeHost` is the fourth inventory in the [boot
seam](../../src/boot/mod.rs): a real renderer that is **not** a browser — the
combination the original three profiles could express but none occupied. It
aborts on a broken world (like headless: it is launched from a command line
naming its world), takes the render stack, and adds two things no browser
profile has:

- **`NativeRenderSurface`** — `Window` (winit), `Offscreen` (real wgpu, no
  window, as `capture-billboard` runs) or `Contract` (no wgpu at all). A
  runtime choice rather than a `cfg`, because native is simultaneously the
  shipped host target and the target `cargo test` runs on, and the test runner
  has no GPU.
- **A native template-cache check.** `boot::build` runs no preload of its own,
  and every cache-only reader reads the native entity-template cache with no
  filesystem fallback (`asteroids::lifecycle`, `lobby::server`, `server::radar`,
  `server::reference_grid`, `server::asset_preload`, `server_app::world_setup`,
  `world::server`) — `lobby::server::update_session_with_config` worst of all,
  which on a miss keeps a *default* `ShipClientConfig` and lets a mission run on
  with the wrong radar range and nothing in the log. For this profile boot
  refuses to compose unless every template the **composed** world declares — the
  root and every `extra_worlds` child, through
  `world::config::entity_template_paths` — is already cached. The selected hull
  is checked separately by `build_native_host_app`, because issue #935 made the
  player's own hull authored content that need not be in the world's declared
  set at all, so an explicit `--ship` would otherwise walk past the boot gate.

## The catalogue restriction cuts both ways

`--manifest assets/scenarios.demo.toml` is issue #917's curated catalogue, and it
narrows what this process **flies** as well as what it publishes:
`curated_hulls_for_world` reads the manifest's allowlist for the `--world` in
force and `build_native_host_app` draws its default hull from it. An explicit
`--ship` still wins, exactly as `?ship=` does in the browser — curation narrows
the default, it is not a second admission gate.

## Threading and content roots

Bevy owns the main thread (winit requires it on Windows) and `App::run()` blocks
natively, so `HostServer` moves to a worker with `serve_until` + a
`ShutdownSignal`. The poll seam that signal needs is installed by
`enable_shutdown_polling` **before** the worker starts and before Bevy takes the
main thread: a listener that cannot be made non-blocking has no stop path, and a
delivery thread with no stop path turns a clean window close into a process that
hangs on the join with port 8080 still held. That is fatal at the prompt, not a
fallback. A native process also resolves content two independent ways —
Bevy's `AssetServer` (rooted at `BEVY_ASSET_ROOT`, else the executable's own
directory) and raw `std::fs` against the working directory, for world TOML,
templates, Rhai scripts and rig sidecars. `pin_content_root` sets both from the
one `--content-dir`, because pinning one and not the other half-loads content
silently.

## The transport seam

`native_host::transport` is two systems over the three `lobby::server` messages
— `InboundMessage`/`PlayerDisconnected` in `PreUpdate`, `OutboundMessage` in
`PostUpdate` — behind a `NativeTransport` trait, plus a `LoopbackTransport` for
tests and for in-process participants. It carries **decoded** messages, not
bytes, so an in-process pane can skip the codec while still entering through
`Messages<InboundMessage>` and still leaving through an `Audience`-resolved
`Target`.

The one authorisation decision the seam makes is the **reserved-token refusal**
(`lobby::handler::is_reserved_token`): `__local_console__` and `ai:`-prefixed
tokens are dropped at ingress, because the browser refuses them at its own
PeerJS ingress too and `__local_console__` skips the station-tenure branch of
`is_command_authorized` entirely.

**Not yet connected.** The transport itself is issue #1112 — PeerJS is browser
JavaScript and cannot run in a native process, which is why #1121 is filed as
blocked by it. Everything downstream of the seam (admission, projection,
protocol) is the same code a phone goes through, and
`tests/native_host_sim.rs` drives a participant through it end to end with the
loopback transport.

The operator-visible consequence: with no transport and no panes, **`--solo` is
the only mode that reaches a running mission.** Without it the host waits in the
lobby, and every route out of it needs a session — collective `SetReady`
auto-start, or the host page's force-start (`drain_force_start_input`,
wasm-only). The mode is not refused, because it becomes correct the day #1112
lands; `build_native_host_app` warns loudly at boot instead (`LogCat::Lobby`),
and `--help` and AGENTS.md carry the same caveat. A host with `--pane` does have
participants, so it does not take that arm.

Two transports on one host compose with `PairedTransport` — poll both, dispatch
to both, neither told about the other's traffic. That is how #1112's network
transport arrives beside the panes: an `insert_resource`, not a re-plumb.

## Local Station panes (issue #1122)

```bash
node scripts/build-client.mjs        # panes load the BUILT bundle's page
cargo build --release --features ultralight --bin phoenix-host
./target/release/phoenix-host --client-dir dist \
    --world assets/worlds/combat_test.toml --pane Ada --pane Grace
```

A pane is **one logical client running inside the host process**: its own
document, its own minted session token, its own input queue, its own lifecycle.
It joins, claims a Station, readies and plays through exactly the contracts a
phone does, because PRD #1093 allows an in-process participant to skip network
serialisation and nothing else.

| Piece | File |
|---|---|
| Registry, lifecycle, the outbound cap | `src/native_host/panes/registry.rs` |
| Identity and its three refusals | `src/native_host/panes/identity.rs` |
| Audience projection | `src/native_host/panes/routing.rs` |
| The `NativeTransport` over the panes | `src/native_host/panes/transport.rs` |
| The per-frame loop and its surface trait | `src/native_host/panes/surface.rs` |
| The document, the URL, the injected scripts | `src/native_host/panes/document.rs` + `pane_boot.js`, `pane_link.js` |
| Ultralight views, drawing, input | `src/native_host/panes/ultralight.rs` (feature `ultralight`) |
| Shared plumbing | `vellum-ultralight` (see the audit below) |

### A pane is a phone, not the host operator

`console_bridge::LOCAL_CONSOLE_TOKEN` exists, is reserved, and its own doc
comment names a native host as a future user — and reaching for it is the wrong
move. It takes branch 2 of `is_command_authorized` (`policy.accept_human_input`,
**no station-tenure check at all**) and carries `ReturnToLobbyAuthority::Host`,
which can abort a running mission. It stays exactly where it is, for the host's
own viewscreen-side controls.

A pane mints an ordinary UUIDv4 instead, and three separate gates keep it that
way, because they close different holes:

1. `PaneIdentity::adopt` refuses a reserved token, so a pane cannot be
   *configured* with host authority;
2. `PaneBus::submit` refuses an `Identify` whose **body** token is not the
   pane's own — `handle_identify` uses the body token, so this is the only gate
   that stops a pane impersonating *another participant's ordinary token*;
3. the #1121 seam refuses reserved tokens at ingress, as `server.html` does at
   its PeerJS ingress.

Outbound, `routing::pane_receives` is the projection boundary and is four lines:
`Audience::Holding*` has already resolved through
`SessionManager::holder_for_station` into a `Target::Token` before a transport
sees it, so a pane that holds no Station is named by no audience.

### What a pane loads, and from where

The pane navigates to
`http://<host>/client/pane-<n>-<nonce>.html#native&token=…&name=…`, a document
this same process publishes in memory (`delivery::serve::HostedDocuments`,
checked ahead of the static bundle). That document is the built bundle's own
`client/index.html` with **two scripts injected and one removed**:

| edit | why |
|---|---|
| a classic `<script>` first in `<head>` (`pane_boot.js`) | reads the session token and participant name out of `location.hash` before the page's own inline script wants them; rewrites the fragment down to the join code the page's one route expects; installs the page→host queue |
| the PeerJS `<script>` tag removed | a no-op since #1112 deleted the tag from `client.html`, kept because the claim it backs — no pane document loads PeerJS — is worth asserting in both states. Matched on `peerjs` in the opening tag rather than a CDN host, so a move or a self-host could not silently turn the strip into a no-op either |
| the `<audio>` elements removed | **not cosmetic** — see below |
| a `<script type="module">` last in `<body>` (`pane_link.js`) | installs `window.PhoenixTransportFactories`, the in-process stand-ins the page's own joiner then uses — see "A pane joins through the page's own front door". A module, and last, because it *imports* the channel labels from `gui/rendezvous-transport.js` rather than spelling them itself |

The `<audio>` strip is the one worth stating in full, because it was the
difference between a pane that works and a pane that looks like it does.
Ultralight ships **no media backend**: `HTMLMediaElement.play` is undefined and
calling it throws. `client.html`'s `send()` plays the UI click *before* handing
the message to the link, so that exception propagated out of the
`console_action` listener and the command never reached the transport. The
page's own `NO_CLICK` set exempts `SetThrust`, `SetSteering`, `Identify` and
`SetName` — so a pane joined, renamed itself and flew with the joystick, while
every deliberate console command, every `ControlSystem`, was silently dropped
with a clean log on both sides. With the element gone, `playClick`'s own
`if (!el) return` is the path; `gui/settings-panel.js` already `.filter(Boolean)`s
its `audioEls`. Fixing it in `client.html` would mean changing the page a phone
loads to suit a pane, which is the thing this arrangement exists not to do.

Nothing in `gui/` or in any console page is touched; the shim is native-side.
Serving it at the **client directory's own depth** is the load-bearing detail:
every relative URL in the page then resolves exactly as it does for a phone,
same-origin, with no `<base>` tag, no rewriting and no CORS question. The three
rejected alternatives (a native reimplementation of the shell, an
`include_str!`'d document with no base URL, and the unmodified page with no way
in) are argued in `document.rs`'s module docs.

Host→page is `window.__phoenixPaneApply('<json>')`; page→host is a queue drained
once a frame with `window.__phoenixPaneOutDrain()`. Both directions carry the
same JSON a phone would send or receive, decoded by `core::codec` — the codec
seam is not bypassed either. A frame pushes at most
`surface::MAX_PUSHES_PER_FRAME` messages per pane and requeues the rest: this
loop runs in `Update` on the Bevy main thread, which is the thread `FixedUpdate`
runs `SimSet` on, so page JavaScript time is *simulation* time for everyone on
the ship.

### The identity is in the URL, not in the page

`phoenix-host` binds `0.0.0.0:8080` by default, with no TLS and no
authentication — that is the shape PRD #855 wanted, because the audience is
phones on a LAN. So **nothing about who a pane is may appear in a body this host
serves**: a live participant's session token there is a seat on the bridge
available to anyone on the network.

A URL **fragment** is the one part of a URL a browser never transmits — not in
the request line, not in a header, not in any byte the host writes — so
`pane_url` carries `#<code>&token=…&name=…` and `pane_boot.js` reads
`location.hash` at parse time. `build_pane_document` takes no identity at all,
and every pane's document is byte-identical. (`fragment_encode` escapes
everything outside RFC 3986's unreserved set, which is what keeps the fragment's
own `&`/`=`/`%` grammar out of a participant's name. It no longer singles the
underscore out: that escape existed to keep a name like `ada_lovelace` off the
rendezvous route, and since #1112 *every* non-empty fragment is that route.)

### A pane joins through the page's own front door

The fragment is also the client page's **one join input**, and since #1112 there
is no other route: `joinRouteFromLocation` reads any non-empty fragment as a
code and hands it to `parseJoinCode`, which refuses `token=…&name=…` and drops
the join-entry overlay over the console. So the pane's route is composed rather
than dodged — the fragment leads with `document::PANE_JOIN_CODE`, a five-letter
suffix the authored table in `assets/join/join-codes.toml` accepts, and
`pane_boot.js` rewrites `location.hash` down to just that before any page code
reads it. Two consequences worth naming: the page's URL is then
indistinguishable from a phone's that scanned a QR, and the pane's session token
has stopped being readable out of its own `location.hash`.

From there `client.html`'s `startPhoenixJoin` runs **unchanged**. What the pane
supplies is one documented override: `gui/rendezvous-transport.js`'s
`defaultFactories()` reads `window.PhoenixTransportFactories` on every call and
names "a native in-process host" as an intended user of it, and `pane_link.js`
is that user. Its socket answers the two frames the joiner waits on; its peer
connection's reliable channel opens at once, answers the compatibility handshake
itself (one process, one bundle, nothing to disagree about), and is a pipe onto
`window.phoenixPaneOut` outbound and `window.__phoenixPaneApply` inbound.

### A pane's `requestAnimationFrame` is a timer

`pane_boot.js` replaces `requestAnimationFrame`/`cancelAnimationFrame` with
`setTimeout` before any page script runs — before `gui/bg-raf-keepalive.js`,
which captures whatever it finds and delegates to it whenever the document is
visible (a pane always is).

This is not about smoothness. An offscreen Ultralight view services rAF inside a
*rendering update*, and only runs one when the page is dirty. The client page's
whole render loop is a single outstanding rAF (`scheduleRender`'s `_renderFrame`
guard), so a pane that reaches a quiet moment deadlocks against itself: no
rendering update, so the callback never fires; the callback never fires, so
nothing mutates the DOM; nothing mutates the DOM, so there is no rendering
update. `_renderFrame` stays non-null, every later `scheduleRender()` returns at
its first line, and the console is frozen at its last paint while its transport
goes on delivering perfectly good state. It presented as roughly one pane in
four (5 failures in 13 runs of `tests/native_host_pane_ultralight.rs`) coming up
with the lobby still over a Station it knew it held. Timers are not starved that
way — `Renderer::update()` runs them whether or not anything painted, which is
why the page's own 500 ms name-field debounce fired in exactly the runs whose
rAF never did.

### The seam, not a convenience

`currentLink()` reads a closure only
`startPhoenixJoin` assigns, so a pane-owned link object cannot be reached
without editing the page a phone loads; and going through the front door is what
makes `localiseTree`, the status line, the `#conn-diag` readout and
`window.phoenixLink` (which `gui/command-gateway.js` resolves against) the
page's own, rather than a second implementation of them that can rot. It rotted
once: the first `pane_link.js` published `window.connectionManager`, a global
#1112 retired with PeerJS, and the page never called it.

Three further defences, because one is a single point of failure:

1. the document path carries a **per-pane random nonce**, so it is not
   enumerable the way `/client/pane-0.html` was;
2. `delivery::serve::route` serves a hosted document only to a **loopback**
   peer — a pane always connects from this machine, and a remote GET gets what
   any unknown path gets. The bundle, the manifest and the stamp stay LAN-open,
   which is their job;
3. `PaneBus::close` **withdraws** the document, so a closed pane's path stops
   resolving; `phoenix-host` withdraws the rest at shutdown, before it joins the
   delivery thread.

### A closed pane is torn down, not left on screen

`drive_panes` closes a pane whose page stopped draining, and the *view* goes at
the top of the next frame (`retire_closed_panes`): the `PaneWindow` is dropped,
its canvas node despawned, its document withdrawn, and the closure logged once.
Reaching that state at all takes **both** caps — the host's outbound queue only
holds what it has not handed over, so `pane_boot.js` caps the page's own inbox
and throws past it, which `pump_pane` requeues as an ordinary failed push.

The closed pane's station is held and flipped to `Backfill`, exactly as a dropped
phone's is. **Restoring a human there — recreating the pane on the same identity
— is issue #1125's, and does not need #1112's transport** because a pane's
reconnect is in-process; see the recovery section below.

Each pane's view is created in its **own Ultralight `Session`**, named after the
pane and never written to disk. Ultralight keys cookies, `localStorage` and
IndexedDB on the Session rather than on the view, and a view created without one
lands in the renderer's single persistent default session — so, since every pane
document comes from this host's one origin, panes would otherwise share a
`localStorage`, `gui/session-token.js`'s `session-token` key included.

### The Void and Thunder audit (acceptance criterion 1)

void-and-thunder's `crates/vt_client/src/hud.rs` is a working, shipping
Ultralight integration: one full-screen HUD `View`, CPU-rendered, mouse-only,
its document `include_str!`'d, its DLLs staged by a PowerShell block inside
`run.bat`. It was read and split as follows.

**Extracted to `vellum-ultralight`** (a new fleet crate; `ul-next` is no longer
a per-game exception — see vellum's `docs/handbook/dependencies.md`):

| piece | why it generalises |
|---|---|
| SDK discovery, linking, DLL + `resources/` staging | identical for any consumer, and **neither** game had automated it — a fresh v&t clone's first run died on a missing DLL with no message |
| `Renderer`/`View` → premultiplied-BGRA surface → straight-alpha RGBA copy, stride-aware and dirty-rect gated | pure plumbing with no product in it, and the piece most worth testing |
| the `evaluate_script` push/poll bridge: escaping, call construction, the queue shim, record splitting | Ultralight exposes no `window.ipc`, and both games independently built a queue-and-drain around that one primitive |

**Kept in phoenix**, because void-and-thunder has no version of it at all:

| piece | why it is ours |
|---|---|
| pane registry and lifecycle | v&t has exactly one view, created once, never closed |
| session identity and its refusals | there is no such concept in a single-player HUD |
| audience projection | ditto |
| input-translation *policy* | v&t deliberately withholds the keyboard ("handing Ultralight the keyboard would fight the game for every key"); a station console has form fields, so panes forward text — that is net-new either way |
| the document and what is injected into it | v&t compiles its page into the binary; a pane loads the shipped client over HTTP |
| the payload schema in both directions | v&t's `action\|key=value` line exists because its host has no JSON parser; phoenix's wire protocol already is JSON |

### SDK, licence and redistributables

`ul-next-sys`'s build script downloads a prebuilt SDK archive from a public,
**unauthenticated** URL into its own `OUT_DIR`. Nothing is vendored here and no
credential exists to check in. Cargo links it but stages nothing, so
`panes::ultralight::stage_sdk` runs at startup: it finds
`target/<profile>/build/ul-next-sys-*/out/ul-sdk`, copies the four shared
libraries beside the executable and the SDK's `resources/` (CA bundle, ICU
table) into the working directory, and prints the licence files it found. On
Windows a missing DLL is a process that exits with an OS code and *no message*,
which is why the operator log names them.

The licence is the **Ultralight Free License Agreement V1**, shipped inside the
download at `<ul-sdk>/license/LICENSE.txt` (with `EULA.txt` and `NOTICES.md`);
`vellum_ultralight::staging::licence_files` names them from a checkout. Its terms
govern redistributing those libraries with a packaged build — see
`docs/delivery-checklist.md`.

The `ultralight` cargo feature is what keeps all of this off every other build.
No CI job sets it; `default`, `server`, `host`, `headless`, `capture` and
`viewer` must never imply it. `/resources/`, `/default/` and `ultralight.log` are
gitignored runtime debris.

**Default-off is not sufficient on its own**, and assuming it was is how the
feature reached CI on the commit that introduced it: `--all-features` enables
everything declared, whatever implies what, and the `test` job's clippy step
asked for exactly that. That step now names its features — every one
`Cargo.toml` declares except `ultralight` — and `AGENTS.md`'s local gate command
mirrors the list, so **a new cargo feature has to be added to both**. Vellum
handles the same problem the other way: its engine job selects the workspace
with `--exclude vellum-ultralight` and type-checks the SDK half in a manual,
non-required job.

### Deferred

- **A real browser participant beside a pane** is issue #1112's, exactly as in
  #1121: `tests/native_host_panes.rs` proves the contract with a
  `LoopbackTransport` participant, which is the seam a network transport plugs
  into. **Issue #1122's acceptance criterion 5 therefore stays unticked** and is
  carried on #1112 instead: the same test swaps `LoopbackTransport` for the real
  network transport and changes nothing else, which is what `PairedTransport`
  was built for.
- **Bridge display profiles** (which monitor is which) are issue #1123's — see
  the section below. What that issue leaves for the follow-on is **compositing a
  pane onto its assigned Station window**: #1123 opens the Station windows and
  computes each pane's rectangle, but the Ultralight pane host still tiles panes
  on the viewscreen window as it always did (`panes::ultralight`), and
  `bridge_display::BridgeStationSurfaces` is the seam that rehoming reads.
- **Independent input routing** between panes is issue #1124's. Today it is one
  pointer, one focused pane, the left button, the wheel and text.

## Bridge display profiles (issue #1123)

A bridge is a room of monitors: one shared viewscreen, the rest crew Stations.
A **bridge profile** writes down which is which so it survives a reboot, and the
host covers every configured monitor with one borderless-fullscreen surface.

```bash
# See the connected monitors, their stable identities and geometry:
phoenix-host --setup
# Validate a profile against them (nothing is opened):
phoenix-host --setup --profile bridge.toml
# Run the authoritative host with the profile applied:
phoenix-host --world assets/worlds/combat_test.toml --profile bridge.toml
```

| Piece | File |
|---|---|
| The pure model — identity, density, geometry, round-trip, resolution | `src/native_host/bridge_profile.rs` |
| The winit adapter — enumerate, resolve, open surfaces, `--setup` | `src/native_host/bridge_display.rs` |
| The `--setup`/`--profile` flags | `src/delivery/args.rs`, `src/bin/phoenix_host.rs` |

The **pure model is Bevy-free** (`bridge_profile`): stable monitor identities,
the one/two-pane density rule, the pane geometry, the TOML round-trip and the
missing-display resolution are all decided there and tested by the ordinary
`cargo test` CI runs. The **winit adapter** (`bridge_display`) reads real
`Monitor` components and opens windows, and is provable only under the
`#[ignore]`d `tests/native_bridge_displays.rs` on a machine with displays.

### The stable identity scheme, and its limit

winit (and so Bevy's `Monitor`) exposes **no serial number or EDID** — nothing a
display carries in hardware. So a monitor's stable identity is `name@WxH`
([`identify`](../../src/native_host/bridge_profile.rs)) — the OS-reported name
and the native resolution — deliberately **excluding position and scale
factor**, both of which change under an ordinary rearrange that the identity must
survive. Two *identical* monitors (same model, same mode) report the same name
and size and are indistinguishable to winit; those, and only those, get a
position suffix (`name@WxH#x,y`), which does not survive physically swapping the
two. On Windows the name is often the GDI **device name** (`\\.\DISPLAY5`, as the
dev machine reports), so the identity is only as stable as that slot name across
a re-plug — the honest limit of what the platform gives. (When hand-authoring a
profile, write a Windows identity as a TOML **literal** string with single quotes
— `id = '\\.\DISPLAY5@1920x1080'` — so the backslashes need no escaping; the
serializer escapes them for you when it writes a basic string.)

### Roles, the density rule, and geometry

A profile is an **ordered** list of `[[display]]` assignments plus a `[[touch]]`
mapping. Each display is `viewscreen` or `station`; a Station carries one or two
`[[display.pane]]` slots. **Three or more is refused** at validation
(`MAX_PANES_PER_STATION`, the PRD's pane-density rule) with an authored
explanation naming the monitor — a console is authored to be read one, or
side-by-side two, to a screen, and three at bridge distance is unreadable. The
one/two-pane geometry is computed by `pane_rects`: one pane is the whole monitor,
two divide it side-by-side (default) or stacked, tiling exactly with the odd
pixel absorbed by the last pane.

The profile is **not** the private player Accessibility profile (#1127): this is
shared operator configuration of the physical room, carrying nothing about any
one player, and the two are kept in separate files.

### The winit adapter, and missing displays

Once winit reports the monitors, `BridgeDisplayPlugin` resolves the profile
against them and covers each configured monitor:

- the **viewscreen** goes on the process's **primary window** — the one #1121
  already opened and the game cameras already target — put into
  `WindowMode::BorderlessFullscreen` on its monitor, so no camera retargeting is
  needed;
- each **Station** spawns its own borderless-fullscreen window, tagged
  `BridgeSurface`, its pane rectangles published in `BridgeStationSurfaces`.

A monitor the profile assigns but that is **not present** is named and reported
(`ProfileProblem::MonitorMissing`) and its role is **left unfilled — never
re-homed onto another display**; a present monitor with no assignment is reported
too. A display whose *resolution* changed has a different identity, so it
surfaces as one missing (the old id) and one unassigned (the new id) — both
named, nothing moved. That is the whole of "missing or changed displays are
reported explicitly and are not silently replaced or rearranged" **at setup**;
losing one *mid-mission* is the runtime companion, issue #1125's — see the
recovery section below.

### The setup surface ([ai] decision)

The setup surface for #1123 is the **hand-editable profile file plus the
`--setup` enumeration mode** — not an interactive on-screen UI; the
touch/keyboard-operable setup screen is #1124/#1128's. `--setup` opens a hidden
winit window purely to enumerate the monitors, prints each one's identity,
geometry and current assignment (validating `--profile` against them if given),
and exits. Its whole *content* is the pure `render_setup_report`, so it is tested
without a display; the winit window is the one part that needs the machine.

### What is deferred

Compositing an Ultralight pane onto its assigned Station window is the
continuation, sharing #1124's input-routing concern: this slice opens the Station
windows and lays out the pane rectangles, and leaves the pane rendering pointed
at those windows through `BridgeStationSurfaces`. Until then a host launched with
both `--profile` and `--pane` opens the Station windows **and** tiles the panes on
the viewscreen window as #1122 always did — nothing regresses.

Station windows are borderless-fullscreen but visually **empty** until #1124
lands pane compositing: nothing renders into one on its own (no camera targets
it), so an operator seeing a blank Station display before then is seeing the
disclosed, correct state — not a broken render.

## Recovering a failed pane and a lost display (issue #1125)

A local pane is "just another logical client", so a pane *failing* must ride the
same disconnect → Backfill → reconnect machinery a dropped phone does — never a
native-only fatal error. #1125 routes three new triggers into that path and
recreates a crashed pane on the same identity. Nothing in the simulation gains a
pane-shaped branch: the whole of it is `PaneBus::close`/`recreate` and the
runtime display watcher, and the sim sees only the ordinary `PlayerDisconnected`
and reconnect `Identify`.

- **A view crash.** `drive_panes` counts a pane's consecutive frame-copy
  failures; a run past `VIEW_CRASH_COPY_FAILURES` (a lost surface, a renderer
  that stopped answering — where a one-off is a transient) faults the pane
  `PaneFault::ViewCrashed`. The pre-existing inbox-overflow trigger becomes
  `PaneFault::ReliableOverflow`; both are serviced by `recovery::service_faults`,
  which **closes** each faulted pane (its token disconnects, its station flips to
  Backfill) and, for a crash only, **recreates** it.
- **A display lost mid-mission.** `watch_runtime_displays` (in `bridge_display`)
  diffs the live `Monitor` set frame to frame; when a configured monitor that was
  present goes away, the pure `runtime_display_losses` names it exactly (a
  `RuntimeDisplayLoss`, the runtime companion to `ProfileProblem::MonitorMissing`)
  and the watcher closes the panes that Station carried — resolved by participant
  name through `PaneBus::open_pane_for_name`. A lost **viewscreen** monitor is
  named but fails no pane (nobody sits there). Nothing is re-homed.
- **Recreation, on the same identity.** A crash's `PaneBus::recreate` opens a
  fresh pane carrying the **same session token** (a new `PaneId`, ids are never
  reissued), republishes its document at a fresh nonce, and enqueues its view for
  `open_pending_views` to build next frame in the crashed pane's stored slot. The
  page reloads and its `Identify` is a *reconnect* — `handle_identify`'s
  reconnect-yield restores the held station (still Backfill, still unclaimed) to
  the human and pushes the current projection. This is the in-process analogue of
  a phone redialling on its saved token; a pane's reconnect needs **no** #1112
  transport.

Two lines keep the boundaries honest. A **display loss does not recreate** — the
display is gone, there is nowhere to rebuild the view, and bringing it back is an
**explicit** repair (re-apply the profile), reported by `runtime_display_returns`
and never done silently; that is #1123's no-silent-rehome doctrine, held at
runtime. And **no surviving pane inherits a failed one's projection**: audience
projection resolves through `SessionManager::holder_for_station`, which gates on
`connected`, so the instant a token disconnects nothing resolves to it — the
recreated pane, carrying the same token, is the only thing that receives that
token's projection again, and that is the reconnect, not a leak.

## Tests

| File | Claim |
|---|---|
| `src/native_host/bridge_profile.rs` | The pure model: stable identity across a simulated OS-settings rearrange, identical-monitor disambiguation (including a TOML round-trip of a position-suffixed id), the one/two-pane geometry math (even and odd, side-by-side and stacked), the >2 density refusal, the one-viewscreen refusal (`ProfileError::MultipleViewscreens`, naming both monitors), the TOML round-trip (Windows backslash ids included), the missing/unassigned/changed-display reporting, and (#1125) the runtime loss/return detection — a lost Station names its panes, a lost viewscreen names none, a return is for explicit repair only. All feature-agnostic, run by the ordinary `cargo test` |
| `src/native_host/bridge_display.rs` | A Bevy `Monitor` lifts into a `RawMonitor` and carries the documented identity; `--setup`'s exit code is clean only when a supplied profile both validates and resolves with no problems against the connected displays (`setup_profile_is_clean`); and (#1125) `watch_runtime_displays` itself — driven with *fake* `Monitor` entities spawned and despawned as bevy_winit does on hot-plug, so it runs in CI without a display — closes a lost Station's pane (→ Backfill) and no pane for a lost viewscreen |
| `src/native_host/panes/recovery.rs` | `PaneFault`'s two kinds and `service_faults`: a view crash closes the pane and recreates it on the same token; a reliable overflow closes and does not; the recreated pane receives its own token's projection while the failed handle and a bystander receive nothing; and the recreated pane re-identifies on that token as a reconnecting phone does. All feature-off |
| `tests/native_bridge_displays.rs` | On the real machine's monitors, a profile opens one borderless-fullscreen surface per monitor at the monitor's geometry — the viewscreen on the primary window, a Station on its own. `#[ignore]`d: it opens real winit windows, which CI has no display for. Verified once locally |
| `src/delivery/args.rs` | `--setup` is a standalone diagnostic needing no world and refuses every simulation/crew flag (`--world`, `--ship`, `--seed`, `--solo`, `--pane`, `--log`, `--log-entity`, `--rendezvous`, `--origin`) rather than silently discarding them; `--profile` applies with a world or validates with `--setup`, and is refused alone |
| `src/boot/tests.rs` | Four-profile parity; only the render-stack profiles take that path; a native host refuses a configless boot, including a hull declared only by a static child |
| `src/native_host/transport.rs` | The seam's ingress/egress and the reserved-token refusal |
| `tests/native_host_sim.rs` | Shipped content boots and runs; the hull's own config reaches the client config; an uncached `--ship` is refused; a curating manifest narrows the default hull; a participant joins through the seam |
| `tests/native_headless_digest.rs` | AC5's content half — native↔headless digest equivalence, both apps on a pinned single-threaded pool. **Scope:** the default run is `NativeRenderSurface::Contract`, so it covers the `render: true` *simulation* plugins and not the wgpu stack; the `#[ignore]`d `Offscreen` companion in the same file covers that on a real GPU |
| `tests/native_host_snapshot.rs` | AC5's snapshot half — a native-host capture restores into a fresh native host at the same digest, and the duel continues byte-identically for 120 frames (Combat Test's continuation bound is the payload gap `tests/snapshot_resume.rs` measured, not a native one) |
| `tests/native_viewscreen_render.rs` | The viewscreen draws a scene — not one flat colour, and lit — over the **middle 40%** of a real-GPU frame, so a live HUD over a dead 3-D scene cannot pass. `#[ignore]`d: CI is ubuntu-only with no display |
| `tests/native_host.rs` | The delivery half, unchanged, plus the serving loop's shutdown path (polled to a deadline, so a stuck loop fails rather than wedging the run) |
| `src/native_host/panes/*` | Identity's three refusals, the projection boundary, the outbound cap's reliable/snapshot split, the document assembly (against the repository's own `client.html`, not only a stub), the identity's absence from the served body, the wildcard-bind normalisation, and the per-frame loop's push budget and deferral of a failed push. All feature-**off**, so the ordinary `cargo test` CI runs them |
| `src/delivery/serve.rs` | A hosted document is served to a loopback peer and to nothing else, while the bundle and the version-pin endpoints stay LAN-open; `peer_origin` classifies IPv4, IPv6, IPv4-mapped and "the OS would not say" |
| `tests/client/pane-scripts.test.js` | The two injected scripts, in jsdom, **driven through the real seam**: the boot script reads the identity out of the fragment, leaves a fragment `joinRouteFromLocation`/`parseJoinCode` accept (the literal is read out of `document.rs`, so the cross-language pin is checked), and caps the page's inbox; then the repository's own `createRendezvousJoiner` is run over the link's factories and asserted to produce the host-minted `Identify` on the page→host queue, to keep `JoinHandshake` off it, and to hand `onData` a `localiseTree`d message |
| `tests/native_host_panes.rs` | A pane joins/claims/readies through the ordinary contracts; it is admitted for its own Station and refused another's by the real policy; it cannot read another pane's projection; a pane and a transport participant hold different Stations on the same running ship; a closed pane hands the lobby the disconnect a dropped phone would; and a pane's identity is in its URL, its document unenumerable, LAN-refused, and withdrawn on close. **#1125:** on a running ship, a view crash flips the seat to Backfill through the ordinary session path; no surviving pane inherits the failed pane's projection; recreating the pane reconnects on the same token and restores its held station out of Backfill with a Welcome; and a lost Station display disconnects its pane without recreating it |
| `tests/native_host_pane_ultralight.rs` | The real built `client/index.html` loads in a real Ultralight view over this process's own HTTP, joins on the identity it read from the fragment, paints, answers a real click + keystroke on `#name-input` with a `SetName`, then claims a Station and operates its console: the iframe mounts with `__updateConsole` installed and a click on the Captain's Red Alert button inside it produces the expected `ControlSystem`. A second test proves two panes' `localStorage` are separate. Both `#[ignore]`d: they need the SDK and a built bundle, which CI has neither of |

Each of the two digest binaries stands alone on purpose: pinning the scheduler
means a one-thread `TaskPoolPlugin`, and Bevy's task pools are process-global and
fixed by whichever app in the process builds first, so a digest claim made in a
shared binary is a claim about whoever won that race.

## Related

- [Build & Deployment](./build-and-deployment.md) · [Networking](./networking.md) · [Architecture](./architecture.md)
- Issue #1121 — the host. Issue #1122 — local Ultralight panes. Issue #1123 — bridge display profiles (above). Issue #1125 — recovering a failed pane and a lost display (above). Issue #1112 — the transport. Issue #1124 — input routing between displays.
- [vellum](https://github.com/jkeywo/vellum) `crates/vellum-ultralight` — the extracted plumbing; `docs/handbook/dependencies.md` records why `ul-next` stopped being a per-game exception
- `pasm/spec/architecture/native-delivery.yaml` — PRD #855's delivery declarations
