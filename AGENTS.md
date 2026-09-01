@RTK.md

# Project Phoenix — Bridge Simulator

## TL;DR

A browser-based spaceship bridge simulator. One browser tab shows a shared 3D view of space. Players join from phones by scanning a QR code, or by typing the five letters shown next to it — no installation. The host (view screen) runs Rust/Bevy compiled to WebAssembly and is the authoritative server; the client (phone console) is **pure HTML/CSS/JS** — no client-side WASM. Clients send inputs and receive state snapshots. Networking is the **Phoenix transport** (issue #1112) in a star topology: a Phoenix-owned rendezvous service (`worker-rendezvous/`, a Cloudflare Worker + Durable Object) carries typed join-code lookup and WebRTC signalling over a secure WebSocket, and the game traffic then runs over direct WebRTC DataChannels — a reliable ordered one for commands and reliable messages, and a lossy unordered one for the snapshot class. PeerJS and its public cloud broker were retired in #1112. When a network builds no direct link at all, the same rendezvous socket carries the game's own frames instead (issue #1113) — the two delivery classes preserved, the same admission gate, no second protocol — and that fallback is also how a browser client joins a **native** host, which has no WebRTC.

For the current feature set, read **[wiki/concepts/project-overview.md](./wiki/concepts/project-overview.md)** and the relevant PASM slice under [`pasm/spec/`](./pasm/spec/). Planned work lives on the GitHub issue tracker (label `PRD`). Domain vocabulary lives in **[CONTEXT.md](./CONTEXT.md)** — use those terms, don't invent synonyms.

---

## Wiki — Read It and Maintain It

This repo carries a **small LLM-maintained wiki** under `wiki/`. It indexes current code-oriented concepts and entities. Treat it as the orientation index, not as an archive of historical PRDs or design drafts.

**Read [wiki/SCHEMA.md](./wiki/SCHEMA.md) at the start of any non-trivial task.** It defines the layout, page conventions, and workflows.

- *Orienting* — open `wiki/index.md`, find candidate pages, read them; follow their `sources:` links when precision matters.
- *Recording current understanding* — update the affected `entities/` or `concepts/` page and `wiki/index.md` if pages were created. Record intended design and architecture in `pasm/spec/` instead of duplicating it in the wiki.
- *Answering a query* — synthesise from the wiki; update a current `concepts/` or `entities/` page only when the runtime understanding changes. Record design in PASM and planning in GitHub instead.
- *Linting* — **run the SCHEMA.md lint pass whenever you close out a PRD or a batch of issues**: check `file.rs:LINE` references still resolve, index entries match files on disk, and superseded material has been removed or replaced. This is the step that historically never ran; don't skip it.

The wiki is not a replacement for code, `README.md`, `CONTEXT.md`, this file, PASM, or GitHub issues. Code is runtime truth; PASM is in-repository design truth; GitHub is planning truth.

**Scenario design work starts with the digest, not the world file.** Before
reasoning about or changing a scenario's design (pacing, deadlines, causality,
consequences, the window arithmetic), run `uv run pasm design digest` and treat
its output as the design source of truth — it merges the declared intent in
`pasm/spec/design/` with the live authored values and answers questions like
"what happens if the crew miss `lyra_clear`?" without opening the 5,000-line
TOML. Tuning edits flow through `pasm design writeback` where possible (it is
hash-guarded, preserves the world file's commentary byte-for-byte, and refuses
values outside declared design bounds); structural script work is still a
direct edit, after which `uv run pasm validate` must stay green — a new
deadline the design model does not claim is an error by design.

---

## Common Commands

```bash
# ── CI gates you can run locally — ALL green before each PUSH, run ONCE ──────
# These are the three fast CI jobs (test, editor-test, pasm) in full. Anything
# red here fails the build. `cargo test` alone is NOT sufficient: clippy denies
# warnings, and `pasm validate` gates on the spec model. The remaining two jobs
# (build, smoke) need a WASM build — see the trunk/playwright commands below.
#
# Do NOT run this list after every edit, implementation pass, review pass, or
# issue commit — clippy alone is a near-full rebuild. While iterating, use
# `cargo check` plus targeted tests for the area you touched. After issue
# commits have been replayed onto local main, run the full list exactly once as
# the final gate between pushes. Review passes are read-only and run no gates.
cargo fmt -- --check                           # CI: test job, step 1
# NOT --all-features: it now implies an SDK download via the `ultralight`
# feature (issue #1122). This is the same explicit list ci.yml's clippy step
# uses — every feature Cargo.toml declares that GATES CODE except that one.
# `falling-skyway-sim-tests` is left out too and costs nothing: its only use is
# `#[cfg_attr(not(feature = …), ignore)]`, so it flips 55 tests between run and
# ignored and compiles not one extra line.
cargo clippy --workspace --all-targets \
  --features server,viewer,debug,headless,perf,host,capture -- -D warnings   # CI: test job, step 2
cargo test --workspace --features headless     # CI: test job, step 3
npm run debug-surfaces:check                   # CI: editor-test job (Rust -> JS drift)
npx vitest run                                 # CI: editor-test job (tests/client/*.test.js)
node scripts/check-strings.mjs --strict        # CI: editor-test job
npm run lods:check                             # CI: editor-test job (LOD drift)
npm run lod-captures:check                     # CI: editor-test job (billboard capture drift)
uv run pasm validate                           # CI: pasm job, gates on the spec model
uv run pasm scan                               # CI: pasm job, gating (scan: gate)
uv run pasm traceability                       # CI: pasm job, report (still exits nonzero on error)
#   `pasm validate` prints ~39 pre-existing informational warnings and exits 0
#   with `Status: OK`. That IS the green state — only `[error]` findings fail
#   the job, so do not go chasing the warning list before committing.
#   There is NO PASM pytest suite in this repo. The tool was de-vendored onto
#   the fleet copy in ada7a172 and its tests went with it to vellum; only the
#   spec model (pasm/spec/) is phoenix's. Editing a slice still needs the three
#   commands above, because they assert on that YAML — `cargo test` will not.

# Falling Skyway's 55 full-timeline simulations are manual-only. This enables
# them and runs only that scenario suite; the 14 narrow probe worlds stay in
# the ordinary native CI command above.
cargo test -p project-phoenix --features falling-skyway-sim-tests --test headless_runner falling_skyway_

# Quick compile check while iterating (not a CI gate)
cargo check

# Local dev — server page (WASM, Bevy, peer host)
trunk serve                                    # → http://localhost:8080

# Headless sim — no window, no renderer, player ship on AI backfill, fixed
# timestep as fast as the CPU allows. Prints a JSON exit summary.
cargo run --release --features headless --bin phoenix-headless -- \
  --world assets/worlds/combat_test.toml --sim-seconds 60
cargo run --features headless --bin phoenix-headless -- --help

# Balance batch runner — TOML matchup×seed matrix over phoenix-headless, fanned
# out in parallel; merges the per-run reports into win/loss/draw rates, TTK
# distributions, and damage margins (merged JSON + a markdown table). Needs the
# release binary above built + `npm install` (for smol-toml). Markdown → stdout;
# `--out <dir>` also writes merged.json + summary.md (keep that dir out of git).
node scripts/balance-runs.mjs scripts/balance-runs.example.toml [--out <dir>]
# Ratified Alliance Cruiser gate (issue #1082): the same 4-opponent fixed-seed
# duel matrix the CI `balance` job makes blocking. The output directory contains
# merged metrics plus every source AAR; all stations are Backfill in headless.
npm run balance:cruiser

