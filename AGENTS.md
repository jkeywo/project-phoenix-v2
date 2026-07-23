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
# ── CI gates you can run locally — run ALL of these before calling work done ─
# These are the three fast CI jobs (test, editor-test, pasm) in full. Anything
# red here fails the build. `cargo test` alone is NOT sufficient: clippy denies
# warnings, and the PASM suite gates on the spec model. The remaining two jobs
# (build, smoke) need a WASM build — see the trunk/playwright commands below.
cargo fmt -- --check                           # CI: test job, step 1
cargo clippy --all-targets --all-features -- -D warnings   # CI: test job, step 2
cargo test                                     # CI: test job, step 3
npx vitest run                                 # CI: editor-test job (tests/client/*.test.js)
node scripts/check-strings.mjs --strict        # CI: editor-test job
uv run pytest -q tests/pasm                    # CI: pasm job — asserts on the spec model
uv run pasm validate                           # CI: pasm job

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

# Smoke tests (Playwright, Chromium) — requires dist/ built first
cd tests/smoke && npm install && npx playwright install chromium
npx playwright test                            # from tests/smoke/

# CI: ci.yml — five jobs. `pasm`, `test` and `editor-test` run in PARALLEL and
# gate independently (any one of them red fails the build); `build` needs
# `test`; `smoke` needs `build`; `deploy` runs on main.
#
#   pasm         uv run pytest -q tests/pasm ; uv run pasm validate
#   test         cargo fmt --check ; cargo clippy --all-targets --all-features
#                -D warnings ; cargo test
#   editor-test  npx vitest run ; node scripts/check-strings.mjs --strict
#   build        TRUNK_BUILD_RELEASE=true trunk build --release ;
#                node scripts/build-client.mjs
#   smoke        npx playwright test (against the built dist/)
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
  server/       — wasm-bindgen exports, renderer, viewscreen border
  gui/          — Rust-side GenericRadar UI widget (server viewscreen)
  server_app.rs — Server App builder: plugin registration + SimSet chain ordering
  sim_sets.rs   — SimSet: Input → Physics → Damage → Modifiers → Publish → PublishAggregate → Broadcast

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

1. **`serde_json` only in `codec.rs`.** Never import it directly in other modules. (Planned exception: `save.rs`, PRD #116.)
2. **Server = authority.** Bevy runs the simulation and decides everything; clients are stateless spokes that never talk to each other. Session tokens (UUIDv4 in `localStorage`) are the identity system — peer IDs are ephemeral.
3. **Client is pure JS.** No client-side Rust/WASM, no new Rust glue for the client. Client state is built by pure `gui/*.js` modules (Vitest-tested); console UIs are per-console HTML iframes.
4. **Captain authority.** Only the player at `CaptainChair` can set Red Alert (`SetRedAlert { active }`). Game start is collective `SetReady` auto-start, not a captain-only command.
5. **Station ownership is authoritative.** `Player.station: Option<StationId>` is the ownership field; console access derives from the station + `ShipConfig`. On disconnect the station keeps its holder and flips to the `Backfill` rating (AI operates its systems) until reconnect or a new claim.
6. **Humans and AI are symmetric.** Both issue `ControlSystem { target: SystemId, payload }`; admission strips source identity. Never branch on human-vs-AI downstream of admission.
7. **Helm decisions run on fixed ticks, not frames.** Helm commands apply the tick they are admitted (`AdmittedCommands` is cleared and refilled at admission each tick); AI helm decisions run on the shared sim tick (`[global] ai_helm_tick_hz` in the world TOML, default 30Hz) — never once per rendered frame.
8. **Deterministic asteroids.** Per-cell density is seeded from `(field_idx, gx, gz) + Perlin noise`. Destroyed asteroids respawn fresh when the player leaves the cell and returns.
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
- **Smoke tests (`tests/smoke/`, Playwright):** boot real server WASM in headless Chromium with a `BroadcastChannel`-backed PeerJS shim (no real WebRTC).
- **Headless runner (`tests/headless_runner.rs`, `--features headless`):** boots the whole simulation natively with nobody connected and asserts on end state. Lives in an *integration* test, not an inline `mod tests`, because building a headless app populates the process-global native template cache — inside the lib test binary that leaks into ~2500 unrelated unit tests. Anything calling `config_cache::insert_native_config` belongs here.
- **PASM tests (`uv run pytest -q tests/pasm`):** Python tests over the design model in `pasm/spec/` — traceability roll-ups, cross-domain link integrity, CLI output. **These assert on the spec YAML, so editing a slice can fail them without touching a line of Rust.** `cargo test` will not catch it; CI's `pasm` job will. Run them whenever you touch `pasm/spec/`.
- **Not tested:** renderer visual output, bridge internals, CI pipeline.

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
# The client page (client.html) is pure JS (gui/*.js) — there is no
# `client` cargo feature and no client-side WASM (removed in #463).

# WASM: no parallel physics, needs getrandom wasm_js backend.
# Native: parallel physics enabled. (See [target.'cfg(...)'] sections.)
```

---

## Deployed URLs

- Server: `https://jkeywo.github.io/project-phoenix-v2/`
- Client: `https://jkeywo.github.io/project-phoenix-v2/client/`
- Server QR encodes: `https://jkeywo.github.io/project-phoenix-v2/client/index.html#<peerId>`

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
