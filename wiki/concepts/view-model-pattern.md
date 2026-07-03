---
title: View-Model Pattern
type: concept
tags: [view-model, renderer, pure-derived, refactor]
sources: [CONTEXT.md, src/client/lobby_state.rs, src/server/renderer.rs]
updated: 2026-05-08
status: current
---

# View-Model Pattern

> **Update (2026-07-03):** the pattern survives but the client examples below describe deleted Rust code. Current view-models: server `GameState` (in `GameStateCache`) for the renderer; client-side pure JS — `gui/lobby-state.js`, `gui/sim-state.js`, and the `build*(state)` functions in `gui/console-state.js` (see [Client Architecture](./client-architecture.md)).

Renderers read **pure derived snapshots**, not raw session/simulation state. This was one of the architectural deepenings (commit `f3ef92c`).

## Definition

A view-model is a struct that:

- Contains exactly the data a particular renderer needs.
- Is derived from authoritative state by a pure function.
- Has no knowledge of Bevy entities, Rapier rigid bodies, or `SessionManager` internals.

## Examples

| Renderer | View-model | Source state |
|---|---|---|
| Client lobby | `LobbyView` (`src/client/lobby_state.rs`) | `GameState` from `Welcome` + subsequent events |
| Server lobby | `GameState` resource | `SessionManager` |
| Client helm | `HelmState` (`src/client/helm_state.rs`) | Latest `SimSnapshot` + `WorldData` |
| Client sim mirror | `SimState` (`src/client/sim_state.rs`) | Latest `SimSnapshot` |

## Why

- **Renderers stay dumb.** They translate fields into UI, nothing else. No business logic.
- **State sources stay swappable.** Replace the session manager with a different storage strategy and the renderer doesn't know.
- **Test the derivation, not the rendering.** Unit-test `LobbyView::from(state)`. Visual rendering stays manual.
- **Reconnect is free.** A view-model rebuilt from `Welcome` is identical to one built incrementally from events.

## Anti-pattern this replaces

Pre-deepening, renderers reached into `SessionManager` and `ShipState` directly via Bevy queries. Every session-shape change broke the renderer. Now derivation lives in one place; the renderer reads its own type.

## Related

- [Architecture](./architecture.md) · [Console Plugin Pattern](./console-plugin-pattern.md)
