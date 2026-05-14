---
title: Architecture
type: concept
tags: [architecture, layers, server, client, wasm]
sources: [AGENTS.md, src/lib.rs]
updated: 2026-05-14
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
- `src/lib.rs` declares ~56 modules in a **flat layout** under `src/`. Audience and concern are encoded in module *names* (`client_*`, `*_plugin`) and `#[cfg(feature = "server" | "client")]` gates. A folder reorg into domain-grouped subdirectories (`ship/`, `weapons/`, `regions/`, `console/<name>/`, `lobby/`, `world/`, `core/`, `client/`, `server/`) is planned (see [Open Architectural Questions](../roadmap/open-architectural-questions.md)).

## Module map

The current flat layout groups modules by concern through naming:

| Naming pattern | Role |
|---|---|
| `messages.rs` | Pure data types — `ClientMessage`, `ServerMessage`, `Console`, `GamePhase`, `SimSnapshot`. |
| `codec.rs` | The **only** place `serde_json` is used. Implements `MessageCodec` trait. See [Codec Seam](./codec-seam.md). |
| `radar.rs`, `radar_config.rs` | Pure radar projection, reused by server renderer and Helm/Weapons consoles. |
| `session.rs`, `stations.rs` | Server identity + station assignment. |
| `lobby.rs` (plugin) + `lobby_handler.rs` (pure) | Lobby-phase Bevy plugin + pure handler functions. |
| `simulation.rs` | God-module Bevy plugin: helm input → physics → weapons → collision → breakdown → broadcast. Slated for split per the Plugin Pattern. |
| `ship_physics.rs`, `ship_state.rs`, `impulse.rs` | Pure physics + Bevy resource. |
| `phaser.rs`, `torpedo.rs`, `shield.rs` | Pure weapon/defence state machines. |
| `damage.rs`, `breakdown.rs`, `repair_teams.rs` | Pure damage formula + breakdown queue + repair dispatch. |
| `power_system.rs`, `modifiers.rs`, `flag_kind.rs` | Pure 6+2 power model + modifier cache + typed flags. |
| `asteroid_spawner.rs`, `asteroid_window.rs`, `asteroid_lifecycle.rs` | Pure density + ring-buffer window + Bevy lifecycle systems. |
| `region_*.rs`, `region_plugin.rs` | Region effects (damage zones, slow zones, jammers). |
| `entity_config.rs`, `entity_loader.rs`, `entity_spawner.rs`, `entity_override.rs`, `entity_tags.rs`, `map_config.rs`, `config_cache.rs` | Data-driven entity pipeline (PRD #153). |
| `scenario.rs`, `scenario_plugin.rs`, `objectives.rs`, `comms_inbox.rs` | Scenario engine (PRD #119, in flight). Planned to merge into a unified `world/` domain — see [#218](https://github.com/jkeywo/project-phoenix-v2/issues/218). |
| `ai.rs`, `ai_plugin.rs`, `faction.rs` | NPC state machines (PRD #142, partially landed via issues #175/#176/#177/#179). |
| `console_ai.rs`, `console_ai_plugin.rs`, `complexity.rs`, `delegation.rs` | Server-side AI that operates hidden console controls (PRD #154) + cross-console delegation allowlist. |
| `renderer.rs`, `beam_render.rs`, `viewscreen_border.rs`, `debug_overlay.rs` | Server-side Bevy rendering. |
| `client_app.rs`, `client_lobby.rs`, `client_sim.rs`, `client_helm.rs`, `client_comms.rs`, `client_complexity.rs`, `client_elements.rs` | Client-side Bevy app + per-console state. |
| `comms_plugin.rs`, `phone_border/` | Client-side console plugin shells + phone bezel chrome (PRD #187). |
| `bridge.rs` (server feature) · `client_bridge.rs` (client feature) | `wasm-bindgen` exports. |

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
