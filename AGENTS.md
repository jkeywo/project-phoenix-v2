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
- **PRD #115:** [Native PC Server](https://github.com/jkeywo/project-phoenix-v2/issues/115) — PRD itself closed; deployment slices #135–#141 are on hold and not yet built
- **PRD #117:** [Modifier System](https://github.com/jkeywo/project-phoenix-v2/issues/117) — Pure `modifiers.rs` cache + `ModifierAdded`/`ModifierRemoved` wire
- **PRD #118:** [Repair + Power Consoles](https://github.com/jkeywo/project-phoenix-v2/issues/118) — Renamed `Engineering` → `Repair`, added `Power`, shape-matching repair with three teams, 6+2 power allocation
- **PRD #120:** [Station-Based Lobby & Crew Assignment](https://github.com/jkeywo/project-phoenix-v2/issues/120) — Per-station picking, auto-shuffle, spectator FIFO. `SelectStation`/`ReleaseStation`/`StationAssigned` wire
- **PRD #153:** [Region Entities, Component-Driven Spawning & Modifier Flags](https://github.com/jkeywo/project-phoenix-v2/issues/153) — Single `[[entity]]` pipeline, six region effects, `f32` hull, `FlagKind`, unified `EntitySnapshot` wire
- **PRD #154:** [Console Complexity: UI Hiding + AI Automation](https://github.com/jkeywo/project-phoenix-v2/issues/154) — Per-console `Low`/`Full` presets, hide UI + server-side `console_ai` to operate hidden controls
- **PRD #180:** [Viewscreen Frame](https://github.com/jkeywo/project-phoenix-v2/issues/180) — Bevy UI border, `RedAlertVignetteMaterial`, designation + HEADING / HULL / CONDITION HUD
- **PRD #187:** [Phone Console HUD — Diegetic Bezel Frame](https://github.com/jkeywo/project-phoenix-v2/issues/187) — `phone_border/` plugin: bezel wraps every console; full helm + captain chrome
- **PRD #191:** [Grid-Based Asteroid Lifecycle with Deterministic Ring Buffer Window](https://github.com/jkeywo/project-phoenix-v2/issues/191) — `asteroid_window.rs`, player-centred grid, destroyed asteroids respawn on return

**Open PRDs (planned work):**
- **PRD #116:** [Save/Load Game Sessions](https://github.com/jkeywo/project-phoenix-v2/issues/116) — `localStorage`-backed save slots, periodic + lifecycle saves, version-gated load. Introduces `save.rs` (the *second* sanctioned `serde_json` surface).
- **PRD #119:** [Space Stations, Scenario Engine & Comms Console](https://github.com/jkeywo/project-phoenix-v2/issues/119) — TOML scenarios with triggers/actions, station entities, `Console::Comms`. Builds on the PRD #153 entity pipeline.
- **PRD #142:** [AI and Behaviour System](https://github.com/jkeywo/project-phoenix-v2/issues/142) — Data-driven state-machine NPCs that emit the same input messages as players. Depends on #119.

