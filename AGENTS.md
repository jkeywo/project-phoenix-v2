# Project Phoenix — Bridge Simulator

## TL;DR

A browser-based spaceship bridge simulator. One browser tab shows a shared 3D view of space. Players join from phones by scanning a QR code — no installation. Both the host (view screen) and the client (phone console) run Rust/Bevy compiled to WebAssembly. The host acts as the authoritative server; clients send inputs and receive state snapshots. Networking uses PeerJS (WebRTC) in a star topology.

**Open PRDs (planned work):**
- **PRD #116:** [Save/Load Game Sessions](https://github.com/jkeywo/project-phoenix-v2/issues/116) — `localStorage`-backed save slots, periodic + lifecycle saves, version-gated load. Introduces `save.rs` (the *second* sanctioned `serde_json` surface).

**Current state:** Nine player-facing consoles (`CaptainChair`, `Helm`, `Tactical`, `Repair`, `Sensors`, `Shields`, `Navigation`, `Power`, `Comms`) plus `Core` as an ownerless repair target. Players claim fixed *stations* from `player_ship.toml`; `Player.station: Option<StationId>` is the authoritative ownership field, and held consoles are derived from the station + `ShipConfig`. Full simulation: ship physics loaded from TOML, grid-based streaming asteroid field, phaser banks, torpedoes, four-quadrant shields, impulse drive, per-console hull damage, three-team dispatch repair, 6+2 power allocation, region effects, station ratings with Backfill AI, TOML-driven world engine with objectives and NPC AI patrols. See **[wiki/](./wiki/)** for the deeper map of the codebase.

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

# Local dev — client page (pure HTML/JS, no WASM; connects to server)
node scripts/build-client.mjs                  # → dist/client/, then serve dist/ statically

# Production build
trunk build --release                          # server page (WASM)
node scripts/build-client.mjs                  # client page (pure JS, file copy → dist/client/)

# Smoke tests (Playwright, Chromium) — requires dist/ built first
trunk build --release
node scripts/build-client.mjs
cd tests/smoke && npm install && npx playwright install chromium
npx playwright test                            # from tests/smoke/

# CI: ci.yml — unit tests → WASM build → Playwright smoke tests → deploy (on main)
```

---

## Message Flow (The Core Loop)

```
Player phone (client.html)
  ↓  sends WebRTC message with JSON
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
  ↓  handleMessage() → wasm_client_receive(json)
```

---

## File Layout

```
src/
  core/         - Wire types, codec, flag_kind, broadcaster; `Player` carries `station` / `last_rating`
  lobby/        — Session management, station assignment, lobby handler (pure + Bevy)
  ship/         — Physics, state, damage, impulse (mostly pure)
  weapons/      — Phaser, torpedo, shield state machines + beam renderer
  modifiers/    — Modifier cache, breakdown queue, repair teams, power system
  asteroids/    — Deterministic density spawner, ring-buffer window, Bevy lifecycle
  regions/      — Region containment, effect components, shape types
  entities/     — TOML entity config types, config cache (JS fetch), spawner, loader
  world/        — WorldPlugin, parse_world, runtime trigger/comms evaluators
  ai/           — NPC patrol loop, pure AI state machine, faction configs
  console/      — Per-console server + client plugins (captain, helm, weapons, repair, power, science, comms)
  console_ai/   — Server-side AI for Low-complexity consoles
  server/       — wasm-bindgen exports, renderer, viewscreen border, debug overlay
  client/       — wasm-bindgen exports, lobby + console panels, phone border
  sim_sets.rs   — SimSet enum: Input → Physics → Damage → Modifiers → Broadcast
  server_app.rs — Server App builder: plugin registration + SimSet chain ordering

assets/
  worlds/       — TOML world files (anchors, [[entity]], [[trigger]], [[comms]])
  entities/     — TOML entity configs (player ship, asteroids, pirates, regions)
  factions/     — AI faction definitions
  complexity/   — Per-console complexity presets (Low / Full) + AI tuning

