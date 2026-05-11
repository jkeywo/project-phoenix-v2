# Project Phoenix — Bridge Simulator

## TL;DR

A browser-based spaceship bridge simulator. One browser tab shows a shared 3D view of space. Players join from phones by scanning a QR code — no installation. Both the host (view screen) and the client (phone console) run Rust/Bevy compiled to WebAssembly. The host acts as the authoritative server; clients send inputs and receive state snapshots. Networking uses PeerJS (WebRTC) in a star topology.

**Shipped PRDs:**
- **PRD #1:** [Browser-Based Bridge Simulator](https://github.com/jkeywo/project-phoenix-v2/issues/1) — PoC: lobby, captain's chair, red alert, rotating cube
- **PRD #17:** [Mobile UX, Canvas Resize, Connection Status](https://github.com/jkeywo/project-phoenix-v2/issues/17)
- **PRD #22:** [Helm and Game World](https://github.com/jkeywo/project-phoenix-v2/issues/22) — Ship physics, asteroids, helm console with thrust/steering
- **PRD #36:** [Captain View Selector](https://github.com/jkeywo/project-phoenix-v2/issues/36)
- **PRD #51:** [Smoke Test Harness](https://github.com/jkeywo/project-phoenix-v2/issues/51)
- **PRD #66:** [Weapons & Engineering Consoles](https://github.com/jkeywo/project-phoenix-v2/issues/66) — Phasers, hull integrity, breakdown queue, repair loop

**Open PRDs (planned work):**
- **PRD #115:** [Native PC Server](https://github.com/jkeywo/project-phoenix-v2/issues/115) — Native binary host with bundled cloudflared tunnel and embedded WebSocket transport. Adds a `native` Cargo feature alongside `server`/`client`.
- **PRD #116:** [Save/Load Game Sessions](https://github.com/jkeywo/project-phoenix-v2/issues/116) — `localStorage`-backed save slots, periodic + lifecycle saves, version-gated load. Introduces `save.rs` (the *second* sanctioned `serde_json` surface).
- **PRD #117:** [Modifier System for Cross-Console Multipliers](https://github.com/jkeywo/project-phoenix-v2/issues/117) — Pure `modifiers.rs`: `ModifierSlot`, `ModifierSource`, `ShipModifiers` cache. Infrastructure for #118 and beyond.
- **PRD #118:** [Engineering Split: Repair + Power Consoles](https://github.com/jkeywo/project-phoenix-v2/issues/118) — Renames `Engineering` → `Repair`, adds `Power` console, shape-matching repair with 3 teams, 6+2 power allocation. Depends on #117.
- **PRD #119:** [Space Stations, Scenario Engine & Comms Console](https://github.com/jkeywo/project-phoenix-v2/issues/119) — TOML scenarios with triggers/actions, station entities, `Console::Comms`. Depends on Science + Power.
- **PRD #120:** [Station-Based Lobby & Crew Assignment](https://github.com/jkeywo/project-phoenix-v2/issues/120) — Replaces per-console picking with per-station picking; auto-shuffle on join/leave; spectator FIFO queue.

**Current state:** Five consoles in the wire types (`CaptainChair`, `Helm`, `Tactical`, `Engineering`, `Science`). Full simulation: ship physics, destroyable asteroid field, phaser banks (port/starboard), torpedoes, four-quadrant shields, impulse drive, hull damage, breakdown/repair loop. Data-driven entities and maps loaded from TOML via `assets/`. Client is a full Bevy/WASM app. See **[wiki/](./wiki/)** for the deeper map of the codebase.

---

## Wiki — Read It and Maintain It

This repo carries an **LLM-maintained wiki** under `wiki/`. It is a persistent, compounding knowledge base that summarises and indexes the raw sources (this file, `README.md`, `CONTEXT.md`, the codebase, PRDs, design drafts). Treat it as the index you reach for first when orienting in a new session, and update it whenever you ingest a new source or learn something non-trivial.

**Read [wiki/SCHEMA.md](./wiki/SCHEMA.md) at the start of any non-trivial task.** It defines the layout (`entities/`, `concepts/`, `sources/`, `roadmap/`, `index.md`, `log.md`), the page conventions (YAML frontmatter, code references, cross-links), and the workflows.

