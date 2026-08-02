---
title: Build & Deployment
type: concept
tags: [trunk, wasm, github-pages, ci]
sources: [Trunk.toml, scripts/build-client.mjs, .github/workflows/, README.md, worker/wrangler.toml, worker/wrangler.demo.toml]
updated: 2026-08-02
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

## Demo deployment (issue #931)

A second, manual-only deploy publishes a curated public demo to Cloudflare
Pages, alongside (not instead of) the GitHub Pages dev host above.

| | Dev (this file's sections above) | Demo |
|---|---|---|
| Trigger | `ci.yml` `deploy` job, automatic on push to `main` | `.github/workflows/deploy-demo.yml`, `workflow_dispatch` only — **never** on push |
| Host | GitHub Pages (`jkeywo.github.io`) | Cloudflare Pages, `https://project-phoenix-demo.pages.dev` |
| Scenario manifest | `dist/assets/scenarios.toml` as authored (full catalogue) | overwritten with `assets/scenarios.demo.toml` (issue #917 curation — bare URL serves `combat_test` + the Alliance Destroyer only; mod-pack upload stays enabled) |
| TURN worker | `worker/wrangler.toml` → `phoenix-turn-credentials`, `ALLOWED_ORIGIN = jkeywo.github.io` | `worker/wrangler.demo.toml` → `phoenix-turn-credentials-demo`, `ALLOWED_ORIGIN = project-phoenix-demo.pages.dev` — a **separate worker**, deployed by the same run; `wrangler.toml` and the dev worker are never touched by the demo workflow |

**Trigger procedure**: Actions tab → *Deploy Demo* → Run workflow. The
`turn_worker_url` input defaults to the projected `workers.dev` URL
(`https://phoenix-turn-credentials-demo.project-phoenix.workers.dev`); the
"Deploy demo TURN worker" step's own log shows the actual URL wrangler
deployed to — if it differs from the default (different account/zone), pass
that value in as `turn_worker_url` on the next run so the patch step below
targets the right host.

Steps, in order: mirror `ci.yml`'s `build` job exactly (Rust/Trunk/Node
toolchain → `trunk build --release` → `node scripts/build-client.mjs`), swap
in the demo manifest, patch the TURN credential URL literal from the dev
worker to the demo worker's URL in the two built files that embed it
(`dist/index.html` — server.html's inline script, renamed by trunk — and
`dist/client/gui/connection-manager.js` — client.html's module, copied
verbatim by `build-client.mjs`), a verification step that fails the run if
either mutation didn't take (dev URL still present, or the demo manifest's
curated `ships = [...]` line missing from `dist/assets/scenarios.toml`), then
deploy the worker and the Pages site via `cloudflare/wrangler-action@v3`.

**Secrets**: `CLOUDFLARE_API_TOKEN` / `CLOUDFLARE_ACCOUNT_ID` (repo secrets,
owner-added, shared by both wrangler-action steps). The demo worker's own
`METERED_KEY` is a per-worker Cloudflare secret, not a repo secret — set it
once, before the first successful worker deploy, with
`wrangler secret put METERED_KEY --config wrangler.demo.toml` from `worker/`
(owner action; not something CI can do).

## Related

- [Architecture](./architecture.md) · [Testing Strategy](./testing-strategy.md)
- PRD #1 — original deploy decision
- Issue #931 — demo release: manual Cloudflare Pages deploy
