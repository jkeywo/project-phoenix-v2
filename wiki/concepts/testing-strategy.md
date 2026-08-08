---
title: Testing Strategy
type: concept
tags: [tests, cargo, playwright, smoke, pyramid]
sources: [tests/smoke/, src/server/, AGENTS.md]
updated: 2026-06-24
---

# Testing Strategy

Two layers, sharply separated.

## 1. Rust unit tests (`cargo test`)

Inline `#[cfg(test)] mod tests` in each module. The Rust side is the test-heavy layer because so many seams are pure functions.

| Module | What it covers |
|---|---|
| `session.rs` | Registration, duplicates, console assignment, vacancy, reconnect, conflict resolution, `helm_token()`/`captain_token()` |
| `codec.rs` | Round-trip for **every** `ClientMessage` and `ServerMessage` variant |
| `lobby.rs` / `lobby_handler.rs` | Bevy App harness: Identify→Welcome, station selection, SetReady auto-start, `ControlSystem` (e.g. `SetThrust`) phase-gating, disconnect |
| `ship_physics.rs` | Zero input, accel curve, decel curve, steering yaw, dt scaling, speed cap |
| `ship_state.rs` | Red Alert toggle, snapshot generation |
| `asteroid_spawner.rs` | Count, bounds, clear zone, no duplicates |

Test-style rule (from `AGENTS.md`):

> Set up state → perform action → assert on observable output through the public interface. Do **not** assert on private fields, internal call counts, or implementation-specific details.

## 2. Smoke tests (Playwright + Chromium)

Live in `tests/smoke/`. Boot the **real** WASM server in a headless browser; mock only the WebRTC layer.

### The PeerJS shim

`peerjs-shim.js` replaces `window.Peer` with a `BroadcastChannel`-backed fake before any page script runs (Playwright's `addInitScript`). Same surface as PeerJS — `open`, `connection`, `data`, `close` events. Production HTML is **never modified** for tests.

### WASM readiness and host ticking

The shim sets `window.__wasmReady` (and fires `wasm-ready`) only after **both** the fake peer opens **and** `server.html` dispatches `PhoenixReady`. `PhoenixReady` fires after async config preload, station validation, callback registration, and `wasm_receive_message` exposure, just before `wasm_init()`.

The server WASM uses continuous Bevy updates while focused and unfocused. This matters because Playwright often brings a client page to the front immediately after reading the host peer ID; if the backgrounded host stalls, `Identify` remains queued and the client times out waiting for `Welcome`.

Chromium reports Bevy's WASM app-runner handoff as a bare `unreachable` page error. `captureServerPageErrors()` ignores that exact message, but still records Rust panic messages and other runtime errors.

### Default scenario stub

`fixtures.js` installs a context-wide route that fulfils every `**/assets/worlds/default.toml` request with `MINIMAL_DEFAULT_WORLD` — an inline TOML with the player ship, "Starbase Alpha", and a single `[[comms]] on_hailed` block. The production `default.toml` references a planet (~36 MB GLB), an asteroid field (12 asteroid templates, ~150 MB of GLBs), a sun, and a nebula region; the [asset-preload](./asset-preload.md) gate waits for every GLB to reach a terminal `LoadState` before allowing game start, and headless Chromium can't realistically fetch + parse all of that within the smoke-test timeouts.

The minimal world keeps only what the smoke suite actually inspects:

- the player ship (no GLB — `player_ship.toml` is icon-only);
- "Starbase Alpha" (one ~16 MB station GLB) — `comms.spec.js` hails it and `world-bootstrap.spec.js` asserts on its tag;
- an `[[comms]] on_hailed` block with a response carrying an `add_objective` action — required by `comms.spec.js`.

Tests that need a different scenario (`tactical-fire-flow.spec.js` with its inline `MINIMAL_TEST_WORLD`, `patrol.spec.js` and `ship-mesh-load.spec.js` with `patrol.toml`) keep routing their own world; Playwright matches the most-recently-added route first, so the per-test override wins.

For tests that route a real production TOML but don't need its heavy entities, `fixtures.js` exports `stripHeavyEntities(toml)` — a regex helper that removes any `[[entity]]` block whose `template_path` references the asteroid field, planet, sun, or nebula. `patrol.spec.js` and `ship-mesh-load.spec.js` use this on `patrol.toml` to drop the asteroid field while preserving the raider they actually inspect.

### What's covered

| Spec | Issue | Verifies |
|---|---|---|
| `shim.spec.js` | #52 | The shim itself (BroadcastChannel routing) |
| `server-load.spec.js` | #54 | WASM boots, no JS console errors |
| `client-connect.spec.js` | #55 | Real `client.html` connects, `#status` = "Connected" after Welcome |
| `lobby.spec.js` | #56/#57 | Station assignment and lobby phase transitions |
| `sim-state.spec.js` | #58/#59 | `SimState` shape valid; `SetThrust` changes ship position |

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
- PRD #51
