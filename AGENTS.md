# Project Phoenix — Bridge Simulator

## TL;DR

A browser-based tabletop spaceship bridge simulator. One browser tab shows a shared 3D view of space. Players join from phones by scanning a QR code — no installation. The host (view screen) runs Rust/Bevy compiled to WebAssembly and acts as the authoritative server. Phone clients are plain HTML/JS. Networking uses PeerJS (WebRTC) in a star topology.

**Two PRDs define the project:**
- **PRD #1:** [Project Phoenix — Browser-Based Bridge Simulator](https://github.com/jkeywo/project-phoenix-v2/issues/1) — PoC: lobby, captain's chair, red alert, rotating cube
- **PRD #22:** [Helm and Game World](https://github.com/jkeywo/project-phoenix-v2/issues/22) — Ship physics, asteroids, helm console with thrust/steering

**Current state:** Both PRDs are fully implemented. The game has a lobby with player management, two consoles (Captain + Helm), a physics-simulated ship, randomized asteroid field, and Red Alert.

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
mkdir -p dist/client && cp client.html dist/client/index.html
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
- `client-trunk.toml` — builds `client.html` (no WASM, just static HTML/JS)

### File Layout

```
src/
  messages.rs       — Pure data types (no logic). Console, Player, GameState, ClientMessage, ServerMessage
  codec.rs          — MessageCodec trait + JsonCodec impl. ONLY place serde_json is used directly.
  session.rs        — SessionManager: tokens → players, console assignment, reconnect/vacancy logic
  lobby.rs          — Bevy plugin: processes lobby-phase messages, phase transitions
  simulation.rs     — Bevy plugin: helm input → physics → ship state, asteroid spawning, collision
  ship_physics.rs   — Pure Rust physics controller (no Bevy). Input/output function, fully testable.
  ship_state.rs     — ShipState resource: position, yaw, speed, red_alert toggle
  asteroid_spawner.rs — Pure Rust asteroid position generator (seeded, deterministic)
  renderer.rs       — Bevy plugin: lobby UI, 3D camera, Red Alert border overlay
  bridge.rs         — wasm-bindgen exports. ONLY for WASM target.
  lib.rs            — Module declarations

server.html         — Host page: loads WASM, runs Bevy, owns PeerJS host peer
client.html         — Client page: plain HTML/JS, connects to host via PeerJS peer ID from URL hash
Cargo.toml          — Single crate: cdylib (WASM) + rlib (tests)
Trunk.toml          — Build config for server.html
client-trunk.toml   — Build config for client.html
.github/workflows/  — CI: builds both pages, deploys to gh-pages
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

- **Captain:** toggles Red Alert via `ToggleRedAlert`
- **Helm:** sends `HelmInput { thrust, steering }` at 10Hz
- **Server simulation:**
  - Reads helm inputs tagged with `helm_token()`
  - Feeds into `compute_physics()` (pure function in `ship_physics.rs`)
  - Applies to ship's Rapier rigid body as direct velocity
  - Every 100ms, broadcasts `SimState { red_alert }` snapshot
- **Renderer:** 3D camera follows ship, Red Alert border shows on view screen

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
| `lobby.rs` | Bevy plugin: lobby message routing | messages, session | Yes |
| `simulation.rs` | Bevy plugin: physics integration | messages, session, ship_physics, ship_state, asteroid_spawner | Yes |
| `ship_physics.rs` | Pure physics: inputs → new state | None (pure Rust) | No |
| `ship_state.rs` | ShipState resource | messages | Yes |
| `asteroid_spawner.rs` | Pure asteroid position generator | rand | No |
| `renderer.rs` | Bevy plugin: 2D lobby + 3D game view | messages, session, ship_state | Yes |
| `bridge.rs` | wasm-bindgen exports | codec, lobby, renderer, simulation | WASM only |

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

- **CaptainChair:** Red Alert toggle (exclusive). Only captain can `StartGame` and `ToggleRedAlert`.
- **Helm:** Thrust slider + steering joystick/sliders. Sends `HelmInput` at 10Hz while active. Ship only moves when Helm is occupied.

---

## Testing Strategy

### Rust unit tests (`cargo test`)

Tests live inline with modules (`#[cfg(test)] mod tests`).

- **`session.rs`** — Player registration, duplicate tokens, console assignment/clearing, disconnect vacancy, reconnect auto-assign, `helm_token()` / `captain_token()` lookups, conflict resolution
- **`codec.rs`** — Round-trip serialization for every `ClientMessage` and `ServerMessage` variant
- **`lobby.rs`** — Bevy App harness: Identify → Welcome, console select → broadcast, captain only can start, HelmInput ignored in lobby, disconnect handling
- **`ship_physics.rs`** — Zero input, thrust curve, deceleration curve, steering yaw, diagonal motion, dt scaling, speed cap
- **`asteroid_spawner.rs`** — Exact count, within bounds, clear zone, no duplicates
- **`ship_state.rs`** — Red alert toggle, snapshot generation

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
- `dist/client/index.html` is the plain `client.html` copy (no client WASM built in CI). JS message handling works; Bevy UI overlay does not. Sufficient for smoke testing.
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
2. **`bridge.rs` is WASM-only.** Gated behind `#[cfg(target_arch = "wasm32")]`.
3. **Captain authority.** Only the player at `CaptainChair` can `StartGame` and `ToggleRedAlert`. The server enforces this.
4. **Console vacancy on disconnect.** Immediately — in all game phases.
5. **Helm sends at 10Hz.** Simulation reads helm inputs at 10Hz tick intervals.
6. **Deterministic asteroids.** Seeded generator, fixed per session.
7. **WebGL2 rendering.** For broad browser support.
8. **PeerJS cloud broker.** Not self-hosted (deferred post-PoC).

---

## Cargo.toml Notes

```toml
[lib]
crate-type = ["cdylib", "rlib"]  # cdylib for WASM, rlib for testing

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

The client is pure HTML/JS — no WASM, no framework. Key patterns:

- `localStorage` for session token and player name persistence
- Reads host peer ID from `location.hash.slice(1)`
- PeerJS `clientPeer.connect(hostPeerId, { reliable: true })` for DataConnection
- Sends JSON: `{ "type": "<MessageType>", "data": { ... } }`
- `handleMessage()` dispatches on `msg.type`, mutates local `state`, calls `render(state)`
- `render(state)` toggles sections: `lobby-ui`, `game-ui`, `helm-ui` with CSS class `active`

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
3. Handle in `lobby.rs` (process or pass through based on game phase)
4. Handle in `simulation.rs` if in-game message
5. Handle in `client.html` `handleMessage()` and update `render()` if UI changes
6. Handle in `server.html` JS if routing needs adjustment