**Current state:** Six consoles in the wire types (`CaptainChair`, `Helm`, `Tactical`, `Repair`, `Science`, `Power`). Players join *stations* (bundles of one or more consoles defined per player count in `player_ship.toml`), not individual consoles. Full simulation: ship physics, grid-based streaming asteroid field, phaser banks (port/starboard), torpedoes, four-quadrant shields, impulse drive, hull damage, shape-matching repair with three teams, 6+2 power allocation driving cross-system modifiers, region effects (damage zones, slow zones, comms/sensor jammers), per-console complexity presets with server-side AI to operate hidden controls. Viewscreen and phone consoles both have diegetic bezel frames with red-alert vignette + HUD. Data-driven entities and maps loaded from TOML via `assets/`. Client is a full Bevy/WASM app. See **[wiki/](./wiki/)** for the deeper map of the codebase.

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
  modifiers.rs        — Pure cross-console multiplier table + flag set (PRD #117 + #153 extension).
  flag_kind.rs        — Typed boolean flags (`CommsJammed`, `SensorBlind`) carried by `ShipModifiers`.
  power_system.rs     — Pure 6+2 power allocation, battery, exhaustion lock (PRD #118).
  repair_teams.rs     — Pure three-team shape-matching repair dispatch (PRD #118).
  stations.rs         — Pure station model: parse, validate, lookup, reassign-on-join/leave (PRD #120).
  beam_render.rs      — Bevy plugin: renders the active phaser beam(s) as line meshes (server only).
  entity_config.rs    — TOML entity config types (`EntityConfig`, asteroid + ship + station + region fields).
  map_config.rs       — TOML map config: spawn anchors, asteroid fields, default scenario reference.
  config_cache.rs     — `ConfigCachePlugin` — preloads map + entity TOML files via JS fetch on WASM, exposes them as Bevy resources.
  entity_tags.rs      — String-tag helpers for `tags = [...]` lookups across entity configs.
  asteroid_window.rs  — Pure ring-buffer window: player-centred grid lifecycle (PRD #191).
  asteroid_lifecycle.rs — Bevy systems: spawn/despawn cells as the player moves; broadcast deltas.
  renderer.rs         — Bevy plugin: lobby UI, 3D camera, console panels (server only).
  viewscreen_border.rs — Bevy plugin: viewscreen bezel + `RedAlertVignetteMaterial` + HUD (server only, PRD #180).
  bridge.rs           — wasm-bindgen exports. Compiled when `server` feature is active.

  client_lobby.rs     — Pure client lobby state model: LobbyState, LobbyView, ConsoleSlot. No Bevy.
  client_sim.rs       — Pure client sim-state model: ClientSimState. No Bevy.
  client_helm.rs      — Pure joystick logic: drag/release/tick, clamp_to_circle. No Bevy.
  client_app.rs       — Bevy plugin: lobby panel + tactical / repair / power / science panels.
  phone_border/       — Bevy plugin: phone bezel framing + full helm + captain chrome (client, PRD #187).
  client_bridge.rs    — wasm-bindgen exports. Compiled when `client` feature is active.
  lib.rs              — Module declarations + feature gates

assets/
  maps/default.toml             — Default map: anchors, asteroid fields, default scenario path.
  entities/asteroid_*.toml      — Asteroid variants (large, small, cosmetic).
  entities/player_ship.toml     — Ship config: physics, phaser banks, torpedo tubes, shields, impulse, [stations] block.
  entities/region_*.toml        — Region templates per effect type (PRD #153).
  complexity/*.toml             — Per-console complexity presets (Low / Full) + AI tuning (PRD #154).

server.html           — Host page: loads server WASM, runs Bevy, owns PeerJS host peer
client.html           — Client page: loads client WASM, connects to host via PeerJS peer ID in URL hash
Cargo.toml            — Single crate: cdylib (WASM) + rlib (tests). Features: server | client.
Trunk.toml            — Build config for server.html (default = server feature)
client-trunk.toml     — Build config for client.html (client feature)
.github/workflows/    — CI: builds both pages, deploys to gh-pages
wiki/                 — LLM-maintained knowledge base. Read SCHEMA.md first; update as you work.
docs/                 — Draft design notes (numbered). Drafts 1-8 mostly shipped or absorbed into PRDs; draft 9 (AI) is still the basis for open PRD #142; drafts 10 (regions) and 11 (complexity) shipped via PRDs #153 and #154.
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
- Server sends `Welcome` with full `GameState` snapshot **and** `ShipStations` (the parsed `[stations]` block from `player_ship.toml` for the current player count)
- Players edit name and pick a **station** via `SelectStation { station }` / `ReleaseStation`. A station bundles one or more consoles, so picking a station may grant Helm + Tactical (etc.) at once
- Server broadcasts `StationAssigned { token, station, consoles }`. Joining/leaving auto-shuffles via `reassign_on_join` / `reassign_on_leave` (`stations.rs`); spectators wait in a FIFO queue managed by `SessionManager`
- Only the player whose station contains `CaptainChair` sees and can press "Engage"
- Server validates: captain owns `CaptainChair`, all stations filled, `phase == Lobby`
- On `StartGame`: phase → `InProgress`, broadcast `GameStarted` to all

### 2. In-Progress Phase

- **Captain:** toggles Red Alert via `ToggleRedAlert`; changes view via `SetView`
- **Helm:** sends `HelmInput { thrust, steering }` at 10Hz; can push radar to viewscreen via `SetView { Radar }`; triggers impulse via `StartImpulseCharge`
- **Tactical:** sends `SetTarget { uuid }` to lock a target; sends `FirePhaser` (in range + forward arc); fires torpedoes via `FireTorpedo { tube, target_uuid }`; sets `SetPhaserMode { Auto | Manual }`
- **Repair:** sends `Repair { shape }` — must match the head of `BreakdownQueue`; dispatched to one of three repair teams (`repair_teams.rs`); wrong shape, wrong console, or no free team incurs a penalty cooldown
- **Power:** sends `IncreasePower { console }` / `DecreasePower { console }` distributing 6 base + up to 2 battery points across `Helm` / `Tactical` / `Science`; battery exhaustion locks all to level 1 until recharged
- **Science:** `SetScienceTarget { uuid }` for advisory target hand-off; `CancelImpulse` to abort Helm's charge; pushes `ScienceRadar` / `SystemChart` view modes
- **Console Complexity (PRD #154):** any console may switch via `SetComplexity { console, preset_name }`; broadcast as `ComplexityChanged`. Low complexity hides UI elements and runs `console_ai` server-side to operate hidden controls (auto-fire torpedoes, auto-match phaser frequency, auto-manage power overflow)
- **Server simulation:**
  - Reads helm inputs tagged with the helm holder's token
  - Feeds into `compute_physics()` (pure function in `ship_physics.rs`), modulated by `ShipModifiers`
  - Applies to ship's Rapier rigid body as direct velocity
  - Region containment runs each tick; entry/exit fires `RegionEntered`/`RegionExited` events that register/remove modifiers and `FlagKind` flags via `ShipModifiers`
  - Asteroid lifecycle: `update_asteroid_window` (PRD #191) tracks player grid cell, despawns cells outside `despawn_cells`, evaluates fresh density in cells entering `spawn_cells`; broadcasts `EntitySpawned` / `AsteroidDestroyed`
  - Damage from collisions and `damage_zone` regions both call the shared `apply_hull_damage` helper → `f32` hull → `breakdowns_from_damage()` → `BreakdownQueue::push_random()` (each breakdown gets a random `Shape`)
  - Every 100ms: broadcasts `SimState` (red alert, `f32` hull, `power_levels`, `flags`, `entity_states`, `radar_state`); sends `WeaponsUpdate` to Tactical; sends `RepairState` (teams + current breakdown shape) to Repair; sends `PowerState` to Power; emits `ModifierAdded` / `ModifierRemoved` deltas
- **Renderer:** 3D camera follows ship; `viewscreen_border.rs` wraps the viewscreen with a Bevy UI bezel + red-alert vignette + designation/HEADING/HULL/CONDITION HUD; phaser beams drawn when active (`beam_render.rs`); phone clients render their own bezel via `phone_border/`

### 3. Disconnection / Reconnection

- JS fires `wasm_player_disconnected(token)` when peer drops
- Server marks player disconnected; their station becomes vacant immediately and `reassign_on_leave` cascades the next eligible spectator into it
- On re-identify with same token: auto-reassign previous station if still free, otherwise the player goes to the back of the spectator queue
- `PlayerJoined` and `StationAssigned` broadcast to others on reconnect

---

## Module Map

| File | Role | Depends On | Bevy? |
|---|---|---|---|
| `messages.rs` | Pure data types. No logic. | serde | No |
| `codec.rs` | MessageCodec trait + JsonCodec | messages, serde_json | No |
| `session.rs` | SessionManager: player lifecycle | messages | No |
| `stations.rs` | Pure station model + reassignment | messages | No |
| `lobby_handler.rs` | Pure lobby message handler | messages, session, stations | No |
| `lobby.rs` | Bevy plugin: lobby message routing | lobby_handler, session | Yes |
| `simulation.rs` | Bevy plugin: physics + weapons + damage + regions | messages, session, ship_physics, ship_state, asteroid_window, radar, damage, breakdown, modifiers, repair_teams, power_system | Yes |
| `ship_physics.rs` | Pure physics: inputs → new state | None (pure Rust) | No |
| `ship_state.rs` | ShipState resource | messages | Yes |
| `asteroid_spawner.rs` | Pure per-cell density evaluation | rand, noise | No |
| `asteroid_window.rs` | Pure ring-buffer grid window | messages | No |
| `asteroid_lifecycle.rs` | Bevy systems: streaming spawn/despawn | asteroid_window, asteroid_spawner | Yes |
| `radar.rs` | Pure radar projection + fire-ready check | messages | No |
| `radar_config.rs` | Pure radar viewport configs | messages | No |
| `damage.rs` | apply_hull_damage + HullIntegrity (`f32`) | breakdown | No |
| `breakdown.rs` | BreakdownQueue + Shape + breakdowns_from_damage() | messages, rand | No |
| `repair_teams.rs` | Pure 3-team dispatch + cooldowns | messages | No |
| `modifiers.rs` | ShipModifiers cache + flag-set | messages, flag_kind | No |
| `flag_kind.rs` | FlagKind enum (`CommsJammed`, `SensorBlind`) | serde | No |
| `power_system.rs` | 6+2 power allocation + battery + lock | messages | No |
| `phaser.rs` | Pure phaser bank state machine | messages | No |
| `torpedo.rs` | Pure torpedo + tube state machine | messages | No |
| `shield.rs` | Pure four-quadrant shield model | messages | No |
| `impulse.rs` | Pure impulse-drive charge state machine | messages | No |
| `entity_config.rs` | TOML entity config types | serde | No |
| `entity_tags.rs` | String-tag helpers | serde | No |
| `map_config.rs` | TOML map config | serde | No |
| `config_cache.rs` | Bevy plugin: TOML preload via JS fetch | entity_config, map_config | Yes |
| `beam_render.rs` | Bevy plugin: phaser beam meshes | messages | Yes (server) |
| `viewscreen_border.rs` | Bevy plugin: viewscreen bezel + vignette + HUD | messages, ship_state | Yes (server) |
| `renderer.rs` | Bevy plugin: 2D lobby + 3D game view | messages, session, ship_state | Yes (server) |
| `bridge.rs` | wasm-bindgen exports (server feature) | codec, lobby, renderer, simulation | WASM+server |
| `client_lobby.rs` | Pure client lobby state + LobbyView | messages, stations | No |
| `client_sim.rs` | Pure client sim state (ClientSimState) | messages | No |
| `client_helm.rs` | Pure joystick logic | messages | No |
| `client_app.rs` | Bevy plugin: lobby + tactical/repair/power/science panels | client_lobby, client_sim | Yes (client) |
| `phone_border/` | Bevy plugin: phone bezel + helm/captain chrome | client_lobby, client_sim, client_helm | Yes (client) |
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

### Asteroid Field (`asteroid_spawner.rs` + `asteroid_window.rs`)

Player-centred ring-buffer grid (PRD #191). The world is divided into `resolution × resolution` cells. A `WindowedGrid` of size `(2 × despawn_cells + 1)²` sits centred on the player. As the player moves between grid cells, `update_asteroid_window` (Bevy system) computes which cells just entered the despawn ring (`None` them out — destroyed asteroids are forgotten and will respawn fresh on return) and which cells just entered the spawn ring (evaluate density from `(field_idx, gx, gz) + Perlin noise`; if the density check passes, spawn an `EntitySnapshot`-broadcast asteroid). The donut-bounded `asteroid_spawner.rs` density formula is preserved; only the lifecycle changed. No persistent destroyed-asteroid set.

### Consoles

- **CaptainChair:** Red Alert toggle (exclusive). Only captain can `StartGame` and `ToggleRedAlert`. View selector (Fore/Aft/Port/Starboard or Radar).
- **Helm:** Thrust + steering joystick. Sends `HelmInput` at 10Hz while active. Ship only moves when Helm is occupied. Displays radar overlay and "On Screen" button to push radar to the viewscreen. Triggers impulse charge via `StartImpulseCharge`.
- **Tactical:** Target lock (`SetTarget`), fire phasers (`FirePhaser`), set phaser mode (`SetPhaserMode { Auto | Manual }`), fire torpedoes (`FireTorpedo { tube, target_uuid }`). Receives `WeaponsUpdate` at 10Hz with lock status, fire readiness, cooldown, torpedo magazine, and per-tube reload state. Beam events (`BeamStarted`, `BeamEnded`, `PhaserFired`) and torpedo events (`TorpedoLaunched`, `TorpedoDestroyed`) broadcast to all.
- **Repair:** Shape-matching repair via `Repair { shape: Shape }` — shape must match the head of `BreakdownQueue`. Three repair teams accept dispatched work in parallel (`repair_teams.rs`); wrong shape, wrong console, or no free team incurs a penalty cooldown. `ShowRepairIcon` / `ClearRepairIcon` broadcasts add decoy shapes to the puzzle. Receives `RepairState` at 10Hz with team statuses + current breakdown shape.
- **Power:** Distributes 6 base + up to 2 battery points across `Helm`, `Tactical`, `Science` via `IncreasePower { console }` / `DecreasePower { console }`. Levels register modifiers on each console's relevant slots through `power_system.rs`. Battery exhaustion locks all consoles to level 1 until recharged to an emergency threshold. Receives `PowerState` at 10Hz; broadcast `power_levels` rides on `SimSnapshot`.
- **Science:** Long-range radar overlay, system chart on viewscreen, advisory target suggestion (`SetScienceTarget`), cancel an active impulse charge (`CancelImpulse`). View modes `ScienceRadar` and `SystemChart` are pushed to the viewscreen by Science.

Any console may switch its complexity preset via `SetComplexity { console, preset_name }` (PRD #154). At Low complexity, hidden controls are operated server-side by `console_ai`; at Full complexity, all controls are visible and human-driven.

---

## Testing Strategy

### Rust unit tests (`cargo test`)

Tests live inline with modules (`#[cfg(test)] mod tests`).

- **`session.rs`** — Player registration, duplicate tokens, station assignment/clearing, disconnect vacancy + spectator promotion, reconnect auto-assign, `helm_token()` / `captain_token()` lookups, conflict resolution
- **`stations.rs`** — TOML parse + validation, `get_station`, `all_stations_filled`, `reassign_on_join` / `advance_on_join` / `reassign_on_leave` cascade, spectator FIFO interaction
- **`codec.rs`** — Round-trip serialization for every `ClientMessage` and `ServerMessage` variant (incl. `SelectStation`, `StationAssigned`, `ModifierAdded`/`Removed`, `EntitySnapshot`, `PowerState`, `RepairState`, `SetComplexity`, `ComplexityChanged`)
- **`lobby_handler.rs`** — Pure handler: Identify → Welcome (with `ShipStations`), `SelectStation`/`ReleaseStation` → broadcast, captain-by-CaptainChair authority, all-stations-filled gate, HelmInput ignored in lobby, disconnect handling
- **`ship_physics.rs`** — Zero input, thrust curve, deceleration curve, steering yaw, diagonal motion, dt scaling, speed cap
- **`asteroid_spawner.rs`** — Per-cell density determinism, donut bounds, Y offsets, cosmetic vs gameplay layers
- **`asteroid_window.rs`** — Player grid-cell math, slot index wrapping, `eval_on_player_move` (despawn list + spawn list), large-jump fallback
- **`ship_state.rs`** — Red alert toggle, snapshot generation
- **`radar.rs`** — project_to_radar (yaw rotation, range cull), project_asteroid, radar_dots iterator, is_fire_ready (range + arc gates)
- **`damage.rs`** — collision_damage formula (zero speed, max speed, mid speed, clamp), `f32` HullIntegrity (apply/restore/floor/ceiling), shared `apply_hull_damage` helper returns expected breakdowns
- **`breakdown.rs`** — BreakdownQueue push/pop/front (with random `Shape`), no-repeat picker, breakdowns_from_damage bucket math (float input)
- **`repair_teams.rs`** — Dispatch to free slot, no-free-slot returns penalty, cooldown tick, wrong-shape penalty
- **`modifiers.rs`** — Bonus aggregation formula (`s ≥ 0` → `1+s`; `s < 0` → `1/(1+|s|)`), per-source removal, flag set OR-aggregation across sources, `RegionEffect { uuid }` source identity
- **`flag_kind.rs`** — Enum + serde round-trip
- **`power_system.rs`** — Base 6 distribution, battery exhaustion lock, recharge re-engage threshold, modifier registration per level
- **`stations.rs`** — see above
- **`client_lobby.rs`** — LobbyState message application (Welcome+ShipStations, PlayerJoined/Left, StationAssigned, GameStarted), LobbyView derivation (station rows, is_captain, is_helm, all_filled), outbound message builders
- **`client_sim.rs`** — ClientSimState message application (SimState, WorldSetup, Welcome, RepairState, PowerState, EntitySpawned/Despawned, ModifierAdded/Removed), is_active_camera_direction, message builders
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
6. **Deterministic asteroids.** Per-cell density is seeded from `(field_idx, gx, gz) + Perlin noise`, so the same world cell always produces the same asteroid. Destroyed asteroids respawn fresh when the player leaves the cell and returns (no persistent destroyed-set).
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
