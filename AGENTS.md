@RTK.md

# Project Phoenix — Bridge Simulator

## TL;DR

A browser-based spaceship bridge simulator. One browser tab shows a shared 3D view of space. Players join from phones by scanning a QR code — no installation. The host (view screen) runs Rust/Bevy compiled to WebAssembly and is the authoritative server; the client (phone console) is **pure HTML/CSS/JS** — no client-side WASM. Clients send inputs and receive state snapshots. Networking uses PeerJS (WebRTC) in a star topology.

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

---

## Common Commands

```bash
# ── CI gates you can run locally — ALL green before each COMMIT, run ONCE ────
# These are the three fast CI jobs (test, editor-test, pasm) in full. Anything
# red here fails the build. `cargo test` alone is NOT sufficient: clippy denies
# warnings, and `pasm validate` gates on the spec model. The remaining two jobs
# (build, smoke) need a WASM build — see the trunk/playwright commands below.
#
# Do NOT run this list after every edit, implementation pass, or review pass —
# clippy alone is a near-full rebuild. While iterating, use `cargo check` plus
# the targeted tests for the area you touched. Run the full list exactly once,
# as a final gate pass when the change is otherwise sound, before committing.
# Review passes are read-only and run no gates at all.
cargo fmt -- --check                           # CI: test job, step 1
cargo clippy --all-targets --all-features -- -D warnings   # CI: test job, step 2
cargo test                                     # CI: test job, step 3
npx vitest run                                 # CI: editor-test job (tests/client/*.test.js)
node scripts/check-strings.mjs --strict        # CI: editor-test job
npm run lods:check                             # CI: editor-test job (LOD drift)
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

# Local dev — client page (pure HTML/JS, no WASM)
node scripts/build-client.mjs                  # → dist/client/, then serve dist/ statically

# Native delivery host (PRD #855) — serves a built bundle, the content manifest,
# the scenario catalogue and a version stamp from a native process instead of an
# open browser tab. DELIVERY ONLY: the authoritative simulation is still
# server.html or phoenix-headless, PeerJS signalling is unchanged, and there is
# no TLS or auth — LAN or behind something else, never a public address.
cargo build --release --features host --bin phoenix-host
./target/release/phoenix-host --client-dir dist
#   Binds 0.0.0.0:8080 by default — LAN-reachable out of the box; Windows
#   prompts to allow it through the firewall on first run. Pass
#   --addr 127.0.0.1:8080 to restrict to this machine only.
./target/release/phoenix-host --help
#   --manifest assets/scenarios.demo.toml  IS the catalogue restriction — the
#     same lever `?manifest=` pulls in the browser (issue #917).
#   --client-dir is version-pinned at STARTUP against the manifest being served:
#     a bundle built for other content refuses to start, before the port is
#     taken. /host/manifest.json pins a running client's protocol per request.
#   The catalogue it publishes is the browser host's own — src/delivery/payload.rs
#     holds the single field list that wasm_get_scenario_catalog and the JSON
#     encoder both walk, so the two surfaces cannot drift.

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
# turns it into `phoenix_demo_build`, which DELETES four things from a demo
# binary rather than merely refusing them —
#   - the god-mode cheat route (src/command_admission/debug_route.rs),
#   - ClientMessage::ToggleDebugFlag and its drain,
#   - ClientMessage::TogglePause and its drain, and
#   - the host mod-pack upload export, `wasm_add_mod_pack` (PRD #855).
# The third is the blunt one: nothing server-side checks station, captaincy or
# GamePhase before honouring a client pause, so any one of N demo players could
# otherwise freeze the mission for everyone, repeatedly. The HOST's own pause,
# on the server cog (#939), is untouched in every build. A demo binary does not
# decode either client message at all, so the hidden control and the closed
# route cannot come apart.
# The fourth is the catalogue restriction's other half: a demo build curates the
# catalogue down to combat_test + the Alliance Destroyer
# (assets/scenarios.demo.toml, #931), and a mod-pack upload adds whatever
# scenarios and hulls a ZIP carries. `build_flags::accepts_mod_pack_uploads()`
# states the rule, gui/build-flags.js's `offersModPackUpload` removes the
# button, and the export is gone — same doctrine as the cheat route. The overlay
# READERS (wasm_clear_mod_pack / _remove_ / _reorder_ / _active_pack_manifest)
# stay in every build: server.html calls them unconditionally and, with nothing
# able to enter the stack, they answer emptily — gating them would turn a no-op
# into a TypeError.
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
#   test         cargo fmt --check ; cargo clippy --all-targets --all-features
#                -D warnings ; cargo test
#   editor-test  npx vitest run ; node scripts/check-strings.mjs --strict ;
#                npm run lods:check
#   build        TRUNK_BUILD_RELEASE=true trunk build --release ;
#                node scripts/build-client.mjs
#   smoke        npx playwright test (against the built dist/)
#   perf         phoenix-perf assets|mesh|report — GATES on the `assets`
#                scenario only (report --gate, exit 3); every other scenario
#                reports into the job summary and the perf-capture artifact
#   balance      scripts/balance-runs.demo.toml — reports, never gates
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

```
Player phone (client.html, pure JS)
  ↓  sends WebRTC message with JSON