# Local dev — client page (pure HTML/JS, no WASM)
node scripts/build-client.mjs                  # → dist/client/, then serve dist/ statically

# Native host (PRD #855 delivery + issue #1121 simulation) — ONE binary, two
# modes. With no --world it serves a built bundle, the content manifest, the
# scenario catalogue and a version stamp from a native process instead of an
# open browser tab: DELIVERY ONLY, the authoritative simulation is still
# server.html or phoenix-headless, and crew signalling goes through the
# rendezvous service exactly as it does in a browser. There is no TLS or auth
# in either mode — LAN or behind something else, never a public address.
cargo build --release --features host --bin phoenix-host
./target/release/phoenix-host --client-dir dist
#   Binds 0.0.0.0:8080 by default — LAN-reachable out of the box; Windows
#   prompts to allow it through the firewall on first run. Pass
#   --addr 127.0.0.1:8080 to restrict to this machine only. Both modes below
#   bind the same way.
#
# --world makes the SAME process authoritative, running the ordinary simulation
# and plugin graph with the shared viewscreen drawn by native Bevy/wgpu through
# winit (issue #1121). Every delivery flag keeps its exact meaning: the bundle
# serving, the catalogue restriction and the startup version pin are shared
# rather than duplicated, which is why this evolved instead of forking a second
# binary. Bevy owns the main thread (winit requires it on Windows) and the HTTP
# host moves to a worker with a shutdown path.
./target/release/phoenix-host --world assets/worlds/combat_test.toml --solo
#
# --lobby (issue #1326) is the SAME authoritative host with no world: it opens
# the viewscreen on an empty GamePhase::Lobby holding the merged scenario
# catalogue and ingests a world only once a SelectScenario + SelectPlayerShip
# pair has been arbitrated — the same first-valid-wins rule server.html runs in
# gui/scenario-arbiter.js, transcribed into the pure src/lobby/scenario_arbiter.rs.
# --world is that same decision made at the prompt, and the two are refused
# together. A BARE invocation (neither flag) is still PRD #855's delivery-only
# host, unchanged.
#
# Since issue #1328 the picker is ON THE VIEWSCREEN: with a --client-dir the
# lobby surface opens on server.html's own #scenario-panel, driven by
# gui/host-scenarios.js + gui/host-scenario-render.js — the same view model and
# the same renderer the host page uses — so the operator picks the world and
# then the hull with a mouse. The picks travel back over the host-lobby bridge
# as HostLobbyRecords and enter the arbiter as ClientMessages under
# LOCAL_CONSOLE_TOKEN, exactly as server.html's own picker submits them, so a
# phone and the viewscreen are equal senders under first-valid-wins. The lobby's
# AI-launch button works natively too: server::bridge::apply_force_start is no
# longer wasm-gated, so a crew who are all on phones can be launched from the
# viewscreen. Each flag still skips exactly the stage it decides — --world skips
# the scenario stage, --world --ship skips both, --lobby --ship skips the hull.
./target/release/phoenix-host --client-dir dist --lobby --rendezvous <URL> --origin <URL>
#   run-native.bat is the Windows wrapper for the two invocations above: no
#   argument runs the delivery-only host, `run-native.bat lobby` adds --lobby.
#   The default invocation is unchanged and the acceptance kits depend on it.
#   src/native_host/world_load.rs is the whole of it. The runtime load calls the
#   SAME boot::ingest_world a --world boot calls (it takes a &mut World for
#   exactly that reason) and the SAME app::install_world_selection, then runs a
#   RuntimeWorldLoad schedule restating Startup's topological spawn order —
#   including the compile_world_scripts < setup_world < spawn_world_entities
#   mint pin. WorldIdMint is parked at tick 0 across that pass and restored
#   after, so a world's entity uuids do not depend on how long the operator
#   spent choosing; tests/native_host_lobby.rs asserts a runtime-loaded world
#   mints exactly the ids a boot-loaded one mints AND mints each one to the same
#   entity, plus a second test comparing the two SCHEDULES so a system added to
#   WorldPlugin's Startup chain cannot silently not-run on the runtime path.
#   A successful runtime load also re-Welcomes every connected participant: a
#   phone that identified before the pick was welcomed with the world-less
#   lobby's battleship FALLBACK roster and nothing else would ever tell it
#   otherwise. A refused one puts the lobby back, content ledger included — the
#   failed load had already frozen it over content this host does not have.
#   --ship <PATH>   the player's hull [default: the world's first available_ships]
#                   With --lobby it also SATISFIES the hull half of the pick: a
#                   scenario lock alone then loads the world, because a
#                   SelectPlayerShip this host would discard anyway is not worth
#                   waiting for.
#   --seed <N>      overrides the world's [global] seed
#   --solo          start with nobody connected, every station on Backfill.
#                   WITH NEITHER IT NOR --pane NOR A LOBBY SURFACE, NOTHING
#                   REACHES A RUNNING MISSION: the host waits in a lobby nothing
#                   can enter (see the #1112 note below), and it says so loudly
#                   in the log at boot rather than refusing — the mode is
#                   correct, it is the transport that is missing. A --client-dir
#                   host DOES have a third route since issue #1328: the lobby
#                   surface's own AI-launch control.
#   --pane <NAME>   open a local Station pane for a participant of this name —
#                   an embedded Ultralight view showing the ordinary console
#                   surface (issue #1122). Repeatable. NEEDS a build with
#                   --features ultralight and a --client-dir: a pane loads the
#                   client bundle this same process serves, over HTTP, from a
#                   document the host publishes in memory at the client
#                   directory's own depth (so every relative URL in the page
#                   resolves as it does for a phone; gui/ is untouched).
#                   Each pane is an ordinary logical client with its own minted
#                   UUIDv4 session token. It must NOT use LOCAL_CONSOLE_TOKEN,
#                   which skips the station-tenure branch of
#                   is_command_authorized entirely and carries mission-abort
#                   authority — three separate gates enforce that.
#   --log / --log-entity  same grammar as phoenix-headless
#   THE CREW LOBBY IS ON THE VIEWSCREEN (issue #1325), with no flag, given a
#     build with --features ultralight and a --client-dir bundle whose
#     index.html is a host page (i.e. one `trunk build` wrote). It is the same
#     lobby server.html shows, rendered by the same gui/host-lobby-view.js +
#     gui/host-lobby-render.js over the same LobbyStatePayload, in an embedded
#     view fed by src/native_host/host_lobby/'s bridge. The surface is
#     PERMANENT: on mission start the chrome yields (invisible, and out of the
#     #1124 input router so it cannot swallow a viewscreen click), F9 reveals
#     and hides it in play, and returning to the lobby phase brings it back.
#     Missing either prerequisite is a log line and a blank surface, never a
#     refusal to start — chrome does not get to stop a mission.
#     THE JOIN QR IS ON IT TOO (issue #1329): the real crew code, drawn by the
#     shared gui/host-qr.js from a VENDORED encoder (gui/vendor/qrcode.js) this
#     process serves itself, so a bridge machine with no internet still shows a
#     code. Its URL points at the address the LISTENER bound, not the loopback
#     one the surface loaded from — pass --addr <lan-ip>:<port> if the machine's
#     routing cannot answer that (the boot log says when it could not). With no
#     --rendezvous, or --solo, the panel says joining is off rather than framing
#     a dead code. Shown in the lobby, hidden at mission start, and toggled in
#     play from the surface's own control (after F9) or a phone's settings menu.
#     THAT LOBBY CARRIES THE MONITOR ROW (issue #1330): one button per connected
#     display, the current viewscreen marked, and pressing another moves the
#     viewscreen onto it live, through the bridge_layout law. A windowed host
#     therefore ALWAYS runs the display applier and the runtime monitor watcher
#     now, even with no --profile: it synthesises a BridgeDisplayConfig from the
#     displays it finds. That config DESCRIBES rather than instructs — with no
#     --profile and no lobby press the window stays exactly where the OS opened
#     it (the #1121 behaviour), and only a press or an unplug moves it. An
#     explicit --profile still wins at boot, seeding the layout the row edits.
#     AND A SCREEN ROW ON EVERY STATION CARD (issue #1331): one button per
#     eligible monitor plus an off state, so pressing one opens THAT STATION'S
#     CONSOLE ON THAT MONITOR AT RUNTIME — a Station window and a seated pane
#     created on demand, not at init — and off closes it and frees the screen.
#     The console is an ordinary participant: it joins and claims through the
#     normal flow, may be released and re-claimed by anyone, and admission
#     cannot tell it from a phone. The station id in the layout decides only
#     WHICH console document opens on WHICH glass, never who may sit there.
#     A windowed host with a --client-dir therefore carries a PANE BUS whether
#     or not it was given a --pane, and (with --rendezvous) pairs it with the
#     relay through PairedTransport rather than replacing it — before #1331 the
#     relay's insert_resource silently overwrote the pane transport, so a
#     --pane on a crewed host was talking to nothing. The rows are reachable
#     mid-mission through the F9-revealed surface, so a replugged monitor's
#     console reopens without ending the mission; with one monitor they say
#     consoles need a second screen. Unplugging a screen closes the consoles on
#     it — through bridge_layout::reconcile plus
#     bridge_display::follow_layout_stations, NOT through the #1125 watcher's
#     pane_labels, which are the AUTHORED profile's PARTICIPANT panes only.
#     assigned_surfaces EXCLUDES a station-bearing PaneSlot from pane_labels to
#     keep that true: PaneSlot::for_station names its pane for its station, so
#     an authored station slot otherwise put a station id into a boot-time list
#     the watcher resolves against the LIVE bus — and unplugging the monitor the
#     profile named then closed a console the lobby had since moved elsewhere.
#     A station's console is the LAW's on every host, authored or not.
#     follow_layout_stations follows BOTH its inputs — the layout AND the
#     monitors — and drops a Station surface only when the LAYOUT stops naming
#     its monitor, never on a single absent winit frame; a frame reporting no
#     monitors at all does nothing and leaves the pass owed.
#     bridge_display::reconcile_seated_consoles then checks that the applier's
#     work LANDED: a seated station with no console on the bus, or with no
#     BridgeStationSurfaces slot to be composited onto, is rebuilt boundedly on
#     the same token (#1125's own budget) and, when that is spent, has its SEAT
#     GIVEN BACK through the law with a LayoutNotice the row renders — an honest
#     Backfill instead of a station card claiming a screen that is black.
#     A PANE NAME colliding with a station id on the loaded hull is REFUSED
#     (app::install_world_selection): pane names and station ids are one
#     namespace on the pane bus since #1331, so a screen row's off button would
#     otherwise close that person's console instead of the station's. BOTH
#     sources are checked — a --pane <NAME> flag AND a --profile
#     [[display.pane]] carrying a label and no station key — because the second
#     is what puts a station id back into assigned_surfaces's pane_labels: an
#     unplug of the AUTHORED monitor would then close the station's console the
#     lobby had opened on another screen and mint a fresh token, with the law
#     never having unseated anything. A Station window whose display blips and
#     returns is RE-ANCHORED to the new Monitor entity (winit re-applies
#     fullscreen only on a mode change), and BridgeLayoutResource::notices is
#     APPENDED to by every writer and drained by publish_bridge_layout, so a
#     press racing a console surrender cannot erase either sentence.
#   --manifest also narrows what this process FLIES, not only what it publishes:
#     with a curating manifest in force the default hull is drawn from that
#     manifest's allowlist (issue #917). An explicit --ship still wins.
#   The composition seam is BootProfile::NativeHost in src/boot/ — a fourth
#   profile, not a fourth hand-rolled App; src/boot/tests.rs's parity test
#   covers all four. A native host refuses to boot when the COMPOSED world's
#   declared templates (root + every extra_worlds child) or the selected hull
#   are not in the native config cache: every cache-only reader
#   (asteroids::lifecycle, lobby::server, server::{radar, reference_grid,
#   asset_preload}, server_app::world_setup, world::server) has NO filesystem
#   fallback and answers Default on a miss, so a configless boot would run a
#   plausible mission with the wrong numbers.
#   src/entities/template_preload.rs is the one strict populate every native
#   process shares.
#   --rendezvous <URL> --origin <URL>  IS the crew path (issue #1113), and the
#     answer to what #1121 deferred. The host registers with the rendezvous
#     service, prints its five-letter code at startup, and every crew member
#     reaches it over the service's WebSocket game relay — a native process has
#     no WebRTC, so it registers saying `transports: ["ws-relay"]` and joiners
#     skip the direct ladder instead of spending 90 s discovering that.
#     `--origin` is required and deliberately not defaulted: the service refuses
#     an upgrade whose Origin is not on its deployed allowlist. Without these
#     flags nobody can join and the host says so at boot — that is `--solo`.
#     src/native_host/transport.rs is the seam; relay_transport.rs is what
#     plugs into it and relay_socket.rs is the tungstenite half.
#     The socket REDIALS on a backoff if it dies, the way the browser host's
#     lostService() does — a native host has no viewscreen to reload, so a
#     dropped socket would otherwise end every route into a running mission.
#     A redial mints a NEW code: the old record died with the socket, so the
#     letters already read out resolve to nothing (same-code survival needs
#     persistence in the service and is #1115's, on both hosts).
./target/release/phoenix-host --help
#   --manifest assets/scenarios.demo.toml  IS the catalogue restriction — the
#     same lever `?manifest=` pulls in the browser (issue #917).
#   --client-dir is version-pinned at STARTUP against the manifest being served:
#     a bundle built for other content refuses to start, before the port is
#     taken. /host/manifest.json pins a running client's protocol per request.
#   The catalogue it publishes is the browser host's own — src/delivery/payload.rs
#     holds the single field list that wasm_get_scenario_catalog and the JSON
#     encoder both walk, so the two surfaces cannot drift.

