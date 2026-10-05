---
title: Architecture
type: concept
tags: [architecture, server, client, wasm, authority, domains]
sources: [crates/phoenix-presentation/src/server/pfx.rs, AGENTS.md, crates/phoenix-simulation/src/lib.rs, crates/phoenix-simulation/src/server_app/mod.rs, crates/phoenix-simulation/src/server_app/registration.rs, src/server/bridge.rs, src/server/browser_edge.rs, crates/phoenix-simulation/src/lockstep/mod.rs, crates/phoenix-simulation/src/entities/config.rs, crates/phoenix-simulation/src/entities/config/, server.html, client.html, wiki/concepts/client-architecture.md]
updated: 2026-10-05
---

# Architecture

Project Phoenix is an authoritative Rust/Bevy simulation hosted by `server.html`, with pure HTML/CSS/JavaScript phone clients connected through the Phoenix transport — a project-owned rendezvous service for typed join codes and WebRTC signalling, then direct DataChannels — in a star topology.

```text
phone clients
  → intent over WebRTC
server.html + Rust/WASM simulation
  → authoritative snapshots/events over WebRTC
phone clients
  → stateless console rendering
```

## Runtime boundaries

- `server.html` loads the Rust/WASM host, registers with the rendezvous service, owns the per-token connections, and displays the shared viewscreen.
- `client.html` and `gui/` are pure JavaScript; there is no client-side Rust or WASM.
- `src/server/bridge.rs` exports the JavaScript/WASM boundary and drains browser inputs into Bevy. Its private `browser_edge.rs` adapter owns pre-app storage, bounded queues, callbacks and readback mirrors behind typed operations. Callbacks are cloned before invocation so synchronous JavaScript replacement cannot retain a storage borrow. Fleet slot-claim sequencing is a `SlotClaimSequence` Resource in `crates/phoenix-simulation/src/lockstep/mod.rs`. Browser sockets remain in JavaScript.
- `crates/phoenix-simulation/src/server_app/registration.rs` composes the fixed-tick simulation; `crates/phoenix-simulation/src/server_app/mod.rs` is its stable facade.

## Packages

[Reusable Layers](./reusable-layers.md) maps each package to its owner and focused checks. Reusable runtime, transport and platform packages contain no Phoenix game dependencies. The root crate composes hosts; simulation and presentation have separate Cargo boundaries.

## Domain layout

Rust modules are grouped by domain: `lobby`, `ship`, `weapons`, `modifiers`, `asteroids`, `regions`, `entities`, `world`, `ai`, `comms`, and `console`. Pure state/decision code stays beside its Bevy adapter; a pure module never imports Bevy merely to serve an adapter.

`crates/phoenix-simulation/src/entities/config.rs` owns `EntityConfig`, parsing and cross-subsystem validation. Its `config/` leaves hold the individual subsystem schemas; root re-exports preserve their public paths. Visual and LOD definitions live in `crates/phoenix-model/src/entity/visual.rs`, with hull, propulsion, weapons, consoles and other subsystem declarations in corresponding leaves.

Cross-domain infrastructure has narrow homes:

- `crates/phoenix-model/src/messages.rs` owns the wire vocabulary; `crates/phoenix-simulation/src/core/codec.rs` owns the game JSON seam;
- `crates/phoenix-simulation/src/core/broadcast/` owns outbound audience/cadence dispatch;
- `crates/phoenix-simulation/src/command_admission/` owns token/system authority before commands reach a domain;
- `src/server_app/` owns composition, cross-domain publication, world setup, and collision;
- `crates/phoenix-simulation/src/sim_sets.rs` owns the logical order `Input → Physics → Damage → Modifiers → Publish → PublishAggregate → Broadcast`.

## State and authority

Session tokens identify players; rendezvous peer ids and DataChannels identify transient transports. The server owns session tenure, game phase, world/runtime content, ship state, objectives, and outcomes. A phone stores only the latest projected state needed to render its consoles.

Human and AI actors submit the same `ControlSystem` commands. Admission records authority once; domain appliers never branch on actor type. Every gameplay decision advances on the authored logical tick, not on rendered frames.

## Host presentation effects

`crates/phoenix-presentation/src/server/pfx.rs` builds transient flashes, rings, plasma and sparks through
a shared billboard-sprite constructor. Each effect retains its texture,
lifetime, scale, particle count and random-offset recipe. The constructor
assembles render and lifetime state; the existing lifetime and burst systems
fade, scale and expire it. These effects do not feed simulation state.

## Related

- [Server App Composition](./server-app.md)
- [Client Architecture](./client-architecture.md)
- [Message Flow](./message-flow.md)
- [Networking](./networking.md)

Rust unit-test modules live in sibling `*_tests.rs` files (or `tests.rs` beside a `mod.rs`) and are loaded through test-gated path declarations. Their module names and private access remain unchanged. Binary test files live under `src/bin/tests/` to avoid Cargo target discovery. Production files retain test-only hooks when the production type needs them; unit-test bodies and fixtures stay in the sibling files. The placement convention is maintained in `AGENTS.md`.
