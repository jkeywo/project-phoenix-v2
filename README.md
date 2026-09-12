# Project Phoenix — Bridge Simulator

A browser-based spaceship bridge simulator for groups. One browser tab on a shared screen shows a 3D view of space. Players join from their phones by scanning a QR code — no app install required.

**[Play it live →](https://pp-dev.kiwigamedesign.co.uk/)**

---

## How to Play

1. Open the **[view screen](https://pp-dev.kiwigamedesign.co.uk/)** in a browser tab on a shared monitor or TV.
2. Players scan the QR code on screen with their phones and open the link.
3. Each player sets a name and claims a console (Captain's Chair or Helm).
4. The game starts once the connected crew is ready.
5. Navigate the ship through the asteroid field without hitting anything.

### Consoles

| Console | Controls |
|---|---|
| **Captain's Chair** | Toggle Red Alert; switch view camera |
| **Helm** | Joystick — up/down = thrust/reverse, left/right = steering; radar overlay; trigger impulse charge |
| **Tactical** | Lock targets, fire phaser banks (port/starboard), launch torpedoes |
| **Repair** | Shape-matching repair: dispatch the head breakdown to one of three repair teams |
| **Power** | Distribute 6 base + 2 battery power points across Helm / Tactical / Sensors |
| **Sensors** | Long-range radar overlay, suggest targets to Tactical |
| **Shields** | Four-quadrant shield status and focus mechanic |
| **Navigation** | System chart on viewscreen, cancel impulse charge |
| **Comms** | Manage contacts, send and receive messages, track objectives |

Players claim fixed ship-authored stations rather than individual consoles. Vacant stations can be claimed directly; releasing or disconnecting hands their systems to Backfill AI.

See [`wiki/`](./wiki/) for a deeper architectural map and [GitHub PRDs](https://github.com/jkeywo/project-phoenix-v2/issues?q=label%3APRD) for upcoming work (save/load, scenarios + comms console, AI behaviours).

---

## Tech Stack

| Layer | Tech |
|---|---|
| Game engine (authoritative server) | [Bevy](https://bevyengine.org/) 0.18 (Rust/WASM) |
| Physics | [bevy_rapier3d](https://github.com/dimforge/bevy_rapier) 0.33 |
| Networking | Phoenix transport: a Cloudflare Worker rendezvous service (`worker-rendezvous/`) for typed join codes + WebRTC signalling, then direct DataChannels |
| Client | Pure HTML/CSS/JavaScript |
| Build | [Trunk](https://trunkrs.dev/) for the server; Node build script for the client |
| Hosting | GitHub Pages |

The host page (`server.html`) runs the authoritative WebAssembly game simulation. The phone client (`client.html`) is pure HTML/CSS/JavaScript; it renders lobby and console interfaces, sends inputs, and applies JSON state snapshots over its two WebRTC DataChannels.

---

## Architecture

```
        ┌─────────────────────┐
        │  server.html (WASM) │  ← authoritative simulation
        │  Bevy + Rapier3D    │     rendezvous host record
        └────────┬────────────┘
                 │  WebRTC DataChannels (reliable + lossy)
        ┌────────┴────────┐
        ▼                 ▼
  client.html       client.html
  (phone #1)        (phone #2)  ...
```

- **Star topology** — clients never talk to each other, only to the host.
- **Session tokens** — 32 hex characters, held per tab in `sessionStorage` with a persistent `localStorage` copy the first tab adopts. Survives a page refresh; same token = same player, and an automatic reconnect re-sends it to restore the held station and the current projection.
- **Join by typing a code** — the host is issued a private code; the QR carries the full structured form of the same code, so scanning and typing land in the same place.
- **Almost no backend** — the rendezvous service handles code lookup and the WebRTC handshake and then gets out of the way; all game data flows peer-to-peer.

---

## Development

### Prerequisites

```bash
rustup target add wasm32-unknown-unknown
cargo install trunk
```

### Running locally

```bash
# View screen (WASM + Bevy, port 8080)
trunk serve

# Client page (plain HTML, port 8081) — open in a second tab or on your phone
trunk serve --config client-trunk.toml --port 8081
```

Then open `http://localhost:8080` as the view screen and `http://localhost:8081` on your phone (or a second tab).

The first offline Workshop Authoring slice is at `/workshop.html` on the host.
It imports one TOML/Rhai mod ZIP, edits source with pack-wide undo, and exports
through the existing structural checks. `npm run build:workshop` also builds
`dist/workshop.html` without compiling WASM; serve that directory over HTTP.
On Windows, double-click `run-workshop.bat` to build it, start a local server
and open the Workshop in your browser. Keep its window open while editing;
Ctrl+C stops the server. It uses port 8083 and needs the installed npm dependencies.
Runtime validation, disposable Test simulation and model tooling remain later
M6 work. See [Editor and Workshop](wiki/entities/editor.md) for the current boundary.

### Tests

```bash
# Rust unit tests (session, codec, lobby, physics, asteroids)
cargo test
```

Tests cover session management, serialization round-trips, lobby message routing, ship physics, and asteroid spawning.

#### Smoke tests (end-to-end, Playwright + Chromium)

The smoke tests boot the real WASM build in a headless browser and exercise the full message flow — connect → lobby → game start → helm physics — using a `BroadcastChannel` shim instead of real WebRTC.

```bash
# 1. Build dist/ (required before running smoke tests)
trunk build --release
trunk build --release --config client-trunk.toml

# 2. Install Playwright deps (one-time)
cd tests/smoke
npm install
npx playwright install chromium

# 3. Run
npx playwright test

# Optional: headed mode for debugging
npx playwright test --headed
```

The smoke suite covers:
| Spec | What it tests |
|---|---|
| `transport-shim.spec.js` | BroadcastChannel transport stand-in (unit) |
| `rendezvous-join.spec.js` | Typed join, QR link, distinct refusals |
| `multi-client-crew.spec.js` | Four phones on one code, four seats |
| `server-load.spec.js` | WASM initialises without JS errors |
| `client-connect.spec.js` | Client page connects and receives Welcome |
| `lobby.spec.js` | Console selection broadcasts; only captain can start game |
| `sim-state.spec.js` | SimState broadcast; HelmInput changes ship position |

CI runs the smoke suite automatically on every push and pull request via `.github/workflows/ci.yml`.

### Production build

```bash
trunk build --release
trunk build --release --config client-trunk.toml
```

CI builds both pages on every push to `main` and deploys to GitHub Pages automatically.

---

## Project Structure

```
src/
  core/
    messages.rs       — wire types (ClientMessage, ServerMessage, Console, EntitySnapshot, FlagKind, etc.)
    codec.rs          — JSON serialization (only place serde_json is used)
    broadcast/        — Broadcaster, LobbyBroadcaster, SimBroadcaster, Cadence, Audience
  lobby/
    handler.rs        — pure lobby message handler (no Bevy, fully testable)
    server.rs         — Bevy plugin: lobby phase message routing, States<GamePhase>
    session.rs        — player lifecycle: tokens, consoles, reconnect, spectator queue
    stations_config.rs — pure station model: parse, validate, lookup
    stations_policy.rs — reassign-on-join/leave, spectator FIFO
    client_panel.rs   — pure client lobby state model (Bevy-free)
  ship/
    physics.rs        — pure Rust physics function (fully unit-tested)
    state.rs          — ShipState Bevy resource
    damage.rs         — hull integrity (f32) + shared apply_hull_damage helper
    impulse.rs        — pure impulse-drive charge state machine
  weapons/
    phaser.rs         — pure phaser bank state machine
    torpedo.rs        — pure torpedo + tube state machine
    shield.rs         — pure four-quadrant shield model
    beam_render.rs    — Bevy plugin: phaser beam rendering (server)
  modifiers/
    cache.rs          — pure cross-console multiplier table + flag set
    breakdown.rs      — breakdown queue + Shape assignment
    repair_teams.rs   — pure three-team repair dispatch model
    power_system.rs   — pure 6+2 power allocation, battery, exhaustion lock
    coordination.rs   — region modifier registration / removal
  asteroids/
    spawner.rs        — pure Rust per-cell density evaluation
    window.rs         — pure ring-buffer window: player-centred grid lifecycle
    lifecycle.rs      — Bevy systems: spawn/despawn cells as the player moves
  regions/
    server.rs         — Bevy plugin: region containment, Observer-driven entry/exit
    effects.rs        — region effect components
    shape.rs          — RegionShape types (Sphere, Box, Torus)
  entities/
    config.rs         — TOML entity config types
    map_config.rs     — legacy map-half parser (anchors, fields, entity instances) — PRD #337 merges with ScenarioConfig
    config_cache.rs   — Bevy plugin: preloads TOML configs via JS fetch on WASM
    tags.rs           — string-tag helpers for entity configs
    spawner.rs        — entity spawning from EntityConfig
    loader.rs         — world/entity loader
  world/
    server.rs         — WorldPlugin: world-file loading, entity lifecycle, region triggers
    content.rs        — ScenarioConfig (scenario-half types), WorldConfig (thin wrapper, PRD #337), WorldContentRuntime
  ai/
    server.rs         — AI Bevy plugin: patrol + NPC input injection
    core.rs           — pure AI state machine
    faction.rs        — faction config types
  console/
    captain/server.rs — CaptainPlugin: red alert, view selector, start game
    helm/joystick.rs  — pure joystick logic (Bevy-free)
    weapons/          — server.rs + client.rs: phaser/torpedo targeting
    repair/           — server.rs + client.rs: shape-matching repair dispatch
    power/            — server.rs + client.rs: power allocation UI
    science/          — server.rs + client.rs: sensors/shields/navigation
    comms/            — server inbox + client.rs: contacts, messages, objectives
  console_ai/
    server.rs         — Bevy plugin: automated AI for Low-complexity consoles
    core.rs           — pure AI console logic
    complexity.rs     — complexity preset loading
    delegation.rs     — three-tier delegation model
  server/
    bridge.rs         — wasm-bindgen exports (server feature)
    renderer.rs       — Bevy plugin: 2D lobby UI + 3D game camera
    viewscreen_border.rs — Bevy plugin: viewscreen bezel + red-alert vignette + HUD
    debug_overlay.rs  — Bevy plugin: developer overlay
  client/
    app.rs            — Bevy plugin: lobby panel + all console panels
    bridge.rs         — wasm-bindgen exports (client feature)
    elements.rs       — shared UI element helpers
    phone_border/     — Bevy plugin: phone bezel frame + helm/captain chrome
  sim_sets.rs         — SimSet enum (Input, Physics, Damage, Modifiers, Broadcast)
  ship_plugin.rs      — Bevy plugin: ship spawning + Rapier body setup
  server_app.rs       — server App builder: plugin registration + SimSet ordering
  objectives.rs       — pure ObjectiveManager (no Bevy)
  radar.rs            — pure radar projection math (server + client share)
  radar_config.rs     — pure radar viewport configs
  client_sim.rs       — pure client sim-state model (Bevy-free)
  client_comms.rs     — pure client Comms console state model (Bevy-free)
  client_complexity.rs — pure client complexity preset state (Bevy-free)

assets/
  worlds/*.toml                 — unified world files (default, patrol) — one TOML per session
  entities/*.toml               — asteroid, ship, region entity configs
  factions/*.toml               — AI faction definitions
  complexity/*.toml             — per-console complexity presets (Low / Full)

server.html           — host page: loads WASM, registers with the rendezvous
                        service, owns the per-token connection maps
client.html           — client page: pure HTML/JS, joins by typed code
worker-rendezvous/    — the rendezvous service (Worker + Durable Object)
Trunk.toml            — build config for server.html (server feature)
client-trunk.toml     — build config for client.html (client feature)
wiki/                 — LLM-maintained knowledge base (see wiki/SCHEMA.md)
docs/                 — design drafts (numbered)
```
