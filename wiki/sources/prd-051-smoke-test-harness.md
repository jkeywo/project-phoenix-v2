---
title: PRD #51 — Smoke Test Harness
type: source
tags: [prd, testing, playwright, smoke, peerjs-shim, ci]
source_url: https://github.com/jkeywo/project-phoenix-v2/issues/51
status: closed (2026-05-07)
updated: 2026-05-08
---

# PRD #51 — Smoke Test Harness

End-to-end browser tests for the WASM/JS bridge that unit tests can't reach.

## Problem

`cargo test` covered pure Rust well (session, codec, physics, lobby). The WASM bridge, renderer, and HTML pages were only verified manually. Every `gh-pages` deploy was a leap of faith.

## Solution

Playwright (Chromium) tests that boot the **real** WASM build in a headless browser and run the full flow: load → connect → claim consoles → start game → verify SimState → send HelmInput → verify ship moved.

WebRTC is mocked with a **`BroadcastChannel`-backed PeerJS shim** injected via Playwright's `addInitScript`. No production HTML or Rust files are modified.

## Key decisions

- **Chromium only.** Firefox/WebKit out of scope.
- **`BroadcastChannel` shim** replaces `window.Peer`. Same surface (`open`, `connection`, `data`, `close`).
- **`window.__wasmReady`** is the test handshake. Set after both fake-peer-open AND `TrunkApplicationStarted`, with a `setTimeout(0)` so `startPhoenix()` runs first.
- **Tests assert through the message layer**, not DOM/canvas state. Robust to renderer changes.
- **CI workflow** (`smoke-test.yml`) runs on push and pull_request. Uses pre-built `dist/`, no recompile.

## Spec coverage

| Spec | Purpose |
|---|---|
| `shim.spec.ts` | Shim itself routes correctly between two pages |
| `server-load.spec.ts` | WASM boots, no JS console errors |
| `client-connect.spec.ts` | Real `client.html` connects, `#status` becomes "Connected" after Welcome |
| `lobby.spec.ts` | `ConsoleSelected` broadcasts; non-captain `StartGame` is ignored |
| `sim-state.spec.ts` | `SimState` shape valid; `HelmInput` changes ship position |

## Out of scope

Firefox/WebKit, real WebRTC, visual regression, canvas pixel asserts, reconnection flow, client UI rendering details.

## Cross-references

- Concept: [Testing Strategy](../concepts/testing-strategy.md), [Networking](../concepts/networking.md)
