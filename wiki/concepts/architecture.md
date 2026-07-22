---
title: Architecture
type: concept
tags: [architecture, layers, server, client, wasm]
sources: [AGENTS.md, src/lib.rs, wiki/concepts/client-architecture.md]
updated: 2026-07-22
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
- **`client.html`** is **pure HTML/CSS/JS** — no client-side WASM, no Bevy. The Rust/Bevy client and its `src/client/` module were removed in PRD #438 / issues #442, #463. See [Client Architecture](./client-architecture.md).
- `src/lib.rs` declares all top-level modules. Modules are organised by **domain**, not by audience (server vs client) or layer.

## Module layout

```
src/
  core/         — Wire types (messages.rs, incl. FlagKind), codec, broadcast/ (Broadcaster seam)
  lobby/        — Session management, station assignment, lobby handler (pure + Bevy)
  ship/         — Physics, damage, power, shields, sensors, ratings, system registry, coordination (mostly pure)
  weapons/      — Phaser, torpedo state machines + beam renderer
  modifiers/    — Modifier cache, repair teams, coordination plugin
  asteroids/    — Deterministic density spawner + AsteroidWindow lifecycle
  regions/      — Region containment, effect components, shape types
  entities/     — TOML entity config types, config cache (JS fetch), spawner, loader
  world/        — WorldPlugin, parse_world, runtime trigger/comms evaluators
  ai/           — NPC AI plugins (same ControlSystem commands as players)
  comms/        — Comms range check + component
  console/      — Per-console SERVER plugins: captain, comms, helm, navigation, repair, weapons
  console_ai/   — Server-side AI controllers for systems under AI control
  server/       — wasm-bindgen exports, renderer, viewscreen border
  gui/          — Rust-side GenericRadar UI widget (server viewscreen)
  server_app.rs — Server App builder: plugin registration + SimSet chain ordering
  sim_sets.rs   — SimSet: Input → Physics → Damage → Modifiers → Publish → PublishAggregate → Broadcast

gui/            — CLIENT: pure JS modules + one HTML file per console (iframe),
                  mount-plan.js owns the station-id → DOM-id/URL mount plan
assets/         — TOML configs: worlds/, entities/, factions/; models, shaders, sounds
server.html     — Host page: loads server WASM, runs Bevy, owns PeerJS host peer
client.html     — Client page: pure HTML/JS, connects via PeerJS peer ID in URL hash
tests/client/   — Vitest tests for gui/*.js
```

**Design rules:**
- Domain-grouped, not audience-grouped or layer-grouped.
- Pure modules co-located with Bevy plugins inside each domain folder (see [Pure-function seams](#pure-function-seams)).
- Lobby is top-level (it is a phase, not a console).

## Where state lives

| State | Owner | Lifecycle |
|---|---|---|
| Player identity (token) | Phone `localStorage` | Forever, per device |
| Player record | Server `SessionManager` | Until `SessionManager` is dropped (page refresh on view screen) |
| Game phase | Server | Lobby → InProgress |
| World data | Server `GameState` + Bevy entities | Set on `StartGame`, broadcast in `WorldSetup`/`Welcome` |
| Ship state | Server `ShipState` resource + Bevy components | Mutated each tick |
| View Mode | Server `ShipState` (broadcast in `SimSnapshot`) | Captain-controlled |
| PeerJS connection | JS only | WebRTC ephemeral |

The Rust/WASM layer **never touches sockets**. All networking is JavaScript.

## Console delivery

The authoritative half of each console is a server-side Bevy plugin under `src/console/<name>/`. Phone interfaces are pure HTML/JavaScript and compose reusable web components; see [Client Architecture](./client-architecture.md) and [Console UI Authoring Library](./console-ui-library.md).

## Pure-function seams

Several modules are deliberately framework-free so they can be unit-tested without Bevy:

- `ship::physics::compute_physics` — input → output.
- `asteroids::spawner` — seed → positions.
- `lobby::handler` — `derive_game_state`, `process_disconnect`.
- `radar` — radar projection and `is_fire_ready_with_range`.
- `codec::JsonCodec` — encode/decode round-trip.
- `world::content` — scenario parser, trigger evaluator, comms template engine.

This keeps the test pyramid wide. See [Testing Strategy](./testing-strategy.md).

### Modifier coordinator

The **modifier coordinator** (`src/modifiers/coordination.rs:28`) is the single
owner of the `ShipModifiers` lifecycle. `ShipModifiers` is a per-entity
`Component` inserted at spawn time (by `entity_spawner`), not a global
`Resource`. The plugin uses ECS observers for region enter/exit and registers
translator systems for each modifier source (power at `translate_power_modifiers:48`,
impulse at `translate_impulse_modifiers:321`, region effects via
`apply_region_effects:242`) that read source state and write into
`ShipModifiers`. Consumers read `&ShipModifiers` via ECS queries. See
[Modifier Coordination](./modifier-coordination.md).

### Broadcaster seam

The **broadcaster seam** (`src/core/broadcast/`) replaces hand-written broadcast systems with a `register(audience, cadence, producer)` API. Two broadcaster instances (`SimBroadcaster` for `InProgress`, `LobbyBroadcaster` for `Lobby` — now `pub type` aliases of the generic `Broadcaster<M>`, #817) gate on `GamePhase`, resolve `Audience` → `Target`, and call producers. Producers are `Fn(&mut World) -> Vec<ServerMessage>` closures that know nothing about routing or timing. See [Broadcaster Seam](./broadcaster-seam.md) for the full catalogue, recipe, and contract.

## Related

- [Client Architecture](./client-architecture.md) — pure JS pipeline, `gui/*.js` inventory
- [Networking](./networking.md) · [Message Flow](./message-flow.md)
- [Build & Deployment](./build-and-deployment.md)
- [WorldPlugin](./world-plugin.md) — first landed domain module.
