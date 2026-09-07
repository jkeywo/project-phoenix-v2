---
title: Native Host
type: concept
tags: [native, viewscreen, lobby, scenario-selection, boot-profile, wgpu, winit, transport, delivery, ultralight, panes, displays, monitors, bridge-profile, saved-layouts, media-devices, camera, microphone, saves]
sources: [src/world/materialization.rs, tests/native_host_lobby/materialization.rs, src/delivery/payload.rs, tests/native_host_catalogue.rs, tests/client/scenario-catalogue-wire.test.js, src/native_host/mod.rs, src/native_host/direct_join.rs, src/native_host/join_codes.rs, src/native_host/app.rs, src/native_host/world_load.rs, src/lobby/scenario_arbiter.rs, src/lobby/handler.rs, src/content_ledger.rs, tests/fixtures/scenario-arbiter-parity.json, src/native_host/transport.rs, src/native_host/bridge_profile.rs, src/native_host/bridge_layout.rs, src/native_host/bridge_display.rs, src/native_host/layout_store.rs, src/native_host/layout_store_systems.rs, src/native_host/bridge_media.rs, src/native_host/input_routing.rs, src/native_host/panes/mod.rs, src/native_host/panes/identity.rs, src/native_host/panes/routing.rs, src/native_host/panes/document.rs, src/native_host/panes/surface.rs, src/native_host/panes/ultralight.rs, src/native_host/panes/frame_stats.rs, src/native_host/panes/surface_stats.rs, src/native_host/panes/pane_thread.rs, src/native_host/panes/mirror.rs, src/native_host/panes/upload.rs, src/native_host/panes/recovery.rs, src/native_host/host_lobby/mod.rs, src/native_host/host_lobby/document.rs, src/native_host/host_lobby/bridge.rs, src/native_host/host_lobby/reveal.rs, src/native_host/host_lobby/join.rs, gui/host-qr.js, gui/join-url.js, src/delivery/serve.rs, src/boot/mod.rs, src/bin/phoenix_host.rs, src/entities/template_preload.rs, src/delivery/args.rs, src/save_slots_store.rs]
updated: 2026-09-07
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
simulation; with neither `--world` nor `--lobby` the process is byte-for-byte
the delivery host it always was, so the bundle serving, the `--manifest`
catalogue restriction and the startup version pin are shared rather than forked.

