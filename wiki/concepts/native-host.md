---
title: Native Host
type: concept
tags: [native, viewscreen, boot-profile, wgpu, winit, transport, delivery, ultralight, panes]
sources: [src/native_host/mod.rs, src/native_host/app.rs, src/native_host/transport.rs, src/native_host/panes/mod.rs, src/native_host/panes/identity.rs, src/native_host/panes/routing.rs, src/native_host/panes/document.rs, src/native_host/panes/ultralight.rs, src/boot/mod.rs, src/bin/phoenix_host.rs, src/entities/template_preload.rs]
updated: 2026-08-28
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

The pane navigates to `http://<host>/client/pane-<n>.html#native`, a document
this same process publishes in memory (`delivery::serve::HostedDocuments`,
checked ahead of the static bundle). That document is the built bundle's own
`client/index.html` with **three edits**:

| edit | why |
|---|---|
| a classic `<script>` first in `<head>` (`pane_boot.js`) | seeds the session token and participant name the page's own inline script reads *at parse time*, and installs the page→host queue |
| the PeerJS CDN `<script>` tag removed | a pane never uses PeerJS, and a bridge machine may not reach unpkg.com — waiting for that is pure boot latency |
| a `<script type="module">` last in `<body>` (`pane_link.js`) | replaces `window.connectionManager` with the in-process link, which must happen *after* `gui/connection-manager.js` published its own |

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
seam is not bypassed either.

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

### Deferred

- **A real browser participant beside a pane** is issue #1112's, exactly as in
  #1121: `tests/native_host_panes.rs` proves the contract with a
  `LoopbackTransport` participant, which is the seam a network transport plugs
  into.
- **Bridge display profiles** (which pane on which monitor) are issue #1123's.
  Panes are currently tiled evenly left to right across one window.
- **Independent input routing** between panes is issue #1124's. Today it is one
  pointer, one focused pane, the left button, the wheel and text.

## Tests

| File | Claim |
|---|---|
| `src/boot/tests.rs` | Four-profile parity; only the render-stack profiles take that path; a native host refuses a configless boot, including a hull declared only by a static child |
| `src/native_host/transport.rs` | The seam's ingress/egress and the reserved-token refusal |
| `tests/native_host_sim.rs` | Shipped content boots and runs; the hull's own config reaches the client config; an uncached `--ship` is refused; a curating manifest narrows the default hull; a participant joins through the seam |
| `tests/native_headless_digest.rs` | AC5's content half — native↔headless digest equivalence, both apps on a pinned single-threaded pool. **Scope:** the default run is `NativeRenderSurface::Contract`, so it covers the `render: true` *simulation* plugins and not the wgpu stack; the `#[ignore]`d `Offscreen` companion in the same file covers that on a real GPU |
| `tests/native_host_snapshot.rs` | AC5's snapshot half — a native-host capture restores into a fresh native host at the same digest, and the duel continues byte-identically for 120 frames (Combat Test's continuation bound is the payload gap `tests/snapshot_resume.rs` measured, not a native one) |
| `tests/native_viewscreen_render.rs` | The viewscreen draws a scene — not one flat colour, and lit — over the **middle 40%** of a real-GPU frame, so a live HUD over a dead 3-D scene cannot pass. `#[ignore]`d: CI is ubuntu-only with no display |
| `tests/native_host.rs` | The delivery half, unchanged, plus the serving loop's shutdown path (polled to a deadline, so a stuck loop fails rather than wedging the run) |
| `src/native_host/panes/*` | Identity's three refusals, the projection boundary, the outbound cap's reliable/snapshot split, the pane document's three edits, and the per-frame loop's deferral of a failed push. All feature-**off**, so the ordinary `cargo test` CI runs them |
| `tests/native_host_panes.rs` | A pane joins/claims/readies through the ordinary contracts; it is admitted for its own Station and refused another's by the real policy; it cannot read another pane's projection; a pane and a transport participant hold different Stations on the same running ship; a closed pane hands the lobby the disconnect a dropped phone would |
| `tests/native_host_pane_ultralight.rs` | The real built `client/index.html` loads in a real Ultralight view over this process's own HTTP, joins on the host-minted token, paints, and answers a real click + keystroke on `#name-input` with a `SetName`. `#[ignore]`d: needs the SDK and a built bundle, which CI has neither of |

Each of the two digest binaries stands alone on purpose: pinning the scheduler
means a one-thread `TaskPoolPlugin`, and Bevy's task pools are process-global and
fixed by whichever app in the process builds first, so a digest claim made in a
shared binary is a claim about whoever won that race.

## Related

- [Build & Deployment](./build-and-deployment.md) · [Networking](./networking.md) · [Architecture](./architecture.md)
- Issue #1121 — the host. Issue #1122 — local Ultralight panes. Issue #1112 — the transport. Issues #1123/#1124 — display profiles and input routing.
- [vellum](https://github.com/jkeywo/vellum) `crates/vellum-ultralight` — the extracted plumbing; `docs/handbook/dependencies.md` records why `ul-next` stopped being a per-game exception
- `pasm/spec/architecture/native-delivery.yaml` — PRD #855's delivery declarations