# The native host's two determinism binaries. Each is its OWN test binary and
# that is load-bearing, not tidiness: pinning the scheduler means a one-thread
# TaskPoolPlugin, and bevy's task pools are process-global and fixed by
# whichever app in the process builds first — so a digest claim made in a
# shared binary is a claim about whoever won that race (the same reason
# tests/rng_determinism.rs and tests/snapshot_resume.rs stand alone).
cargo test --features headless --test native_headless_digest   # AC5, content half
cargo test --test native_host_snapshot                         # AC5, snapshot half

# The two native #[ignore]d GPU proofs (issue #1121). Both need a real GPU
# adapter, and the repo has nowhere to run one: every ci.yml job is
# ubuntu-latest and the one windows-latest runner (deploy-demo.yml's
# package-native-demo) runs no tests. Both render offscreen through the same
# wgpu path capture-billboard uses.
cargo test --features capture --test native_viewscreen_render -- --ignored --nocapture
cargo test --features headless --test native_headless_digest -- --ignored --nocapture
#   The render proof samples the MIDDLE 40% of the read-back frame and asserts
#   the browser spec's pair — more than one colour, and something lit. Both
#   cameras are retargeted at the offscreen image and the 2-D UI camera stays
#   active through InProgress, so a whole-frame colour count would pass on HUD
#   chrome over a dead 3-D scene; the crop is what makes it a claim about the
#   scene.
#   The digest companion runs the same 240-frame native↔headless comparison
#   under NativeRenderSurface::Offscreen — a REAL wgpu device — because the
#   default Contract composition stands up no render stack and so cannot
#   observe what one does to the main world.

