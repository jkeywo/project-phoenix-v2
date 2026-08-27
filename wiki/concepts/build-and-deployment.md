---
title: Build & Deployment
type: concept
tags: [trunk, wasm, github-pages, cloudflare, native-host, ci]
sources: [Trunk.toml, scripts/build-client.mjs, scripts/check-deploy-headers.mjs, .github/workflows/, README.md, worker/wrangler.toml, worker/wrangler.demo.toml, deploy/cloudflare/_headers, src/delivery/, docs/delivery-checklist.md, pasm/spec/architecture/native-delivery.yaml]
updated: 2026-08-27
---

# Build & Deployment

## Server Trunk build plus JS client

| Build step | Output | Notes |
|---|---|---|
| `TRUNK_BUILD_RELEASE=true trunk build --release` | `dist/index.html` (server.html → view screen) | Builds the Rust/Bevy WASM host with the default `server` feature and enables the release-only post-build optimisation hook. |
| `node scripts/build-client.mjs` | `dist/client/index.html` plus GUI assets | Copies the pure HTML/JS phone client; there is no client-side WASM feature. |

The server is authoritative and runs the simulation. The client is a pure JS shell that connects to the host via PeerJS/WebRTC and renders HTML console panels.

## Local dev

```
trunk serve                                          # http://localhost:8080  (view screen)
node scripts/build-client.mjs                        # copies client into dist/client/
```

## Production build

```
TRUNK_BUILD_RELEASE=true trunk build --release
node scripts/build-client.mjs
```

Outputs land in `dist/` with the client at `dist/client/`. The QR code on the view screen encodes `https://<host>/client/index.html#<peerId>` so phones land on the right page.

## CI workflows

- **`.github/workflows/ci.yml`** runs eleven jobs on pushes to and pull requests
  against `main`: `pasm`, `test`, `viewer-test`, `boundary`, `build`,
  `editor-test`, `smoke`, `native-build`, `perf`, `balance`, and `deploy`.
  The first six independent jobs plus `native-build` start in parallel.
- `pasm` validates/scans the design model and uploads traceability reports.
  `test` owns formatting, workspace Clippy, the headless-enabled native suite,
  demo-build gates, and native feature-binary compile checks. `viewer-test`
  executes the viewer feature suite, while `boundary` proves the simulation
  and standalone viewer compile without the presentation feature.
- `build` produces the release WASM and pure-JS client artifact independently
  of `test`; `smoke` consumes that artifact. `native-build` produces the shared
  release binaries consumed by the warnings-only `perf` report and the balance
  batches. `balance` keeps the ratified Cruiser matrix gating. `deploy` runs
  only for a push to `main` and requires `test`, `build`, `editor-test`,
  `smoke`, `viewer-test`, and `boundary`; `perf` and `balance` do not gate the
  dev deployment.

The Playwright smoke suite has two projects. Its ordinary message/DOM project
runs under WebDriver with `RenderPlugin` omitted; the separate `render` project
uses SwiftShader, hides the WebDriver flag, and boots the real render path so a
blank or broken viewscreen fails CI. The host stays on continuous Bevy updates
when unfocused so a backgrounded server page still drains inbound messages and
sends lobby responses.

## Cargo notes

```toml
[lib]
crate-type = ["cdylib", "rlib"]   # cdylib for WASM, rlib for `cargo test`

[target.'cfg(target_arch = "wasm32")'.dependencies]
bevy_rapier3d = { version = "0.33", features = ["dim3"] }
getrandom = { version = "0.3", features = ["wasm_js"] }

[target.'cfg(not(target_arch = "wasm32"))'.dependencies]
bevy_rapier3d = { version = "0.33" }
```

Native carried `features = ["parallel"]` until issue #896. WASM has no threads
and so never could, and a parallel broadphase does not order contacts the way a
serial one does — so the two targets were running measurably different physics,
invisibly to native-only testing and first visible in real P2P between a browser
and anything else. Both targets now run the serial solver, at a cost in native
physics throughput that the (non-gating) perf baselines register.
`the_deterministic_build_runs_rapier_serially` in `tests/headless_runner.rs`
fails if the feature comes back.

### Bevy debug feature

The Bevy `debug` feature is an opt-in Cargo feature enabled with
`--features debug`; release builds exclude it. Overlays that depend on it
(entity inspector, behaviour overlay, region wireframes, modifier overlay,
damage log) are functional only when the feature is active. Their
`wasm_bindgen` gate exports compile on all targets, while the Bevy backing
resources are no-ops without `debug`.

## Demo deployment (issue #931)

A second, manual-only deploy publishes a curated public demo to Cloudflare
Pages, alongside (not instead of) the GitHub Pages dev host above.

