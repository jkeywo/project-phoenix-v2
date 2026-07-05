---
title: Build & Deployment
type: concept
tags: [trunk, wasm, github-pages, ci]
sources: [Trunk.toml, scripts/build-client.mjs, .github/workflows/, README.md]
updated: 2026-06-24
---

# Build & Deployment

## Server Trunk build plus JS client

| Build step | Output | Notes |
|---|---|---|
| `trunk build --release` | `dist/index.html` (server.html → view screen) | Builds the Rust/Bevy WASM host with the default `server` feature. |
| `node scripts/build-client.mjs` | `dist/client/index.html` plus GUI assets | Copies the pure HTML/JS phone client; there is no client-side WASM feature. |

The server is authoritative and runs the simulation. The client is a pure JS shell that connects to the host via PeerJS/WebRTC and renders HTML console panels.

## Local dev

```
trunk serve                                          # http://localhost:8080  (view screen)
node scripts/build-client.mjs                        # copies client into dist/client/
```

## Production build

```
trunk build --release
node scripts/build-client.mjs
```

Outputs land in `dist/` with the client at `dist/client/`. The QR code on the view screen encodes `https://<host>/client/index.html#<peerId>` so phones land on the right page.

## CI workflows

- **`.github/workflows/ci.yml`** — on push and pull_request: `cargo fmt --check`, `clippy -D warnings`, native `cargo test`, WASM server build, pure JS client copy, editor Vitest suite, Playwright smoke suite; on push to `main`: also deploy to `gh-pages`.

Formatting and lint gates run before `cargo test` to fail fast and cheaply. The build job depends on test (`needs: test`), so a fmt or clippy failure blocks the expensive WASM/trunk build, which transitively blocks smoke and deploy. (#612)

The smoke suite runs the host page in headless Chromium with render-heavy plugins skipped. The host stays on continuous Bevy updates even when unfocused so backgrounded server pages still drain `wasm_receive_message` and send `Welcome` to test clients.

## Cargo notes

```toml
[lib]
crate-type = ["cdylib", "rlib"]   # cdylib for WASM, rlib for `cargo test`

[target.'cfg(target_arch = "wasm32")'.dependencies]
bevy_rapier3d = { version = "0.33", features = ["dim3"] }
getrandom = { version = "0.3", features = ["wasm_js"] }

[target.'cfg(not(target_arch = "wasm32"))'.dependencies]
bevy_rapier3d = { version = "0.33", features = ["parallel"] }
```

`parallel` is fine on native (where `cargo test` runs); WASM has no threads.

### Bevy debug feature (#598)

The bevy `debug` feature (enabling Bevy debug internals like system-name recording and visual debug overlays) was removed from default dependencies and is now a **conditional Cargo feature** (`debug`), opt-in via `--features debug`. Release builds exclude it entirely. Overlays that depend on it (entity inspector, behaviour overlay, region wireframes, modifier overlay, damage log) are functional only when the feature is active — the `wasm_bindgen` exports that gate them compile on all targets, but their Bevy-internal backing resources are no-ops without `debug`.

## Related

- [Architecture](./architecture.md) · [Testing Strategy](./testing-strategy.md)
- [PRD #1](../sources/prd-001-bridge-simulator.md) — original deploy decision
