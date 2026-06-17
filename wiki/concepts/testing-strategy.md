---
title: Testing Strategy
type: concept
tags: [tests, cargo, playwright, smoke, pyramid]
sources: [tests/smoke/, src/server/, AGENTS.md, PRD-051]
updated: 2026-05-08
---

# Testing Strategy

Two layers, sharply separated.

## 1. Rust unit tests (`cargo test`)

Inline `#[cfg(test)] mod tests` in each module. The Rust side is the test-heavy layer because so many seams are pure functions.

| Module | What it covers |
|---|---|
| `session.rs` | Registration, duplicates, console assignment, vacancy, reconnect, conflict resolution, `helm_token()`/`captain_token()` |
| `codec.rs` | Round-trip for **every** `ClientMessage` and `ServerMessage` variant |
| `lobby.rs` / `lobby_handler.rs` | Bevy App harness: Identify→Welcome, console select, captain-only StartGame, HelmInput phase-gating, disconnect |
| `ship_physics.rs` | Zero input, accel curve, decel curve, steering yaw, dt scaling, speed cap |
| `ship_state.rs` | Red Alert toggle, snapshot generation |
| `asteroid_spawner.rs` | Count, bounds, clear zone, no duplicates |

Test-style rule (from `AGENTS.md`):

> Set up state → perform action → assert on observable output through the public interface. Do **not** assert on private fields, internal call counts, or implementation-specific details.

## 2. Smoke tests (Playwright + Chromium)

Live in `tests/smoke/`. Boot the **real** WASM server in a headless browser; mock only the WebRTC layer.

### The PeerJS shim

`peerjs-shim.js` replaces `window.Peer` with a `BroadcastChannel`-backed fake before any page script runs (Playwright's `addInitScript`). Same surface as PeerJS — `open`, `connection`, `data`, `close` events. Production HTML is **never modified** for tests.

### Wasm-ready signal

The shim sets `window.__wasmReady` (and fires `wasm-ready`) only after **both** the fake peer opens **and** Trunk's `TrunkApplicationStarted` event fires, with a `setTimeout(0)` so `startPhoenix()` runs first. Tests `await page.waitForFunction('window.__wasmReady')` before sending anything.

### Asset stubbing

`fixtures.ts` installs a context-wide route that fulfils every `**/*.glb` request with an empty 200 body. The real `assets/models/*.glb` files total ~560 MB (single asteroid GLBs are ~38 MB) and parsing them in headless Chromium reliably blew the `GameStarted` timeout once the [asset-preload](./asset-preload.md) gate started waiting on GLB readiness. Bevy's glTF loader sees the missing header and surfaces `LoadState::Failed` — terminal, counts as "ready", the gate clears immediately. Tests that genuinely need the real bytes (today only `ship-mesh-load.spec.ts`) opt out per-URL with `route.continue()`, which skips all earlier matching handlers (including the fixture stub) and hits the static `dist/` server directly.

### What's covered

| Spec | Issue | Verifies |
|---|---|---|
| `shim.spec.ts` | #52 | The shim itself (BroadcastChannel routing) |
| `server-load.spec.ts` | #54 | WASM boots, no JS console errors |
| `client-connect.spec.ts` | #55 | Real `client.html` connects, `#status` = "Connected" after Welcome |
| `lobby.spec.ts` | #56/#57 | `ConsoleSelected` broadcasts; non-captain `StartGame` ignored |
| `sim-state.spec.ts` | #58/#59 | `SimState` shape valid; `HelmInput` changes ship position |

CI runs the suite on every push and pull request via `.github/workflows/ci.yml`.

## What's **not** automated

- Visual rendering (3D camera, Bevy UI layout, Red Alert vignette, button highlights)
- WASM/JS bridge internals (covered by smoke tests end-to-end)
- Actual WebRTC over the public PeerJS broker

## Why this shape

- The pure-function seams (physics, codec, asteroid spawner, lobby handler) make unit tests cheap and dense.
- The smoke harness covers the integration layer that unit tests can't reach.
- Manual smoke remains for visual concerns — explicit, not aspirational.

## Related

- [Codec Seam](./codec-seam.md) · [Architecture](./architecture.md)
- [PRD #51](../sources/prd-051-smoke-test-harness.md)