server.html       — Host page: loads server WASM, runs Bevy, owns PeerJS host peer
client.html       — Client page: pure HTML/JS (no WASM), connects via PeerJS peer ID in URL hash
Cargo.toml        — Single crate: cdylib (WASM) + rlib (tests). Feature: server
Trunk.toml        — Build config for server.html
scripts/build-client.mjs — Builds the pure-JS client page (file copy → dist/client/)
wiki/             — LLM-maintained knowledge base. Read SCHEMA.md first; update as you work.
docs/             — Draft design notes (numbered).
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
      ┌─────┘     └─────┐
      ▼                 ▼
 client.html       client.html
 (phone #1)        (phone #2)
              ...
```

- **Server = authority.** Bevy runs the simulation, owns session state, decides everything.
- **Clients = stateless spokes.** They send input, receive state snapshots. Clients never talk to each other.
- **Session tokens** (UUIDv4 in `localStorage`) are the identity system — not peer IDs. Tokens survive refreshes; peer IDs are ephemeral.

### Serialization — The Codec Contract

`serde_json` must **never** be called directly outside `src/core/codec.rs`. The `MessageCodec` trait is the only serialization surface. This exists so the wire format can be swapped to binary (MessagePack, etc.) by changing one module.

---

## Game Flow

1. **Lobby:** Players scan QR -> join via `client.html#<peerId>` -> pick a station -> toggle `SetReady`. When all connected players are ready, the server enters `Loading` while assets preload or goes straight to `InProgress`; the legacy start message is gone.
2. **In-Progress:** Each station-owned console sends inputs; server simulation ticks at 10Hz (helm, weapons, repair, power, sensors, shields, navigation, comms). Server broadcasts `SimState` every 100ms with hull, power, flags, entity states. Region containment, asteroid streaming, NPC patrols, and world triggers all run server-side.
3. **Disconnect/Reconnect:** The dropped player's `Player.station` remains on their session record and the station rating flips to `Backfill` so AI runs its systems. The session token (in `localStorage`) is the identity; on browser refresh the client re-sends `Identify`. If no connected player claimed the old station, the server restores the station and `last_rating`; otherwise the player reconnects without a station / as a spectator. Reconnect is handled in every phase.

See `wiki/concepts/game-loop.md` and `wiki/entities/console.md` for per-console details.

---

## Testing Strategy

**Rust unit tests (`cargo test`):** Tests live inline (`#[cfg(test)] mod tests`). Cover pure modules: session, stations, codec round-trips, lobby handler, physics, asteroid spawner/window, damage, breakdown, repair teams, modifier cache, power system, client panels, joystick.

**Smoke tests (`tests/smoke/`, Playwright + Chromium):** Boot real server WASM in headless Chromium with a `BroadcastChannel`-backed PeerJS shim (no real WebRTC). Cover: server load, client connect, lobby, stations, sim-state, world bootstrap, AI patrol, engineering, comms.

**Not tested:** Renderer (visual output), bridge internals, CI pipeline.

> Good tests: set up state → perform action → assert on observable output through the public interface. Do NOT assert on private fields, internal call counts, or implementation-specific details.

See `wiki/concepts/testing-strategy.md` for the full file-by-file breakdown.

---

## Key Constraints & Rules

1. **`serde_json` only in `codec.rs`.** Never import it directly in other modules.
2. **Feature gates for bridges.** `server/bridge.rs` is compiled under the `server` feature; `client/bridge.rs` under the `client` feature. Neither is gated by `cfg(target_arch)` alone.
3. **Captain authority.** Only the player at `CaptainChair` can `ToggleRedAlert`. Game start is collective `SetReady` auto-start, not a captain-only command.
4. **Backfill on disconnect.** A disconnected station holder stays associated with their `StationId`; the station flips to the `Backfill` rating until reconnect or a new claim.
5. **Helm sends at 10Hz.** Simulation reads helm inputs at 10Hz tick intervals.
6. **Deterministic asteroids.** Per-cell density is seeded from `(field_idx, gx, gz) + Perlin noise`. Destroyed asteroids respawn fresh when the player leaves the cell and returns (no persistent destroyed-set).
7. **WebGL2 rendering.** For broad browser support.
8. **PeerJS cloud broker.** Not self-hosted (deferred post-PoC).
9. **Pure modules are Bevy-free.** `lobby/handler`, `radar`, `ship/damage`, `modifiers/breakdown`, `lobby/client_panel`, `client_sim`, `client_comms`, `console/helm/joystick` have no Bevy imports — fully unit-testable on native, shared between server and client.
10. **Station ownership is authoritative.** `Player.station: Option<StationId>` is the ownership field; console tabs and authorization derive held consoles from the station and `ShipConfig`.
11. **No hardcoded gameplay values.** All gameplay data (stats, icons, colours, sizes, behaviours) comes from TOML config, loaded into entities/components and sent over the network where the client needs it. The only acceptable hardcoded values are: (a) defaults applied while parsing a TOML file (`unwrap_or(...)`-style fallbacks), and (b) client-side placeholders shown while waiting for authoritative data from the server. If a value could plausibly be tuned by a designer, it belongs in TOML — never inline it "for now", and never add a hardcoded branch that can override what the config says.

---

## Cargo.toml Notes

```toml
[lib]
crate-type = ["cdylib", "rlib"]  # cdylib for WASM, rlib for testing

[features]
default = ["server"]
server = []   # host build → server.html (bridge.rs compiled in)
# The client page (client.html) is now pure JS (gui/*.js) — there is no
# `client` cargo feature and no client-side WASM (removed in #463).

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