# The two Cloudflare workers' deploy contract, as code (issue #1113): §3 and
# §3a of docs/delivery-checklist.md, which are otherwise two curl commands and
# a careful read at the end of a deploy. Takes LIVE urls; never a push gate, and
# the first precondition of docs/acceptance/1113-networks.md.
node scripts/check-rendezvous.mjs \
  --rendezvous https://phoenix-rendezvous.project-phoenix.workers.dev \
  --turn       https://phoenix-turn-credentials.project-phoenix.workers.dev \
  --origin     https://pp-dev.kiwigamedesign.co.uk
#   Exit 0 = both contracts hold, 1 = a real finding, 2 = unreachable. The
#   judgements are pure (scripts/rendezvous-checks.mjs) and fixture-tested.

# The rendezvous service on localhost, running the REAL registry over real
# sockets — no Cloudflare account, no wrangler, no dependencies. The only way
# to exercise a real WebSocket against this repo's own service.
npm run rendezvous:dev              # → http://127.0.0.1:8788
cargo test --features host --test native_relay_live -- --ignored --nocapture
#   The native host ↔ real relay ↔ real client round trip. #[ignore]d because
#   it needs the server above (or PHOENIX_RENDEZVOUS pointed at a deployed one).

# Deployed header/caching contract (PRD #855). Takes a LIVE url; run it after a
# public deploy, from a laptop (Node 20, no npm install) or by dispatching the
# `Check Deploy Headers` workflow. Never a push gate — the offline half of the
# contract is already covered by tests/client/deploy-headers.test.js and
# src/delivery/http.rs's unit tests.
node scripts/check-deploy-headers.mjs https://pp-demo.kiwigamedesign.co.uk/
#   The rules ship as deploy/cloudflare/_headers, installed into dist/ by
#   deploy-demo.yml. NO TWO PATTERNS IN THAT FILE MAY SET THE SAME HEADER —
#   Pages applies every matching rule and nothing here can test precedence.
#   Manual/credentialed deploy steps live in docs/delivery-checklist.md.

# LOD generation (issue #919) — regenerate a model's decimated levels from the
# `[lod.generate]` blocks in its own rig sidecar. Needs `npm install` (pinned
# @gltf-transform/cli); rewrites .glb files under assets/models and the
# manifest CI checks. `--plan` prints the work without running it.
npm run lods                                   # every declared LOD output
node scripts/generate-lods.mjs asteroid_common_1   # one model
node scripts/generate-lods.mjs --plan
#   --remesh runs the optional Blender voxel pre-pass (scripts/blender-voxel-remesh.py)
#   --adopt re-baselines the manifest from the files already on disk
#   A level with no remesh_voxel_size that regenerates LARGER than its
#   recorded baseline (a stubborn mesh) fails the run instead of just
#   warning; fix it with the Blender voxel pre-pass above, not --force.

# Billboard LOD capture provenance (issue #1245). Recapture is a local GPU
# operation; the check/adopt paths only hash committed inputs and PNGs and do
# not need the native binary. Capture parameters come from `[lod.capture]`.
node scripts/capture-billboards.mjs alliance_cruiser
npm run lod-captures:check
node scripts/capture-billboards.mjs --adopt alliance_cruiser
#   `--adopt` deliberately records a reviewed existing PNG; it is not the fix
#   for unexplained drift. Commit the PNG, sidecar and capture manifest together.

# Model / shader viewer — one model, real render path, switchable lighting.
# Use this to iterate on how things LOOK instead of booting a whole scenario.
npm run dev:viewer                             # → http://localhost:8081
#   (Windows: start-viewer.bat does the same and opens the browser)
#   ?model=assets/models/alliance_cruiser.glb  which GLB to show
#   ?variant=large                             which .model.toml rig variant
#   ?entity=assets/entities/sol.toml           render a [star]/[planet]/[mesh] instead
#   ?lighting=off|ambient|directional          ambient = the game's own default
#   ?gizmos=1                                  overlay rig markers + extents

# Production build (TRUNK_BUILD_RELEASE gates the wasm-opt-fixup post_build
# hook in Trunk.toml — see scripts/wasm-opt-fixup.mjs; requires `npm install`)
TRUNK_BUILD_RELEASE=true trunk build --release
node scripts/build-client.mjs

# Public demo build: same command plus PHOENIX_DEMO_BUILD=true, which is a
# SEPARATE flag (src/build_flags.rs, option_env!) that hides the host settings
# cog's Debug/Cheat tab. Only .github/workflows/deploy-demo.yml sets it —
# ci.yml's GitHub Pages deploy is the dev host and keeps its debug tooling.
# Since #940 the same variable ALSO reaches the compiler as a cfg: build.rs
# turns it into `phoenix_demo_build`, which DELETES five things from a demo
# binary rather than merely refusing them —
#   - the god-mode cheat route (src/command_admission/debug_route.rs),
#   - ClientMessage::ToggleDebugFlag and its drain,
#   - ClientMessage::TogglePause and its drain, and
#   - the host mod-pack upload export, `wasm_add_mod_pack` (PRD #855), and
#   - the host diagnostic mutation export, `wasm_set_debug_surface` (#1267).
# The third is the blunt one: nothing server-side checks station, captaincy or
# GamePhase before honouring a client pause, so any one of N demo players could
# otherwise freeze the mission for everyone, repeatedly. The HOST's own pause,
# on the server cog (#939), is untouched in every build. A demo binary does not
# decode either client message at all, so the hidden control and the closed
# route cannot come apart.
# The fourth is the catalogue restriction's other half: a demo build curates the
# catalogue down to combat_test + the Alliance Destroyer and Alliance Cruiser
# (assets/scenarios.demo.toml, #931), and a mod-pack upload adds whatever
# scenarios and hulls a ZIP carries. `build_flags::accepts_mod_pack_uploads()`
# states the rule, gui/build-flags.js's `offersModPackUpload` removes the
# button, and the export is gone — same doctrine as the cheat route. The overlay
# READERS (wasm_clear_mod_pack / _remove_ / _reorder_ / _active_pack_manifest)
# stay in every build: server.html calls them unconditionally and, with nothing
# able to enter the stack, they answer emptily — gating them would turn a no-op
# into a TypeError.
# deploy-demo.yml checks the generated wasm-bindgen glue for BOTH privileged
# mutation exports. Native cfg tests cannot prove a wasm32 export was actually
# removed from the artifact being shipped.
# The client page has no WASM to bake anything into, so it learns the
# flag from the `phoenix-build-demo` meta tag the deploy workflow stamps.
# The literal both halves compare against lives in ONE place —
# src/demo_build_value.rs, `include!`d by build.rs and src/build_flags.rs —
# because a build script cannot `use` the crate it builds and two copies
# could only diverge in a build nothing but deploy-demo.yml produces.

# The demo cfg compiled and tested (ci.yml's "demo-build gate tests" step):
PHOENIX_DEMO_BUILD=true cargo test --lib -- \
  build_flags command_admission::debug_route route_is_absent_from_a_demo_build
# deploy-demo.yml runs no tests, so without this step nothing in the repo ever
# compiles a `#[cfg(phoenix_demo_build)]` body. Run it after touching the gate.
# Do NOT filter on a module that is itself cfg'd out — debug_overlay's
# client_route tests are, so naming them would match zero tests and pass
# vacuously, which is the exact trap this step exists to close.

