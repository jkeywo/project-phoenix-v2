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
- **PRD #118:** [Repair + Power Consoles](https://github.com/jkeywo/project-phoenix-v2/issues/118) — Renamed `Engineering` → `Repair`, added `Power`, three-team dispatch repair (travel → repair → return) with per-console hull, 6+2 power allocation
- **PRD #120:** [Station-Based Lobby & Crew Assignment](https://github.com/jkeywo/project-phoenix-v2/issues/120) — Per-station picking, auto-shuffle, spectator FIFO. `SelectStation`/`ReleaseStation`/`StationAssigned` wire
- **PRD #153:** [Region Entities, Component-Driven Spawning & Modifier Flags](https://github.com/jkeywo/project-phoenix-v2/issues/153) — Single `[[entity]]` pipeline, six region effects, per-console hull, `FlagKind`, unified `EntitySnapshot` wire
- **PRD #154:** [Console Complexity: UI Hiding + AI Automation](https://github.com/jkeywo/project-phoenix-v2/issues/154) — Per-console `Low`/`Full` presets, hide UI + server-side `console_ai` to operate hidden controls
- **PRD #180:** [Viewscreen Frame](https://github.com/jkeywo/project-phoenix-v2/issues/180) — Bevy UI border, `RedAlertVignetteMaterial`, designation + HEADING / HULL / CONDITION HUD
- **PRD #187:** [Phone Console HUD — Diegetic Bezel Frame](https://github.com/jkeywo/project-phoenix-v2/issues/187) — `phone_border/` plugin: bezel wraps every console; full helm + captain chrome
- **PRD #191:** [Grid-Based Asteroid Lifecycle with Deterministic Ring Buffer Window](https://github.com/jkeywo/project-phoenix-v2/issues/191) — `asteroids/window.rs`, player-centred grid, destroyed asteroids respawn on return
- **PRD #119:** [Space Stations, Scenario Engine & Comms Console](https://github.com/jkeywo/project-phoenix-v2/issues/119) — TOML world files with triggers/actions/objectives, station entities, `Console::Comms`. `WorldPlugin` owns loading; `assets/worlds/` holds content files.
- **PRD #142 (partial):** [AI and Behaviour System](https://github.com/jkeywo/project-phoenix-v2/issues/142) — `src/ai/` plugin: data-driven patrol NPCs injecting inputs via the same message path as players; `assets/factions/` for faction configs.

**Open PRDs (planned work):**
- **PRD #116:** [Save/Load Game Sessions](https://github.com/jkeywo/project-phoenix-v2/issues/116) — `localStorage`-backed save slots, periodic + lifecycle saves, version-gated load. Introduces `save.rs` (the *second* sanctioned `serde_json` surface).

**Current state:** Nine consoles in the wire types (`CaptainChair`, `Helm`, `Tactical`, `Repair`, `Sensors`, `Shields`, `Navigation`, `Power`, `Comms`). The old `Science` console was split into `Sensors` (long-range radar + target suggestion), `Shields` (four-quadrant shield focus), and `Navigation` (system chart + impulse cancel); `Comms` handles contacts, messages, and objectives. Players join *stations* (bundles of one or more consoles defined per player count in `player_ship.toml`), not individual consoles. Full simulation: ship physics loaded from TOML, grid-based streaming asteroid field, phaser banks (port/starboard), torpedoes, four-quadrant shields, impulse drive, per-console hull damage, three-team dispatch repair (travel → repair → return), 6+2 power allocation driving cross-system modifiers, region effects (damage zones, slow zones, comms/sensor jammers), per-console complexity presets with server-side AI to operate hidden controls, TOML-driven world engine with objectives and NPC AI patrols. Viewscreen and phone consoles both have diegetic bezel frames with red-alert vignette + HUD. Swipe-anywhere console switching with tab initials on overflow. Data-driven entities and worlds loaded from TOML via `assets/`. Client is a full Bevy/WASM app. See **[wiki/](./wiki/)** for the deeper map of the codebase.

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
server/bridge.rs: drain_inbound()
  ↓  queues InboundMessage into Bevy's pull-based message system
