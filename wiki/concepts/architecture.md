---
title: Architecture
type: concept
tags: [architecture, layers, server, client, wasm]
sources: [AGENTS.md, src/lib.rs, src/server/, src/client/, src/shared/]
updated: 2026-05-08
---

# Architecture

Project Phoenix ships as **two HTML pages** built from **one Rust crate**, talking over **WebRTC**.

```
        ┌────────────────────────────┐
        │  server.html (view screen) │   authoritative
        │  Bevy + Rapier in WASM     │   PeerJS host peer
        └─────────────┬──────────────┘
                      │ WebRTC (PeerJS)
              ┌───────┴───────┐
              ▼               ▼
       client.html      client.html
       (phone #1)       (phone #2)  …
```

## Two pages, one crate

- **`server.html`** loads the WASM binary built with `cargo` features `["server"]`. Trunk drives the build (`Trunk.toml`).
- **`client.html`** loads the *same* crate with feature `["client"]`. The client now also runs Bevy/WASM (post-PRD #66 prep), but its UI is much smaller and there's no Rapier.
- `src/lib.rs` declares modules grouped under `server/`, `client/`, and `shared/`. Bridge modules are gated by feature flags.

## Module map

| Path | Role |
|---|---|
| `src/shared/messages.rs` | Pure data types — `ClientMessage`, `ServerMessage`, `Console`, `GamePhase`, `SimSnapshot`, `WorldData`. |
| `src/shared/codec.rs` | The **only** place `serde_json` is used. Implements `MessageCodec` trait. See [Codec Seam](./codec-seam.md). |
| `src/shared/radar.rs` | `radar_dots()` — pure iterator projecting asteroids onto the radar plane. Reused by server renderer and Helm console. |
| `src/server/session.rs` | `SessionManager` — token → player record, console assignment, reconnect. |
| `src/server/lobby.rs` | Bevy plugin: lobby-phase message routing. |
| `src/server/lobby_handler.rs` | Pure handler functions producing `LobbyHandlerResult`. |
| `src/server/simulation.rs` | Bevy plugin: helm input → physics → ship state, collisions. |
| `src/server/ship_physics.rs` | Pure controller: `compute_physics(state, input, dt, config) -> result`. |
| `src/server/ship_state.rs` | `ShipState` Bevy resource. |
| `src/server/asteroid_spawner.rs` | Deterministic seeded asteroid placement. |
| `src/server/renderer.rs` | Bevy plugin: 2D lobby UI + 3D game camera + Red Alert overlay. |
| `src/server/bridge.rs` | `wasm-bindgen` exports — `wasm_init`, `wasm_receive_message`, `set_message_callback`, `wasm_player_disconnected`. |
| `src/client/app.rs` | Client Bevy app entry (lobby + console plugins). |
| `src/client/lobby_plugin.rs` · `lobby_state.rs` | Lobby UI + `LobbyView` view-model. |
| `src/client/captain_plugin.rs` | Captain's Chair console UI. |
| `src/client/helm_plugin.rs` · `helm_state.rs` | Helm UI + radar projection. |
| `src/client/sim_state.rs` | Mirror of `SimSnapshot` for client rendering. |
| `src/client/client_bridge.rs` | Client-side `wasm-bindgen` exports. |

## Where state lives

| State | Owner | Lifecycle |
|---|---|---|
| Player identity (token) | Phone `localStorage` | Forever, per device |
| Player record | Server `SessionManager` | Until `SessionManager` is dropped (page refresh on view screen) |
| Game phase | Server | Lobby → InProgress |
| World data | Server `GameState` + Rapier entities | Set on `StartGame`, broadcast in `WorldSetup`/`Welcome` |
| Ship state | Server `ShipState` resource + Rapier rigid body | Mutated each tick |
| View Mode | Server `ShipState` (broadcast in `SimSnapshot`) | Captain-controlled |
| PeerJS connection | JS only | WebRTC ephemeral |

The Rust/WASM layer **never touches sockets**. All networking is JavaScript.

## Plugin pattern

Every console is a **Bevy plugin** owning its UI, marker components, setup systems, and event handlers. See [Console Plugin Pattern](./console-plugin-pattern.md). Adding a console = adding one plugin.

## Pure-function seams

Several modules are deliberately framework-free so they can be unit-tested without Bevy:

- `ship_physics::compute_physics` — input → output.
- `asteroid_spawner` — seed → positions.
- `lobby_handler` — `(state, message) -> LobbyHandlerResult`.
- `radar::radar_dots` — pure iterator.
- `codec::JsonCodec` — encode/decode round-trip.

This keeps the test pyramid wide. See [Testing Strategy](./testing-strategy.md).

## Related

- [Networking](./networking.md) · [Message Flow](./message-flow.md)
- [Build & Deployment](./build-and-deployment.md)