**Workflow at a glance:**

- *Orienting* — open `wiki/index.md`, find candidate pages, read them. If precision matters, follow their `sources:` links into the raw layer (code, PRDs, drafts).
- *Ingesting a new source* (a new PRD lands, a `docs/*.md` is added, or you discover the codebase has shifted from what the wiki says) — create or update the matching `wiki/sources/` page, then update every `entities/` and `concepts/` page the source touches, then append a one-line entry to `wiki/log.md`, then update `wiki/index.md` if pages were created.
- *Answering a query* — synthesise from the wiki, and if the answer is non-trivial, **file the answer back** as a new `concepts/` or `roadmap/` page so the next agent doesn't have to redo the work.
- *Linting* — periodically check that `path/to/file.rs:LINE` references still resolve and that roadmap pages whose backing PRD has shipped get moved/closed.

The wiki is *not* a replacement for `README.md`, `CONTEXT.md`, or this file — those remain the canonical raw sources. The wiki is the navigable, cross-linked overlay on top.

---

## Prerequisites

```bash
# Rust stable + wasm target
rustup target add wasm32-unknown-unknown

# Trunk (build tool for WASM + HTML)
cargo install trunk

# node/npm (for npm scripts)
```

---

## Common Commands

```bash
# Rust unit tests
cargo test

# Native debug build (no WASM, for quick compile checks)
cargo check

# Local dev — server page (WASM, Bevy, peer host)
trunk serve                                    # → http://localhost:8080

# Local dev — client page (plain HTML, connects to server)
trunk serve --config client-trunk.toml --port 8081  # → http://localhost:8081

# Production build
trunk build --release
trunk build --release --config client-trunk.toml

# Smoke tests (Playwright, Chromium) — requires dist/ built first
trunk build --release
trunk build --release --config client-trunk.toml
cd tests/smoke && npm install && npx playwright install chromium
npx playwright test                            # from tests/smoke/

# CI: deploy.yml builds + deploys on push to main
#     smoke-test.yml builds dist/ and runs Playwright on push + PR
```

---

## The Web Stack — For Game Devs

This project compiles Rust → WebAssembly → runs in a browser. Here's what that means for development:

### WASM ≠ Native

- Bevy's `App::run()` returns immediately on WASM (it hooks `requestAnimationFrame` instead of blocking). Code after `run()` never executes unless `wasm-bindgen` is configured to not unwind (it is).
- `bridge.rs` is entirely behind `#[cfg(target_arch = "wasm32")]`. It won't compile on native.
- Threading: WASM is single-threaded. `thread_local!` + `RefCell` is the pattern for sharing state between JS and Rust.
- Networking: The browser handles ALL networking via JavaScript (PeerJS). Bevy never touches sockets.

### Message Flow (The Core Loop)

```
Player phone (client.html)
  ↓  sends WebSocket/RTC message with JSON
  ↓  e.g. {"type":"HelmInput","data":{"thrust":0.75,"steering":-0.5}}
server.html JavaScript
  ↓  resolves peer ID → session token
  ↓  calls wasm_receive_message(token, json)
bridge.rs: drain_inbound()
  ↓  queues InboundMessage into Bevy's pull-based message system
lobby.rs (or simulation.rs)
  ↓  reads InboundMessage, mutates SessionManager / ShipState
  ↓  writes OutboundMessage events
bridge.rs: flush_outbound()
  ↓  encodes ServerMessage → JSON, calls JS callback
server.html JavaScript: routeOutbound()
  ↓  broadcasts to all peers / targeted peer
client.html JavaScript
  ↓  handleMessage() → render()
```

### Trunk

Trunk is like Vite/Webpack but for Rust→WASM. It:
- Compiles the crate to WASM
- Bundles the WASM module
- Inlines the JS bootstrap into the HTML
- Serves the final page

Two configs:
- `Trunk.toml` — builds `server.html` (includes WASM via `<link data-trunk rel="rust">`)
- `client-trunk.toml` — builds `client.html` (client WASM feature; thin Bevy UI app)

### File Layout