lobby/server.rs (or console plugins via SimSet::Input)
  ↓  reads InboundMessage, mutates SessionManager / ShipState
  ↓  writes OutboundMessage events
server/bridge.rs: flush_outbound()
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
  core/
    messages.rs       — Wire types: Console, ClientMessage, ServerMessage, EntitySnapshot, CommsMessage, etc.
    codec.rs          — MessageCodec trait + JsonCodec. ONLY place serde_json is used directly.
    flag_kind.rs      — Typed boolean flags (CommsJammed, SensorBlind).
    broadcast/        — Broadcaster, LobbyBroadcaster, SimBroadcaster, Cadence, Audience.
  lobby/
    handler.rs        — Pure lobby message handler: process_message(), process_disconnect(). No Bevy.
    server.rs         — Bevy plugin: lobby message routing, init_state::<GamePhase>(), StatesPlugin.
    session.rs        — SessionManager: tokens → players, console assignment, reconnect/vacancy logic.
    stations_config.rs — Pure station model: parse, validate, lookup (PRD #120).
    stations_policy.rs — reassign-on-join/leave, spectator FIFO (PRD #120).
    client_panel.rs   — Pure client lobby state model: LobbyState, LobbyView. No Bevy.
  ship/
    physics.rs        — Pure Rust physics controller. Input/output function, fully testable.
    state.rs          — ShipState resource: position, yaw, speed, red_alert, hull, phaser state.
    damage.rs         — apply_hull_damage + HullIntegrity (f32). No Bevy.
    impulse.rs        — Pure impulse-drive charge state machine.
  weapons/
    phaser.rs         — Pure phaser bank state machine: lock, fire, sever, cooldown.
    torpedo.rs        — Pure torpedo + tube state machine: load, launch, homing, expiry.
    shield.rs         — Pure four-quadrant shield model: HP, online/offline, regen.
    beam_render.rs    — Bevy plugin: renders active phaser beam(s) as line meshes (server only).
  modifiers/
    cache.rs          — Pure ShipModifiers multiplier table + flag set.
    breakdown.rs      — BreakdownQueue FIFO + breakdowns_from_damage(). No Bevy.
    repair_teams.rs   — Pure three-team dispatch repair: travel → repair → return (PRD #118).
    power_system.rs   — Pure 6+2 power allocation, battery, exhaustion lock (PRD #118).
    coordination.rs   — Region modifier registration / removal helpers.
  asteroids/
    spawner.rs        — Pure per-cell density evaluation (seeded, deterministic).
    window.rs         — Pure ring-buffer window: player-centred grid lifecycle (PRD #191).
    lifecycle.rs      — Bevy systems: spawn/despawn cells as the player moves; broadcast deltas.
  regions/
    server.rs         — Bevy plugin: containment checks, RegionEntered/RegionExited Observers.
    effects.rs        — Region effect components (blocks_impulse, damage_zone, comms_jammed, etc.).
    shape.rs          — RegionShape types (Sphere, Box, Torus, all in XZ plane).
  entities/
    config.rs         — TOML entity config types (EntityConfig, asteroid/ship/station/region fields).
    map_config.rs     — Legacy TOML map-half parser (anchors, asteroid fields, entity instances). One half of `WorldConfig` pending the type-level merger (PRD #337).
    config_cache.rs   — ConfigCachePlugin: preloads world + entity TOML via JS fetch on WASM. Exposes `wasm_load_world` (JS-facing single entry point) which internally still calls `wasm_load_map` + `wasm_load_world_content` (PRD #337).
    tags.rs           — String-tag helpers for tags=[...] lookups.
    spawner.rs        — Entity spawning from EntityConfig (ECS + wire snapshot).
    loader.rs         — World/entity loading pipeline.
    entity_override.rs — Per-instance entity field overrides (from world TOML).
  world/
    server.rs         — WorldPlugin: world-file loading, entity lifecycle, trigger evaluation,
                        objective tracking, WorldSetup broadcast.
    content.rs        — Pure types: `ScenarioConfig` (the trigger/comms/named-spawn half), `WorldConfig` (currently a thin wrapper over `MapConfig`+`ScenarioConfig` — PRD #337 collapses them), `ScenarioManager`, position resolution.
  ai/
    server.rs         — Bevy plugin: NPC patrol loop, input injection via InboundMessage.
    core.rs           — Pure AI state machine (patrol, idle, attack states).
    faction.rs        — Faction config types loaded from assets/factions/*.toml.
  console/
    captain/server.rs — CaptainPlugin: red alert, view selector, StartGame gate.
    helm/joystick.rs  — Pure joystick logic: drag/release/tick, clamp_to_circle. No Bevy.
    helm/client.rs    — Bevy client plugin: helm panel, HelmInput dispatch.
    weapons/server.rs — WeaponsPlugin: target lock, phaser fire, torpedo launch.
    weapons/client.rs — Tactical panel.
    repair/server.rs  — RepairPlugin: shape-match dispatch, penalty cooldown.
    repair/client.rs  — Repair panel.
    power/server.rs   — PowerPlugin: IncreasePower/DecreasePower routing.
    power/client.rs   — Power panel.
    science/server.rs — SciencePlugin: Sensors + Shields + Navigation server logic.
    science/client.rs — Sensors/Shields/Navigation panels.
    comms/inbox.rs    — Pure CommsInbox state. No Bevy.
    comms/client.rs   — Comms panel: contacts list, message inbox, objectives.
  console_ai/
    server.rs         — Bevy plugin: automated AI for Low-complexity consoles.
    core.rs           — Pure AI console decision logic.
    complexity.rs     — Complexity preset loading from assets/complexity/*.toml.
    delegation.rs     — Three-tier delegation model (native → partner → AI).
  server/
    bridge.rs         — wasm-bindgen exports. Compiled when `server` feature is active.
    renderer.rs       — Bevy plugin: lobby UI + 3D game camera (server only).
    viewscreen_border.rs — Bevy plugin: viewscreen bezel + RedAlertVignetteMaterial + HUD (PRD #180).
    debug_overlay.rs  — Bevy plugin: developer debug overlay.
  client/
    app.rs            — Bevy plugin: lobby panel + all console panels.
    bridge.rs         — wasm-bindgen exports. Compiled when `client` feature is active.
    elements.rs       — Shared UI element helpers.
    phone_border/     — Bevy plugin: phone bezel framing + helm + captain chrome (PRD #187).
  sim_sets.rs         — SimSet enum (Input, Physics, Damage, Modifiers, Broadcast) for system ordering.
  ship_plugin.rs      — Bevy plugin: ship spawning + Rapier rigid body setup.
  server_app.rs       — Server App builder: plugin registration + SimSet chain ordering.
  objectives.rs       — Pure ObjectiveManager: add/complete/fail, dirty tracking. No Bevy.
  radar.rs            — Pure radar projection math. Shared by server renderer and client panels.
  radar_config.rs     — Pure radar viewport configs (helm, weapons, sensors).
  client_sim.rs       — Pure ClientSimState: applies ServerMessages on the client. No Bevy.
  client_comms.rs     — Pure ClientCommsState: contacts, messages, objectives. No Bevy.
  client_complexity.rs — Pure client complexity preset state. No Bevy.
  lib.rs              — Module declarations + feature gates + backward-compat re-exports.

assets/
  worlds/default.toml           — Default world: anchors, [[entity]] instances, named [[spawn]]s, [[trigger]]s, [[comms]].
  worlds/patrol.toml            — Patrol world: three-anchor raider patrol with on-destroyed objective.
  entities/asteroid_*.toml      — Asteroid variants (large, small, cosmetic).
  entities/player_ship.toml     — Ship config: helm_console physics, phaser banks, torpedo tubes, shields, impulse, [stations].
  entities/pirate_raider.toml   — NPC raider entity config.
  entities/region_*.toml        — Region templates per effect type (PRD #153).
  factions/*.toml               — AI faction definitions.
  complexity/*.toml             — Per-console complexity presets (Low / Full) + AI tuning (PRD #154).

server.html           — Host page: loads server WASM, runs Bevy, owns PeerJS host peer.
client.html           — Client page: loads client WASM, connects to host via PeerJS peer ID in URL hash.
Cargo.toml            — Single crate: cdylib (WASM) + rlib (tests). Features: server | client.
Trunk.toml            — Build config for server.html (default = server feature).
client-trunk.toml     — Build config for client.html (client feature).
.github/workflows/    — CI: builds both pages, deploys to gh-pages, runs smoke tests.
wiki/                 — LLM-maintained knowledge base. Read SCHEMA.md first; update as you work.
docs/                 — Draft design notes (numbered).
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

Region lifecycle events (`RegionEntered`, `RegionExited`) use Bevy's **Observer/Trigger** pattern (`commands.trigger(ev)` / `app.add_observer(fn)`) for immediate reaction without polling.

### Phase Management — `States<GamePhase>`

Game phase uses Bevy's native `States` framework. `GamePhase` derives `States`, `Hash`, `Default`. Phase transitions use `NextState<GamePhase>`. The `SimSet` chain is gated by `.run_if(in_state(GamePhase::InProgress))`; start-of-game setup systems use `OnEnter(GamePhase::InProgress)`. `LobbyPlugin` calls `app.init_state::<GamePhase>()` (requires `StatesPlugin`, which is added explicitly before it).

### System Set Ordering — `SimSet`

All in-game systems are placed into one of five `SimSet` variants (defined in `sim_sets.rs`) and chained in `server_app.rs`:

```
Input → Physics → Damage → Modifiers → Broadcast
```

Console input handlers use `.in_set(SimSet::Input)`, physics uses `SimSet::Physics`, collision/hull-damage uses `SimSet::Damage`, modifier cache flush uses `SimSet::Modifiers`, and all broadcast/outbox systems use `SimSet::Broadcast`.

### Serialization — The Codec Contract

`serde_json` must **never** be called directly outside `src/core/codec.rs`. The `MessageCodec` trait is the only serialization surface. This exists so the wire format can be swapped to binary (MessagePack, rmp-serde, etc.) by changing one module.

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
- **Repair:** sends `DispatchRepairTeam { team_idx, console }` to assign one of three teams to repair a damaged console. Each team cycles Idle → Travelling (5s) → Repairing (0.5 HP/s) → Returning (5s). Receives `RepairState` at 10Hz with per-team status
- **Power:** sends `IncreasePower { console }` / `DecreasePower { console }` distributing 6 base + up to 2 battery points across `Helm` / `Tactical` / `Sensors`; battery exhaustion locks all to level 1 until recharged
- **Sensors:** `SetScienceTarget { uuid }` for advisory target hand-off; long-range radar overlay; pushes `SensorsRadar` view mode
- **Shields:** four-quadrant shield status and focus mechanic
- **Navigation:** `CancelImpulse` to abort Helm's charge; pushes `NavigationChart` view mode
- **Comms:** `SelectCommsMessage { message_id }` to open a contact message; receives `CommsState { messages, contacts }` per broadcast
- **Console Complexity (PRD #154):** any console may switch via `SetComplexity { console, preset_name }`; broadcast as `ComplexityChanged`. Low complexity hides UI elements and runs `console_ai` server-side to operate hidden controls (auto-fire torpedoes, auto-match phaser frequency, auto-manage power overflow)
- **Server simulation:**
  - Reads helm inputs tagged with the helm holder's token
  - Feeds into `compute_physics()` (pure function in `ship_physics.rs`), modulated by `ShipModifiers`
  - Applies to ship's Rapier rigid body as direct velocity
  - Region containment runs each tick; entry/exit fires `RegionEntered`/`RegionExited` events that register/remove modifiers and `FlagKind` flags via `ShipModifiers`
  - Asteroid lifecycle: `update_asteroid_window` (PRD #191) tracks player grid cell, despawns cells outside `despawn_cells`, evaluates fresh density in cells entering `spawn_cells`; broadcasts `EntitySpawned` / `AsteroidDestroyed`
  - Damage from collisions and `damage_zone` regions both call the shared `apply_hull_damage` helper → distributes HP loss across per-console hull slots (`ConsoleHull`); when any slot hits 0 the console is offline until repaired
  - Every 100ms: broadcasts `SimState` (red alert, `console_hull`, `power_levels`, `flags`, `entity_states`, `radar_state`); sends `WeaponsUpdate` to Tactical; sends `RepairState` (per-team status) to Repair; sends `PowerState` to Power; emits `ModifierAdded` / `ModifierRemoved` deltas
- **Renderer:** 3D camera follows ship; `viewscreen_border.rs` wraps the viewscreen with a Bevy UI bezel + red-alert vignette + designation/HEADING/HULL/CONDITION HUD; phaser beams drawn when active (`beam_render.rs`); phone clients render their own bezel via `phone_border/`

### 3. Disconnection / Reconnection

- JS fires `wasm_player_disconnected(token)` when peer drops
- Server marks player disconnected; their station becomes vacant immediately and `reassign_on_leave` cascades the next eligible spectator into it
- On re-identify with same token: auto-reassign previous station if still free, otherwise the player goes to the back of the spectator queue
- `PlayerJoined` and `StationAssigned` broadcast to others on reconnect

---

## Module Map

| Module | Role | Bevy? |
|---|---|---|
| `core/messages` | Wire types: Console (9), ClientMessage, ServerMessage, EntitySnapshot, CommsMessage | No |
| `core/codec` | MessageCodec trait + JsonCodec. Only serde_json surface. | No |
| `core/flag_kind` | FlagKind enum (CommsJammed, SensorBlind) | No |
| `core/broadcast` | Broadcaster, LobbyBroadcaster, SimBroadcaster, Cadence, Audience | Yes |
| `lobby/handler` | Pure lobby message handler: process_message(), process_disconnect() | No |
| `lobby/server` | Bevy plugin: lobby routing, `init_state::<GamePhase>()`, `States<GamePhase>` | Yes |
| `lobby/session` | SessionManager: tokens → players, console assignment, reconnect/vacancy | No |
| `lobby/stations_config` | Pure station model: parse, validate, lookup | No |
| `lobby/stations_policy` | reassign-on-join/leave, spectator FIFO | No |
| `lobby/client_panel` | Pure client lobby state: LobbyState, LobbyView | No |
| `ship/physics` | Pure physics: inputs → new ShipState | No |
| `ship/state` | ShipState Bevy resource | Yes |
| `ship/damage` | apply_hull_damage + HullIntegrity (f32) | No |
| `ship/impulse` | Pure impulse-drive charge state machine | No |
| `weapons/phaser` | Pure phaser bank state machine | No |
| `weapons/torpedo` | Pure torpedo + tube state machine | No |
| `weapons/shield` | Pure four-quadrant shield model | No |
| `weapons/beam_render` | Bevy plugin: phaser beam meshes | Yes (server) |
| `modifiers/cache` | ShipModifiers multiplier table + flag-set | No |
| `modifiers/breakdown` | BreakdownQueue + Shape + breakdowns_from_damage() | No |
| `modifiers/repair_teams` | Pure 3-team dispatch + cooldowns | No |
| `modifiers/power_system` | 6+2 power allocation + battery + lock | No |
| `modifiers/coordination` | Region modifier registration helpers | No |
| `asteroids/spawner` | Pure per-cell density evaluation | No |
| `asteroids/window` | Pure ring-buffer grid window (PRD #191) | No |
| `asteroids/lifecycle` | Bevy systems: streaming spawn/despawn | Yes |
| `regions/server` | Bevy plugin: containment, RegionEntered/RegionExited Observers | Yes |
| `regions/effects` | Region effect components | No |
| `regions/shape` | RegionShape types | No |
| `entities/config` | TOML entity config types | No |
| `entities/map_config` | TOML map config | No |
| `entities/config_cache` | Bevy plugin: TOML preload via JS fetch on WASM | Yes |
| `entities/tags` | String-tag helpers | No |
| `entities/spawner` | ECS entity spawning from EntityConfig | Yes |
| `entities/loader` | Scenario/entity loading pipeline | Yes |
| `world/server` | WorldPlugin: world-file loading, triggers, objectives, WorldSetup broadcast | Yes |
| `world/content` | WorldContentResource, WorldContentRuntime | No |
| `ai/server` | Bevy plugin: NPC patrol + input injection | Yes |
| `ai/core` | Pure AI state machine | No |
| `ai/faction` | Faction config types | No |
| `console/captain/server` | CaptainPlugin: red alert, view selector, StartGame gate | Yes (server) |
| `console/helm/joystick` | Pure joystick logic: drag/release/tick, clamp_to_circle | No |
| `console/helm/client` | Bevy plugin: helm panel, HelmInput dispatch | Yes (client) |
| `console/weapons/server` | WeaponsPlugin: target lock, phaser, torpedoes | Yes (server) |
| `console/repair/server` | RepairPlugin: shape-match dispatch | Yes (server) |
| `console/power/server` | PowerPlugin: IncreasePower/DecreasePower routing | Yes (server) |
| `console/science/server` | SciencePlugin: Sensors + Shields + Navigation server logic | Yes (server) |
| `console/comms/inbox` | Pure CommsInbox state | No |
| `console/comms/client` | Bevy plugin: contacts list, message inbox, objectives panel | Yes (client) |
| `console_ai/server` | Bevy plugin: automated AI for Low-complexity consoles | Yes (server) |
| `server/bridge` | wasm-bindgen exports (server feature) | WASM+server |
| `server/renderer` | Bevy plugin: 2D lobby + 3D game camera | Yes (server) |
| `server/viewscreen_border` | Bevy plugin: viewscreen bezel + vignette + HUD (PRD #180) | Yes (server) |
| `client/app` | Bevy plugin: lobby + all console panels | Yes (client) |
| `client/bridge` | wasm-bindgen exports (client feature) | WASM+client |
| `client/phone_border` | Bevy plugin: phone bezel + helm/captain chrome (PRD #187) | Yes (client) |
| `sim_sets` | SimSet enum: Input, Physics, Damage, Modifiers, Broadcast | No |
| `ship_plugin` | Bevy plugin: ship spawning + Rapier rigid body | Yes |
| `server_app` | Server App builder: plugin registration + SimSet chain ordering | Yes |
| `objectives` | Pure ObjectiveManager: add/complete/fail/dirty-tracking | No |
| `radar` | Pure radar projection + fire-ready check | No |
| `radar_config` | Pure radar viewport configs | No |
| `client_sim` | Pure ClientSimState: applies ServerMessages | No |
| `client_comms` | Pure ClientCommsState: contacts, messages, objectives | No |
| `client_complexity` | Pure client complexity preset state | No |

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
- **Repair:** Dispatch repair via `DispatchRepairTeam { team_idx, console }`. Three repair teams run in parallel; each cycles Idle → Travelling (5s) → Repairing (0.5 HP/s until target slot is full) → Returning (5s) → Idle. Receives `RepairState` at 10Hz with per-team phase + current target.
- **Power:** Distributes 6 base + up to 2 battery points across `Helm`, `Tactical`, `Sensors` via `IncreasePower { console }` / `DecreasePower { console }`. Levels register modifiers on each console's relevant slots through `power_system.rs`. Battery exhaustion locks all consoles to level 1 until recharged to an emergency threshold. Receives `PowerState` at 10Hz; broadcast `power_levels` rides on `SimSnapshot`.
- **Sensors:** Long-range radar overlay, advisory target suggestion (`SetScienceTarget`). Pushes `SensorsRadar` view mode to the viewscreen.
- **Shields:** Four-quadrant shield status and focus mechanic.
- **Navigation:** System chart on viewscreen (`NavigationChart`), cancel an active impulse charge (`CancelImpulse`).
- **Comms:** Displays hailable contacts, message inbox, and active objectives. Sends `SelectCommsMessage { message_id }` to open a message. Receives `CommsState { messages, contacts }` per broadcast.

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
- **`damage.rs`** — collision_damage formula (zero speed, max speed, mid speed, clamp); per-console `ConsoleHull` aggregator (apply_damage distributes across slots, restore, total_current/total_max)
- **`breakdown.rs`** — BreakdownQueue push/pop/front (with random `Shape`), no-repeat picker, breakdowns_from_damage bucket math (float input)
- **`modifiers/repair_teams`** — Dispatch to free slot, no-free-slot returns penalty, cooldown tick, wrong-shape penalty
- **`modifiers/cache`** — Bonus aggregation formula (`s ≥ 0` → `1+s`; `s < 0` → `1/(1+|s|)`), per-source removal, flag set OR-aggregation across sources, `RegionEffect { uuid }` source identity
- **`core/flag_kind`** — Enum + serde round-trip
- **`modifiers/power_system`** — Base 6 distribution, battery exhaustion lock, recharge re-engage threshold, modifier registration per level
- **`lobby/stations_config` + `stations_policy`** — see stations_config above
- **`lobby/client_panel`** — LobbyState message application (Welcome+ShipStations, PlayerJoined/Left, StationAssigned, GameStarted), LobbyView derivation (station rows, is_captain, all_filled), outbound message builders
- **`client_sim`** — ClientSimState message application (SimState, WorldSetup, Welcome, RepairState, PowerState, EntitySpawned/Despawned, ModifierAdded/Removed), is_active_camera_direction, message builders
- **`console/helm/joystick`** — clamp_to_circle, compute_thrust_steering, press/drag/release/tick state machine

### Smoke tests (`tests/smoke/`, Playwright + Chromium)

End-to-end tests that boot the real server WASM in a headless browser and exercise the full message flow. They replace `window.Peer` with a `BroadcastChannel`-backed shim so no real WebRTC is needed.

| File | What it covers |
|---|---|
| `shim.spec.ts` | BroadcastChannel PeerJS shim unit tests |
| `server-load.spec.ts` | Server WASM boots, `window.__wasmReady` fires, no JS errors |
| `client-connect.spec.ts` | Real `client.html` connects, `#status` = "Connected" after Welcome |
| `lobby.spec.ts` | Console selection broadcasts; non-captain StartGame is ignored |
| `stations.spec.ts` | Station assignment, auto-shuffle, spectator FIFO |
| `reassignment.spec.ts` | Console reassignment on player join/leave |
| `sim-state.spec.ts` | SimState fields validated; HelmInput changes ship position |
| `world-bootstrap.spec.ts` | Default scenario loads; Starbase Alpha appears in WorldSetup |
| `patrol.spec.ts` | AI raider spawns and patrols from its anchor |
| `engineering.spec.ts` | Repair / power console interactions |
| `comms.spec.ts` | Comms console receives initial CommsState with contacts |
| `debug-overlay.spec.ts` | Debug overlay renders without errors |

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
2. **Feature gates for bridges.** `server/bridge.rs` is compiled under the `server` feature; `client/bridge.rs` under the `client` feature. Neither is gated by `cfg(target_arch)` alone — the feature flag controls it.
3. **Captain authority.** Only the player at `CaptainChair` can `StartGame` and `ToggleRedAlert`. The server enforces this.
4. **Console vacancy on disconnect.** Immediately — in all game phases.
5. **Helm sends at 10Hz.** Simulation reads helm inputs at 10Hz tick intervals.
6. **Deterministic asteroids.** Per-cell density is seeded from `(field_idx, gx, gz) + Perlin noise`, so the same world cell always produces the same asteroid. Destroyed asteroids respawn fresh when the player leaves the cell and returns (no persistent destroyed-set).
7. **WebGL2 rendering.** For broad browser support.
8. **PeerJS cloud broker.** Not self-hosted (deferred post-PoC).
9. **Pure modules are Bevy-free.** `lobby/handler`, `radar`, `ship/damage`, `modifiers/breakdown`, `lobby/client_panel`, `client_sim`, `client_comms`, `console/helm/joystick` have no Bevy imports — they are fully unit-testable on native and shared between server and client.
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

1. Add variant to enum in `core/messages.rs` (derive `Clone, Debug, Serialize, Deserialize, PartialEq`)
2. Add round-trip test in `core/codec.rs` (`codec-tests` module)
3. Handle in `lobby/handler.rs` `process_message()` (pass through or produce outbound)
4. Handle in the appropriate console plugin (`.in_set(SimSet::Input)`) if it is an in-game message
5. Handle in `lobby/client_panel.rs` `LobbyState::apply()` or `client_sim.rs` `ClientSimState::apply()` as appropriate
6. Update `client/app.rs` if a new UI element or button is needed
7. Handle in `server.html` JS `routeOutbound()` if routing logic needs adjustment
8. Handle in `client.html` JS if the handshake / PeerJS wiring changes
