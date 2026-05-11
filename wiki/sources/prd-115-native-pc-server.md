---
title: PRD #115 — Native PC Server
type: source
tags: [prd, native, transport, websocket, cloudflared, tunnel, distribution, planned]
source_url: https://github.com/jkeywo/project-phoenix-v2/issues/115
status: open
updated: 2026-05-11
---

# PRD #115 — Native PC Server

Ship a native PC binary that runs the full bridge simulator server as a regular double-click application, with internet play handled via a bundled Cloudflare quick-tunnel sidecar. The existing GitHub Pages WASM deployment is unaffected.

## Status

Open. No code yet. WASM deployment continues alongside.

## Problem

Hosting today requires keeping a browser tab open as the server. Browsers throttle background tabs; the session dies on tab close; non-technical hosts can't run a reliable session from their own machine.

## Solution

A `cargo build --release --features native` produces an executable. Double-click it: a Bevy native window opens, a Cloudflare quick-tunnel comes up, a QR code renders on the viewscreen pointing at `https://<tunnel>/client/`. Friends scan and join in their browser exactly as today.

## Key decisions

- **New `native` Cargo feature.** Mutually exclusive with `server`/`client` at build time, co-exists in the same crate. `wasm-bindgen` becomes conditional.
- **New `[[bin]]` target** compiled only under `native`. Loads config from disk, builds Bevy `App`, spawns tunnel, blocks on `App::run()`.
- **`native_bridge` module** owns the axum router: `GET /client/*` static files + `GET /ws` WebSocket upgrade. Single TCP port. `ConnectionMap` (`ConnectionId → (SessionToken, sender)`) mirrors the JS `peerTokens`/`tokenConns` maps. Three Bevy systems mirror `bridge.rs`: `drain_inbound`, `drain_disconnects`, `flush_outbound`. Tokio runtime on a background thread; channels (not `thread_local!`) bridge to Bevy.
- **`native_config_loader` module** — pure Rust, sync `std::fs`, reuses `MapConfig`/`EntityConfig` parsers. Replaces the JS-fetch preload chain on native.
- **`tunnel_manager` module** — pure Rust. Spawns `cloudflared` child, parses the `trycloudflare.com` URL from stdout, exposes `TunnelState::{Pending, Ready, Failed}` via a `poll()` callable from the Bevy main thread. Kills child on `Drop`.
- **Single client page.** `client.html` adds one `if/else`: hash starting with `wss://` or `ws://` → open WebSocket and send `Identify` first; otherwise PeerJS as today.
- **In-window QR + status overlay.** Native-only Bevy UI (behind `#[cfg(feature = "native")]`) renders the QR (via `qrcode` crate) plus tunnel status (spinner / URL / error).
- **Distribution:** plain zip per platform (Windows / macOS / Linux) containing exe, `cloudflared`, and `dist/client/`. WASM `deploy.yml` is untouched.

## Schema additions (planned)

- New Cargo feature: `native`.
- New modules: `native_bridge`, `native_config_loader`, `tunnel_manager`.
- Conditional compilation around `wasm-bindgen` and `JsValue` types in `config_cache.rs`.
- One `if/else` in `client.html` for transport detection.

## Out of scope

- Native client (phone app). Clients remain browser-based.
- Auto-download of `cloudflared`. Bundled in zip.
- Cloudflare fallback / self-hosted relay.
- macOS / Windows code signing, installers.
- Native audio backend differences.
- Any change to simulation, lobby, session, physics, damage, breakdown, codec.
- The `src/server/`, `src/client/`, `src/shared/` draft refactor.

## Cross-references

- [Architecture](../concepts/architecture.md) · [Networking](../concepts/networking.md) · [Build & Deployment](../concepts/build-and-deployment.md)
- [Roadmap Overview](../roadmap/overview.md)
