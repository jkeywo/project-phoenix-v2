---
title: Build & Deployment
type: concept
tags: [trunk, wasm, github-pages, ci]
sources: [Trunk.toml, client-trunk.toml, .github/workflows/, README.md]
updated: 2026-05-08
---

# Build & Deployment

## Two Trunk configs, one crate

| Config | Output | Bevy crate features |
|---|---|---|
| `Trunk.toml` | `dist/index.html` (server.html → view screen) | `["server"]` |
| `client-trunk.toml` | `dist/client/index.html` (client.html → phones) | `["client"]` |

Both compile the same crate (`src/lib.rs`) to WebAssembly via Trunk. Feature flags select which bridge module is included.

## Local dev

```
trunk serve                                          # http://localhost:8080  (view screen)
trunk serve --config client-trunk.toml --port 8081   # http://localhost:8081  (phone)
```

## Production build

```
trunk build --release
trunk build --release --config client-trunk.toml
```

Outputs land in `dist/` with the client at `dist/client/`. The QR code on the view screen encodes `https://<host>/client/index.html#<peerId>` so phones land on the right page.

## CI workflows

- **`.github/workflows/deploy.yml`** — on push to `main`: build both pages, merge into `dist/`, deploy to `gh-pages` branch via `peaceiris/actions-gh-pages`.
- **`.github/workflows/smoke-test.yml`** — on push and pull_request: build `dist/`, run Playwright smoke suite (Chromium).

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

## Related

- [Architecture](./architecture.md) · [Testing Strategy](./testing-strategy.md)
- [PRD #1](../sources/prd-001-bridge-simulator.md) — original deploy decision