```
src/
  messages.rs         — Pure data types. Console, Player, GameState, SimSnapshot, ClientMessage, ServerMessage. Wire types for shields, phaser banks, torpedo tubes.
  codec.rs            — MessageCodec trait + JsonCodec impl. ONLY place serde_json is used directly.
  session.rs          — SessionManager: tokens → players, console assignment, reconnect/vacancy logic
  lobby_handler.rs    — Pure lobby message handler: process_message(), process_disconnect(). No Bevy.
  lobby.rs            — Bevy plugin: drives lobby_handler, phase transitions
  simulation.rs       — Bevy plugin: helm input → physics → weapons → collision → breakdown → broadcast
  ship_physics.rs     — Pure Rust physics controller (no Bevy). Input/output function, fully testable.
  ship_state.rs       — ShipState resource: position, yaw, speed, red_alert, hull, phaser state
  asteroid_spawner.rs — Pure Rust asteroid position generator (seeded, deterministic)
  asteroid_lifecycle.rs — Bevy systems: range-gated asteroid spawn/despawn around the ship.
  radar.rs            — Pure radar projection math. Shared by server renderer and client helm panel.
  radar_config.rs     — Pure radar viewport config used by helm + weapons radars.
  damage.rs           — collision_damage() formula + HullIntegrity struct. No Bevy.
  breakdown.rs        — BreakdownQueue FIFO + breakdowns_from_damage() formula. No Bevy.
  phaser.rs           — Pure phaser bank state machine: lock, fire, sever, cooldown.
  torpedo.rs          — Pure torpedo + tube state machine: load, launch, homing, expiry.
  shield.rs           — Pure four-quadrant shield model: HP, online/offline, regen.
  impulse.rs          — Pure impulse-drive charge state machine.
  beam_render.rs      — Bevy plugin: renders the active phaser beam(s) as line meshes (server only).
  entity_config.rs    — TOML entity config types (`EntityConfig`, asteroid + ship + station fields).
  map_config.rs       — TOML map config: spawn anchors, asteroid fields, default scenario reference.
  config_cache.rs     — `ConfigCachePlugin` — preloads map + entity TOML files via JS fetch on WASM, exposes them as Bevy resources.
  entity_tags.rs      — String-tag helpers for `tags = [...]` lookups across entity configs.
  renderer.rs         — Bevy plugin: lobby UI, 3D camera, Red Alert border overlay (server only)
  bridge.rs           — wasm-bindgen exports. Compiled when `server` feature is active.

  client_lobby.rs     — Pure client lobby state model: LobbyState, LobbyView, ConsoleSlot. No Bevy.
  client_sim.rs       — Pure client sim-state model: ClientSimState. No Bevy.
  client_helm.rs      — Pure joystick logic: drag/release/tick, clamp_to_circle. No Bevy.
  client_app.rs       — Bevy plugin: lobby panel, captain panel, helm panel, weapons/tactical panel, science panel, radar drawing.
  client_bridge.rs    — wasm-bindgen exports. Compiled when `client` feature is active.
  lib.rs              — Module declarations + feature gates

src/server/, src/client/, src/shared/  — Draft refactor of the flat layout into subdirectories. NOT
                                         wired into `lib.rs` and NOT compiled. Treat as dead code
                                         until a refactor PRD lands. PRD #115 explicitly excludes it.

assets/
  maps/default.toml             — Default map: anchors, asteroid fields, default scenario path.
  entities/asteroid_*.toml      — Asteroid variants (large, small, cosmetic).
  entities/player_ship.toml     — Ship config: physics, phaser banks, torpedo tubes, shields, impulse.

server.html           — Host page: loads server WASM, runs Bevy, owns PeerJS host peer
client.html           — Client page: loads client WASM, connects to host via PeerJS peer ID in URL hash
Cargo.toml            — Single crate: cdylib (WASM) + rlib (tests). Features: server | client.
Trunk.toml            — Build config for server.html (default = server feature)
client-trunk.toml     — Build config for client.html (client feature)
.github/workflows/    — CI: builds both pages, deploys to gh-pages
wiki/                 — LLM-maintained knowledge base. Read SCHEMA.md first; update as you work.
docs/                 — Draft design notes (numbered). Drafts 9-11 cover AI, regions, complexity.
```

