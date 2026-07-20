---
title: Architecture
type: concept
tags: [architecture, layers, server, client, wasm]
sources: [AGENTS.md, src/lib.rs]
updated: 2026-05-15
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
- **`client.html`** loads the *same* crate with feature `["client"]`. The client runs Bevy/WASM for UI; no Rapier physics.
- `src/lib.rs` declares all top-level modules. The codebase is migrating from a flat layout into **domain-grouped subdirectories** (see below).

## Target module tree

Modules are organised by **domain**, not by audience (server vs client) or layer. Pure logic sits beside its Bevy plugin inside the same domain folder.

```
src/
├── core/           messages.rs (wire types incl. FlagKind), codec.rs, broadcast seam
├── lobby/          lobby.rs, lobby_handler.rs, stations*.rs, client_panel.rs, session.rs
├── ship/           ship_state.rs, ship_physics.rs, impulse.rs, damage.rs
├── weapons/        phaser.rs, torpedo.rs, shield.rs, beam_render.rs
├── regions/        region_*.rs, region_plugin.rs → server.rs
├── world/          content.rs (scenario types), server.rs (WorldPlugin) ← merged #218–222
├── console/
│   ├── captain/    client.rs, server.rs
│   ├── helm/       client.rs, server.rs, joystick.rs
│   ├── weapons/    client.rs, server.rs
│   ├── repair/     client.rs, server.rs
│   ├── power/      client.rs, server.rs
│   ├── science/    client.rs, server.rs
│   └── comms/      client.rs, server.rs
├── console_ai/     console_ai.rs, console_ai_plugin.rs, complexity.rs, delegation.rs
├── ai/             ai.rs, ai_plugin.rs, faction.rs
├── asteroids/      asteroid_spawner.rs, asteroid_window.rs, asteroid_lifecycle.rs
├── entities/       entity_*.rs, map_config.rs, config_cache.rs, entity_tags.rs
├── modifiers/      cache.rs, coordination.rs, power_system.rs, repair_teams.rs, breakdown.rs
├── server/         bridge.rs, renderer.rs, viewscreen_border.rs, debug_overlay.rs
└── client/         app.rs, bridge.rs, elements.rs, phone_border/
```

