---
title: Native Host
type: concept
tags: [native, viewscreen, boot-profile, wgpu, winit, transport, delivery]
sources: [src/native_host/mod.rs, src/native_host/app.rs, src/native_host/transport.rs, src/boot/mod.rs, src/bin/phoenix_host.rs, src/entities/template_preload.rs]
updated: 2026-08-27
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

The operator-visible consequence: **`--solo` is currently the only mode that
reaches a running mission.** Without it the host waits in the lobby, and every
route out of it needs a session — collective `SetReady` auto-start, or the host
page's force-start (`drain_force_start_input`, wasm-only) — so with no transport
there is nothing to wait for. The mode is not refused, because it becomes correct
the day #1112 lands; `build_native_host_app` warns loudly at boot instead
(`LogCat::Lobby`), and `--help` and AGENTS.md carry the same caveat.

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

Each of the two digest binaries stands alone on purpose: pinning the scheduler
means a one-thread `TaskPoolPlugin`, and Bevy's task pools are process-global and
fixed by whichever app in the process builds first, so a digest claim made in a
shared binary is a claim about whoever won that race.

## Related

- [Build & Deployment](./build-and-deployment.md) · [Networking](./networking.md) · [Architecture](./architecture.md)
- Issue #1121 — this page. Issue #1112 — the transport. Issue #1122 — local Ultralight panes.
- `pasm/spec/architecture/native-delivery.yaml` — PRD #855's delivery declarations