---

## Architecture

### Networking — Star Topology

```
        ┌─────────────┐
        │  server.html │  ← PeerJS host (authoritative)
        │  (Bevy+WASM) │     Runs the game simulation
        └──────┬──────┘
            ┌──┴──┐
      ┌─────┐     └─────┐
      ▼              ▼
 client.html    client.html
 (phone #1)     (phone #2)
              ...
```

- **Server = authority.** Bevy on the server page runs the simulation, owns session state, decides everything.
- **Clients = stateless spokes.** They send input, receive state snapshots/events. Clients never talk to each other.
- **PeerJS handles WebRTC.** JS generates a random peer ID for the host, shows it as a QR code. Clients read the peer ID from `location.hash` and connect.

### Session Tokens (NOT Peer IDs)

Session tokens are the identity system:
- Each client generates a UUIDv4 on first visit, stores it in `localStorage`.
- Tokens survive page refreshes. Same token = same player.
- The server maps tokens → `Player` records (name, console, connection status).
- PeerJS peer IDs are ephemeral — they change on every reconnect.
- The JS bridge resolves peer ID → token on the first `Identify` message.

### Event System (Bevy 0.18)

This project uses Bevy's new **pull-based message system** (`add_message<T>()`, `MessageReader<T>`, `MessageWriter<T>`):

- `InboundMessage` — decoded client message with sender token
- `OutboundMessage` — server message to send back, with routing target (All/Token/AllExcept)
- `PlayerDisconnected` — lifecycle event from JS when peer drops

### Serialization — The Codec Contract

`serde_json` must **never** be called directly outside `src/codec.rs`. The `MessageCodec` trait is the only serialization surface. This exists so the wire format can be swapped to binary (MessagePack, rmp-serde, etc.) by changing one module.

---

## Game Flow

### 1. Lobby Phase

- Players scan QR → open `client.html#<peerId>` → connect to host
- Server sends `Welcome` with full `GameState` snapshot
- Players edit name, pick console (CaptainChair or Helm)
- Only the player at CaptainChair sees and can press "Engage"
- Server validates: `captain_token() == sender && phase == Lobby`
- On `StartGame`: phase → `InProgress`, broadcast `GameStarted` to all

### 2. In-Progress Phase

- **Captain:** toggles Red Alert via `ToggleRedAlert`; changes view via `SetView`
- **Helm:** sends `HelmInput { thrust, steering }` at 10Hz; can push radar to viewscreen via `SetView { Radar }`
- **Tactical:** sends `SetTarget { uuid }` to lock an asteroid; sends `FirePhaser` to start a beam (must be in range and forward arc)
- **Engineering:** sends `Repair` to clear the current breakdown; wrong-console repairs incur a penalty cooldown
- **Server simulation:**
  - Reads helm inputs tagged with `helm_token()`
  - Feeds into `compute_physics()` (pure function in `ship_physics.rs`)
  - Applies to ship's Rapier rigid body as direct velocity
  - Collision → `collision_damage()` → `HullIntegrity::apply_damage()` → `breakdowns_from_damage()` → `BreakdownQueue::push_random()`
  - Every 100ms: broadcasts `SimState { red_alert, hull_integrity, authorized_repair_console, … }` snapshot; sends `WeaponsUpdate` to Tactical; sends `RepairState` to Engineering
- **Renderer:** 3D camera follows ship, Red Alert border shows on view screen, phaser beam drawn when active

### 3. Disconnection / Reconnection

- JS fires `wasm_player_disconnected(token)` when peer drops
- Server marks player disconnected, console becomes vacant immediately
- On re-identify with same token: auto-reassign previous console if still free
- `PlayerJoined` broadcast to others on reconnect

---

## Module Map