Since issue #1326 there is a third invocation — `--lobby` opens the same
authoritative host with **no world**, waiting for a scenario to be picked. See
[The world need not be known at boot](#the-world-need-not-be-known-at-boot).

The native scenario surface and crew messages share
[ScenarioCatalogPayload](../../src/core/messages.rs) with the browser host.
[The catalogue projection](../../src/delivery/payload.rs) carries enriched
curated hulls, each scenario's source (base or pack id), and installed pack
id/name/version rows in load order. Real native shelf installation rebuilds
and broadcasts that snapshot. Identify receives it after selection too, so a
fresh or reconnecting phone keeps the active-pack list with the picker closed.
A direct --world boot has no pre-load catalogue or pre-applied-pack CLI path;
its ordinary Welcome and the phone's default empty pack list are unchanged.

## Where the code is

| Piece | File |
|---|---|
| App builder | `src/native_host/app.rs` |
| Runtime world load + selection arbitration | `src/native_host/world_load.rs` |
| The pure first-valid-wins rule | `src/lobby/scenario_arbiter.rs` |
| The viewscreen's own picker | `src/native_host/host_lobby/scenario.rs`, `gui/host-scenario-render.js` |
| Transport seam | `src/native_host/transport.rs` |
| Content-root pin | `src/native_host/mod.rs` (`pin_content_root`) |
| Boot profile + render surface | `src/boot/mod.rs` (`BootProfile::NativeHost`, `NativeRenderSurface`, `WorldIngest`) |
| Process, argv, threading | `src/bin/phoenix_host.rs` |
| Template preload | `src/entities/template_preload.rs` |
| The one native manifest read + world resolver | `src/delivery/serve.rs` (`ManifestSource`) |
| Private save Store and restore adapter | `src/save_slots_store.rs` |

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

## The world need not be known at boot

```bash
./target/release/phoenix-host --client-dir dist --lobby
```

Issue #1326. The host opens its viewscreen on an **empty
`GamePhase::Lobby`** holding the merged scenario catalogue, and ingests a world
only when a `SelectScenario` + `SelectPlayerShip` pair has been arbitrated —
from a phone, a local Station pane, or (since issue #1328) the host's own
on-screen picker, described under [Picking from the
viewscreen](#picking-from-the-viewscreen-issue-1328). `--world` is the same
decision made at the prompt; the two flags are refused together.

The browser host arbitrates this in **JavaScript** (`gui/scenario-arbiter.js`,
driven by `server.html`) and gets away with it because its Bevy app does not
exist yet when the choice is made: `wasm_init` throws to unwind the JS stack, so
it has to run *after* `wasm_load_world`. A native host has no such escape — its
lobby *is* the viewscreen and the viewscreen is the running `App` — so both the
rule and the ingest had to move inside a process that is already ticking.

| Half | Where |
|---|---|
| The rule (first-valid-wins, hulls scoped to their scenario) | `lobby::scenario_arbiter` — pure, Bevy-free, a deliberate transcription of the JS |
| The catalogue | `delivery::serve::ManifestSource::merged_catalog`, the same `build_merged_catalog` call `wasm_get_scenario_catalog` makes |
| The Bevy adapter | `native_host::world_load` — selection drain, exclusive load and post-selection catalogue greeting, `.after(LobbySystemSet)` and `.before(SimSet::Input)` |

The two implementations of that rule are held together by
`tests/fixtures/scenario-arbiter-parity.json`, one case table read by both the
vitest suite driving the JS and the Rust module's own tests. It exists because a
transcription held together by a doc comment had already drifted: the JS's
`normalizeSelection` coerces a falsy field to `null`, so `''` is not a lock — a
rule the Rust port had omitted, which made an empty id lock a native host that a
browser host would still treat as open.

`--ship` given with `--lobby` **satisfies the hull half** of the selection: a
scenario lock alone completes the pick, and the published catalogue reports the
pinned hull as locked. An explicit hull already outranks whatever the arbiter
locks, so the alternative was making a scripted run wait for a
`SelectPlayerShip` whose content cannot matter, and showing phones a picker
offering a choice the host has already overruled.

That report resolves in the **same precedence order the load does** — pinned
first, arbitrated second. The one combination where the two answers differ is
exactly the one the field exists for: `--lobby --ship B` with a phone picking A.
Resolved the other way round the host flies B while telling every phone the
locked hull is A.

### Why the runtime load is the boot load

The claim rests on reuse, and the guards are tests rather than comments — all in
`tests/native_host_lobby.rs`:
`a_runtime_load_mints_the_same_world_entity_ids_as_a_boot_load` compares
`(who, uuid)` pairs and the world's own `name -> uuid` map, not a bare id list
(a bare list is a multiset, so the two spawn passes swapping places permutes ids
between entities and still compares equal), and
`tests/native_host_lobby/materialization.rs` compares the shared registration's
membership and actual composed-world state after different lobby delays. It
checks the selected hull and roster, layer order, scoped flags and trigger
continuation, and the next live mint sequence after a deferred load.

- **`boot::ingest_world` takes a `&mut World`, not a `&mut App`**, precisely so
  both callers are the same function: reset → read → validate → compile →
  abort-on-broken → native template gate → apply → eager record → freeze →
  insert `WorldConfig` + `PreCompiledScripts`, once, in one place.
  `WorldIngest::Deferred` is the "no world yet" mode; it runs only the Rhai
  hashing-seed pin and deliberately does **not** freeze — freezing seals the
  content digest for a world that is not there.
- **`app::install_world_selection`** is the extracted seed precedence, hull
  resolution, hull template-cache gate, #935 hull re-record + re-freeze,
  `PendingShipConfig` and canonical `SelectedShipResource`.
  `build_native_host_app` calls it too.
- **`world::materialization::register`** supplies the same ordered systems to
  `Startup` and `RuntimeWorldLoad`: compilation, anonymous setup, named/asteroid
  spawn, runtime initialization and queued supporting layers. They remain in
  the caller's schedule, preserving other plugins' function-identity edges.
  Native hull/session refresh and radar replacement follow `WorldMaterialization`.
  GameStart roster completion and render-only startup remain separate. The
  runtime pass does not manufacture the durable GameStart marker used by startup
  restore; tests compare GameStart and layers at matching logical start ticks.
- **`WorldIdMint` is parked at tick 0** across that pass and the live mint
  restored after. `begin_tick` resets a namespace's sequence only when the tick
  *moves*, so without parking the same authored world would mint different
  uuids depending on how long the operator spent choosing — and those uuids are
  folded into the digest **by name**, key a snapshot's entity matching, and in a
  fleet have to agree across hosts. Restoring rather than leaving the mint at
  zero is what stops a later id colliding with a world entity's.

What is *not* claimed: that a runtime-loaded host reaches `InProgress` on the
same tick a `--solo --world` host does. It does not, and never could — the
mission starts when the crew ready up, which is already true of every crewed
boot. `spawn_game_start_entities` mints on the phase-transition tick either way.

### What the participants are told

A world landing is not a private event. A phone that identified **before** the
pick was welcomed by a host with no world, so its roster came from
`load_ship_config_from_disk`'s battleship fallback and its client config was a
`Default` — the hull this host is not flying. So a successful runtime load
re-publishes a fresh `Welcome` (built by the one `handler::welcome_message`, plus
the `ShipManual` that always accompanies one) to every connected participant, and
clears the seats first because the roster is being replaced wholesale — the same
four lines `handle_return_to_lobby` runs when issue #756's round two picks a new
hull (ready flags, seats, pending ratings, eligibility reports), plus its
per-player `ReadyChanged { ready: false }`. The ready flags are the easy one to
leave out and the one with teeth: `all_ready` ignores seats entirely, so a flag
set against the pre-load fallback survives a seat-only wipe and could start a
mission with a seatless crew. The shipped phone client cannot reach that state
today — it readies from a console it has already claimed — but the claim here is
parity with `handle_return_to_lobby`, and parity means all four.

The browser host gets that refresh for free and therefore never needed the code:
its phones are welcomed by a Bevy app that does not exist until `wasm_init`, i.e.
until after the world is loaded. `gui/lobby-state.js`'s fully-locked-catalogue
branch — which leaves the picker and resumes the lobby without a new `Welcome` —
is issue #756's round-two world **reuse**, where the roster genuinely has not
moved; a native runtime load is round one. Without the re-`Welcome` the crew get
consoles mounted for the wrong hull with the wrong authored numbers, and
`handle_select_station` (which validates against the *real* roster) silently
ignores every seat they tap.

A second `Welcome` is safe by construction: `replaceFrom` replaces rather than
merges, and the `Welcome` arm emits `MOUNT_CONSOLES` unconditionally.

### A refused world leaves a pickable lobby

A load that fails **after** the ingest (an unreadable or malformed world, an
uncached `--ship`, a hull with no `[[station]]` blocks, a participant pane name
that shadows one of that hull's station ids) puts the lobby back:
`WorldConfig` and `PreCompiledScripts` are removed — nothing has spawned at any
failure point, and leaving them behind would give the operator a host holding a
world it never built and deaf to every later pick — the arbiter's lock is
released, the catalogue is re-published, and the **content ledger is reset**.
That last one matters because `ingest_world` froze it over the refused world's
file set and `install_world_selection` froze it again after re-recording the
hull, both *before* the failure points; left alone, `content_ledger::frozen_or_live`
would go on answering for a world this host does not have — and that is what
`snapshot::versions` answers a fleet peer's content check with and what a save is
bound to. `reset` is the whole undo rather than a restore because `ingest_world`
opens every attempt, including the next successful one, with exactly that reset.
`--world` has none of these cases; it reports at the prompt and exits.

The two paths are **not** equal in what the operator sees, and it is worth
knowing which: `--world` prints the refusal in the terminal it was launched from
and exits 1, while `--lobby` writes one `perror!` to the operator log and simply
returns the lobby to pickable. The refusal itself crosses to **no** surface —
phones see the catalogue come back with nothing locked, and the host's own lobby
surface sees the same. `LayoutNotice` is not a route for it: that channel carries
the bridge *layout's* refusals, and a world that would not load is not a
statement about a monitor. Saying it on a surface needs a message this protocol
does not have.

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

The native process also installs a peer-private `vellum_save::FileStore`, by
default under `.phoenix/saves` relative to its launch directory. Before any
catalogue or resume read it claims that directory through a persistent,
non-`.ron` lock sentinel held for the process lifetime; another native host
pointed at the same directory is refused at startup and needs a distinct
`--save-dir`, and shutdown releases the lock without deleting the sentinel. Its
list, create, rename, export,
confirmed-delete and startup-resume switches are
documented under [Peer-Local Save Catalogues](./save-catalogues.md); resume
builds a new App with the saved hull/fleet ship topology, deliberately without
rejoining the old mesh, and is never a live World mutation.

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
transport arrives beside the panes: an `insert_resource`, not a re-plumb. Since
issue #1353 there are three possible legs (panes, direct LAN accept, the cloud
relay), and `phoenix-host` folds whichever it has into one
`Box<dyn NativeTransport>` rather than spelling out eight combinations.

## The host is its own rendezvous (issue #1353)

```bash
./target/release/phoenix-host --client-dir dist --lobby     # …and that is all
```

A phone loads the client bundle from the host's delivery port. Since #1353 it
opens its **game socket to that same port** as well: a request on `/v1/join`
carrying a WebSocket upgrade is taken off the HTTP path and answered in process.
A LAN game therefore needs **no external service at all** — no worker deployed,
no `wrangler dev`, no internet.

Both host types dialled out to a meet-in-the-middle relay before this, and that
shape was inherited rather than chosen: a **browser** host cannot accept an
inbound connection, so two browsers can only meet at a third party. A native
process can accept — it is already accepting HTTP on that port — and #1121
deferred exactly this leg.

**The door.** `delivery::serve` grows one seam, a `ConnectionUpgrade` handler
consulted before routing. Detection is pure and unit-tested
(`websocket_upgrade`): a plain `GET /v1/join` gets the worker's own `426`, a
malformed upgrade a clean `400`, and everything else routes as HTTP exactly as
before. The handshake's `101` is written by `direct_join` itself with
`tungstenite::handshake::derive_accept_key`, and the socket wrapped with
`WebSocket::from_raw_socket` — because `serve` has *already read the request
head* (that is how the path and the key were known), so there is no handshake
left for `tungstenite::accept` to read and std cannot peek a socket portably.
The head read also grew a timeout to go with its size cap: a connection thread
must not be holdable for free.

**Not a second host implementation.** `native_host::direct_join` is a
`RelaySocket` — the same trait `relay_socket.rs` implements over a real
`wss:` connection — so `RelayTransport` sits on top of it unchanged and
registration, the in-band stamp handshake, the `Identify` gate, the
reserved-token refusal, the duplicate-token sever, audience resolution and the
shedding rule are all the same code on both legs. What the module adds is the
**service** half: the single-game subset of `worker-rendezvous/src/registry.js`,
with the same typed code lookup and its three distinct failures
(`wrong-type` / `version-mismatch` / `unknown`), the same protocol-version
refusal, the same per-connection lookup cap and peer bounds, and the same class
contract — reliable ordered and never shed (a full queue ends that session),
snapshot latest-wins. A phone cannot tell which leg answered it, which is the
whole point.

What collapses relative to the worker: no `/v1/host` socket (the host *is* this
process), one record instead of a map, and no code rotation or reclaim grace — a
record whose host has gone is a process that has exited. What does **not**
collapse is any validation.

**No `Origin` allow-list on this leg, deliberately.** The worker's gate is right
there — a public multi-tenant service on a different origin from every page it
serves — but here the page and the socket are the same origin by construction,
so a list would have to name every address and hostname a phone might reach the
machine by, and getting it wrong refuses the crew with nothing on screen saying
why.

What makes that decision sound is the gate that stands in its place, and it is
**attempt-limiting, not an origin list**. A hostile page open in a crew member's
browser dials this port with its own site's `Origin`, so a header list would
never have stopped it; the join stamp is public, because this host serves it;
and that leaves the code as the only secret between a stranger on the LAN and an
acting participant. So the code is defended at the transport, in three layers
(`AdmissionBudgets` in `direct_join.rs` carries the numbers and the arithmetic):

1. **Budgets that survive reconnection.** `max_lookups_per_connection` is
   charged to a socket, and a socket is exactly what a guesser throws away — an
   unbudgeted door answered wrong codes in the high hundreds to a few thousand
   per second, depending entirely on the machine the probe ran from. So the
   failed-guess budget is keyed on the peer address read at accept and held by
   the *record*: a token bucket, 20 wrong guesses burst and one refunded every
   5 s, alongside a per-source cap on sockets that never join (4 of the 16 the
   whole service allows — a quarter rather than a half, so that *two* addresses
   cannot hold every slot, which IPv6 privacy addressing would make free). Only
   **failed** lookups are charged, so a correct code costs nothing — a crew
   behind one NAT address is spending a typo allowance, never a join allowance.
2. **A global circuit-breaker.** Wrong guesses in a rolling minute ramp a delay
   onto every lookup answer, correct ones included (answering a right code
   faster during an attack would be a timing oracle), capped at 2 s — inside the
   client's own 8 s first-connect timeout, so a guest caught in an attack waits
   once and gets in. It is the layer that covers a guesser spread across more
   addresses than the per-source table can hold, and it is *asserted* as a
   composite: the whole service, driven from an address per guess, evaluates
   **8.2 wrong codes a second** against a stated ceiling of
   `unjoined_total / breaker_max + breaker_free / breaker_window` = 8.5, and
   ~10.2/s through the ramp's own first window before it saturates.
3. **Enough letters.** The authored suffix is eight, not five: 25^8 ≈ 1.5 × 10^11
   (37.15 bits), about 377 days of expected search at the *old* unthrottled
   rate, where five letters (25^5, 23.2 bits) fell in about 35 minutes. At the
   composite rate the two layers above allow, it is **centuries**.

Every refusal is **soft** and says so — a stated reason in band, a `Retry-After`
on a refused upgrade, a bucket that refills on a clock — because a whole crew
can share one address, and a lockout that did not lift would be a self-inflicted
outage waiting for one clumsy typist. On top of all of it, the compatibility
handshake and the reserved-token gate still decide what a joiner may *be*.

**The residual, stated rather than defended against.** A guesser with many
addresses is not slowed by layer 1 at all, and layer 2 answers it on the one
resource that cannot be multiplied — the un-joined socket. What such a caller
*can* still do is hold that budget, so a crew member's upgrade meets
`join-sockets-busy` and a `Retry-After` for as long as the attack runs. That is
lockout **pressure** during an active attack rather than a lockout: every slot
is reclaimed within the 30 s join deadline whether the caller cooperates or not.
The alternative — refusing addresses the source table cannot track — is a
venue-NAT outage bought to defend a port that already serves the whole client
bundle to anyone on the LAN who asks.

**Silence has its own budget.** A socket that upgrades and never joins is
counted against `AdmissionBudgets::unjoined_total`, not the authored
`max_peers_per_record`: a hostile device holding thirty-two silent sockets used
to refuse the room with `join-sockets-full`. The thread ceiling this leg can
reach is therefore peers *plus* un-joined, and both directions of a joiner
socket are time-bounded — the write timeout is what stops a joiner that stalls
its own reads from parking a host thread inside `send` for as long as the OS
will hold a full buffer. The socket's WebSocket config is sized from
`max_relay_frame_bytes` too, rather than left at `tungstenite`'s 64 MiB message
/ 16 MiB frame defaults, which were buffered and *decoded* before any budget was
consulted.

**Every state a socket can be in has a clock.** `JOIN_DEADLINE` is gated on
`!joined`, so a socket that resolved the code used to be reaped by nothing at
all: its read loop returned `WouldBlock` for ever and it held one of
`max_peers_per_record`'s places until the process exited. Thirty-two such
sockets fill the room. The three clocks now cover the whole life of a
connection:

| State | Clock | Value |
| --- | --- | --- |
| upgraded, not joined | `JOIN_DEADLINE` | 30 s |
| joined, never opened its relay | `attach_deadline` | 30 s |
| attached | WebSocket ping/pong | ping after 10 s quiet, detach after 30 s unanswered |

A deadline cannot answer the attached case, because an attached console is
legitimately silent for as long as its player is — so the host asks, with a
WebSocket Ping that every client answers inside its library rather than in its
own code. What that closes is the room's real failure: a phone dropping without
a FIN (out of range, a venue AP losing the association, a battery gone
mid-frame) leaves a **half-open** TCP, so the seat is held by nobody, and a bad
night accumulates them until the game is locked out with nothing on screen
saying why. Every reap goes down the ordinary departure path, so the seat flips
to Backfill and a returning phone's own reconnect yields it straight back.

**The client half is one rule**: `gui/join-url.js`'s
`rendezvousBaseForOrigin` — *the service that served you the page is the service
you dial*, unless the page came from one of the published browser-game origins
(`KNOWN_WEB_ORIGINS`, the twin of `worker-rendezvous/wrangler.toml`'s
`ALLOWED_ORIGIN`, pinned by a test that reads that file). Those are static hosts
that cannot accept a socket, so they keep the cloud service. It is a default
rather than a parameter, which supersedes #1336's `?rendezvous=` posture
question for the served-by-a-host case: no link can point a guest's join
anywhere. One caveat worth knowing, because `localhost:8080` is on that list as
`trunk serve`: a browser opening a native host at `http://localhost:8080` dials
the cloud service. Every phone gets the LAN address from the QR instead.

**Where the code comes from.** `native_host::join_codes` reads
`assets/join/join-codes.toml` — the authored table `gui/join-code.js` and the
worker already read — and mints from it. `core::rendezvous` says plainly that a
host does no minting and no parsing, and that stays true of the module: a host
that is its own service is the party that mints, so the two operations live in
the native-only module that needs them.

**Composition.** `--rendezvous` still works and both legs run at once; peer ids
this leg mints are prefixed so the namespaces cannot be confused, each leg keeps
its own peer table, and a `Disconnected` routes back through the leg that saw the
socket die. The viewscreen QR is the direct code — see [The join
QR](#the-join-qr-issue-1329).

`tests/native_direct_join.rs` drives a real `tungstenite` client through a bound
host: handshake, code, attach, stamp verdict, `Identify`, both delivery classes,
and the socket dying as exactly one `Disconnected`. It needs no service running,
so unlike `tests/native_relay_live.rs` it is **not** `#[ignore]`d.

It also runs the attack, rather than asserting that the limits exist. The same
churned-connection probe fires wrong codes at two hosts differing only in their
budgets, and the assertion is **relative** — a collapse to the burst plus what
refilled, with the rest refused in band — precisely because the absolute figure
is the prober's own machine talking: two runs on different hardware measured
1,856 → 20 and 411 → 20, and both are the same result. A real crew member with
the right code joins **from the same loopback address the guessing is coming
from** while it is going on (a few hundred milliseconds, again machine-dependent
— what is asserted is that it is inside the client's own 8 s connect timeout),
which is the property the soft limits are for and the one an over-eager lockout
would have quietly broken.

Loopback is one address, though, so the real-socket tests can only prove layer 1
plus the breaker's *shape*. The composite ceiling the design rests on is
asserted at the pure layer instead, driving the real `Admissions` from a
distinct synthetic source per guess with the sockets' waiting simulated
(`the_composite_guess_rate_holds_however_many_addresses_it_is_spread_over`), and
one real-socket test times the breaker's delay actually being taken out of a
guesser's thread before an answer is written. The liveness clocks have their own
three: a joined-then-silent hoard giving the room's places back, a peer that
stops answering its pings being detached, and a quiet peer that *does* answer
being left alone.

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

Host→page is `window.__phoenixPaneApply('<json>')`; page→host is a queue
drained once per pane iteration with `window.__phoenixPaneOutDrain()`. Both
directions carry the same JSON a phone would send or receive, decoded by
`core::codec`. Each iteration pushes at most `surface::MAX_PUSHES_PER_FRAME`
messages per pane and requeues the rest.

### The pane thread (issue #1404)

`pane_thread::spawn_pane_thread` owns the Ultralight runtime and every view on
the named `phoenix-panes` thread. The factory crosses the channel boundary;
the runtime and views remain on their owning thread, including teardown.
Startup waits for `Started` before creating Bevy seats. The thread drains
commands, drives one iteration, and applies arriving commands during its wait
to the 16 ms deadline. Slow iterations run at the renderer's available cadence.

`PaneHost` is an ordinary Send resource holding a
`PaneMirror<PaneCanvasData>`, routing state and the thread handle.
`drive_pane_host` accepts only frames at the mirror's current epoch and
queues persistent texture uploads. The measured pool remains three buffers
per pane, capped at four returns; resize replaces both the pool and its return
channel, so old frames cannot refill a new generation. Mouse capture is
released when its pane closes.

Input and lifecycle operations are asynchronous. A failed seat creation uses
the console's existing bounded recovery; camera cleanup checks whether another
seat shares it. Lobby actions round-trip one pane period plus a Bevy frame.
A resize can show its new texture before the first whole frame arrives.
Hidden lobby and HUD surfaces keep receiving state but stop copying frames.
The gamepad slot keeps an empty snapshot after unplug until the next pad
state replaces it, so coalescing cannot hide a disconnect from page JavaScript.

A renderer failure closes every console, including one still awaiting creation,
drops permanent canvases, and leaves the simulation running without thread
respawn. Unwind reporting is available in dev builds; release uses
`panic = "abort"`. `AppExit` and handle drop request shutdown and wait at most
two seconds; an unresponsive renderer is left on its own detached thread.

**Measuring frames and iterations (`--frame-stats`).** A windowed host run
with `--frame-stats --log info` logs one line a second from
`src/native_host/panes/frame_stats.rs`: Bevy frame timing, the pane thread's
five phases and copied pixels per iteration, iterations/s and ms/iteration,
main-world event drain and upload queueing per frame, stale frames, GPU upload
counts and bytes, changed Image assets, fixed ticks and their cost.
Every thread sample is accumulated. The residual subtracts main-world pane
cost and the fixed loop; concurrent pane-thread work is reported separately.

The upload counters changed meaning with issue #1404. `image assets changed`
used to be the pane count, and each such event **was** a full GPU texture
re-creation and bind-group eviction on the render thread — that was the whole
reason to measure it. It no longer counts any pane: a pane image is minted
`RenderAssetUsages::RENDER_WORLD` and is never written from the main world
again, and a copied frame reaches the GPU as a `write_texture` of its dirty
rectangle into the texture bevy_ui is already sampling
(`src/native_host/panes/upload.rs`). The pane's share of that line is what
`uploads` is now, and `lost` — a copy skipped for want of a staging buffer, or
a frame the render world had to drop — is the number that should read 0.0.

**Raw surface attribution.** `src/native_host/panes/surface_stats.rs` records
bounded integer-nanosecond events when `PHOENIX_SURFACE_CAPTURE` is set, or as a
`PHOENIX_FRAME_CAPTURE` companion. It separates HUD slot revisions from actual
applications, identifies full-copy causes, and follows each owned buffer's
original surface metadata through drain, extract, deferral and upload/disposal.
Global SDK update/render stay aggregate. Forced-copy dirty pixels are unknown,
and `write_texture` measures host enqueue time. Truncation and frames still in
flight at closure are explicit. The capture owner lives outside App; no worker
join or per-frame file write is required. See
[the capture and interpretation notes](../../docs/acceptance/1405-surface-attribution.md).
This observer does not suppress repeated HUD or hidden DOM applications.

The `PHOENIX_FRAME_EXPERIMENTS` variable (`novsync`,
`raf33`) switches one suspected cost off per run so the lines can be
compared; the toggles are scaffolding for the multi-screen
frame-rate investigation and go once the fixes land. Every clock read is
presentation time, in `Update` or around the fixed loop —
`tests/native_headless_digest.rs` stands guard that none reaches authoritative
state.

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
than dodged — the fragment leads with `document::PANE_JOIN_CODE`, a typed
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

`drive_pane_host` closes a pane whose page stopped draining, and the *view* goes at
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
  the section below. It opened the Station windows and computed each pane's
  rectangle; **compositing a pane onto its assigned Station window** and routing
  input to it landed in issue #1124 (see "Input routing" below).
- **Independent input routing** between panes landed in issue #1124: one mouse
  traverses every surface, keyboard focus moves between panes with a visible
  non-colour indicator, and each touch contact is captured by the pane it began
  on. See the section below.

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
| The pure layout **law** — assign/move/unassign, refusals, eligibility | `src/native_host/bridge_layout.rs` |
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

A profile that assigns monitors but names **no** `viewscreen` is refused too
(`ProfileError::MissingViewscreen`, issue #1327): the viewscreen role is what
places the process's primary window, so without one that window stays wherever
the OS opened it — on a monitor a `station` entry may also name, covering the
shared view with a console. A profile with **no** `[[display]]` tables at all
(the `[[touch]]`/`[[media]]`-only shape) opens no Station window and stays valid.
A `[[display.pane]]` may also carry an optional `station = "…"` naming the
station id whose console it shows; it is absent for a hand-authored
`--pane <NAME>` pane, so pre-#1327 profiles round-trip byte-identically. Two panes
naming the **same** station are refused (`ProfileError::DuplicatePaneStation`): a
station has one console, so a file seating it on two screens leaves nothing to say
which screen it opens on.

The profile is **not** the private player Accessibility profile (#1127): this is
shared operator configuration of the physical room, carrying nothing about any
one player, and the two are kept in separate files.

### The layout law (issue #1327)

`bridge_profile` judges a **file**; `bridge_layout` judges a **transition** — the
one place the three arrangement rules live, so the lobby's buttons, a saved
per-ship-class layout and a CLI profile cannot disagree:

1. exactly one viewscreen (a `BridgeLayout` is built with one and can only move
   it — it is never `None`);
2. a station's console never opens on the viewscreen's monitor, and the
   viewscreen never moves onto a monitor holding consoles (refused, never a
   silent eviction — unassign them first). A console a hand-authored `--pane`
   profile opened counts, even though the layout cannot move it: those labels go
   into a **reserved** set (`reserved_on`), because they seat no `StationId` and
   the screen would otherwise read as free (issue #1330);
3. at most `MAX_STATIONS_PER_MONITOR` (= `MAX_PANES_PER_STATION`) consoles per
   screen, split along that screen's own axis — the `split` a `--profile`
   authored for it (`split_on`), or `LAYOUT_SPLIT` (side by side) for a screen no
   profile named.

`BridgeLayout::apply(&self, &LayoutAction)` answers a **new** layout or a
`LayoutRefusal` and never mutates its input; `SetViewscreen` / `AssignStation` /
`UnassignStation` are the whole vocabulary (assigning a seated station *is* the
move). Stations are keyed by `StationId`, never by participant name — a console
on a wall monitor is claimable by anyone — and that id survives persistence in
the pane slot's `station` field. `occupancy()` and `eligibility()` report enough
to grey a button row with no further logic: every monitor is `Selected`,
`Eligible`, or `Excluded(IsViewscreen | Full)` (and `free_slots` is `None` for the
viewscreen, not `0` — it takes no console rather than being full). `to_profile` /
`to_validated_profile` / `write_displays_into` / `adopt_profile` convert to and
from a `ValidatedProfile`, routing every adopted seat through the same law so a
saved layout cannot smuggle in an arrangement a button press could not make;
anything that will not fit is reported as a `LayoutAdoption` rather than dropped.
`to_validated_profile` is infallible: a lawful layout cannot write a profile
`validate` rejects, so no consumer is handed an impossible error.

An action that asks for what already holds is a **no-op**, not a refusal — all
three of them: naming the current viewscreen, seating a station on the monitor it
is already on, and closing a console that is not open. Two of those are reachable
without a double-press (an off button pressed by two clients at once), so refusing
them would make a race look like a fault. An action naming a monitor this bridge
lacks or a station off its roster is still refused: that is a stale button.

`BridgeLayout::reconcile(monitors, roster)` rebuilds a layout when the bridge
changes underneath it — a screen unplugged, a different ship class. It is never an
error and never silent: surviving seats keep their monitor and their order, a
station whose monitor or station id vanished degrades to unassigned, and the
viewscreen is **kept** when its own monitor survived (even off the primary — that
was the operator's choice) or falls back to primary-else-first with a
`LayoutAdoption::ViewscreenMonitorGone` note when it did not.

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

### Pane compositing onto Station windows (landed in #1124)

Compositing an Ultralight pane onto its assigned Station window was #1123's
deferred continuation, and it landed with #1124's input routing (below). This
slice's contribution is the seam: `BridgeStationSurface` now carries the
monitor's `MonitorGeometry` (scale and desktop position) alongside its window
entity and pane rectangles, which is what the pane host needs to composite each
pane at the right physical size and route input in the monitor's own coordinate
space. A `--profile` launch no longer tiles its panes on the viewscreen — each
pane the profile names a Station slot for is rendered on that Station window by a
per-Station 2-D camera; a pane the profile leaves unseated still falls back to
viewscreen tiling, so a mixed launch never leaves a pane with nowhere to draw.

## Bridge media profiles (issue #1126)

A bridge is a room of media devices as well as monitors: the cameras,
microphones and speakers the crew talks to the rest of the fleet through. Issue
#1126 is the **media-device analogue of the display profile** — each bridge
*surface* (the shared viewscreen, and each named Station, Comms above all) is
assigned a camera, microphone(s) and audio output(s), written in the **same**
`BridgeProfile` TOML the `[[display]]` roles live in.

```toml
version = 1

[[media]]
surface = "viewscreen"
camera = "camera:Logitech BRIO"
microphone = ["mic:Blue Yeti"]
output = ["output:Bridge Speakers"]

[[media]]
surface = "comms"
microphone = ["mic:Headset Boom"]
output = ["output:Comms Headset"]
allow_shared = []          # a device on two surfaces must be listed here on both
```

| Piece | File |
|---|---|
| The pure model — kinds, identity, validate, resolve, default, report | `src/native_host/bridge_media.rs` |
| The `[[media]]` field + the folded validation | `src/native_host/bridge_profile.rs` (`BridgeProfile::media`, `ProfileError::Media`) |
| The `--setup` media section | `src/native_host/bridge_profile.rs` (`render_setup_report_with_media`) → `bridge_media::render_media_setup_report` |
| The acceptance kit | `docs/acceptance/1126-media.md` |

The **pure model is Bevy-free** (`bridge_media`), exactly as `bridge_profile` is,
and is tested by the ordinary `cargo test` CI runs. Media assignments validate in
the **same pass** as display roles (`BridgeProfile::validate`), so a bad
assignment fails at the prompt just as a bad display role does —
`ProfileError::Media` wraps the `MediaError` taxonomy.

### Device identity, and its limit

A device's stable identity is `kind:name` (`identify_media`) — the kind tag
(`camera` / `mic` / `output`) and the OS device name — so an operator reads it and
knows what it is, exactly as `name@WxH` does for a monitor. **Encoding the kind
into the identity is load-bearing:** it is what lets validation reject a
microphone dropped into the camera slot as a pure string check, before any device
is opened. Two devices of the same kind and name are indistinguishable by name and
fall back to a suffix — `kind:name#<hardware-id>` when the OS gives one, else
`kind:name#<ordinal>` in enumeration order — the same honest limit the display
scheme has.

### Validation, sharing and resolution

`validate_media` is the failure taxonomy: a wrong-kind device in a slot
(`WrongKind`), an id with no kind tag (`MalformedId`), the same device twice on
one surface (`DuplicateDevice`), two surfaces of one name (`DuplicateSurface`),
and — acceptance criterion 2 — a device assigned to two surfaces without **every**
surface consenting via `allow_shared` (`SharedWithoutConsent`). A *consented*
share is allowed and returns a `MediaWarning::Contention`, because one device
rarely captures or plays for two surfaces at once and the OS may refuse the second
open. `resolve_media` matches assignments against the devices actually present:
a **missing** device (`MediaProblem::DeviceMissing`) or a present-but-**denied**
one (`DeviceDenied`) is named and its slot left empty, and the surface keeps every
device it *does* have — a missing, removed or denied device never makes the
Station unusable (acceptance criterion 4). A present device assigned to no surface
is not a problem; a bridge need not use every device it can see.
`default_media_assignment` is the deterministic default when the operator has not
chosen: the OS default (else first) of each kind per surface, consenting to the
forced share on a one-device box so the generated default validates.

### The setup surface, and the backend gap

`--setup` gains a media section (`render_media_setup_report`): it lists devices by
kind and validates the profile's assignments, reporting missing/denied devices and
contention. But **there is no OS media backend in the tree.** The display profile
gets its real enumeration free from Bevy's `Monitor`; there is no equivalent
already-present source for cameras, microphones and outputs, and no native media
crate is a dependency here. So `--setup`'s media half validates hand-authored
assignments and prints that no backend is compiled in (`enumerate_note`). Wiring a
real backend — `cpal` for microphones/outputs, a camera crate such as `nokhwa` (or
Windows Media Foundation) for cameras — behind the `host`/`ultralight` feature
seam is the winit-adapter analogue, out of the CI default build exactly as the
real-monitor surface is. Live enumeration, camera preview, mic metering, output
tone-test and real capture/play are the acceptance kit's Part B — deferred until
that backend lands, so acceptance criterion 3 stays parked there, the way #1124's
touch criterion 6 stays parked on real touch hardware.

This profile is **still not** the private per-player Accessibility profile
(#1127): it is shared operator configuration of the physical room, carrying
nothing about any one player.

## Input routing (issue #1124)

Across a configured multi-monitor bridge, one mouse must traverse every surface,
keyboard focus must be explicit and visibly indicated, and independent
touchscreen contacts must land on — and stay captured by — the pane where each
gesture began. The **routing logic is pure** and lives in
`src/native_host/input_routing.rs`, Bevy-free and CI-tested; the winit/Ultralight
adapter that feeds it real events is the feature-gated
`panes::ultralight` (`--features ultralight`).

| Piece | File |
|---|---|
| The pure model — router, focus order, contact capture | `src/native_host/input_routing.rs` |
| The winit/Ultralight adapter — per-window events → the model → the views | `src/native_host/panes/ultralight.rs` (feature `ultralight`) |
| The acceptance kit (mouse/keyboard now; touch when hardware exists) | `docs/acceptance/1124-input.md` |

### The three pure models

- **`PaneRouter`** — a flat list of `PanePlacement`s (a pane, the window it is
  composited on, its rectangle in that window's physical pixels, the window's
  desktop origin, and the scale factor). `resolve_in_window` routes an event a
  window delivered in its own coordinates; `resolve_desktop` routes a
  device-global physical point (a touchscreen the profile maps to a monitor by
  position). Both return the pane and the pointer position in that pane's **own
  logical (page CSS) pixels** — `(physical − pane_origin) / scale`, which is the
  whole of "display scaling". `project_into_pane` is the capture path's
  projection: it maps a point into a *specific* pane without the containment test,
  so a pinned contact's drift past the pane edge is a legitimate drag, not lost.
- **`FocusRing`** — the keyboard-focus order over the panes and which one holds
  focus. Traversal cycles; `sync_order` reconciles the order when a pane opens or
  closes and **clears** focus if the focused pane went, never carrying one
  participant's keystrokes onto whoever inherits its slot.
- **`ContactCaptureMap`** — a touch pinned to the pane its first point resolved
  to, routed there for its whole life however far the finger drifts; simultaneous
  contacts are independent because each is one entry keyed by its own id.

### The adapter, and its decisions

- **One mouse, every window.** The OS moves the cursor across the extended
  desktop; exactly one window reports a `cursor_position` at a time. The adapter
  converts it to physical, asks the router which pane it is over, focuses that
  pane, and injects the move, buttons and wheel into its view — one pointer
  operating every surface with no test-only mode.
- **Keyboard focus is Ctrl+Tab / Ctrl+Shift+Tab**, deliberately not plain Tab: a
  console has real form fields and plain Tab must stay the page's own
  field-to-field traversal. Ctrl+Tab is the convention for moving between panes
  and no console binds it. A bare Tab is forwarded to the page as text; Ctrl+Tab
  is consumed as a focus command.
- **The focus indicator is a ring plus four corner brackets** drawn on exactly
  the focused pane. Focus is shown by the *presence* of that shape, not by a
  colour change — so it does not rely on colour alone (WCAG 1.4.1): an unfocused
  pane has no ring at all, so no colour discrimination is needed to tell focused
  from unfocused. Its geometry is documented `FOCUS_RING_*` constants (thickness,
  bracket length, inset), not tunables. The reticle is re-homed onto the right
  window's camera by despawn-and-respawn when focus crosses windows.
- **Touch maps to pointer events per contact.** Ultralight has no multi-touch
  surface, so each contact is expressed as a down / drag / up on its pinned
  pane's view. Two contacts on *different* panes are genuinely independent; two on
  the *same* pane share the one pointer — the honest limit of this mapping, and
  why the kit's simultaneous-touch part is the hardware acceptance. The adapter
  treats `TouchInput.position` as window-logical (the same space as
  `cursor_position`) and converts to physical for the router; the id-stability
  assumption is that winit carries one stable contact id from down to up, which
  the kit verifies on real hardware.

### MVP vs. full

The pane→Station-window **compositing is the honest MVP** the issue sanctioned:
each pane is drawn on its Station window via a per-Station 2-D camera and receives
that window's input, and the pure routing model handles arbitrary multi-monitor
geometry. What is **not** verified in code on the dev machine is multi-monitor and
multi-touch behaviour — the box has one monitor and no touch — so those are the
acceptance kit's, and the pure tests carry the logic. A Station window whose panes
have all closed has its camera despawned; the window itself stays
`bridge_display`'s to own.

## Operator profiles in native panes (issue #1280)

A pane consumes the same `project-phoenix/operator-profile` v1 JSON as a phone.
`gui/operator-profile.js` remains the only schema, migration, persistence and
export owner, and the page's existing `localStorage` path stays private because
each Ultralight pane already runs in its own session. There is no native profile
file and no setting crosses the simulation transport.

`pane_boot.js` declares the small capability difference before the shared client
modules load. Keyboard remains available through #1124's focused
`input_routing` adapter, while Gamepad API sampling is unavailable; Accessibility
continues through #1127's injected OS defaults and the page's existing
`applyAccessibilityProfile`. Ultralight also has no vibration backend. The
shared `gui/operator-surface-adapter.js` therefore distinguishes the retained
portable profile from its active projection: bindings, tuning, Accessibility
and semantic-cue choices apply normally, while a preferred gamepad slot or
enabled vibration choice remains in JSON/local storage but is inactive and is
reported explicitly in Settings. Exporting from the pane and importing into a
capable browser restores those retained choices.

Feedback preferences reach console iframes over the same private parent-to-iframe
update seam as semantic bindings. They gate optional cue/vibration events only;
the visual and accessible action status is always emitted. No native input,
Accessibility or feedback route competes with the ordinary client page.

## Recovering a failed pane and a lost display (issue #1125)

A local pane is "just another logical client", so a pane *failing* must ride the
same disconnect → Backfill → reconnect machinery a dropped phone does — never a
native-only fatal error. #1125 routes three new triggers into that path and
recreates a crashed pane on the same identity. Nothing in the simulation gains a
pane-shaped branch: the whole of it is `PaneBus::close`/`recreate` and the
runtime display watcher, and the sim sees only the ordinary `PlayerDisconnected`
and reconnect `Identify`.

- **A view crash.** The pane thread reports consecutive frame-copy
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
  named but fails no pane (nobody sits there). Nothing is re-homed. The report
  branches on the `DisplayRole`, never on whether any pane was named: since
  #1331 a station-bearing slot contributes no label, so a Station monitor
  carrying only station consoles has an empty `pane_labels` too — and reading
  that emptiness as "the viewscreen" printed *the shared 3-D view has nowhere to
  draw, no station is affected* about a screen whose stations the layout law
  was, on the same frame, reporting as `StationMonitorGone`.
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

## The host lobby on the viewscreen (issue #1325)

`phoenix-host --client-dir dist --world <w>`, built with `--features
ultralight`, shows **the crew lobby the browser host shows** on the viewscreen
window: scenario title, crew counter, the station grid filling in as phones
claim seats, ready badge, countdown. No flag — it is what a windowed
authoritative host has in front of it before a mission, and a native host had
nothing there between #1121 and this.

**One rendering path, not two.** The lobby DOM glue was inline in
`server.html`'s `__updateLobby`; it is now `gui/host-lobby-render.js`, over the
pure `gui/host-lobby-view.js` (#1229) that was already extracted, with the
panel's stylesheet in `gui/host-lobby.css`. The host page links both, and so
does the native document — which is *built from the host page's own
`#lobby-panel` markup*, sliced out of the served `dist/index.html`, so there is
no second copy of those element ids to drift.

```text
viewscreen_border::push_lobby_state       the same system the browser host runs
  │  Messages<LobbyStateChanged>          the same codec::encode_lobby_state bytes
  ▼
host_lobby::feed_lobby_state              latest-wins, identical snapshots dropped
  ▼
HostLobbyBridge → pump_host_lobby         over panes::surface::PaneSurface
  ▼
window.__phoenixHostLobbyApply(json)      host_lobby_boot.js
  ▼
localiseHostPayload → hostLobbyViewModel → renderHostLobby
   gui/host-channel.js  gui/host-lobby-view.js  gui/host-lobby-render.js
```

| Piece | File |
|---|---|
| Document assembly, bridge scripts, paths | `src/native_host/host_lobby/document.rs` |
| Injected page scripts | `src/native_host/host_lobby/host_lobby_{boot,link}.js` |
| The bridge and its frame loop | `src/native_host/host_lobby/bridge.rs` |
| When the surface is on screen | `src/native_host/host_lobby/reveal.rs` |
| Bevy wiring + `LocalHostLobby` | `src/native_host/host_lobby/mod.rs` |
| Compositing and input | `src/native_host/panes/ultralight.rs` |

Four things are worth knowing before touching it.

**It is not a pane, and deliberately not on the pane bus.** A pane is a
participant: minted session token, `Identify`, a claimed Station, command
admission. This surface has no identity at all — it renders what the host is
*already* broadcasting to every phone in the room and sends nothing back. Its
URL therefore carries no fragment, and there is no token in a served body to
worry about. It shares the runtime, the texture, the compositing node and the
input router with the panes, and nothing above that.

**It sits at the served root, not `/client/`.** The pane document is published
at `/client/pane-<n>-<nonce>.html` because the *client* page's relative URLs
resolve from there. This one borrows the **host** page's markup and modules, so
it is published at `/host-lobby-<nonce>.html` and `gui/host-lobby-render.js`
and `../assets/strings/strings.csv` resolve exactly as they do for
`dist/index.html`. Same trick, other page. Loopback-only, like every hosted
document.

**The surface is permanent.** On mission start it is not torn down: the chrome
*yields* — the page renders nothing (the shared view model already hides the
panel outside `GamePhase::Lobby`), the Bevy node's `display` goes to `None`, and
the placement leaves the #1124 router so a click over it reaches the viewscreen.
**F9** reveals and hides it during play; returning to the lobby phase brings the
chrome back on its own. Every phase change clears the manual latch, so a key
pressed in the lobby — where it looks inert, the chrome already being on —
cannot arm a reveal that springs open at launch. That decision is the pure
`host_lobby::reveal::RevealState`, which is why it is CI-tested rather than only
provable under the ignored GPU test.

**One `PaneId` is reserved for it.** The router, the focus ring and the touch
capture map are keyed by `PaneId`, and the surface has to appear in them for a
mouse and a keyboard to operate it. `PaneRegistry` mints ids from `0` upward and
never reuses one, so `HOST_LOBBY_SURFACE_ID` is `PaneId(u32::MAX)`; everything
that treats a `PaneId` as a participant — the pump, the fault path, the
close-and-retire sweep — checks for it and skips. Its canvas draws at
`ZIndex(-1)` and it is placed **last** in the router, so where a tiled `--pane`
overlaps it the pane both draws on top and wins the hit test. Taken to its
conclusion: tiled `--pane` consoles divide the whole primary window between them
and so cover the surface completely, which means the lobby is visible only with
**no `--pane`** or with a `--profile` that seats every pane on a Station window.
It is still composited and still pushed to in the tiled case — merely occluded.

It is also **never the seeded keyboard focus**. `FocusRing::focused_on_first_pane`
seeds the first *pane*, skipping this handle: the reticle is a promise that the
next keystroke lands somewhere, and the surface carries no typeable control, so a
`--pane`-less host would otherwise boot with a whole-window focus frame around
chrome that accepts nothing. It stays in the focus order, so Ctrl+Tab still
reaches it deliberately, and a reveal does not re-seed it either.

The bridge drops a lobby payload identical to the last one accepted and sends
the reveal flag only on changes. Its synchronous `evaluate_script` calls run
on the pane thread; the simulation sees the bridge's queued records on a
subsequent frame.

### The join QR (issue #1329)

The surface shows the host's **real** crew join code, in the same framed panel a
browser host shows, drawn by the same `gui/host-qr.js` from the same encoder —
which is now **vendored** (`gui/vendor/qrcode.js`) and served by this process's
own delivery server, because a bridge machine is not assumed to have internet
and the code was hanging off a CDN `<script>`.

**Where it points is the native decision.** The surface loads over loopback —
that is what an embedded view on this machine can dial — and that is the one
address in the building no phone can open. `host_lobby::join` therefore builds
the page base from what the *listener* bound: a specific `--addr` is used as
given; `0.0.0.0` takes the interface the machine would route out of (a UDP
`connect` to TEST-NET-3: a routing-table lookup, no packet, no DNS); with no
route it falls back to loopback and says so at the prompt. The URL itself is
`gui/join-url.js`'s, shared with the browser host.

**Which code it carries** is issue #1353's question, and it has one answer: the
host's own. Since that issue a native host mints its code at bind and answers
the join socket on its own port (see [The host is its own
rendezvous](#the-host-is-its-own-rendezvous-issue-1353)), so the panel is live
on a plain `--lobby` launch with no service anywhere. A host that *also* has
`--rendezvous` holds two codes and shows the direct one, because the QR carries
the **page** as well as the code and the page it opens is served from here — the
cloud code goes to the operator's terminal for anybody joining from outside the
LAN.

Only a host with no way in at all — `--solo`, or one serving no client bundle —
makes the panel say joining is off, in words from the string table: there will
never be a code, and a framed empty QR would have a crew scanning something that
cannot work.

Three things move it: the phase (shown in the lobby, hidden at mission start,
untouched in play), the surface's own `#host-lobby-qr-toggle` — this window has
no settings cog — and a phone's `ClientMessage::ToggleQrCode`, which is a real
wire variant since #1329 precisely because a native host has no page to
intercept it the way `server.html` does. A phone's press does **not** uncover
the surface: the view composites into an opaque texture, so revealing it in play
covers the mission, and F9 stays the only thing that does that.

### The monitor row (issue #1330)


**Revert hazard.** This surface only drains its *own* outbound queue because of a
fix that landed in **issue #1330's `cfbff20b`**, not in any #1325 commit. #1325
installed `phoenixHostLobbyOut` on the page and left `UltralightPaneSurface`
evaluating `window.__phoenixPaneOutDrain` — a function the lobby document never
defines — so the first record the page queued would have been swallowed with a
completely clean log. The per-surface drain script (`UltralightPaneSurface::new`
vs `::for_host_lobby`) is what fixed it. Backing out #1330 to remove the monitor
row therefore silently re-breaks the lobby's page→host channel; take the drain
script out of any such revert.

**The monitor row** (issue #1330) is this surface's first control: one button per
connected display, the current viewscreen marked, and pressing another moves it
there live through the [layout law](#the-layout-law-issue-1327). Two
properties are worth knowing before touching it.

*Only a press or an unplug moves the window.* A windowed host always runs the
display applier and the runtime watcher now, even with no `--profile` — it
synthesises a `BridgeDisplayConfig` that **describes** rather than instructs. The
live roster is therefore re-identified every settle, and it is re-identified
against what the layout already knows (`identify_stable`) rather than re-derived
with `identify`. That matters because `identify`'s answer depends on the set it
is given: a twin arriving suffixes both of a pair, that twin leaving collapses
the survivor back to the short key, and a display renegotiating its resolution
rewrites the `name@WxH` key outright. Comparing a re-derived string against the
stored one read all three as "the viewscreen's monitor has gone" and slammed the
primary window into borderless fullscreen on a host nobody had touched. A display
still plugged in now carries the identity it was known by, matched by *form* —
the same technique `present_assigned_identities` uses on the pane side.

*A console the layout cannot move is still a console.* A hand-authored `--pane`
profile's panes name a person rather than a station, so `adopt_profile` seats
nothing for them — but the adapter opens a real borderless-fullscreen Station
window on that display all the same. The layout records those labels in a
**reserved** set (`BridgeLayout::reserved_on`) so rule 2's mirror refuses the
viewscreen moving onto them, and the row draws them on the button. Labels rather
than minted `StationId`s: a faked id would appear on the roster-driven rows as a
station no ship has. Since #1332 a reserved label also **fills one of that
screen's two console slots** — see *Two consoles per screen* below, which is
where counting only the seats turned out to overlap two consoles rather than
tile them.

### Picking from the viewscreen (issue #1328)

`phoenix-host --client-dir dist --lobby` boots onto the **scenario panel** on
the viewscreen. The operator picks the world, then the hull — the host page's
own two-stage flow, single-hull auto-resolve included — the world loads, and the
crew lobby appears underneath. `run-native.bat lobby` is the Windows wrapper;
plain `run-native.bat` is the delivery host, unchanged.

**One picker, not two**, by the third application of the same extraction the
lobby and the join panel got. `gui/host-scenarios.js` already held the stage
decision (#1230); `gui/host-scenario-render.js` now holds every DOM write
`renderScenarioLockState()` made inside `#world-list`, and
`gui/host-scenarios.css` its rules. The native document carries the served
page's own `#scenario-panel` markup, minus its two host-tooling blocks — the
mod-pack upload and the save importer are file inputs with page-lifetime
handlers this document does not carry — and starts `display: none`, because a
`--world` host never publishes a picker and a panel covering *its* lobby would
be a viewscreen that never moves.

What crosses the bridge is exactly `scenarioCatalogView`'s three arguments:

```text
world_load::published_catalog             ONE derivation, two audiences
  ├─ .wire()    → ServerMessage::ScenarioCatalog   → every phone in the room
  └─ .surface() → ScenarioPanelPayload             → the viewscreen
        ▼
host_lobby::feed_scenario_panel → HostLobbyBridge::push_scenario
        ▼
window.__phoenixHostLobbyScenario(json)   host_lobby_boot.js
        ▼
scenarioCatalogView → renderHostScenarios
   gui/host-scenarios.js   gui/host-scenario-render.js
```

and what comes back is a `HostLobbyRecord` — a closed **four**-tag vocabulary
(`select_scenario`, `select_ship`, `force_start`, and #1330's
`set-viewscreen`), deliberately **not** a `ClientMessage`, on the page→host
namespace that has always been distinct from the pane bus's.
`drain_surface_records` turns a pick into an `InboundMessage` under
`LOCAL_CONSOLE_TOKEN` — the token `server.html`'s own picker submits under — so
the arbiter sees one participant among the phones under first-valid-wins, with
no priority. The token is reserved, so no network peer can claim it, and both
selection variants are explicit no-ops in the lobby handler.

#### One vocabulary, one drain, one reader

That fourth tag is where #1328 and #1330 meet, and the rule it encodes is a
correctness constraint rather than tidiness. `HostLobbyBridge::take_records` is
`records.drain(..)`: what it returns, nobody else will see. The two slices were
built in parallel and each shaped its own record type and its own reader; they
merge with **zero textual conflicts** into a build where the monitor row
silently stops working. Whichever system ran first would take *every* record,
warn "the surface sent something this bridge does not speak" about the other's,
and leave the second reading an empty queue for the rest of the run — with a
clean log at both ends.

So there is one enum and one reader. `drain_surface_records` runs in
`PreUpdate` — where every other participant's input enters the app, and before
the fixed loop that arbitrates it — and dispatches on the tag; what each verb
*does* still belongs to the module that owns it (`layout::set_viewscreen_action`
is the layout half). `publish_bridge_layout` stays in `Update`, so a press
applied in `PreUpdate` and the row that answers it are one frame and one
repaint. **A new page→host control is a variant here, never a second record type
and never a second `take_records` caller.** The host→page direction has no such
rule and splits per concern freely: those are latest-wins snapshot slots, and a
slot costs at most one push a frame.

One wart is deliberate: `set-viewscreen` is **kebab** where its three siblings
are snake_case, kept by an explicit `serde(rename)`. It shipped that way in
#1330, `gui/host-lobby-view.js` writes it by hand, and the page is assembled
from a bundle that may be older than the host — so the fold was not worth a wire
break. A test pins both directions: the kebab spelling decodes and the
snake_case one does *not*.

**A refused pick repaints nothing**, which is exactly what the host page does
with one: `arbiterSelectScenario` returns before its render on any non-accepted
outcome, so on both surfaces the panel goes on showing the stage that is
actually true. It is not silent — `drain_scenario_selection` logs it at warn
level on both hosts.

**Force-start is host policy now, not browser glue.** `PendingForceStart` and
`apply_force_start` were `#[cfg(target_arch = "wasm32")]`, so a native lobby
whose crew are all on phones could not be launched at all. Both are de-gated,
and `native_host::app` registers the system on the same two edges `wasm_init`
gives it — `.before(SimSet::Input)` for #907's tick-scoped transition, and
`.after(NativeWorldLoadSet)` so a press on the tick a runtime world lands sees
that world. Only `drain_force_start_input`, which reads a thread-local
JavaScript sets, is still wasm-only. The rule gained one guard: refuse with no
`WorldConfig`, which is inert in the browser (the world is loaded before
`wasm_init` composes the `App`) and is what stops a viewscreen press starting a
mission over nothing. The lobby's `#ai-launch-btn` — stripped by #1325 because
nothing answered it — is therefore back, shown by the same `vm.aiLaunchVisible`
the host page's is.

Each flag still skips exactly the stage it decides: `--world` the scenario
stage, `--world --ship` both, `--lobby --ship` the hull (the pinned hull is
reported as `locked_ship`, so the picker shows a settled choice rather than one
the host has already overruled). `--solo`, `--pane` and `--profile` behave as
they did.

While the picker is up it covers the crew lobby (`#scenario-panel` is `z-index:
200`, the host page's own stacking), and the **join QR has to stay above it** —
showing the code during selection is how a crew joins while the operator is
still choosing. `server.html` does that in JavaScript (`showJoinQrOverPanel()`,
undone by `resetJoinQrLayer()`, because that page has a HUD and a canvas whose
stacking it must return to). This document has neither, so the lift is two lines
of its ground CSS and there is nothing to restore.

### The station screen rows (issue #1331)

Every station card in the lobby carries a **screen row**: one button per eligible
monitor plus an off state. Pressing one opens *that station's console on that
monitor at runtime* — a Station window and a seated pane created on demand, not
at init — and off closes it and frees the screen.

*The console is an ordinary participant.* `PaneBus::open_console` mints a fresh
UUIDv4 session token, exactly as a browser tab does, and the pane loads the same
client document a `--pane` and a phone load. It joins, claims its station and may
be released and re-claimed by anyone; admission cannot tell it from a phone. The
station id in the layout decides only **which console document opens on which
glass**, never who may sit there.

*Every seated station has a console, and the bus is asked.* Whether one needs
opening is not derived from a diff of what the surfaces gained — it is asked of
the pane bus, for every station the layout seats. That is what a `--profile`
which seats a *station* (`PaneSlot::for_station`) needs: its Station window and
its pane slot exist from boot, so a diff would find nothing new and leave the
operator looking at an empty borderless-fullscreen screen.

*Moving one is rebuilding it, on the same identity.* An Ultralight view is
created at one size on one window, so a console that moved screens — or whose
rectangle changed because a second console joined its screen or left it — cannot
be re-placed and has to be built again. That goes through `PaneBus::close` +
`recreate`, which is #1125's crash path used deliberately rather than a second
mechanism: the **same session token** survives, so the rebuilt page's `Identify`
is a reconnect the lobby answers by restoring the held station. Whoever claimed
that console keeps it across the move, with the same moment on `Backfill` a view
crash costs.

*Consoles are opened by the display layer, never by the lobby.*
`drain_surface_records`'s layout arm only moves the layout;
`bridge_display::follow_layout_stations` is what notices and acts. That is
because an **unplug** reconciles a console away with nobody pressing anything —
if the press path opened consoles, the unplug path would need its own closing
code, and two implementations of one rule disagree the first time either is
touched. It also makes the #1125 display-loss semantics fall out rather than be
restated: `reconcile` leaves a station whose monitor is gone unassigned, its
console closes on the same frame through the ordinary `PaneBus::close`, and its
station flips to `Backfill` exactly as a dropped phone's does.

*#1330's tripwire is settled, and not the way it guessed.* Unplug-closes-no-pane
on a no-`--profile` host holds because the synthesised `BridgeDisplayConfig`
carries no pane labels, and #1330 flagged rebuilding that config from the live
layout as the obvious next move. It is **not**: the console already closes
through the law (above), so a rebuild would close it twice — and
`assigned_surfaces`'s `pane_labels` are the *authored* profile's participant
names, answering #1125's question about the arrangement the host was **started**
with. Folding lobby-opened station ids into them makes one list answer two
questions. The config stays the boot description, and the tripwire test was split
into its two claims instead.

What that does **not** mean — and a first reading of it got this wrong — is that
the two lists cannot *overlap*. They can, and a `--profile` was how:
`PaneSlot::for_station` names its pane for its station, so an authored **station**
slot put a station id straight into `pane_labels`, and the watcher then resolved
it against the *live* bus. Unplugging the monitor the profile named therefore
closed a console the lobby had since moved to a different screen, ending a
human's watch over a display their console was not on and minting a fresh token
in its place. The lists are kept apart at the **source** instead:
`assigned_surfaces` excludes a station-bearing slot outright, so `pane_labels`
really is participants only — and a station's console is the law's on every host,
authored or not.

Excluding the station-bearing slot closes only half of it. The *other* half is a
profile pane that names **no** station and takes a station's name anyway:
`label = "helm"` with no `station` key stays in `pane_labels` by design, so the
same unplug of the authored monitor closes the `helm` console the lobby had
opened somewhere else and the open sweep mints a fresh token — the identical
harm, reached through a `label` rather than through a `station`. That is a *name*
problem, and it is refused where every other pane-name collision is:
`app::install_world_selection` checks the hull's station ids against **both**
sources of participant pane names — the `--pane` flags and the profile's
station-less slots (`ValidatedProfile::participant_pane_labels`) — on the
`--world` path and on the `--lobby` one. With that refusal in force a duplicate
pane label cannot come from authored input at all, and
`BridgeStationSurfaces::slot_for` prefers the station-bearing slot so the
unreachable case is *decided* rather than left to iteration order.

*A surface is the law's to drop, not this frame's winit report.* Every reader of
a lost display waits out `DISPLAY_LOSS_DEBOUNCE_FRAMES` before believing it,
because winit's monitor list blips through a GPU reset, a display waking and a
dock. `follow_layout_stations` therefore drops a Station surface only when the
**layout** stops naming its monitor — the reconcile has already done the waiting,
so following it rather than re-deciding beside it makes the two agree by
construction. A frame reporting *no* monitors at all does nothing and leaves the
pass owed. The applier also follows **both** of its inputs, the layout and the
monitors, because a display that left and came back has to get its Station window
rebuilt for a console the layout never stopped seating. And the close sweep asks
the bus ∩ the law — every rostered station the layout does not seat whose console
is still open — rather than the surfaces it has just rewritten, because a console
outliving its seat is the one failure with no way back.

Rebuilding that window means **re-anchoring** it. `bevy_winit` despawns a
`Monitor` entity when a display stops being reported and spawns a *new* one when
it returns, so a surface that survived the blip still names an entity that is
gone — and winit re-applies fullscreen only when `Window::mode` changes, so the
window would sit wherever the OS parked it while the geometry, the pane rects and
the input router all used the returned monitor's coordinates. `BridgeStationSurface`
records the `Monitor` entity it is anchored to, and the applier rewrites the mode
when that differs, exactly as `follow_layout_viewscreen` does for the primary
window.

*The law and the adapter are reconciled, and a seat can be given back.*
`follow_layout_stations` applies the layout; it cannot see whether the
application worked. `reconcile_seated_consoles` asks the question it cannot: does
every seated station have a console **open on the bus** *and* a
**`BridgeStationSurfaces` slot** to be composited into? A station that fails that
for the same grace window is rebuilt on its own identity through #1125's
`close` + `recreate`, **bounded** by the same per-identity budget a flapping view
crash is held to. When the budget is spent, the seat is *surrendered* through the
law (`UnassignStation`) with a `LayoutNotice` the row renders — the screen frees
everywhere at once and the station is on `Backfill` honestly, rather than a card
claiming a screen that is black. On the pane host's side, a failed
`make_pane_view` now takes back the Station camera it minted and **faults** the
pane, so the failure enters that same path instead of leaving an open pane with
nothing behind it.

It declines a frame reporting **no** monitors at all, as the applier and the
watcher already do: with none, `follow_layout_stations` leaves its pass owed, so
a station seated on such a frame has neither surface nor console through nothing's
fault, and ten of those frames used to surrender a seat the applier never got to
attempt.

*Notices are appended, and drained by the publisher.*
`BridgeLayoutResource::notices` is written by three systems in **two unordered
chains** — the lobby's `drain_surface_records` (in `PreUpdate`), and
`reconcile_layout` and `reconcile_seated_consoles` here — and all three used to
assign, so whichever ran last erased the others. The frame where that decides
something is the frame worth reporting: a press landing as a seat is surrendered
dropped `ConsoleCouldNotOpen`, the one notice the reconciler exists to deliver.
Every writer extends now, so a frame's notices surface together, and
`publish_bridge_layout` clears what it has pushed — through
`bypass_change_detection`, so the drain cannot schedule a second push that blanks
the row it has just filled. The list is therefore *what the lobby is owed*, not
everything that ever happened.

*A host with a bundle always carries a pane bus.* A screen row can open a console
at any moment, so `phoenix-host` opens `LocalPanes` (with however many `--pane`
names it was given, including none) whenever it has a `--client-dir` bundle and
an `ultralight` build. With `--rendezvous` the bus is **paired** with the relay
through `PairedTransport` rather than replaced by it — before #1331 the relay's
`insert_resource` silently overwrote the pane transport, so a `--pane` on a
crewed host was talking to nothing.

*A pane name may not shadow a station id.* Pane names and station ids became one
namespace when the screen rows started opening a console under its station's own
id, so `--pane helm` on a hull with a `helm` station is two participants under
one key — and the row's off button would close the person's console rather than
the station's. `app::install_world_selection` refuses it where the roster is
finally known: at the prompt for a `--world` host, and at the pick for a
`--lobby` one. **Both** sources of a participant pane name are checked — the
`--pane` flags and a `--profile`'s station-less `[[display.pane]]` slots — for
the reason given under the two lists above: the profile route is the one that
puts a station id back into `pane_labels`.

### Two consoles per screen, and what a screen is holding (issue #1332)

*A console is a console whoever opened it.* Rule 3's two-per-screen bound counts
the stations the layout seated **and** the authored surfaces it merely knows
about (`reserved_on`), through one `BridgeLayout::occupant_count` that
`assign`, `eligibility` and `occupancy`'s `free_slots` all share. A screen
carrying one hand-authored `--pane` console has one free slot; one carrying two
has none. This closes #1331's carried defect: counting only seats let the screen
row *offer* a monitor already holding an authored console, let the law *accept*
the press, and left the adapter laying the new console across the whole monitor
on top of the old one — two consoles, one rectangle. `LayoutRefusal::MonitorFull`
carries the authored labels beside the station ids for the same reason
`ViewscreenMonitorHoldsStations` does: an operator looking at two consoles must
not be told the screen holds one.

*One tiling per screen, and the runtime console shares the window.* A monitor
has exactly **one** Station window — the #1124 pane host composites N panes onto
it — so there is no second window to open, and refusing to double-book the
window would have made the law offer a slot the adapter always refused. So
`BridgeLayout::surface_rects` lays out the whole occupancy in **one**
`pane_rects` call. `follow_layout_stations` writes that list over the surface
wholesale rather than tiling the stations around the authored panes. Two tilings,
each handed the full rectangle, *was* the bug; one tiling over a count rule 3
already bounds is the fix, and it invents no new tiler. `station_rects` is that
same tiling filtered, not a tiling of its own.

*And it is the AUTHORED tiling, from the boot frame onward.* #1332 shipped that
paragraph with **two** tilings still live: `apply_bridge_profile` laid a
`--profile`'s Station out from the file — that entry's pane order, that entry's
`split` — while `surface_rects` laid the same screen out reserved-first and
always `LAYOUT_SPLIT`. The follower is chained immediately after the applier in
the same `Update`, so the boot frame drew the operator's arrangement and the very
next system overwrote it: an authored `[helm(station), Ada(participant)]` booted
helm-left and flipped to Ada-left, closing and recreating a console nobody had
touched and pushing a `ConsoleRetiling` notice — around the deliberate rule that
boot notes are logged and never pushed — and an authored `split = "stacked"`
screen was re-carved side by side. The fix round gave the law what it was missing
and made boot a reader of it: each reservation carries the **index** its
`[[display]]` entry gave it (`Reservation`) and each monitor carries its authored
**split** (`split_on`), both adopted in `adopt_profile`, and
`apply_bridge_profile` now builds its Station windows from `surface_rects` like
everything else. One tiling, and it draws what the operator wrote. A screen no
profile authored — every screen on a host with no `--profile` — is unchanged:
`LAYOUT_SPLIT`, and a lobby-seated console takes the slot that is free.

*The neighbour rebuild is surfaced, not hidden.* Seating or unseating a console
re-lays out the whole monitor, so the console **beside** it loses its rectangle —
and a view built at one size on one window cannot follow, so it goes through
#1125's `close` + `recreate` and its crew member spends a page load
disconnected, their station on `Backfill`. **The same-token reclaim holds**:
`process_disconnect_with_stations` leaves the station on the `Player` record and
snapshots its rating, and `handle_identify`'s reconnect-yield restores both when
that token identifies again — *gated on no other connected peer holding that
station*, so for the length of one page load the seat is genuinely claimable by
somebody else. That window is #1125's and #1331's, not new; what #1332 adds is
that it is no longer silent, and that what it says is **true**. The adapter tells
the three cases apart by the evidence it already has (`Rebuild`): a console whose
**monitor** changed was moved by the press the operator just made and stays
silent — narrating a press back to the person who made it buries the line that is
news; one whose **rectangle** changed on a screen whose **occupancy** also
changed was re-tiled by a neighbour arriving or leaving
(`LayoutAdoption::ConsoleRetiling`); and one whose rectangle changed on a screen
holding exactly what it held before was **resized** with its monitor
(`LayoutAdoption::ConsoleResized`). That last case is real — a television
renegotiating its mode in place keeps its identity through `identify_stable` and
reports new pixels — and #1332 first shipped calling it "the split changed",
which is a sentence the code cannot support and sends an operator hunting for the
console that arrived. Both notices name the console rather than a `StationId`,
because an authored `--pane` participant is rebuilt by the same rules. And both
say what the reclaim actually promises: the crew member reconnects on the same
identity, *and stays on AI control if somebody else claimed the station in the
gap*.

*The greying is still the law's alone.* `hostLobbyStationRows` maps
`BridgeLayout::eligibility` verbatim and adds no judgement of its own — a screen
the law calls `eligible` is offered however many consoles the row can see on it.
What the greyed button now *says* is which consoles are holding it, joined from
the same `occupants_on` list the monitor button draws and the refusal names, **in
the order they are drawn on the glass** — `occupants_on` is `surface_rects`'
order, so the button reads left to right (or top to bottom) the way the screen
does, rather than merely naming the same set. That matters most for the occupant
no station row can ever show: a console a `--profile` opened for a named crew
member, which fills a slot and has no card. And because that console fills a slot
the lobby has **no control to free** — no station row, no off button, nothing
short of restarting the host with different arguments — the button says so
(`server.station_row.full_authored`, fed by the payload's `reserved` list). A
greyed screen that only listed the names would be true and useless: the operator
reads "close one of these" and finds nothing to close.

*The other branch, and why it was not taken.* The alternative was to feed the
pane bus's **liveness** back into the law and free a reservation whose pane the
bus no longer has. `BridgeLayout` is pure and Bevy-free by construction, so it
has no bus to ask — but the deciding reason is that every liveness signal has a
multi-frame window in which it lies, and the arrangement would change under an
operator who pressed nothing. Keyed on `Live`, a **rebuilt** console is open but
not live from the moment `recreate` returns until its page has reloaded and
`pump_pane` calls `mark_live` — a whole page load, on every move, re-tile and
resize. Keyed on `Closed`, #1125's crash path leaves a faulted pane closed from
the fault until the recovery pass rebuilds it, and once its per-identity budget
is spent, closed for the rest of the run. Either frees a slot for a station that
then *overlaps* the console coming back — the very defect #1332 exists to close.
(The `close`-then-`recreate` race this rationale first cited is **not** one of
them: both calls are consecutive statements in one system body, so no frame
observes the gap, and the lobby's own assign already ran in `PreUpdate`.)

### A crashed console reopens on its own monitor (issue #1333)

Where a rebuilt view goes is now one stated rule in one pure module,
`panes::placement::home_for_pane`, instead of a fallback chain inside
`open_pending_views`. That move is the substance of #1333: `open_pending_views`
is behind `--features ultralight`, which **no CI job in this repository
compiles**, so the one rule this host has about which screen a console lives on
sat in the half nothing checks — the same objection `panes/mod.rs` records
against every other pane claim.

The rule, in the order it is asked:

1. **The live Station slot wins.** `BridgeStationSurfaces::slot_for` first, so a
   console the operator put on a wall monitor — a lobby-seated station's, or a
   `--profile`'s authored participant pane — is rebuilt on that monitor's own
   window, at the rectangle the layout gives it *now*. The surfaces are live, so
   a console that moved screens is rebuilt on the screen it moved to.
2. **A console the LAW seats has no other home.** If `BridgeLayout` seats a
   station of this pane's name and step 1 found no slot — a monitor between
   hot-plug frames, a Station window not yet rebuilt — the answer is
   `NoHome::SeatedButUnplaced` and **nothing is built**. Tiling it on the primary
   window would put a console the operator assigned to a wall screen straight
   over the shared view, which is the failure #1333 exists to make impossible.
   It is **faulted** rather than skipped (below), and `reconcile_seated_consoles`
   then repairs it boundedly or surrenders the seat with a notice, so "not built"
   cannot mean "a station card claiming a black screen forever".
3. **Otherwise the stored primary tile** — the legacy
   `--pane`-without-`--profile` host, unchanged since #1125.
4. **Otherwise nowhere**, with a reason, rather than a guess.

*"Not built" must not mean "forgotten".* The pending-view entry is already
drained by the time the rule is asked, so a `Nowhere` the pane host merely skips
leaves a pane the bus lists as open with no view behind it and nothing left to
rebuild it. For a seated console that is reachable in one frame:
`reconcile_seated_consoles` hits its grace and rebuilds — resetting its own
strike counter as it does — the pane host drains that entry later in the same
frame while the slot is still missing, and the next frame the slot returns, so a
health check that asks only "pane open *and* slot present" reads healthy for ever
over a black screen. So which reasons retry is part of the rule and lives in
`NoHome::should_retry`, in the pure module CI compiles, rather than in the
adapter: `SeatedButUnplaced` is faulted (`PaneFault::ViewCrashed`) and rides
#1125's bounded path to a rebuild that lands or a seat given back with a notice;
`Unplaced` is not, because a retry would have nowhere to aim.

*Was the violation reachable before this?* No — and the reason it was not is
worth writing down, because it was an accident of three separate facts rather
than a rule. The tile fallback is keyed on a pane's participant name; `PaneHost`
records tiles only at `init_pane_host`, from the `--pane` list; and #1331's
`install_world_selection` refuses a `--pane` label that shadows a station id. So
a lobby-seated console simply had no tile to fall back to and fell through the
old `continue` instead. Nothing said so, and nothing tested it — while #1332,
whose whole subject is a monitor carrying *both* an authored pane and a station
console, is exactly the slice that would have made it reachable.

What *was* a live trap is that `PaneSlotGeometry` was recorded for a pane the
profile seated on a **Station window** as well as for a tiled one, carrying that
monitor's rectangle. Applied to the primary window that is an arbitrary strip
over the viewscreen. Rule 2 refuses to reach for it anyway; `PaneTile` is no
longer recorded for a seated pane at all, so the trap is removed rather than
guarded.

*What #1333 accepts as residue: the authored participant's console.* A
`--profile` `[[display.pane]]` with a label and no station key is a **person's**
console on a Station window. It is placed by rule 1 and, since #1333, records no
tile at all. If its Station surface is dropped — the monitor unplugged, the
window rebuilt — and its view then faults, the rebuild reaches rule 4: no slot,
no seat in the law (the law seats *stations*), no tile. So it is
`NoHome::Unplaced`, and it is **not** retried, because there is nowhere for a
retry to aim; and nothing else picks it up, because `reconcile_seated_consoles`
iterates the roster's seated stations only. The pane stays open on the bus with
no view, and no notice is raised — a dead end for that one leg. It is a trade
taken on purpose rather than an oversight: the behaviour it replaced was worse
(that pane was rebuilt on the primary window, over the shared view, at a
rectangle measured on a different monitor), and doing better than "leave it"
needs a lifecycle for participant panes that the layout does not own — the same
lifecycle a resizable or re-homable participant pane needs. A fix belongs with
that work, not with a placement rule.

*The crash-during-a-move edge.* A crash serviced by `service_faults` queues a
view for a recreated pane; a move landing before the pane host drains that queue
closes *that* pane and recreates it again, through the same `close` + `recreate`.
Two entries, one console: `open_pending_views` skips the first because
`PaneBus::is_open` says its pane was closed in the interval, and builds the
second against the surfaces the move rewrote. Nothing double-builds, nothing is
orphaned, and the session token survives both hops — so whoever claimed that
console keeps it across a crash and a move together.

*The unplug is unchanged, and now stated.* A display loss is a `close`, never a
`fault`, so a console whose monitor is unplugged mid-mission queues **no** view
at all: its crew drops to `Backfill` through the ordinary disconnect and there is
nothing for the viewscreen to catch. Bringing it back is the operator's press on
the screen row, which opens a fresh console on the replugged monitor — never a
silent re-home, which is #1123's doctrine held at runtime.

### Remembering the bridge, per ship class (issue #1334)

An accepted lobby arrangement is **filed under the hull it was built for**, in
the operator's own settings directory, and pre-applied the next time they pick
that class.

```text
%APPDATA%\ProjectPhoenix\bridge-layouts\alliance_destroyer.toml
%APPDATA%\ProjectPhoenix\bridge-layouts\alliance_cruiser.toml
```

| Piece | Where |
|---|---|
| The store — class key, atomic write, load + re-validate. Bevy-free, location injected | `src/native_host/layout_store.rs` |
| The two systems — pre-apply at the hull-known moment, file on every accepted change | `src/native_host/layout_store_systems.rs` |
| The `BridgeLayoutStore` resource, inserted only for a run the lobby may remember | `src/native_host/app.rs` (`build_native_host_app`) |

**The class key is the hull template's file stem.** There is no authored class
identity to key on — `ShipConfig` carries stations, systems and power groups and
no class id — so the only stable name a hull answers to is the template it was
loaded from, which is exactly what `SelectedShipResource` already holds on both
world-arrival paths, in the canonical form `install_world_selection` stored.
The *stem* rather than the whole path because the directory is one an operator
opens, copies between machines and deletes single entries from;
`assets_entities_alliance_destroyer.toml` is not that directory. The trade is
that two hulls of the same file name in different directories (a mod pack
shipping its own `alliance_destroyer.toml`) share one saved layout — a
degradation rather than a fault, because the file is adopted through
`reconcile` + `adopt_profile` against the hull actually flying, so stations the
other hull lacks simply arrive unassigned and are reported. One Windows caveat is
recorded rather than fixed ([ai]): a hull template named for a DOS device —
`con.toml`, `nul.toml`, `aux.toml`, `com1.toml` — reduces to that reserved word,
and Windows resolves `bridge-layouts\con.toml` to the device rather than a file,
so that class's saves fail with an ordinary write warning and it never remembers.
No shipped hull is named that, the failure is loud and costs nothing else, and a
reserved-name escape would make the file name stop matching the hull the operator
is looking at.

**Pre-apply happens at the hull-known moment, not at boot.** A layout is filed
per class, so it cannot be applied until the class is known — and the two
world-arrival paths learn that at different times: a `--world` host in
`install_world_selection` before the `App` runs, a `--lobby` host (#1326) at the
pick. `adopt_remembered_layout` is written against the *state* rather than
against either path, firing the frame `SelectedShipResource` first names a class
this host is not already remembering. It does two things there, and the first is
not optional: it **reconciles onto the hull's roster** (a world-less host was
seeded with none, so without this its per-station screen rows — a `map` over
`eligibility()` — would stay empty for the whole run), and then adopts the saved
file through `adopt_profile`, the same door a hand-authored `--profile` comes
through. So a saved layout gets no privileges a written one does not have: a
monitor that is no longer plugged in is a `SeatRefused` note and an unassigned
station, and a saved viewscreen whose screen is gone leaves the bridge on the
one it booted with. Both are logged rather than pushed to the lobby's notice
row, for the reason boot-time adoption notes already are — this is a bridge
arriving, not an answer to a press.

Both systems are ordered **after the whole display adapter**
(`bridge_display::BridgeDisplaySet`, added for this), not merely after
`apply_bridge_profile`. They take `ResMut<BridgeLayoutResource>`, which
`follow_layout_stations` and `watch_runtime_displays` also want, so a narrower
constraint would have the executor serialise them in an order that is arbitrary
but silent — and the consoles a remembered layout seats would open on the seed
frame or the one after it depending on how the run went. After the set it is
always the frame after: the adoption lands in the frame the layout is first
seeded, and the follower opens the consoles on the next pass, which is the same
one-frame settle a lobby press already takes.

One row of that ordering story is deliberately left open, and it is worth naming
rather than leaving to be rediscovered: `host_lobby::publish_bridge_layout` also
holds `ResMut<BridgeLayoutResource>` in `Update` and is in neither set, so the
executor may run it either side of the adoption — the same ambiguity class
`BridgeDisplaySet` closed. It is **accepted**, because the cost is bounded and
one-directional: an adoption marks the resource changed and the publisher pushes
any layout that changed, so a publisher that ran first simply republishes the
adopted arrangement on the next frame. The lobby's rows are at worst one frame
late on the boot-or-pick frame and never wrong, and the consoles themselves
follow `follow_layout_stations`, which is inside the set. Ordering it would mean
dragging the lobby's whole `Update` chain after the display adapter, moving every
notice's frame with it, to buy one frame once per class.

**Write on every accepted change** ([ai]). Not debounced and not deferred to
shutdown: a saved layout is a handful of `[[display]]` tables written through one
atomic rename, so the cost per press is a rounding error beside the frame that
press already caused, while both alternatives carry a lossy window — a debounce
loses the arrangement the operator just made if the host dies inside it, and a
write-on-exit needs a shutdown hook a windowed process is not guaranteed to
reach (closed from the taskbar, or taken down with a display driver). The
trigger is the law's change detection (`resource_exists_and_changed`, on the
writer's own run condition) *plus* a value compare against what the file holds,
because change detection answers "did somebody hold a `ResMut`", not "is the
bridge different" — a press the no-op doctrine accepted, or a reconcile that
rebuilt an identical layout, must not rewrite the file.

**A failing disk is warned about once per change, not once per frame.** A failed
write leaves that record alone so the next accepted change retries rather than
the host giving up for the run — and taken alone that is a log storm waiting for
a read-only `%APPDATA%`, because a `LayoutStoreError::Write` is a *persistent*
condition (a locked-down profile directory, a full disk, a scanner holding the
file) and "the arrangement differs from the file" would then stay true on every
frame forever, burying `LogCat::Lobby` under one repeated sentence. So the retry
is keyed on the change in two layers: the run condition means an idle bridge does
not reach the writer at all, and `Remembered::unsaved` records the arrangement
the disk *refused*, so a frame that does get there — this is not the law's only
writer — offers the disk something new or nothing.

**And the record is cleared as soon as the screen and the disk agree again**,
which is what makes that a suppression rather than a giving-up. The operator's
own recovery from a failed write is to put the bridge back while they go and fix
the directory, and then make the change again — and that third press produces
*the arrangement the disk refused*, because undo-then-redo is the only path back
to it. A record that cleared only on a successful write would swallow that press
and every one after it, ending the session with one bridge on screen and another
on disk, after a line that had promised a retry. Clearing it at the
nothing-to-do return costs the storm guard nothing: reaching that return means
the file already holds what is on screen, so a warning is still a whole accepted
change away.

**A cable coming out is not the operator changing their mind.** The law's
resource has a second writer that is not them: `bridge_display`'s reconcile,
degrading a station whose monitor has gone to unassigned. Writing *that* would
mean a screen blipping through a dock permanently forgets where two consoles
were, on a bridge nobody touched — the same silent loss #1123's never-re-home
doctrine refuses, arriving through the file instead of through a window. So the
trigger is "the arrangement changed while **the bridge did not**": `Remembered`
carries the monitor identities its `saved` layout was agreed on, and a frame
that changed the monitor set **re-baselines and writes nothing**. The file keeps
the fuller arrangement, and the next press writes the updated layout because by
then the bridge and the baseline agree again — which is exactly how the
changed-monitor criterion says it comes back. It is also what makes the
cross-session case right *as far as it goes*: launching on a laptop with two of
four screens adopts a degraded layout and writes nothing, so merely **running**
there costs the operator nothing. The moment they rearrange anything on the
laptop that press is filed and the file becomes the two-screen bridge, because a
press is intent and nothing here can tell "tidying up on the road" from "this is
my layout now". The arrangement survives the trip, not the editing. Two honest
edges, both recorded at
`remember_bridge_layout`: a press landing in the same frame as an unplug is
re-baselined with it and reaches the file only on the next press, and
`reconcile_seated_consoles` surrendering a seat changes no monitor, so that
*is* filed.

**The write is atomic**: temp file beside the target, `sync_all`, rename over
the top. A host killed mid-write leaves either the old layout or the new one,
never half a TOML file the next boot would report as corrupt. On Windows
`std::fs::rename` is `MoveFileExW` with `MOVEFILE_REPLACE_EXISTING`, so it does
replace an existing destination; it can fail with a sharing violation while
another process holds the target open, and that surfaces as an ordinary warning
with the **previous file untouched**. The temporary is removed on either
failure, so an *ordinary* failed save leaves no debris — but a **hard** kill
between the create and the rename does, because nothing runs at all. This is a
directory an operator browses, so `build_native_host_app` calls
`LayoutStore::sweep_temporaries` once when it opens the real store, clearing the
debris and logging each at debug. What it matches is the **writer's own name
shape** — `<file name>.<process id>.tmp` — rather than every `*.tmp` in the
directory: this is a directory an operator opens, and their own `notes.tmp`
beside their layouts is not a file the host has any business deleting on the next
launch. Swept by name rather than by
age, and the race that buys is recorded rather than closed: a second host of the
same user launching in the microseconds another is mid-write would delete that
temporary, whose owner then reports an ordinary write failure with its previous
file untouched and files again on the next change — the failure mode a sharing
violation already has.

**`--profile` wins for its run and never writes — and that is a data-loss
guard.** `to_validated_profile` emits seats and only seats, so the authored
`--pane` participant surfaces a layout merely knows about
(`BridgeLayout::reserved_on`) do not come back out of a file. Persisting a
`--profile`-seeded layout would therefore drop the operator's
`[[display.pane]]` entries silently, and the *next* boot would read those
screens as free and move the viewscreen on top of a crew member's live console
— the exact failure `reserved` exists to prevent, re-introduced by the save.
#1334 had two answers available (re-emit the slots, or refuse to persist such a
layout) and took the second, in three places because the failure is silent and
permanent:

1. `build_native_host_app` gives a `--profile` run **no store resource**, so
   both systems are inert.
2. Both systems are additionally gated on `BridgeDisplayConfig::authored` being
   false, so a composition that inserted one anyway still writes nothing.
3. `LayoutStore::save` **refuses** a reservation-carrying layout outright, at
   the only door onto the disk, so a caller added later inherits the guard
   rather than having to remember it.
4. `LayoutStore::load` **refuses** a *file* carrying a station-less
   `[[display.pane]]` — `LayoutStoreError::NotALobbyLayout`, the mirror of the
   same rule at the other door.

The fourth is not belt-and-braces, and it is the one the fix round added. The
saved file *is* a bridge profile in a directory an operator browses, so "copy
your `--profile` in here" is an invitation this feature extends. Taken, and
without a load-side refusal, the chain is: the pre-apply adopts it, so `reserved`
is populated on a run **nobody authored**; the bridge acquires a phantom occupant
— a screen reported full at seats nobody can see, a Station window with a
permanently dead half, refusals naming a console that is nowhere; and the
operator's first press is then refused by `save` for ever, because the file that
caused it is never rewritten. One class, silently un-saveable, with the only line
in the log being the wrong sentence. Refused at the door, the file is warned
about and ignored like any other unusable one, and the warning names the remedy:
delete the pane entries with no `station =`, or point `--profile` at the file
instead.

Which leaves the invariant worth stating positively: **a saved layout is only
ever a lobby-built one, and a lobby-built layout is reservation-free by
construction** — nothing but `adopt_profile` populates `reserved`, and (because
`load` refuses a station-less pane slot) nothing but an authored `--profile`
reaches it.

**`[[touch]]` and `[[media]]` are refused too, rather than preserved** ([ai]).
`save` writes `to_profile()` — an *empty* profile with the displays written into
it — so any touch or media table an operator added to a store file was silently
wiped by the next press. `write_displays_into` exists precisely so a layout can
be written back over an existing profile, and preserving them that way was the
alternative. It is not taken, because it would not actually be lossless
(`write_displays_into` replaces the whole `[[display]]` list, so the file's
participant slots would still be dropped, one table over and out of the
reservation guard's sight) and because it puts a read, a parse and a policy for
an unparseable file onto the **write** path, which runs on every press. So the
store's schema is displays-only, and the claim that survives is the one true in
the direction that matters: **a file this store wrote is a profile an operator
may hand back to the flag; a `--profile` is not in general a file this store will
read.**

Refusing them does not *save* those tables, and the note says so rather than
claiming the refusal is free: the file is ignored for the run and the first press
files a saved layout over it, so they are gone by the end of the session either
way. What changes is that the loss is announced by a sentence naming where they
belong, before it happens, instead of being discovered by an operator wondering
why their touchscreen stopped working.

**The remedy is composed from what the file actually carries.** One blended
sentence naming every refusal class was the first shape and it was wrong in the
ordinary case: an operator whose file held a `[[touch]]` mapping and nothing else
was told to "delete the `[[display.pane]]` entries that have no `station =`" —
entries their file does not contain — and was never told what to do with the
table that was actually refused. `LayoutStoreError::NotALobbyLayout` now emits
one imperative per class present (delete the station-less panes; move every
`[[touch]]`/`[[media]]` table into a `--profile` of your own) with the one move
that always works as the tail (point `--profile` at this file instead), and the
finding and the remedy are built from the same three table-name constants so they
cannot drift into naming different things.

**A screen's split is written whatever its occupancy** ([ai], the fix round).
`write_displays_into` used to emit `split` only for a screen holding two
consoles, on the grounds that a single console is the whole monitor and the field
is ignored for it. That read as tidiness and was the same class of silent drop as
the touch wipe: the law carries a split for every screen (`split_on`), `validate`
and `adopt_profile` take one on **any** Station entry, and no `LayoutAction` can
set one — so a screen an operator authored `stacked` survived exactly as long as
it held two consoles and vanished from the file on the first save that caught it
holding one, to come back side by side next session with nothing said. It is now
emitted unconditionally: one redundant line in a file an operator reads is a
smaller cost than a setting they cannot see disappearing. One narrow loss is left
and is structural rather than chosen — a screen holding **no** consoles is not
written at all, because a `[[display]]` with an empty pane list is not a shape the
file has (`validate` refuses it on the density rule), so emptying an authored
`stacked` screen still forgets its axis.

**A saved layout that no longer works is ignored with a warning, never fatal.**
`load` puts the file through two doors, outermost first: `BridgeProfile::validate`
— every refusal a hand-authored `--profile` meets, so a file edited by hand,
written by an older schema version or truncated by a full disk is caught there —
and then the store's own narrower schema above. Either way the caller's whole
answer is a `pwarn!` naming the file and the bridge it already has. The file is
left where it is rather than deleted: an operator who hand-edited it wants to see
what they wrote. Their next lobby press files a real layout over it, so a
refused file costs one session's arrangement rather than the class.

**The location is injectable.** A `LayoutStore` is a directory and nothing else;
`LayoutStore::user()` is the only function that consults the environment, and it
is never on a test's path. `BaseDirs::data_dir()` rather than `ProjectDirs`
([ai]) because `ProjectDirs` appends its own `config`/`data` leaf on Windows
(`%APPDATA%\<Org>\<App>\data`), which is neither the authored path nor a
directory an operator would look in; `data_dir()` is `%APPDATA%` exactly, and
Linux (`~/.local/share`) and macOS (`~/Library/Application Support`) get their
own conventional root rather than a Windows-shaped one.

### The guided acceptance kit (issue #1335)

Everything from #1325 to #1334 has a pure half CI runs and a half that only a
room with monitors in it can settle. `docs/acceptance/1335-native-lobby.md` is
that second half for the whole feature at once — one operator, one evening, in
the order a game night happens rather than in issue order.

| Piece | Where |
|---|---|
| The kit | `docs/acceptance/1335-native-lobby.md` |
| Its entry point — builds, then `phoenix-host --client-dir dist --lobby` | `run-native.bat lobby` |

| § | What it settles | Slice |
|---|---|---|
| 1 | Boot to the lobby; scenario and hull picked on the viewscreen | #1326 / #1328 |
| 2 | The join QR: joining-off, a phone's scan-to-claim, the address warnings and `--addr`, the toggle in lobby and in play | #1329 |
| 3 | The monitor row moves the viewscreen live, re-marks, and is inert on the screen it is already on | #1330 |
| 4 | A station console opened, moved (seat kept), closed, and reopened mid-mission through F9 | #1331 |
| 5 | Two per screen: the split, **both halves operable**, **legible at bridge distance**, **the re-tile blink timed**, the named greying, the survivor regrowing | #1332 |
| 6 | A cable out mid-mission: the console closes, the crew drops to Backfill, nothing crashes, a press brings it back | #1125 / #1333 |
| 7 | Arrange-quit-relaunch per ship class, the missing-monitor degradation, the copied-`--profile` refusal and its remedy | #1334 |
| 8 | Keyboard-only operation of both rows, with visible focus and no colour-only state | #1128's bar |

Three things are **parked in the kit's §9** rather than dropped, following
#1124's touch leg. Do not read an unticked box for them as a failed run:

- **Touch operation of the lobby** — no touch hardware, and PRD #1324 puts it out
  of scope explicitly, parked with #1124's Part B.
- **`prefers-contrast` / reduced motion reaching the surface.** The CSS is here
  (`gui/host-lobby.css`) and so is the reticle's Rust response
  (`FocusReticleStyle::for_os_prefs`), but neither can be driven from Windows
  today: Ultralight ships no OS-backed `matchMedia`, and
  `panes::os_prefs::query_os_accessibility_prefs` is a documented stub returning
  "no preference" on every target, because a live Windows read needs `unsafe` FFI
  this crate forbids. A sanctioned live read drops into that one function and
  unparks it.
- **A crashed console's rebuild, observed.** Not constructible by hand — a view
  crash is an internal renderer fault and a borderless-fullscreen Station window
  has nothing to close. The rule is proved instead against a real running bridge
  with an injected failure in `bridge_display.rs`.

The kit's §2 also carries a **prerequisite** rather than a park, and it is worth
knowing before a session: a phone scanning the QR loads the client bundle from
the *bridge machine's own LAN address*, so the rendezvous service's
`ALLOWED_ORIGIN` has to carry that origin (`http://192.168.x.y:8080`) as well as
whatever is passed as `--origin`. A browser-hosted game never meets this, because
its phones load the same public page the operator is on.

## Tests

| File | Claim |
|---|---|
| `src/native_host/host_lobby/*` | The lobby surface (#1325), all feature-**off**: the document assembles from the repository's own `server.html` and carries every element id the shared renderers write into, stops at each panel (a comment mentioning a `div` cannot unbalance the count), links the shared stylesheets, refuses a page with no lobby, no join panel or no picker by name; the bridge's latest-wins collapse, its identical-snapshot drop, its deferral of a failed push and the newer-wins restore; and the reveal state machine — boot showing, mission start yielding invisible *and* input-transparent, F9 both ways, a return to the lobby restoring it, and a latch that cannot survive a phase change. **#1328:** the picker is carried minus its host tooling and starts hidden, the AI-launch button is kept and its `data-i18n` with it, the assembled document's controls are exactly `{ai-launch-btn, host-lobby-qr-toggle}` (an allowlist over both the stub and the repository's own `server.html`), the join panel is lifted above the picker, a surface record becomes the host page's own `LOCAL_CONSOLE_TOKEN` `ClientMessage` while an unrecognised one reaches no bus at all, a launch sets the same latch the browser button sets, and a host with no catalogue never publishes a picker. **#1330 + the fold:** one `set-viewscreen` record through the *same* drain moves the live layout and publishes the row that reports it, a stale monitor is refused with a `LayoutNotice` the row renders, an accepted press clears the refusal before it, a press arriving before a layout exists is dropped rather than queued, the six record tags round-trip (and the three kebab layout verbs decode where their snake_case spellings do not), and one frame carrying all six bridge slots pins the documented pump order. **#1331:** the two station verbs reach the *same* drain arm and the same law, and the notices every writer appends to are drained by the publisher that pushed them |
| `tests/client/host-lobby-view.test.js` | The host lobby's pure view model, including the monitor row (#1330) and the per-station screen rows (#1331) — every case fed the law's verdict verbatim, so the page is proved to add no judgement of its own. **#1332:** a greyed `full` button names the consoles holding that screen (including a `--pane` participant, who has no station card anywhere), falls back to the bare marker when told nothing, never greys a screen the law called `eligible` however many consoles it can see on it, and un-greys a vacated screen on every other station's row. **#1332 fix round:** a screen one of whose consoles the lobby cannot free says so (`full_authored`), and one whose consoles can all be closed from here keeps the short marker |
| `tests/client/host-lobby-render.test.js` | The extracted renderer, in jsdom, driven against `server.html`'s own `#lobby-panel` subtree: cards, avatars, chips, pills, the ready badge's `go` class, the countdown, a re-render replacing rather than appending — and the two documents, one with the AI-launch button and one (the native lobby's) without |
| `tests/native_host_lobby_ultralight.rs` | The real lobby document in a real Ultralight view over this process's own HTTP: a real `LobbyStatePayload` fills the station grid through the shared modules, the chrome yields on mission start and comes back on the reveal flag with no reload, the surface rasterises, and (#1329) the vendored encoder loads from this process's own server and rasterises a join QR whose printed URL is the join URL a phone needs, which a phone's toggle and the surface's own control both hide and show, and which gives way to "joining is off" for a host nobody can join. **#1328:** a scenario payload builds the picker's buttons through the shared renderer, a real click queues the record on the queue *this* surface drains, and a loaded world closes the panel. **#1330:** the monitor row draws real `<button>`s through the shared renderer, a real click comes back as a `set-viewscreen` record over the same bridge and the same drain, and a refusal renders as a sentence the operator can read. **#1331:** a station card's screen row draws inside it, never offers the viewscreen's own display, and both its verbs — a screen press and the off button — come back as `assign-station` and `unassign-station` on that same one queue. `#[ignore]`d: needs the SDK and a `trunk build`ed `dist/`, which CI has neither of |
| `tests/client/join-url.test.js` | Where a page sends its join socket (#1353): a page served by a native host dials that host (LAN address, name or loopback alike), the published web origins keep the built-in service (case and a trailing slash included), anything that is not an origin falls back to it, and `KNOWN_WEB_ORIGINS` is asserted equal to `worker-rendezvous/wrangler.toml`'s `ALLOWED_ORIGIN` read off disk. Plus `joinUrlForCode` carrying no `?rendezvous=` for the direct case and still naming a non-default service for the cloud-only one |
| `tests/client/host-qr.test.js` + `tests/client/qr-encoder.test.js` | The shared join panel in jsdom against `server.html`'s own `#overlay` subtree — the visibility law (including the null that leaves a mid-mission QR alone), the draw, the native surface's link-less variant, the joining-off caption — plus the browser host's draw site pinned from `onCode` to the shared module (#1329 AC5), and the vendored encoder loaded from disk with no network and pinned to a known code |
| `src/native_host/bridge_profile.rs` | The pure model: stable identity across a simulated OS-settings rearrange, identical-monitor disambiguation (including a TOML round-trip of a position-suffixed id), the one/two-pane geometry math (even and odd, side-by-side and stacked), the >2 density refusal, the one-viewscreen refusal (`ProfileError::MultipleViewscreens`, naming both monitors), the TOML round-trip (Windows backslash ids included), the missing/unassigned/changed-display reporting, and (#1125) the runtime loss/return detection — a lost Station names its panes, a lost viewscreen names none, a return is for explicit repair only. **#1331:** a station-bearing `PaneSlot` contributes NO label to `assigned_surfaces`, so the watcher's list is participants only and a lost screen names only the panes it really closes. **#1330:** `identify_stable` — nothing known is exactly `identify`, a known display keeps its short key when its twin arrives and its suffixed key when its twin leaves, a renegotiated mode is matched by name and place while the geometry follows, a display that only moved is still matched, one that moved *and* re-moded is honestly treated as new, a newcomer never takes a carried key, and an unplug still reads as an unplug. All feature-agnostic, run by the ordinary `cargo test` |
| `src/native_host/bridge_display.rs` | A Bevy `Monitor` lifts into a `RawMonitor` and carries the documented identity; `--setup`'s exit code is clean only when a supplied profile both validates and resolves with no problems against the connected displays (`setup_profile_is_clean`); and (#1125) `watch_runtime_displays` itself — driven with *fake* `Monitor` entities spawned and despawned as bevy_winit does on hot-plug, so it runs in CI without a display — closes a lost Station's pane (→ Backfill) and no pane for a lost viewscreen. **#1330:** the whole apply-on-change loop on the same fake hardware — a no-`--profile` host gains a config and a layout and keeps its `Windowed` window on every later frame, a press moves the window once, an unplug rebuilds the row and the viewscreen follows its note; and the three roster changes that must move **nothing** (an identical twin plugged in, an identical twin unplugged with the viewscreen on the survivor, the viewscreen's own display renegotiating its resolution), plus the invariant that the viewscreen may not move onto an authored participant's console. **#1331:** the runtime open/close transitions on that same fake hardware — seating a station opens a Station window and a console pane on an ordinary token, unassigning closes it and shuts the window and frees the screen everywhere, a move keeps whoever claimed it (a rebuilt handle on the same token, queued for the pane host against the surfaces this pass rewrote), two consoles divide a screen with the first rebuilt at its new half and the survivor of a close grown back to fill it, a bridge nobody rearranged opens and closes nothing forever, a replugged monitor takes its console back on an explicit press, and a host with no pane bus declines rather than opening a black window; plus the #1330 tripwire split into its two halves — an unplug with no console on that screen closes no pane, and an unplug of a screen holding a runtime console closes exactly it. **#1331 fix round:** a surface is dropped only when the LAW stops naming its monitor — an unplug plus a lobby press inside the settle window leaves the console alone until the reconcile unseats it, and then closes it with one Disconnected; a frame reporting no monitors at all closes nothing; a re-seat afterwards gets a real slot and a queued view; an unplug of the screen an AUTHORED console has left does not touch it (the same handle, the same token, nobody disconnected); and the seat reconciler — a healthy console is never touched, a console with nowhere to be built is rebuilt on its own identity exactly MAX_RECREATIONS_PER_WINDOW times and then has its seat given back with a notice the row renders, and an injected view failure serviced through #1125's own fault path ends on an honest Backfill. **#1332:** a station seated beside a hand-authored `--pane` console TILES with it on the one Station window each takes half of, rather than covering it; the authored pane is rebuilt at its new half on its own session token; the console nobody asked to move earns a `ConsoleRetiling` notice on the row and the one the operator moved between screens earns none; moving a console off a shared screen regrows the survivor to the whole of it, keeps its token and un-greys the vacated slot; and the notice settles rather than rebuilding a console once a frame for the rest of the run. **#1332 fix round:** a pure boot frame draws the authored arrangement and re-tiles **nothing** — `[helm, Ada]` boots helm-left and stays there with the row owed no notice and the participant's own pane handle untouched, `[Ada, helm]` boots the other way round, and an authored `stacked` screen boots stacked and is not re-carved; a console rebuilt because its display renegotiated its resolution is told the screen changed **size**, not that the split changed; and the settled re-tile is exactly one notice, not "at most one" **#1333:** the placement rule asked of a *real* running bridge, every case handed a tempting primary-window tile under the console's own name — a crashed seated console is rebuilt on its own Station window on the same token with exactly one view queued; a crash landing in a move's gap leaves one open pane, one buildable view and the same token, on the screen it moved to; a seated console with no slot is `Nowhere(SeatedButUnplaced)` and is then repaired-or-surrendered rather than tiled; the one-frame interleaving that would strand it (the reconciler rebuilds and resets its strike counter, the drain finds no slot, the slot returns and the health check reads healthy over a black screen) ends in a retry rather than in that stuck state, because the answer is faulted rather than dropped; every hop of a bounded flap goes back to the same screen and the give-up surrenders the seat; an unplug queues *no* view at all and an operator's row press puts the console back on the replugged monitor; and a legacy tiled `--pane` still rebuilds on its own tile across the crash path |
| `src/native_host/bridge_layout_tests.rs` | The pure layout law (#1327): the three rules, the no-op doctrine, the profile round-trip keyed by station id, adoption and reconcile with every degradation named, and (#1330) an authored `--pane` surface counting as an occupant for rule 2's mirror. **#1332:** it counts for rule 3 too — a screen holding one authored console offers exactly one more slot and greys for every other station once taken, two authored consoles offer none, `MonitorFull` counts and names both kinds, `surface_rects` tiles the pair with no gap and no overlap (and `station_rects` is that tiling filtered, not a second one), and moving a console out of a 2-up regrows the survivor to full width while the vacated slot comes back on every other station's row. **#1332 fix round:** the law tiles an authored screen exactly as its file authored it, over every shape a `--profile` can take (one station, one participant, two participants, two stations, station-then-participant and participant-then-station, side by side and stacked) — the same names in the same order at the same rectangles on the same axis; an authored station keeps the half its file gave it and `occupants_on` reports that order; an authored `stacked` screen is not re-carved, and a station the lobby seats on it lands on the authored axis in the slot the authored console did not take; and an authored index past the end of a shrunken screen still lands somewhere rather than dropping a live console. **#1334 fix round:** a written `[[display]]` carries its screen's own `split` whatever the occupancy — a one-console screen included, and on the axis that screen was carved on rather than the constant |
| `src/native_host/bridge_media.rs` + `bridge_media_tests.rs` | The pure media model (#1126): stable `kind:name` identity (recovered to its kind, stable across a re-enumeration, kind keeps a same-named camera/mic distinct, identical devices disambiguated by hardware id or ordinal); the validate failure taxonomy (wrong-kind, malformed id, duplicate-on-surface, duplicate-surface, shared-without-consent); the consented-share warning; resolve naming a missing vs a denied device while the surface stays usable; the deterministic default (OS-default/first per kind, denied skipped, forced share consented); and the setup report. Also the `[[media]]` TOML round-trip in `bridge_profile_tests.rs`. All feature-agnostic, run by the ordinary `cargo test` |
| `src/native_host/layout_store_tests.rs` | The saved layouts (#1334), pure and against an injected scratch directory: the class key (the hull's file stem, identical whichever separator or case the path was spelled with, and reduced so it can never name a file outside the store — `..` and a bare `/` are `None`); the save/load round trip adopted onto a *fresh* bridge rather than compared in memory; two classes filed independently and a re-arrange of one leaving the other exactly as it was; a class nobody has arranged reading `Ok(None)` and creating no directory; a saved station whose monitor is gone coming back unassigned while everything else applies, and a saved viewscreen whose monitor is gone leaving the boot layout's choice; the three revalidation refusals (density, unparseable half-a-file, a newer schema version) each naming the file; the reservation refusal with nothing written; a second save replacing the first and leaving no `.tmp` behind; and the written file being a profile `--profile` itself would take. **Fix round:** the store's own narrower schema at the load door — a valid `--profile` copied in is refused as `NotALobbyLayout` naming both participant slots, the file left where it is, and the message carrying the remedy verbatim; a `[[touch]]` table is refused at the same door and named as it appears in the file; every shape the lobby can write still reads back (no consoles, one, two on a screen); and the hard-kill `.tmp` sweep clearing debris while leaving the layouts beside it, idempotent, and silent on a store directory that was never created. **Second fix round:** the remedy is composed from what the file carries — a panes-only file gets the pane clause, a media-only file gets the table clause and no mention of panes, all three classes get one clause each and the `--profile` tail once; a screen's authored axis survives the round trip on **one** console as well as on two; and the sweep matches the writer's own `<name>.<pid>.tmp` shape, so an operator's `notes.tmp` in the same directory is still there afterwards |
| `src/native_host/layout_store_systems_tests.rs` | The same slice against a **real running host** — `BridgeDisplayPlugin` seeding the law from injected `Monitor` entities, the store plugin beside it: the file is created on the first press and not at boot; arrange-quit-relaunch puts the viewscreen and both consoles back; a `--lobby` host has no roster and no class until the pick and gains both from it; two classes stay independent and neither inherits the other's; closing a console is filed like opening one; a bridge nobody touched is not rewritten (the same press eight times leaves the file's mtime alone); a station whose remembered monitor is missing comes back unassigned and re-assigning writes the update; a cable coming out mid-session degrades the live bridge but leaves the file's fuller arrangement alone, and the re-assignment after it does write; an unusable file leaves the host running on the displays as found with the file untouched; and the data-loss guard from both ends — an authored run is given no store, pre-applies nothing over its profile and leaves an existing saved file byte-for-byte identical after a lobby edit, a store handed to an authored run anyway is *still* never written (the run condition alone), and the store refuses that run's live layout by name. **Fix round:** a valid `--profile` copied into the store leaves the host on the bridge it booted with, seats **no phantom console** (nothing reserved, nothing occupying, the screen row still offering that monitor), leaves the file untouched, and the next press saves normally over it; and a store whose directory is a *file* records the arrangement the disk refused, then writes nothing across thirty idle frames after the directory is repaired — the retry is once per accepted change, not once per frame — before the next press files both arrangements and clears the record. **Second fix round:** the operator's own recovery, end to end — a refused press, an undo back to what the disk holds (which files nothing and clears the record), and the same press again, which lands; and the suppression branch itself, which the run condition alone never reaches — another writer's changed frame carrying the arrangement the disk has already declined offers it nothing, three times over, with the directory repaired and the record still standing |
| `tests/native_bridge_displays.rs` | On the real machine's monitors, a profile opens one borderless-fullscreen surface per monitor at the monitor's geometry — the viewscreen on the primary window, a Station on its own. `#[ignore]`d: it opens real winit windows, which CI has no display for. Verified once locally. The three 2-up claims it names and declines — both halves operable, legible at bridge distance, the re-tile blink timed — are `docs/acceptance/1335-native-lobby.md` §5's, because each ends in a person saying whether what they are looking at is right |
| `docs/acceptance/1335-native-lobby.md` | The **human** half of PRD #1324, on real monitors: boot-to-lobby and the on-screen picks, the join QR and its address warnings, the monitor row moving the viewscreen live, a console opened/moved/closed/reopened on chosen screens, two per screen with the split judged by eye and the re-tile blink timed, an unplug mid-mission degrading rather than crashing, arrange-quit-relaunch per ship class with the copied-`--profile` refusal, and keyboard-only operation of both rows. Runs end to end through `run-native.bat lobby`. Its §9 parks touch, the OS accessibility preferences and an observed view crash, with what unparks each |
| `src/delivery/args.rs` | `--setup` is a standalone diagnostic needing no world and refuses every simulation/crew flag (`--world`, `--ship`, `--seed`, `--solo`, `--pane`, `--log`, `--log-entity`, `--rendezvous`, `--origin`) rather than silently discarding them; `--profile` applies with a world or validates with `--setup`, and is refused alone |
| `src/boot/tests.rs` | Four-profile parity; only the render-stack profiles take that path; a native host refuses a configless boot, including a hull declared only by a static child |
| `src/native_host/transport.rs` | The seam's ingress/egress and the reserved-token refusal |
| `src/native_host/join_codes.rs` | The authored table read in Rust (#1353): the shipped `assets/join/join-codes.toml` loads and a table from another `format_version` is refused rather than half-read; canonicalisation folds exactly what `gui/join-code.js` folds (`0`→`O`, `1`→`I`, `L`→`I`, `J` untouched, punctuation dropped); a denied word is refused in every confusable spelling and the mint never draws one however hard a scripted draw insists; a minted code is one the client's own parser would accept; the full form and the bare suffix resolve to the same record; and the registry's three typed failures stay three answers |
| `src/native_host/direct_join_tests.rs` | The in-process rendezvous's protocol half, with no sockets (#1353): the `ready` that makes a host register and the `hosted` that carries the code this process minted; a joiner told the host answers only on the relay; the worker's own refusals (`unknown`, `unsupported-protocol` and its cut, `too-many-attempts`, `not-joined`, `not-relaying`, `relay-too-large`, `malformed`); the host told about an attachment *before* the joiner is; a game frame each way; the class contract (snapshot latest-wins at the authored depth, reliable ordered and its overflow ending the session, reliable draining first); `no-peer` as one refusal rather than a lost crew; an eviction stated in band before detaching; and the peer-id prefix that keeps two legs' namespaces apart |
| `tests/native_direct_join.rs` | A real `tungstenite` client through a real bound host, **not** `#[ignore]`d because it needs no service (#1353): dial, code, relay attach, stamp verdict, `Identify` on the inbound bus, both delivery classes back down, and a socket dying as exactly one `Disconnected`; the reserved-token refusal at this ingress; a build the stamp check refuses; the code and protocol-version answers; and the accept-loop robustness — a plain `GET /v1/join` (426), an upgrade with no key or an old version (400), bundle delivery untouched and a real joiner still getting in afterwards |
| `tests/native_host_sim.rs` | Shipped content boots and runs; the hull's own config reaches the client config; an uncached `--ship` is refused; a curating manifest narrows the default hull; a participant joins through the seam |
| `tests/native_headless_digest.rs` | AC5's content half — native↔headless digest equivalence, both apps on a pinned single-threaded pool. **Scope:** the default run is `NativeRenderSurface::Contract`, so it covers the `render: true` *simulation* plugins and not the wgpu stack; the `#[ignore]`d `Offscreen` companion in the same file covers that on a real GPU |
| `tests/native_host_snapshot.rs` | AC5's snapshot half — a native-host capture restores into a fresh native host at the same digest, and the duel continues byte-identically for 120 frames (Combat Test's continuation bound is the payload gap `tests/snapshot_resume.rs` measured, not a native one) |
| `tests/native_viewscreen_render.rs` | The viewscreen draws a scene — not one flat colour, and lit — over the **middle 40%** of a real-GPU frame, so a live HUD over a dead 3-D scene cannot pass. `#[ignore]`d: CI is ubuntu-only with no display |
| `tests/native_host.rs` | The delivery half, unchanged, plus the serving loop's shutdown path (polled to a deadline, so a stuck loop fails rather than wedging the run) |
| `src/native_host/panes/*` | Identity's three refusals, the projection boundary, the outbound cap's reliable/snapshot split, the document assembly (against the repository's own `client.html`, not only a stub), the identity's absence from the served body, the wildcard-bind normalisation, and the per-frame loop's push budget and deferral of a failed push. **#1333:** `placement::home_for_pane`'s four-step rule — a live Station slot beats a stored tile, a console the law seats is `Nowhere(SeatedButUnplaced)` rather than tiled, a legacy tile still tiles, an authored participant's slot is a Station home like any other, a host with no display adapter still tiles, an unknown name is refused, and a station whose seat was surrendered may take a tile again — plus the retry decision per `Nowhere` reason, since a pending view is already drained when it is made: a seated console is faulted so something retries it, and a pane with no home at all (including the authored participant's console whose slot went away) is left as it is. All feature-**off**, so the ordinary `cargo test` CI runs them |
| `src/delivery/serve.rs` | A hosted document is served to a loopback peer and to nothing else, while the bundle and the version-pin endpoints stay LAN-open; `peer_origin` classifies IPv4, IPv6, IPv4-mapped and "the OS would not say". **#1353:** `websocket_upgrade` — a well-formed upgrade on a claimed path hands its key over, an `Upgrade` header on any other path is still just a file request, a plain `GET` of the join endpoint gets the worker's own 426, each malformed shape (no `Connection: upgrade`, version 8, no key, a short key, a POST) is a clean 400, and the header tokens are read the way browsers actually write them (`keep-alive, Upgrade`, mixed case) |
| `tests/client/pane-scripts.test.js` | The two injected scripts, in jsdom, **driven through the real seam**: the boot script reads the identity out of the fragment, leaves a fragment `joinRouteFromLocation`/`parseJoinCode` accept (the literal is read out of `document.rs`, so the cross-language pin is checked), and caps the page's inbox; then the repository's own `createRendezvousJoiner` is run over the link's factories and asserted to produce the host-minted `Identify` on the page→host queue, to keep `JoinHandshake` off it, and to hand `onData` a `localiseTree`d message |
| `tests/native_host_panes.rs` | A pane joins/claims/readies through the ordinary contracts; it is admitted for its own Station and refused another's by the real policy; it cannot read another pane's projection; a pane and a transport participant hold different Stations on the same running ship; a closed pane hands the lobby the disconnect a dropped phone would; and a pane's identity is in its URL, its document unenumerable, LAN-refused, and withdrawn on close. **#1125:** on a running ship, a view crash flips the seat to Backfill through the ordinary session path; no surviving pane inherits the failed pane's projection; recreating the pane reconnects on the same token and restores its held station out of Backfill with a Welcome; and a lost Station display disconnects its pane without recreating it |
| `src/native_host/input_routing.rs` + `input_routing_tests.rs` | The pure input-routing model (issue #1124): coordinate transforms at scale 1.0/1.5/2.0 and at a non-zero monitor origin, the pane-boundary hit test (the shared seam belongs to one pane; side-by-side and stacked splits), mouse traversal across a boundary, per-window isolation, keyboard-focus cycling and the closed-focused-pane clear, seeded focus landing on the first *pane* (a lobby-surface-only ring seeds nothing, a mixed ring skips the surface at either end, and the surface is still reachable by Ctrl+Tab), and touch contact capture (pinned through drift, per-screen independence, duplicate-Started ignored, a closing pane releasing its contacts). All feature-agnostic, run by the ordinary `cargo test` |
| `tests/client/operator-surface-adapter.test.js` + `pane-scripts.test.js` | One imported v1 operator profile applies identical Accessibility, bindings and tuning in browser/native projections; native capability gaps suppress active gamepad/vibration without changing re-exported JSON; and the pane declaration runs before the shared client modules. `src/native_host/panes/document.rs` separately pins that the real pane document keeps one profile owner and no replacement route. |
| `tests/native_host_input.rs` | The pane input adapter builds a router over the real primary window's geometry and scale, resolves a synthetic point to the correct tiled pane, and runs its whole input + draw pipeline for many frames against a live Ultralight runtime without panic. `#[ignore]`d: needs the SDK, a real window and a GPU. Multi-monitor and multi-touch are the kit's — one monitor, no touch, on the dev box |
| `tests/native_host_pane_ultralight.rs` | The real built `client/index.html` loads in a real Ultralight view over this process's own HTTP, joins on the identity it read from the fragment, paints, answers a real click + keystroke on `#name-input` with a `SetName`, then claims a Station and operates its console: the iframe mounts with `__updateConsole` installed and a click on the Captain's Red Alert button inside it produces the expected `ControlSystem`. A second test proves two panes' `localStorage` are separate. Both `#[ignore]`d: they need the SDK and a built bundle, which CI has neither of |

Each of the two digest binaries stands alone on purpose: pinning the scheduler
means a one-thread `TaskPoolPlugin`, and Bevy's task pools are process-global and
fixed by whichever app in the process builds first, so a digest claim made in a
shared binary is a claim about whoever won that race.

## Related

- [Build & Deployment](./build-and-deployment.md) · [Networking](./networking.md) · [Architecture](./architecture.md)
- [Server HTML Lobby UI](./server-lobby-ui.md) — the lobby this surface renders, and the modules both surfaces share
- Issue #1121 — the host. Issue #1122 — local Ultralight panes. Issue #1325 — the host lobby on the native viewscreen (above). Issue #1329 — the join QR on that lobby, the vendored encoder, and `ToggleQrCode` as a wire message (above). Issue #1328 — the scenario/hull picker and the AI launch on that same surface, and force-start ceasing to be wasm-only (above). Issue #1123 — bridge display profiles (above). Issue #1125 — recovering a failed pane and a lost display (above). Issue #1112 — the transport. Issue #1124 — input routing between displays + pane→Station-window compositing (above). Issue #1126 — bridge media profiles: per-surface camera/microphone/output assignment (above). Issue #1334 — the saved per-ship-class bridge layouts (above). Issue #1335 — the guided acceptance kit for the whole of PRD #1324 on real monitors, `docs/acceptance/1335-native-lobby.md` (above).
- [vellum](https://github.com/jkeywo/vellum) `crates/vellum-ultralight` — the extracted plumbing; `docs/handbook/dependencies.md` records why `ul-next` stopped being a per-game exception
- `pasm/spec/architecture/native-delivery.yaml` — PRD #855's delivery declarations