# Smoke tests (Playwright, Chromium) — requires dist/ built first
cd tests/smoke && npm install && npx playwright install chromium
npx playwright test                            # from tests/smoke/
#   PHOENIX_SMOKE_PORT=3100 npx playwright test  — serve dist/ on another port
#   AND refuse to adopt an existing server. `reuseExistingServer` otherwise
#   silently picks up a stale `npx serve` on 3000 (another worktree's dist/, or
#   one built hours ago), and the whole suite then times out against a bundle
#   with nothing to do with the change under test. If a local run reports every
#   spec failing, check that FIRST: it looks identical to a broken build.

# ── Performance measurement (issue #868, gating decided in #905) ─────────────
# Captures are compared against committed baselines in perf/baselines/*.ron.
# ONE of the four scenarios gates: `assets`, because bytes on disk and counts
# in authored TOML are a function of the checkout rather than of the machine.
# The rule for the others is in src/perf/mod.rs; the short version is that
# wall-clock on a shared runner stays non-gating until post-demo.
cargo run --release --features perf --bin phoenix-perf -- assets --capture target/perf/assets.json
cargo run --release --features perf --bin phoenix-perf -- mesh   --capture target/perf/mesh.json
cargo run --release --features perf --bin phoenix-perf -- report --capture target/perf/assets.json --gate
#   `mesh` resolves top-level entity templates (including composed fragments),
#   follows each selected rig sidecar, and loads every runtime-reachable GLB
#   level through Bevy's own loader (headless, one model at a time). Remesh
#   intermediates and unrelated files are excluded; the aggregate triangle
#   population counts only each model's deduplicated first/near level. Minutes,
#   not seconds — it decodes every embedded texture in the reachable levels.
#
# Re-recording a baseline from the runner that compares against it. CI cannot
# commit, so the perf job uploads the baselines it WOULD record and a human
# adopts them into a reviewable diff:
gh run download <run-id> -n perf-capture -D target/perf-artifact
cargo run --release --features perf --bin phoenix-perf -- adopt --artifact target/perf-artifact
git diff perf/baselines
#   Adoption moves the numbers and keeps the judgement: statistics, tolerances
#   and header prose survive. Write commentary in the HEADER — the RON value
#   below it is regenerated. See src/perf/baseline.rs.

# CI: ci.yml — eight jobs. `pasm`, `test` and `editor-test` run in PARALLEL and
# gate independently (any one of them red fails the build); `build` needs
# `test`; `smoke` needs `build`; `perf` needs `test` and `smoke`; `balance`
# needs `test`; `deploy` runs on main and needs none of `perf`/`balance`.
#
#   pasm         uv run pasm validate ; uv run pasm scan — both through
#                vellum's `pasm-validate` composite action (fleet-standard,
#                pinned by rev) ; then uv run pasm scan/traceability --json
#                uploaded as the `pasm-reports` artifact. No pytest step.
#   test         cargo fmt --check ; cargo clippy --workspace --all-targets
#                --features <every feature but `ultralight`> -D warnings ;
#                cargo test
#   editor-test  npm run debug-surfaces:check ; npx vitest run ;
#                node scripts/check-strings.mjs --strict ; npm run lods:check ;
#                npm run lod-captures:check
#   build        TRUNK_BUILD_RELEASE=true trunk build --release ;
#                node scripts/build-client.mjs
#   smoke        npx playwright test (against the built dist/)
#   perf         phoenix-perf assets|mesh|report — GATES on the `assets`
#                scenario only (report --gate, exit 3); every other scenario
#                reports into the job summary and the perf-capture artifact
#   balance      destroyer report (non-gating) plus the ratified cruiser matrix
#                (`scripts/balance-runs.cruiser.toml`, gating)
#   deploy       peaceiris/actions-gh-pages@v4 — publishes dist/ to GitHub
#                Pages; main branch only, no gate of its own
#
# Keep this list in sync with .github/workflows/ci.yml — if you add a gate
# there, add it above, and vice versa. Trusting a stale list here is how a
# batch lands "green" and breaks the build.
```

Prerequisites: Rust stable + `rustup target add wasm32-unknown-unknown`, `cargo install trunk`, node/npm.

---

## Message Flow (The Core Loop)

### Getting on the wire (the Phoenix transport, issues #1111/#1112)

```
server.html initHostTransport()          gui/rendezvous-transport.js
  ↓  wss:// to the rendezvous service, `host-open`
worker-rendezvous registry               worker-rendezvous/src/registry.js
  ↓  mints a private five-letter suffix, answers `hosted`
server.html showJoinCode()
  ↓  paints the letters + a QR of client/index.html#<PROJECT_VERSION_CODE>
client.html startPhoenixJoin()           five letters typed, pasted, or scanned
  ↓  wss:// `join` with the full code → `joined` (unknown / wrong-type /
  ↓  version-mismatch are three distinct refusals)
  ↓  SDP + ICE relayed by the service; the JOINER creates BOTH channels:
  ↓    'reliable'  {ordered: true}                — commands, reliable messages
  ↓    'snapshot'  {ordered:false, maxRetransmits:0} — the snapshot class
  ↓  in-band JoinHandshake { stamp } on the reliable channel