| File | Role | Depends On | Bevy? |
|---|---|---|---|
| `messages.rs` | Pure data types. No logic. | serde | No |
| `codec.rs` | MessageCodec trait + JsonCodec | messages, serde_json | No |
| `session.rs` | SessionManager: player lifecycle | messages | No |
| `lobby_handler.rs` | Pure lobby message handler | messages, session | No |
| `lobby.rs` | Bevy plugin: lobby message routing | lobby_handler, session | Yes |
| `simulation.rs` | Bevy plugin: physics + weapons + damage | messages, session, ship_physics, ship_state, asteroid_spawner, radar, damage, breakdown | Yes |
| `ship_physics.rs` | Pure physics: inputs → new state | None (pure Rust) | No |
| `ship_state.rs` | ShipState resource | messages | Yes |
| `asteroid_spawner.rs` | Pure asteroid position generator | rand | No |
| `radar.rs` | Pure radar projection + fire-ready check | messages | No |
| `damage.rs` | collision_damage() + HullIntegrity | None (pure Rust) | No |
| `breakdown.rs` | BreakdownQueue + breakdowns_from_damage() | messages, rand | No |
| `renderer.rs` | Bevy plugin: 2D lobby + 3D game view | messages, session, ship_state | Yes (server) |
| `bridge.rs` | wasm-bindgen exports (server feature) | codec, lobby, renderer, simulation | WASM+server |
| `client_lobby.rs` | Pure client lobby state + LobbyView | messages | No |
| `client_sim.rs` | Pure client sim state (ClientSimState) | messages | No |
| `client_helm.rs` | Pure joystick logic | messages | No |
| `client_app.rs` | Bevy plugin: all client UI panels | client_lobby, client_sim, client_helm | Yes (client) |
| `client_bridge.rs` | wasm-bindgen exports (client feature) | codec, client_app | WASM+client |

---

## Key Game Mechanics

### Ship Physics (`ship_physics.rs`)

```rust
fn compute_physics(state, input, dt, config) -> ShipPhysicsResult
```

Pure function. No framework, no Bevy. Takes current ship state + helm inputs + delta time → returns new state.

- **Thrust:** 0.0–1.0. Acceleration: 16.7 units/s² (3s to max 50 units/s). No thrust: deceleration 50 units/s² (1s to stop).
- **Steering:** -1.0 to 1.0. Max yaw rate: π/2 rad/s (90°/s).
- **Movement:** XZ plane, Y-up. Ship's forward is along negative Z when yaw=0.
- **Collision:** Ship hits asteroid → velocity zeroed (handled in `simulation.rs`).

### Asteroid Field (`asteroid_spawner.rs`)

Deterministic seeded generation. Fixed layout per game session. Spheres at randomized positions within spawn radius, with clear zone around origin. Called once during world setup.

### Consoles

- **CaptainChair:** Red Alert toggle (exclusive). Only captain can `StartGame` and `ToggleRedAlert`. View selector (Fore/Aft/Port/Starboard or Radar).
- **Helm:** Thrust + steering joystick. Sends `HelmInput` at 10Hz while active. Ship only moves when Helm is occupied. Displays radar overlay and "On Screen" button to push radar to the viewscreen. Triggers impulse charge via `StartImpulseCharge`.
- **Tactical:** Target lock (`SetTarget`), fire phasers (`FirePhaser`), set phaser mode (`SetPhaserMode { Auto | Manual }`), fire torpedoes (`FireTorpedo { tube, target_uuid }`). Receives `WeaponsUpdate` at 10Hz with lock status, fire readiness, cooldown, torpedo magazine, and per-tube reload state. Beam events (`BeamStarted`, `BeamEnded`, `PhaserFired`) and torpedo events (`TorpedoLaunched`, `TorpedoDestroyed`) broadcast to all.
- **Engineering:** Repair hull breakdowns (`Repair { console }`). Receives `RepairState` at 10Hz with cooldown status. Repairing without authorization incurs a penalty cooldown.
- **Science:** Long-range radar overlay, system chart on viewscreen, advisory target suggestion (`SetScienceTarget`), cancel an active impulse charge (`CancelImpulse`). View modes `ScienceRadar` and `SystemChart` are pushed to the viewscreen by Science.

---

## Testing Strategy

### Rust unit tests (`cargo test`)

Tests live inline with modules (`#[cfg(test)] mod tests`).