**Design rules:**
- Domain-grouped, not audience-grouped or layer-grouped.
- Pure modules co-located with Bevy plugins inside each domain folder.
- Lobby is top-level (it is a phase, not a console).
- `src/world/` is complete (landed via #218–222).
- All other domains are being migrated slice-by-slice via issues #229 → #256.

## Current module map (transitional)

While migration is in progress, modules live at their old flat paths alongside the new `src/world/` domain. `pub use` re-exports in each domain's `mod.rs` keep external paths stable.

| Naming pattern | Role |
|---|---|
| `messages.rs` | Pure data types — `ClientMessage`, `ServerMessage`, `Console`, `GamePhase`, `SimSnapshot`. |
| `codec.rs` | The **only** place `serde_json` is used. Implements `MessageCodec` trait. See [Codec Seam](./codec-seam.md). |
| `radar.rs`, `radar_config.rs` | Pure radar projection, reused by server renderer and Helm/Weapons consoles. |
| `session.rs`, `stations.rs` | Server identity + station assignment. |
| `lobby.rs` (plugin) + `lobby_handler.rs` (pure) | Lobby-phase Bevy plugin + pure handler functions. |
| `server_app.rs` | Composition root: `add_simulation_plugins(&mut App)` wires all per-table plugins (CaptainPlugin, ShipPlugin, WeaponsPlugin, RepairPlugin, PowerPlugin, SciencePlugin) plus core resources, broadcaster registrations, and shared systems (collision, shields, world setup, entity reconciliation). Replaces the former `simulation.rs` god-module. |
| `ship_physics.rs`, `ship_state.rs`, `impulse.rs` | Pure physics + Bevy resource. |
| `phaser.rs`, `torpedo.rs`, `shield.rs` | Pure weapon/defence state machines. |
| `damage.rs`, `breakdown.rs`, `repair_teams.rs` | Pure damage formula + breakdown queue + repair dispatch. |
| `power_system.rs`, `modifiers.rs` | Pure 6+2 power model + modifier cache + typed flags (`FlagKind` lives in `core/messages.rs`). |
| `asteroid_spawner.rs`, `asteroid_window.rs`, `asteroid_lifecycle.rs` | Pure density + ring-buffer window + Bevy lifecycle systems. |
| `region_*.rs`, `region_plugin.rs` | Region effects (damage zones, slow zones, jammers). |
| `entity_config.rs`, `entity_loader.rs`, `entity_spawner.rs`, `entity_override.rs`, `entity_tags.rs`, `map_config.rs`, `config_cache.rs` | Data-driven entity pipeline (PRD #153). |
| `world/content.rs`, `objectives.rs`, `comms_inbox.rs` | Scenario engine (PRD #119). Merged into `src/world/` via #218–222. |
| `ai.rs`, `ai_plugin.rs`, `faction.rs` | NPC state machines (PRD #142). |
| `console_ai.rs`, `console_ai_plugin.rs`, `complexity.rs`, `delegation.rs` | Server-side AI for hidden console controls (PRD #154). |
| `renderer.rs`, `beam_render.rs`, `viewscreen_border.rs`, `debug_overlay.rs` | Server-side Bevy rendering. |
| `client_app.rs`, `client_lobby.rs`, `client_sim.rs`, `client_helm.rs`, `client_comms.rs`, `client_complexity.rs`, `client_elements.rs` | Client-side Bevy app + per-console state. |
| `comms_plugin.rs` | Client-side comms console plugin. |
| `bridge.rs` (server) · `client_bridge.rs` (client) | `wasm-bindgen` exports. |

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

## Console delivery

The authoritative half of each console is a server-side Bevy plugin under `src/console/<name>/`. Phone interfaces are pure HTML/JavaScript and compose reusable web components; see [Client Architecture](./client-architecture.md) and [Console UI Authoring Library](./console-ui-library.md).

## Pure-function seams

Several modules are deliberately framework-free so they can be unit-tested without Bevy:

- `ship_physics::compute_physics` — input → output.
- `asteroid_spawner` — seed → positions.
- `lobby_handler` — `(state, message) -> LobbyHandlerResult`.
- `radar::radar_dots` — pure iterator.
- `codec::JsonCodec` — encode/decode round-trip.
- `world::content` — scenario parser, trigger evaluator, comms template engine.

This keeps the test pyramid wide. See [Testing Strategy](./testing-strategy.md).

### Modifier coordinator

The **modifier coordinator** (`src/modifiers/coordination.rs`) is the single
owner of the `ShipModifiers` resource lifecycle. `ModifierCoordinationPlugin`
is the sole call site for `init_resource::<ShipModifiers>()`. Translator systems
for each modifier source (power at `translate_power_modifiers:43`, regions at
`translate_region_modifiers:194`, impulse at `translate_impulse_modifiers:174`)
read source state and write through pure helpers into `ShipModifiers`. Consumers
read `Res<ShipModifiers>` only. See [Modifier Coordination](./modifier-coordination.md).

### Broadcaster seam

The **broadcaster seam** (`src/core/broadcast/`) replaces hand-written broadcast systems with a `register(audience, cadence, producer)` API. Two plugins (`SimBroadcaster` for `InProgress`, `LobbyBroadcaster` for `Lobby`) gate on `GamePhase`, resolve `Audience` → `Target`, and call producers. Producers are `Fn(&mut World) -> Vec<ServerMessage>` closures that know nothing about routing or timing. See [Broadcaster Seam](./broadcaster-seam.md) for the full catalogue, recipe, and contract.

## Related

- [Networking](./networking.md) · [Message Flow](./message-flow.md)
- [Build & Deployment](./build-and-deployment.md)
- [WorldPlugin](./world-plugin.md) — first landed domain module.
