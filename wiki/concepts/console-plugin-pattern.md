---
title: Console Plugin Pattern
type: concept
tags: [bevy, plugin, console, modularity, planned]
sources: [src/client_app.rs, src/comms_plugin.rs, src/phone_border/, CONTEXT.md]
updated: 2026-05-14
status: superseded
---

# Console Plugin Pattern

> **Update (2026-07-03):** the client half of this pattern is gone — the Bevy/WASM client was deleted (PRD #438, #463) and console UIs are now HTML iframes under `gui/` (see [Client Architecture](./client-architecture.md)). The **server** half survives: one Bevy plugin per console at `src/console/<name>/server.rs`, registered in `server_app.rs`.

The intended pattern is that each client-side console is a **single Bevy plugin** owning everything for that console:

- UI nodes (Bevy `Node` hierarchy)
- Marker components (e.g. `RedAlertButton`, `ThrustSlider`)
- Setup systems (build the UI on `OnEnter(GamePhase::InProgress)`)
- Event handlers (button clicks → `OutboundMessage` writers)
- Teardown systems (despawn on phase change)

## Current state — partially realised

The pattern is **only partially realised in code today**:

- `client_app.rs` (~2329 LoC, 0 tests) is a god-module that owns the UI for *every* console: Captain, Helm, Tactical, Repair, Power, Science. There are no per-console plugins.
- `client_sim.rs` (~2136 LoC) is a god-resource (`ClientSimState` with 45+ fields) that every panel reads from, plus 22 message-builder functions in one file.
- `phone_border/` (PRD #187) is one example of a separate plugin (the diegetic bezel + helm/captain chrome), but it sits *alongside* the god-module rather than replacing it.
- `comms_plugin.rs` (24 LoC) is a husk: just `init_resource::<ClientCommsState>()`. It will fold into a real `CommsConsolePlugin` once the per-console split lands.

A reorg into `src/console/<name>/{client.rs,server.rs}` plus a two-layer split of `ClientSimState` (shared `ShipView` + per-console view resources) is planned. See [Open Architectural Questions](../roadmap/open-architectural-questions.md) for the design.

## Why this shape (target)

- **Adding a console = adding a plugin.** No surgery on a god-module UI.
- **Removal is safe.** If a console is dropped, deleting its plugin file removes everything: state, UI, handlers, markers.
- **Test isolation.** A plugin can be tested with a minimal Bevy app harness containing just it.
- **No cross-console coupling.** Helm doesn't know about Captain, and vice versa.
- **Locality of behaviour.** Each panel owns the system that reads its own `InboundMessage`s and the message-builders it needs to send. No central dispatcher.

## Adding a new console (in the current god-module shape)

Until the split lands, adding a console means:

1. Add a `Console` variant to `src/messages.rs` and update the codec round-trip tests.
2. Add panel UI inside `client_app.rs` alongside the existing panels, gated by `ActiveConsole`.
3. Add view-model fields and apply logic to `ClientSimState` in `client_sim.rs`.
4. Wire any new outbound messages through the message-builder functions in `client_sim.rs`.
5. Handle the new server messages in `simulation.rs` (or the appropriate server-side plugin).

## Related

- [Console](../entities/console.md) · [Bridge Crew Stations (planned)](../entities/bridge-crew-stations-planned.md)
- [View-Model Pattern](./view-model-pattern.md) — what plugins read from
- [Open Architectural Questions](../roadmap/open-architectural-questions.md) — the per-console plugin reorg