server.html JavaScript
  ↓  resolves peer ID → session token, calls wasm_receive_message(token, json)
server/bridge.rs: drain_inbound()
  ↓  queues InboundMessage into Bevy's pull-based message system
lobby/server.rs (or console plugins via SimSet::Input)
  ↓  reads InboundMessage, mutates SessionManager / ship state
  ↓  writes OutboundMessage events
server/bridge.rs: flush_outbound()
  ↓  encodes ServerMessage → JSON, calls JS callback
server.html JavaScript: routeOutbound()
  ↓  broadcasts to all peers / targeted peer
client.html JavaScript: handleMessage()
  ↓  gui/sim-state.js apply() folds message into client state
  ↓  gui/console-state.js build*() → JSON pushed into per-console iframes
```

In-game commands are `ClientMessage::ControlSystem { target: SystemId, payload }` — humans and AI issue the same commands. See `wiki/concepts/message-flow.md`.

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
server.html     — Host page: loads server WASM, runs Bevy, owns PeerJS host peer
client.html     — Client page: pure HTML/JS, connects via PeerJS peer ID in URL hash
tests/client/   — Vitest tests for gui/*.js
tests/smoke/    — Playwright smoke tests
wiki/           — LLM-maintained knowledge base. Read SCHEMA.md first; update as you work.
docs/           — Draft design notes (numbered).
```

---

## Key Constraints & Rules