- **`session.rs`** — Player registration, duplicate tokens, console assignment/clearing, disconnect vacancy, reconnect auto-assign, `helm_token()` / `captain_token()` lookups, conflict resolution
- **`codec.rs`** — Round-trip serialization for every `ClientMessage` and `ServerMessage` variant
- **`lobby_handler.rs`** — Pure handler: Identify → Welcome, console select → broadcast, captain only can start, HelmInput ignored in lobby, disconnect handling
- **`ship_physics.rs`** — Zero input, thrust curve, deceleration curve, steering yaw, diagonal motion, dt scaling, speed cap
- **`asteroid_spawner.rs`** — Exact count, within bounds, clear zone, no duplicates
- **`ship_state.rs`** — Red alert toggle, snapshot generation
- **`radar.rs`** — project_to_radar (yaw rotation, range cull), project_asteroid, radar_dots iterator, is_fire_ready (range + arc gates)
- **`damage.rs`** — collision_damage formula (zero speed, max speed, mid speed, clamp), HullIntegrity (apply/restore/floor/ceiling)
- **`breakdown.rs`** — BreakdownQueue push/pop/front, no-repeat picker, breakdowns_from_damage bucket math
- **`client_lobby.rs`** — LobbyState message application (Welcome, PlayerJoined/Left, ConsoleSelected/Cleared, GameStarted), LobbyView derivation (slots, is_captain, is_helm, all_consoles_filled), outbound message builders
- **`client_sim.rs`** — ClientSimState message application (SimState, WorldSetup, Welcome, RepairState), is_active_camera_direction, message builders
- **`client_helm.rs`** — clamp_to_circle, compute_thrust_steering, press/drag/release/tick state machine

### Smoke tests (`tests/smoke/`, Playwright + Chromium)

End-to-end tests that boot the real server WASM in a headless browser and exercise the full message flow. They replace `window.Peer` with a `BroadcastChannel`-backed shim so no real WebRTC is needed.

| File | Issues | What it covers |
|---|---|---|
| `peerjs-shim.js` | #52 | The shim itself — host open, connect, bidirectional routing, close |
| `shim.spec.ts` | #52 | Shim unit tests (two blank pages, BroadcastChannel IPC) |
| `fixtures.ts` | #53 | Shared `test` fixture (auto-injects shim), `createTestClient` helper |
| `playwright.config.ts` | #53 | Chromium only, `webServer: npx serve dist/`, 1 worker |
| `server-load.spec.ts` | #54 | Server WASM boots, `window.__wasmReady` fires, no JS errors |
| `client-connect.spec.ts` | #55 | Real `client.html` connects, `#status` = "Connected" after Welcome |
| `lobby.spec.ts` | #56/#57 | ConsoleSelected broadcasts; non-captain StartGame is ignored |
| `sim-state.spec.ts` | #58/#59 | SimState fields validated; HelmInput changes ship position |

**Key shim design decisions:**
- `window.__wasmReady` is set (and `wasm-ready` event fired) only after BOTH the Peer opens AND `TrunkApplicationStarted` fires, using `setTimeout(0)` so `startPhoenix()` runs first.
- `dist/client/index.html` is the Trunk-built client page (WASM + JS). Both the JS message handling and the Bevy UI overlay are fully functional in CI and in production.
- Lobby/sim-state specs use blank test pages with the shim API directly rather than `client.html`, giving full control over which messages are sent.

### What is NOT tested (manual only)

- Renderer (visual output, Bevy UI)
- Bridge (WASM/JS boundary internals)
- CI pipeline (validated by it passing)

### Test Style Rules

> Good tests: set up state → perform action → assert on observable output through the public interface. Do NOT assert on private fields, internal call counts, or implementation-specific details.

---

## Key Constraints & Rules