server.html checkStamp → wasm_check_client_stamp → delivery::check_join_stamp
  ↓  JoinAccepted (or JoinRefused with a StampMismatch code — a stamp is
  ↓  REQUIRED since #1112) — the page only sees an ADMITTED connection
client.html → Identify { token, name }   the ordinary crew protocol starts here
```

The signalling socket is expendable once the DataChannels are up. A link that
drops is re-resolved against the SAME code on a backoff, with the same session
token re-sent as `Identify` — the host restores the held station and pushes the
current projection, and nobody re-types five letters.

### Assembling a fleet (issue #1114)

A multi-ship session adds ONE privileged code in the `server` namespace, which
admits ship HOSTS and never reaches the simulation. The fleet lead opens a
SECOND rendezvous registration beside its own crew one; a second host types
those letters (or opens `server.html#<full code>`), passes the same in-band
`JoinHandshake` — judged by the STRICTER `delivery::check_host_stamp`, which
drops both of the crew path's leniencies — and then speaks a separate
vocabulary, `gui/host-mesh.js`'s `{ m, t, tick, d }` envelope. A fleet member
sends no `Identify`, holds no session token and never reaches
`wasm_receive_message`, so each crew star stays on its own host structurally
rather than by a filter. `gui/fleet-session.js` is the wiring; `gui/host-mesh.js`
is the pure model. #1114 assembles and freezes a fleet; running one as a shared
deterministic mission is #1116's, which is what the unset `tick` field is for.

Two rules that are load-bearing and easy to undo by accident:

- **A host's content identity is established on every boot path.** The stamp
  both sides compare comes from the scenario manifest, so `server.html` pushes
  it in `pushScenarioManifest()` — from the `?scenario=` bypass as well as from
  the catalogue build. `check_host_stamp` refuses an EMPTY identity on either
  side rather than comparing two of them, so a missed push is a loud refusal
  instead of two hosts agreeing on nothing.
- **The `namespace` on a `resolve`/`join` frame names the FIELD, never the
  code.** `parseJoinCode` reads a full code's namespace out of the project GUID
  inside it, so sending that makes the asker agree with the record by
  construction and "you typed the other kind of code" becomes unanswerable.

### Once on the wire

```
Player phone (client.html, pure JS)
  ↓  sends JSON on the reliable DataChannel
server.html JavaScript: attachHostConn()
  ↓  Identify gate resolves rendezvous peer → session token,
  ↓  calls wasm_receive_message(token, json)
server/bridge.rs: drain_inbound()
  ↓  queues InboundMessage into Bevy's pull-based message system
lobby/server.rs (or console plugins via SimSet::Input)
  ↓  reads InboundMessage, mutates SessionManager / ship state
  ↓  writes OutboundMessage events
server/bridge.rs: flush_outbound()
  ↓  encodes ServerMessage → JSON, calls JS callback with a DeliveryClass
server.html JavaScript: routeOutbound(target, payload, deliveryClass)
  ↓  gui/host-peer-routing.js resolves 'all' | 'token:<t>' | 'except:<t>' to
  ↓  connections, preferring the lossy channel for 'snapshot' and falling back
  ↓  PER TOKEN to the reliable one for any client that has none
client.html JavaScript: handleMessage()
  ↓  gui/sim-state.js apply() folds message into client state
  ↓  gui/console-state.js build*() → JSON pushed into per-console iframes
```

In-game commands are `ClientMessage::ControlSystem { target: SystemId, payload }` — humans and AI issue the same commands. See `wiki/concepts/message-flow.md` and `wiki/concepts/networking.md`.

---

## File Layout

```
src/
  core/         — Wire types (messages.rs, incl. FlagKind), codec, broadcast/ (Broadcaster seam)
  lobby/        — Session management, station assignment, lobby handler (pure + Bevy)
  ship/         — Physics, damage, power, shields, sensors, ratings, system registry, coordination (mostly pure)
  weapons/      — Phaser, torpedo state machines + beam renderer
  modifiers/    — Modifier cache, repair teams, coordination plugin
  asteroids/    — Deterministic density spawner + AsteroidWindow lifecycle
  regions/      — Region containment, effect components, shape types
  entities/     — TOML entity config types, config cache (JS fetch), spawner, loader
  world/        — WorldPlugin, parse_world, runtime trigger/comms evaluators
  ai/           — NPC AI plugins (same ControlSystem commands as players)
  comms/        — Comms range check + component
  console/      — Per-console SERVER plugins: captain, comms, helm, navigation, repair, weapons
  console_ai/   — Server-side AI controllers for systems under AI control
  delivery/     — How a host publishes its client, manifest, catalogue and
                  version pin (PRD #855). Bevy-free; compiles on BOTH targets on
                  purpose (the catalogue field list and the pin are shared with
                  the browser host); only delivery/serve.rs is native-only
  server/       — wasm-bindgen exports, renderer, viewscreen border
  gui/          — Rust-side GenericRadar UI widget (server viewscreen)
  server_app.rs — Server App builder: plugin registration + SimSet chain ordering
  sim_sets.rs   — SimSet: Input → Physics → Damage → Modifiers → Publish → PublishAggregate → Broadcast
  sim_tick.rs   — The fixed logical tick (SimTick counter + Time<Fixed> reconcile); SimSet runs in FixedUpdate

gui/            — CLIENT: pure JS modules + one HTML file per console (iframe),
                  mount-plan.js owns the station-id → DOM-id/URL mount plan
assets/         — TOML configs: worlds/, entities/, factions/; models, shaders, sounds
server.html     — Host page: loads server WASM, runs Bevy, registers with the
                  rendezvous service and owns the per-token connection maps
client.html     — Client page: pure HTML/JS, joins by typed five-letter code or
                  by the structured code a QR link puts in the URL fragment
worker-rendezvous/ — The rendezvous service (Cloudflare Worker + Durable
                  Object): typed join codes, presence, WebRTC signalling relay,
                  and (issue #1113) a bounded game relay for when no direct
                  link can be built. `src/registry.js` is the whole protocol as
                  a pure state machine, `src/relay.js` the game relay's
                  mailboxes; `src/index.js` only terminates the socket. NOT
                  DEPLOYED yet — see docs/delivery-checklist.md
gui/host-mesh.js  — HOST-to-host protocol + the pure fleet-lobby model (#1114);
gui/fleet-session.js — its two ends wired onto the transport. Separate from the
                  crew protocol by design; see "Assembling a fleet" above
docs/acceptance/ — Step-by-step kits for the human half of a HITL issue, one
                  file per issue. `1113-networks.md` is the field script for
                  connecting across real networks.
tests/client/   — Vitest tests for gui/*.js
tests/smoke/    — Playwright smoke tests
wiki/           — LLM-maintained knowledge base. Read SCHEMA.md first; update as you work.
docs/           — Draft design notes (numbered).
```

---

## Key Constraints & Rules

1. **`serde_json` only in `codec.rs`.** Never import it directly in other modules. (Planned exception: PRD #116's own save path, which does not exist yet. The module issue #862 actually created is **`src/snapshot.rs`**, and it is deliberately *not* that exception: a world snapshot is written as RON inside `vellum-save`'s envelope, so it imports no `serde_json` at all. If #116 ever lands a JSON save, it needs its own line here rather than inheriting this one.)
2. **Server = authority.** Bevy runs the simulation and decides everything; clients are stateless spokes that never talk to each other. Session tokens are the identity system — rendezvous peer ids and DataChannels are ephemeral. A token is **32 lowercase hex characters** (`crypto.getRandomValues(new Uint8Array(16))`, not a UUIDv4) held **primarily in `sessionStorage`**, so two console tabs on one desktop are two distinct players; `localStorage` holds a *persistent* copy that the first/only tab adopts, so one phone reconnects onto its station after a full browser restart. The resolution rule is the pure `decideToken()` in `gui/session-token.js`, backed by a `phoenix-live-tabs` liveness registry with a 2 s heartbeat and a 6 s TTL. Reserved token shapes (`__local_console__`, the `ai:` prefix) are refused at the network edge in `server.html` *and* in `lobby/handler.rs`.
3. **Client is pure JS.** No client-side Rust/WASM, no new Rust glue for the client. Client state is built by pure `gui/*.js` modules (Vitest-tested); console UIs are per-console HTML iframes.
4. **Captain authority.** Only the player at `CaptainChair` can set Red Alert (`SetRedAlert { active }`). Game start is collective `SetReady` auto-start, not a captain-only command.
5. **Station ownership is authoritative.** `Player.station: Option<StationId>` is the ownership field; console access derives from the station + `ShipConfig`. On disconnect the station keeps its holder and flips to the `Backfill` rating (AI operates its systems) until reconnect or a new claim.
6. **Humans and AI are symmetric.** Both issue `ControlSystem { target: SystemId, payload }`; admission strips source identity. Never branch on human-vs-AI downstream of admission. The command log (issue #898) keeps this at the *recording* site too: it records everything the network boundary admits, without asking what a token looks like. What stays out of it is what a replay re-derives — the in-process AI emissions of `emit_ai_command`, which never cross that boundary.
7. **AI decisions run on fixed ticks, not frames.** The whole simulation advances on a fixed logical tick (issue #895): `SimSet` is configured in Bevy's `FixedUpdate` at the TOML-authored `[global] sim_tick_hz` (default 60 Hz), counted by `SimTick` (`src/sim_tick.rs`). Helm commands apply the tick they are admitted (`AdmittedCommands` is cleared and refilled at admission each tick). **Every** AI policy host — the six per-axis helm systems, shield focus, power allocation, torpedo load/auto-fire, frequency hint, phaser and blaster auto-fire, AI target selection, Captain, Sensors — runs under `run_if` on the one shared cadence in `src/ai/cadence.rs`, derived from the tick count as `sim_tick_hz / ai_tick_hz` logical ticks per decision (default 30 Hz; the slower `ai_snapshot_hz` cadence is a further whole multiple; both ratios are validated at world load), never once per rendered frame and never off a wall clock. An ungated sim system now runs once per *logical tick* — still gate deciders that must run slower. Never gate a decider inside its own body with an `Option<Res<_>>` that falls back to running every tick: every bare-`App` fixture takes that arm, so the shipped cadence ends up covered by no test at all (issue #889). **A command applies on the tick it is STAMPED for, and that tick travels with it (issues #898, #1116).** A logged command carries the tick it applies on; `command_admission::log::CommandDelay` is the gap between admission and that tick.

  - **A lone host runs at `0`**, so the apply tick *is* the admission tick and "helm commands apply the tick they are admitted" holds exactly as it always did. Nothing in single-player play changed.
  - **A host in a FLEET runs at the mission's authored `[global] command_delay_ticks`** (default 6 — 100 ms at 60 Hz; validated at world load like the tick ratios beside it). Issue #1116 is the deliberate amendment this rule always said a non-zero delay would be, and it is a change of *value*, not of plumbing: the tick is still written on the command, still the key the future-tick queue drains on, and still the tick the log records. The delay is what buys agreement — every host has every peer's input for tick *T* before it simulates *T*, and a host that does not withholds the tick honestly (`Time<Virtual>` paused, so the tick never begins) rather than speculating.
  - **`crate::lockstep::join_fleet` is the only writer of `CommandDelay`.** That is what keeps the amendment deliberate: a delay cannot appear anywhere else in the plumbing by accident, and leaving a fleet puts it back to zero.
  - Commands from different hosts are ordered by `CommandOrder` — `(origin fleet slot, that slot's own sequence)` — a **peer-independent** total order, never by any receiver's arrival index. The log is written when a command APPLIES, so two hosts of one mission write byte-identical logs; comparing them is the first step in diagnosing a divergence.
  - `LocalShip` says which ship's crew is on THIS machine, so it is a different ship on each host of a fleet. **Nothing it gates may reach the authoritative digest** — `tests/local_ship_neutrality.rs` is the standing guard, and `tests/lockstep_mesh.rs` runs two hosts through one mission and folds both after every tick.
8. **Deterministic asteroids.** Per-cell density is seeded from `(layer salt, gx, gz) + Perlin noise` over the single composed lattice. Destroyed asteroids respawn fresh when the player leaves the cell and returns.
9. **WebGL2 rendering; Phoenix-owned rendezvous** (`worker-rendezvous/`, issue #1112 — no third-party broker). TURN credentials still come from the separate stateless `worker/`. **Neither worker is deployed by CI**: both are manual `wrangler deploy` steps on the delivery checklist, and since PeerJS was retired an undeployed rendezvous service means *nobody can join at all*, not a degraded connection.
10. **Pure modules are Bevy-free.** `lobby/handler`, `radar`, `ship/{damage,physics,rating,control_source,coordination}`, `modifiers/repair_teams`, `world/{content,flags,dispatch,layers,scenario,delayed}`, `comms/{content,range}`, and friends have no Bevy imports — fully unit-testable on native. Where a pure module needs a Bevy adapter, the adapter is a sibling (`ship/*_systems.rs`, `comms/server.rs`, `world/server.rs`) — never an import into the pure file.
11. **No hardcoded gameplay values.** All gameplay data (stats, icons, colours, sizes, behaviours) comes from TOML config, loaded into entities/components and sent over the network where the client needs it. The only acceptable hardcoded values are: (a) defaults applied while parsing a TOML file (`unwrap_or(...)`-style fallbacks), and (b) client-side placeholders shown while waiting for authoritative data from the server. If a value could plausibly be tuned by a designer, it belongs in TOML — never inline it "for now", and never add a hardcoded branch that can override what the config says. **Display text is the one sanctioned exception**: it lives in `assets/strings/strings.csv` (not TOML), referenced by string id — see `docs/strings-authoring-guide.md`. Never hardcode player-visible English in Rust, JS, or HTML; `scripts/check-strings.mjs --strict` gates this in CI.

---

## Logging

Use the `plog!` family in `src/logging/`, never bare `println!` / `eprintln!` /
`web_sys::console`. Two filter dimensions, configured identically on both
targets by the same parser:

```bash
phoenix-headless --log info,ai=debug,admit=trace --log-entity Ironveil
server.html?log=info,ai=debug,admit=trace&log_entity=Ironveil
```

Categories are the `LogCat` enum (`ai helm weapons shields damage power sensors
comms repair nav captain lobby admit world regions physics broadcast assets
config`); the entity filter matches `EntityName` — exactly first, then
case-insensitive substring.

```rust
fn my_system(log: Option<Res<LogFilterConfig>>, q: Query<(Entity, &Hull)>) {
    pdebug!(log, LogCat::Damage, entity = e, "hull now {}", hull.current);
    pwarn!(log, LogCat::World, "scenario {path} missing");   // no entity
}
```

Two rules that are easy to get wrong:

1. **Take `Option<Res<LogFilterConfig>>`, never bare `Res<_>`.** A bare `Res`
   fails Bevy parameter validation in any app that never inserted the resource
   — which is every bare-`App` unit test in this crate. `None` falls back to
   warn with no entity filtering.
2. **Plain helper fns with no config in scope** keep a bare
   `warn!(target: LogCat::Config.target(), ...)` rather than growing a
   parameter for it.

---

## Testing Strategy

- **Rust unit tests (`cargo test`):** inline `#[cfg(test)] mod tests` covering the pure modules (session, stations, codec round-trips, lobby handler, physics, damage, repair teams, modifiers, power, ratings, system registry).
- **Large inline test modules relocate to a sibling file (issue #1180; pattern piloted on `core/codec.rs`, prior art `console/weapons/mod.rs` + `server_tests.rs`).** Once an inline `#[cfg(test)] mod tests { ... }` crosses roughly **1,000 lines**, move its body out of the production file into a sibling — `<module>_tests.rs` next to `<module>.rs` (e.g. `codec_tests.rs`; console plugins use the established `server_tests.rs` name) — and leave one line behind:
  ```rust
  #[cfg(test)]
  #[path = "codec_tests.rs"]
  mod tests;
  ```
  The sibling is the old `mod tests { ... }` body dedented one level, unchanged otherwise: same `use super::*;` (still resolves — `super` is the production module, unaffected by where the file lives) plus whatever other imports the tests need, same `#[test]` fns, same fixture helpers. This is a **test-only move** — it must not touch a line of production code and must not change what a test does, only where it lives; a relocation commit's diff on the production file should be `-mod tests { ... }` / `+#[path] mod tests;` and nothing else.
  Companion convention for the fixture bodies themselves: an inline literal (a large JSON payload, an embedded Rhai program) that's reused by name across several tests hoists to a named `const`/helper item near the top of the sibling, instead of staying duplicated inline at each call site. A literal used by exactly one test stays inline next to the assertion it supports — hoisting a single-use fixture away from its only reader makes the test harder to read, not easier.
- **JS tests (`npx vitest run`):** `tests/client/*.test.js` covering the pure `gui/*.js` modules (state builders, action map, registries, panels) and the pure `scripts/balance-runs.mjs` merge/format/expand fns (`tests/client/balance-runs.test.js`, fabricated report JSON — no sim).
- **Smoke tests (`tests/smoke/`, Playwright):** boot real server WASM in headless Chromium with a `BroadcastChannel`-backed transport stand-in, `tests/smoke/rendezvous-shim.js` (CI has no real WebRTC and no deployed worker). It fakes only the WebSocket and the `RTCPeerConnection`: the rendezvous protocol it terminates is the REAL `worker-rendezvous/src/registry.js`, imported into the host page. `tests/smoke/transport-fixture.js` is the single seam every transport assumption lives behind — `fixtures.js` and the ~40 specs that use `readHostPeerId`/`createTestClient` know nothing about it. `tests/smoke/transport-shim.spec.js` tests the stand-in itself. Two projects in `playwright.config.js`: `chromium` runs the message/DOM specs with no GPU (`src/server/bridge.rs` skips `RenderPlugin` under `navigator.webdriver`), and `render` runs `*.render.spec.js` under SwiftShader with that flag hidden, so the viewscreen actually draws. `npx playwright test` runs both.
- **Viewscreen render check (`tests/smoke/viewscreen.render.spec.js`):** boots combat_test and falling_skyway to a live viewscreen and reads canvas pixels back through a screenshot, asserting the scene area is not one flat colour. It exists because a render-graph break need not log anything — the PRD #1023 HDR regression turned the canvas black with a completely clean console (see `render_setup::apply_target_hdr`), and no other test in this repo draws a frame. Covers both the shipped `[render]` defaults and the documented `hdr = false` retreat.
- **Headless runner (`tests/headless_runner.rs`, `--features headless`):** boots the whole simulation natively with nobody connected and asserts on end state. Lives in an *integration* test, not an inline `mod tests`, because building a headless app populates the process-global native template cache — inside the lib test binary that leaks into ~2500 unrelated unit tests. Anything calling `config_cache::insert_native_config` belongs here.
- **PASM model checks (`uv run pasm validate`, `uv run pasm scan`, `uv run pasm traceability`):** the fleet tool's own deterministic checks over the design model in `pasm/spec/` — reference integrity, cross-domain links, declared-versus-observed drift, traceability roll-ups. **These assert on the spec YAML, so editing a slice can fail them without touching a line of Rust.** `cargo test` will not catch it; CI's `pasm` job will. Run them whenever you touch `pasm/spec/`. `validate` is green at `Status: OK` with ~39 informational warnings and exit 0. There is no pytest suite here — the tool, and its tests, live in [vellum](https://github.com/jkeywo/vellum) (de-vendored in `ada7a172`); see [pasm/README.md](./pasm/README.md).
- **Not tested:** renderer visual *fidelity* (what the picture looks like — that is what the `*.capture.js` aids are for), bridge internals, CI pipeline. Whether the viewscreen draws *at all* IS tested — see the render project above.

> Good tests: set up state → perform action → assert on observable output through the public interface. Do NOT assert on private fields, internal call counts, or implementation-specific details.

See `wiki/concepts/testing-strategy.md` for the file-by-file breakdown.

---

## Cargo.toml Notes

```toml
[lib]
crate-type = ["cdylib", "rlib"]  # cdylib for WASM, rlib for testing

[features]
default = ["server"]
server = []   # host build → server.html (bridge.rs compiled in)
host = ["server"]
              # native host binary → phoenix-host (PRD #855 delivery + issue
              # #1121 simulation). It still gates no code of its OWN:
              # `crate::delivery` is unconditional, because a feature-gated copy
              # of the catalogue contract would be the fork PRD #855 forbids —
              # and would leave its tests out of the plain `cargo test` CI runs.
              # It DEPENDS on `server` because #1121's authoritative half draws
              # the viewscreen through `crate::native_host`, which names the
              # presentation `crate::server::{renderer,viewscreen_border}`
              # plugins. Without that dependency `--no-default-features
              # --features host` compiles a `main` naming a module that is not
              # there — a combination CI never builds, so nothing would catch it.
ultralight = ["host", "vellum-ultralight/ultralight"]
              # local Station panes in the native host (issue #1122). Gates ONE
              # module — `native_host::panes::ultralight` — and links the
              # Ultralight SDK through vellum-ultralight. NOTHING ELSE MAY EVER
              # IMPLY IT: `ul-next-sys`'s build script DOWNLOADS a proprietary
              # ~100 MB archive at build time, every ci.yml job is
              # ubuntu-latest, and a plain `cargo test` must not pay a download
              # to run four thousand unit tests. Everything about what a pane
              # may say and hear (identity, registry, routing, the frame loop,
              # the document) compiles and is tested with the feature OFF,
              # because that is where the acceptance criteria live.
              #
              # DEFAULT-OFF IS NOT ENOUGH, and assuming it was is how this
              # reached CI once already: `--all-features` enables it. The
              # clippy step in ci.yml — and the local gate command near the top
              # of this file — therefore name their features EXPLICITLY, as
              # every feature declared here except this one. Adding a feature
              # means adding it to both lists.
# The client page (client.html) is pure JS (gui/*.js) — there is no
# `client` cargo feature and no client-side WASM (removed in #463).

# WASM: needs the getrandom wasm_js backend.
# Physics is SERIAL on both targets (issue #896): the native `parallel` feature
# is off, because a parallel broadphase orders contacts differently from the
# serial one the browser is stuck with. (See [target.'cfg(...)'] sections.)
```

---

## Deployed URLs

- Server: `https://pp-dev.kiwigamedesign.co.uk/`
- Client: `https://pp-dev.kiwigamedesign.co.uk/client/`
- Server QR encodes: `https://pp-dev.kiwigamedesign.co.uk/client/index.html#<PROJECT_GUID>_<VERSION_GUID>_<CODE>` — the same structured code whose five-letter suffix is printed beside it, so scanning and typing are one join by two routes.

---

## Adding New Message Types

When extending `ClientMessage` or `ServerMessage` (prefer a new `SystemControlPayload` variant over a new top-level `ClientMessage` for in-game commands):

1. Add the variant in `core/messages.rs` (derive `Clone, Debug, Serialize, Deserialize, PartialEq`)
2. Add a round-trip test in `core/codec.rs` (`codec-tests` module)
3. Handle in `lobby/handler.rs` `process_message()` (pass through or produce outbound), or in the appropriate console server plugin (`.in_set(SimSet::Input)`) for in-game messages
4. Client inbound: fold into state in `gui/sim-state.js` `apply()` (or `gui/comms-state.js` / `gui/lobby-state.js`), then surface via the relevant `build*()` in `gui/console-state.js`
5. Client outbound: add the UI action to `gui/action-map.js` (and the button/control to the console's `gui/<name>-console.html`)
6. Add/extend Vitest coverage in `tests/client/`
7. Touch `server.html` `routeOutbound()` / `client.html` only if routing or the join handshake changes. The transport-plane frames (`JoinHandshake`/`JoinAccepted`/`JoinRefused`) are deliberately NOT `ClientMessage`/`ServerMessage` variants — see `gui/rendezvous-transport.js` and `pasm/spec/design/p2p-design-deltas.yaml`. **Host-to-host traffic is a third vocabulary and belongs in `gui/host-mesh.js`, never here**: its envelope is `{ m, t, tick, d }` precisely so a host frame that lands on the crew wire is refused by a decoder switching on `.type` rather than half understood.

## AI-origin decisions

A decision you (an agent) make while working is marked in the spec:
`origin: ai` on the entity you originated, or a literal `[ai] ` prefix on the
rationale bullet you wrote. Unmarked decisions are the human's. AI-origin
items may be revised without asking when evidence warrants — say so in the
commit. Never alter an unmarked decision without asking, and never remove a
marker: ratification is the human deleting it after reviewing

```bash
uv run pasm review pasm/spec
```