1. **`serde_json` only in `codec.rs`.** Never import it directly in other modules. (Planned exception: PRD #116's own save path, which does not exist yet. The module issue #862 actually created is **`src/snapshot.rs`**, and it is deliberately *not* that exception: a world snapshot is written as RON inside `vellum-save`'s envelope, so it imports no `serde_json` at all. If #116 ever lands a JSON save, it needs its own line here rather than inheriting this one.)
2. **Server = authority.** Bevy runs the simulation and decides everything; clients are stateless spokes that never talk to each other. Session tokens (UUIDv4 in `localStorage`) are the identity system — peer IDs are ephemeral.
3. **Client is pure JS.** No client-side Rust/WASM, no new Rust glue for the client. Client state is built by pure `gui/*.js` modules (Vitest-tested); console UIs are per-console HTML iframes.
4. **Captain authority.** Only the player at `CaptainChair` can set Red Alert (`SetRedAlert { active }`). Game start is collective `SetReady` auto-start, not a captain-only command.
5. **Station ownership is authoritative.** `Player.station: Option<StationId>` is the ownership field; console access derives from the station + `ShipConfig`. On disconnect the station keeps its holder and flips to the `Backfill` rating (AI operates its systems) until reconnect or a new claim.
6. **Humans and AI are symmetric.** Both issue `ControlSystem { target: SystemId, payload }`; admission strips source identity. Never branch on human-vs-AI downstream of admission. The command log (issue #898) keeps this at the *recording* site too: it records everything the network boundary admits, without asking what a token looks like. What stays out of it is what a replay re-derives — the in-process AI emissions of `emit_ai_command`, which never cross that boundary.
7. **AI decisions run on fixed ticks, not frames.** The whole simulation advances on a fixed logical tick (issue #895): `SimSet` is configured in Bevy's `FixedUpdate` at the TOML-authored `[global] sim_tick_hz` (default 60 Hz), counted by `SimTick` (`src/sim_tick.rs`). Helm commands apply the tick they are admitted (`AdmittedCommands` is cleared and refilled at admission each tick). **Every** AI policy host — the six per-axis helm systems, shield focus, power allocation, torpedo load/auto-fire, frequency hint, phaser and blaster auto-fire, AI target selection, Captain, Sensors — runs under `run_if` on the one shared cadence in `src/ai/cadence.rs`, derived from the tick count as `sim_tick_hz / ai_tick_hz` logical ticks per decision (default 30 Hz; the slower `ai_snapshot_hz` cadence is a further whole multiple; both ratios are validated at world load), never once per rendered frame and never off a wall clock. An ungated sim system now runs once per *logical tick* — still gate deciders that must run slower. Never gate a decider inside its own body with an `Option<Res<_>>` that falls back to running every tick: every bare-`App` fixture takes that arm, so the shipped cadence ends up covered by no test at all (issue #889). **"Apply the tick they are admitted" still stands after the command log (issue #898), and now says so explicitly rather than by implication:** a logged command carries the tick it applies on, `command_admission::log::CommandDelay` is the gap between admission and that tick, and it is `0` for a local host. A non-zero delay is P2P lockstep's (#854) to negotiate, and setting one is the deliberate amendment of this rule — not something the plumbing can do by accident.
8. **Deterministic asteroids.** Per-cell density is seeded from `(layer salt, gx, gz) + Perlin noise` over the single composed lattice. Destroyed asteroids respawn fresh when the player leaves the cell and returns.
9. **WebGL2 rendering; PeerJS cloud broker** (not self-hosted, deferred post-PoC).
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
- **JS tests (`npx vitest run`):** `tests/client/*.test.js` covering the pure `gui/*.js` modules (state builders, action map, registries, panels) and the pure `scripts/balance-runs.mjs` merge/format/expand fns (`tests/client/balance-runs.test.js`, fabricated report JSON — no sim).
- **Smoke tests (`tests/smoke/`, Playwright):** boot real server WASM in headless Chromium with a `BroadcastChannel`-backed PeerJS shim (no real WebRTC). Two projects in `playwright.config.js`: `chromium` runs the message/DOM specs with no GPU (`src/server/bridge.rs` skips `RenderPlugin` under `navigator.webdriver`), and `render` runs `*.render.spec.js` under SwiftShader with that flag hidden, so the viewscreen actually draws. `npx playwright test` runs both.
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
host = []     # native delivery binary → phoenix-host. Gates the BINARY only:
              # `crate::delivery` is unconditional, because a feature-gated copy
              # of the catalogue contract would be the fork PRD #855 forbids —
              # and would leave its tests out of the plain `cargo test` CI runs.
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
- Server QR encodes: `https://pp-dev.kiwigamedesign.co.uk/client/index.html#<peerId>`

---

## Adding New Message Types

When extending `ClientMessage` or `ServerMessage` (prefer a new `SystemControlPayload` variant over a new top-level `ClientMessage` for in-game commands):

1. Add the variant in `core/messages.rs` (derive `Clone, Debug, Serialize, Deserialize, PartialEq`)
2. Add a round-trip test in `core/codec.rs` (`codec-tests` module)
3. Handle in `lobby/handler.rs` `process_message()` (pass through or produce outbound), or in the appropriate console server plugin (`.in_set(SimSet::Input)`) for in-game messages
4. Client inbound: fold into state in `gui/sim-state.js` `apply()` (or `gui/comms-state.js` / `gui/lobby-state.js`), then surface via the relevant `build*()` in `gui/console-state.js`
5. Client outbound: add the UI action to `gui/action-map.js` (and the button/control to the console's `gui/<name>-console.html`)
6. Add/extend Vitest coverage in `tests/client/`
7. Touch `server.html` `routeOutbound()` / `client.html` only if routing or the PeerJS handshake changes

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