| | Dev (this file's sections above) | Demo |
|---|---|---|
| Trigger | `ci.yml` `deploy` job, automatic on push to `main` | `.github/workflows/deploy-demo.yml`, `workflow_dispatch` only — **never** on push |
| Host | GitHub Pages behind `https://pp-dev.kiwigamedesign.co.uk` | Cloudflare Pages behind `https://pp-demo.kiwigamedesign.co.uk` |
| Scenario manifest | `dist/assets/scenarios.toml` as authored (full catalogue) | overwritten with `assets/scenarios.demo.toml` (issue #917 curation — bare URL serves `combat_test` with the Alliance Destroyer first and the Alliance Cruiser second) |
| Mod-pack upload | enabled | **absent** — `wasm_add_mod_pack` carries `#[cfg(not(phoenix_demo_build))]` and `gui/build-flags.js`'s `offersModPackUpload` removes the control, so nothing can widen the curated catalogue at runtime (PRD #855) |
| Caching rules | none — GitHub Pages ignores `_headers` | `deploy/cloudflare/_headers`, copied to `dist/_headers` by the workflow |
| TURN worker | `worker/wrangler.toml` → `phoenix-turn-credentials`, `ALLOWED_ORIGIN = pp-dev.kiwigamedesign.co.uk` | `worker/wrangler.demo.toml` → `phoenix-turn-credentials-demo`, `ALLOWED_ORIGIN = pp-demo.kiwigamedesign.co.uk` — a **separate worker**, deployed by the same run; `wrangler.toml` and the dev worker are never touched by the demo workflow |

**Trigger procedure**: Actions tab → *Deploy Demo* → Run workflow. The
`turn_worker_url` input defaults to the projected `workers.dev` URL
(`https://phoenix-turn-credentials-demo.project-phoenix.workers.dev`); the
"Deploy demo TURN worker" step's own log shows the actual URL wrangler
deployed to — if it differs from the default (different account/zone), pass
that value in as `turn_worker_url` on the next run so the patch step below
targets the right host.

Steps, in order: mirror `ci.yml`'s `build` job exactly (Rust/Trunk/Node
toolchain → `TRUNK_BUILD_RELEASE=true PHOENIX_DEMO_BUILD=true trunk build
--release` → `node scripts/build-client.mjs`), swap
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

Everything else the deploy needs that CI cannot do — Pages project, custom
domain, both workers' `ALLOWED_ORIGIN` redeploys, the header check, the native
host's packaging decisions — is enumerated in
[`docs/delivery-checklist.md`](../../docs/delivery-checklist.md).

### Deployed caching (PRD #855)

`deploy/cloudflare/_headers` is the caching contract; Pages reads it from the
root of the uploaded directory. Its one structural rule is that no two patterns
may set the same header, because Pages applies every matching rule and nothing
here can test precedence — so `/*` carries only security headers, media
directories are named individually rather than as `/assets/*`, and everything
unnamed keeps Pages' revalidating default. The win it buys is the ~11-13 MiB
gzipped WASM and its glue being cached for a year, which is what
`/project-phoenix-*` names.

Checked in three places: `tests/client/deploy-headers.test.js` (the rules, over
canned fixtures, every push), `src/delivery/http.rs`'s unit tests (the same
contract as the native host serves it, every push), and
`scripts/check-deploy-headers.mjs <url>` against a real deploy — run from a
laptop (Node 20, no dependencies) or by dispatching the *Check Deploy Headers*
workflow. That last one is `workflow_dispatch` only on purpose: as a push gate
it would turn someone else's uptime into a red branch.

## Native host (PRD #855)

`phoenix-host` serves the client bundle, the content manifest, the scenario
catalogue and a version stamp from a native process, so a host need not be an
open browser tab.

```bash
cargo build --release --features host --bin phoenix-host
./target/release/phoenix-host --client-dir dist
```

Binds `0.0.0.0:8080` by default, so it's LAN-reachable out of the box —
Windows prompts to allow it through the firewall on first run. Pass
`--addr 127.0.0.1:8080` to restrict it to this machine only.

- `--manifest assets/scenarios.demo.toml` is the catalogue restriction — the
  same lever `?manifest=` pulls in the browser.
- `--client-dir` is version-pinned at startup against the manifest the host
  serves: a bundle built for other content refuses to start, before the port is
  taken. `/host/manifest.json` pins a running client's protocol per request.
- Serves **delivery only**. The authoritative simulation is still `server.html`
  or `phoenix-headless`, PeerJS signalling is unchanged, and there is no TLS or
  auth — LAN or behind something else, never a public address.

The catalogue it publishes is the browser host's own: `src/delivery/payload.rs`
holds the single field list that both `wasm_get_scenario_catalog` and the JSON
encoder walk. See `pasm/spec/architecture/native-delivery.yaml`.

## Related

- [Architecture](./architecture.md) · [Testing Strategy](./testing-strategy.md)