1. **`serde_json` only in `codec.rs`.** Never import it directly in other modules.
2. **Feature gates for bridges.** `bridge.rs` is compiled under the `server` feature; `client_bridge.rs` under the `client` feature. Neither is gated by `cfg(target_arch)` alone — the feature flag controls it.
3. **Captain authority.** Only the player at `CaptainChair` can `StartGame` and `ToggleRedAlert`. The server enforces this.
4. **Console vacancy on disconnect.** Immediately — in all game phases.
5. **Helm sends at 10Hz.** Simulation reads helm inputs at 10Hz tick intervals.
6. **Deterministic asteroids.** Seeded generator, fixed per session.
7. **WebGL2 rendering.** For broad browser support.
8. **PeerJS cloud broker.** Not self-hosted (deferred post-PoC).
9. **Pure modules are Bevy-free.** `lobby_handler`, `radar`, `damage`, `breakdown`, `client_lobby`, `client_sim`, `client_helm` have no Bevy imports — they are fully unit-testable on native and shared between server and client.
10. **A player may hold multiple consoles.** `Player.consoles` is `Vec<Console>`. The JS tab bar controls which panel is visible via `wasm_client_set_active_console`.

---

## Cargo.toml Notes

```toml
[lib]
crate-type = ["cdylib", "rlib"]  # cdylib for WASM, rlib for testing

[features]
default = ["server"]
server = []   # host build → server.html (bridge.rs compiled in)
client = []   # client build → client.html (client_bridge.rs compiled in)

# WASM-specific: no parallel physics, needs getrandom wasm_js backend
[target.'cfg(target_arch = "wasm32")'.dependencies]
bevy_rapier3d = { version = "0.33", features = ["dim3"] }
getrandom = { version = "0.3", features = ["wasm_js"] }

# Native: parallel physics enabled
[target.'cfg(not(target_arch = "wasm32"))'.dependencies]
bevy_rapier3d = { version = "0.33", features = ["parallel"] }
```

---

## Deployed URLs

- Server: `https://jkeywo.github.io/project-phoenix-v2/`
- Client: `https://jkeywo.github.io/project-phoenix-v2/client/`
- Server QR encodes: `https://jkeywo.github.io/project-phoenix-v2/client/index.html#<peerId>`

---

## Client Page (client.html) Quick Reference

The client is a WASM Bevy app built with the `client` Cargo feature. Key patterns:

- `client.html` minimal JS wires up PeerJS, localStorage, and the WASM bridge
- `localStorage` for session token and player name persistence
- Reads host peer ID from `location.hash.slice(1)`
- PeerJS `clientPeer.connect(hostPeerId, { reliable: true })` for DataConnection
- Inbound `ServerMessage` JSON → `wasm_client_receive(json)` → decoded next Bevy frame
- Outbound `ClientMessage` JSON → JS callback registered via `set_client_send_callback`
- `wasm_client_set_token(token)` sets `LocalPlayerToken` resource each frame
- `wasm_client_set_active_console(name)` sets `ActiveConsole` resource (drives panel visibility)
- Panels (`LobbyRoot`, `CaptainPanel`, `HelmPanel`) toggle `Visibility` based on phase + held consoles

---

## Server Page (server.html) Quick Reference

The server loads the WASM binary via Trunk. Key patterns:

- Trunk fires `TrunkApplicationStarted` when WASM is ready, sets `window.wasmBindings`
- `startPhoenix()` wires up `set_message_callback(routeOutbound)` then calls `wasm_init()`
- `peerTokens` Map: resolves `peer ID → session token` on first Identify
- `tokenConns` Map: stores `session token → DataConnection` for outbound routing
- `dispatchToWasm(token, json)` queues messages until WASM is ready
- AudioContext is resumed on first user gesture (autoplay policy)

---

## Adding New Message Types

When extending `ClientMessage` or `ServerMessage`:

1. Add variant to enum in `messages.rs` (derive `Clone, Debug, Serialize, Deserialize, PartialEq`)
2. Add round-trip test in `codec.rs` (`codec-tests` module)
3. Handle in `lobby_handler.rs` `process_message()` (pass through or produce outbound)
4. Handle in `simulation.rs` if it is an in-game message
5. Handle in `client_lobby.rs` `LobbyState::apply()` or `client_sim.rs` `ClientSimState::apply()` as appropriate
6. Update `client_app.rs` if a new UI element or button is needed
7. Handle in `server.html` JS `routeOutbound()` if routing logic needs adjustment
8. Handle in `client.html` JS if the handshake / PeerJS wiring changes
