---
title: Architecture
type: concept
tags: [architecture, server, client, wasm, authority, domains]
sources: [AGENTS.md, src/lib.rs, src/server_app/mod.rs, src/server_app/registration.rs, src/server/bridge.rs, src/server/browser_edge.rs, src/lockstep/mod.rs, src/entities/config.rs, src/entities/config/, server.html, client.html, wiki/concepts/client-architecture.md]
updated: 2026-09-08
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
- `src/server/bridge.rs` exports the JavaScript/WASM boundary and drains browser inputs into Bevy. Its private `browser_edge.rs` adapter owns pre-app storage, bounded queues, callbacks and readback mirrors behind typed operations. Callbacks are cloned before invocation so synchronous JavaScript replacement cannot retain a storage borrow. Fleet slot-claim sequencing is a `SlotClaimSequence` Resource in `src/lockstep/mod.rs`. Browser sockets remain in JavaScript.
- `src/server_app/registration.rs` composes the fixed-tick simulation; `src/server_app/mod.rs` is its stable facade.

## Domain layout

Rust modules are grouped by domain: `lobby`, `ship`, `weapons`, `modifiers`, `asteroids`, `regions`, `entities`, `world`, `ai`, `comms`, and `console`. Pure state/decision code stays beside its Bevy adapter; a pure module never imports Bevy merely to serve an adapter.

`src/entities/config.rs` owns `EntityConfig`, parsing and cross-subsystem validation. Its `config/` leaves hold the individual subsystem schemas; root re-exports preserve their public paths. Visual and LOD definitions live in `config/visual.rs`, with hull, propulsion, weapons, consoles and other subsystem declarations in corresponding leaves.

Cross-domain infrastructure has narrow homes:

- `src/core/messages.rs` and `src/core/codec.rs` own the wire vocabulary and JSON seam;
- `src/core/broadcast/` owns outbound audience/cadence dispatch;
- `src/command_admission/` owns token/system authority before commands reach a domain;
- `src/server_app/` owns composition, cross-domain publication, world setup, and collision;
- `src/sim_sets.rs` owns the logical order `Input → Physics → Damage → Modifiers → Publish → PublishAggregate → Broadcast`.

## State and authority

Session tokens identify players; rendezvous peer ids and DataChannels identify transient transports. The server owns session tenure, game phase, world/runtime content, ship state, objectives, and outcomes. A phone stores only the latest projected state needed to render its consoles.

Human and AI actors submit the same `ControlSystem` commands. Admission records authority once; domain appliers never branch on actor type. Every gameplay decision advances on the authored logical tick, not on rendered frames.

## Related

- [Server App Composition](./server-app.md)
- [Client Architecture](./client-architecture.md)
- [Message Flow](./message-flow.md)
- [Networking](./networking.md)
