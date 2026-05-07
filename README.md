# Project Phoenix — Bridge Simulator

A browser-based tabletop spaceship bridge simulator for groups. One browser tab on a shared screen shows a 3D view of space. Players join from their phones by scanning a QR code — no app install required.

**[Play it live →](https://jkeywo.github.io/project-phoenix-v2/)**

---

## How to Play

1. Open the **[view screen](https://jkeywo.github.io/project-phoenix-v2/)** in a browser tab on a shared monitor or TV.
2. Players scan the QR code on screen with their phones and open the link.
3. Each player sets a name and claims a console (Captain's Chair or Helm).
4. The captain presses **Engage** to start the game.
5. Navigate the ship through the asteroid field without hitting anything.

### Consoles

| Console | Controls |
|---|---|
| **Captain's Chair** | Toggle Red Alert |
| **Helm** | Joystick — up/down = thrust/reverse, left/right = steering |

---

## Tech Stack

| Layer | Tech |
|---|---|
| Game engine | [Bevy](https://bevyengine.org/) 0.18 (Rust) |
| Physics | [bevy_rapier3d](https://github.com/dimforge/bevy_rapier) 0.33 |
| Networking | [PeerJS](https://peerjs.com/) (WebRTC, no server needed) |
| Build | [Trunk](https://trunkrs.dev/) (Rust → WASM) |
| Hosting | GitHub Pages |

The host page (`server.html`) compiles to WebAssembly and runs the authoritative game simulation in the browser. Phone clients are plain HTML/JS — no framework, no install.

---

## Architecture

```
        ┌─────────────────────┐
        │  server.html (WASM) │  ← authoritative simulation
        │  Bevy + Rapier3D    │     PeerJS host peer
        └────────┬────────────┘
                 │  WebRTC (PeerJS)
        ┌────────┴────────┐
        ▼                 ▼
  client.html       client.html
  (phone #1)        (phone #2)  ...
```

- **Star topology** — clients never talk to each other, only to the host.
- **Session tokens** — UUID stored in `localStorage`, survives page refresh. Same token = same player, auto-reconnect restores console assignment.
- **No backend** — PeerJS uses a public broker for the initial WebRTC handshake; all game data flows peer-to-peer.

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

### Tests

```bash
cargo test
```

Tests cover session management, serialization round-trips, lobby message routing, ship physics, and asteroid spawning. The renderer, WASM bridge, and HTML pages are tested manually.

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
  messages.rs         — wire types (ClientMessage, ServerMessage, Console)
  codec.rs            — JSON serialization (only place serde_json is used)
  session.rs          — player lifecycle: tokens, consoles, reconnect
  lobby.rs            — Bevy plugin: lobby phase message routing
  simulation.rs       — Bevy plugin: physics, helm input, collisions
  ship_physics.rs     — pure Rust physics function (fully unit-tested)
  ship_state.rs       — ShipState Bevy resource
  asteroid_spawner.rs — deterministic seeded asteroid placement
  renderer.rs         — Bevy plugin: 2D lobby UI + 3D game camera
  bridge.rs           — wasm-bindgen exports (WASM target only)

server.html           — host page: loads WASM, owns PeerJS host peer
client.html           — client page: phone UI, plain HTML/JS
Trunk.toml            — build config for server.html
client-trunk.toml     — build config for client.html
```
