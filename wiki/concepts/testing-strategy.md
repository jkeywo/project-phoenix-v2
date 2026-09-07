---
title: Testing Strategy
type: concept
tags: [tests, rust, javascript, playwright, pasm, ci]
sources: [AGENTS.md, .github/workflows/ci.yml, tests/client/, tests/smoke/, tests/headless_runner.rs, src/core/codec_tests.rs]
updated: 2026-09-07
---

# Testing Strategy

Tests are placed at the narrowest public seam that can prove the behavior, with
the full CI workflow providing integration coverage across Rust, client JavaScript,
PASM, WebAssembly, rendering, performance, and balance.

## Rust

Pure modules use native unit tests: arrange public state, perform an action, and
assert on observable output. Bevy adapters use small `App` fixtures where
schedule ordering or ECS integration is part of the contract. Large test modules
live in sibling `*_tests.rs` files while remaining children of the production
module.

The headless runner is an integration test because it loads native entity
templates and boots the whole authoritative simulation. Tests that populate
the process-global native config cache belong there rather than in the library
test binary.

The manual Falling Skyway timeline tests keep scenario damage and terminal
outcomes live. Fixtures observing late dialogue or campaign records shelter
the player through storm exposure, then approach the relevant contact. An
idle-crew fixture takes Tactical designation before its operate objective
opens, so Backfill cannot complete a deliberately deferred rescue. Civilian
loss fixtures leave the traffic on its authored lanes while protecting only
the observing crew. These setup boundaries live in `tests/headless_runner.rs`.

## Client JavaScript

Vitest under `tests/client/` covers pure state builders, routing, localization,
components, authoring scripts, and other browser-independent modules. Player
console behavior stays in pure HTML/CSS/JS; tests do not introduce a client
WASM layer.

## Browser smoke and rendering

Playwright boots the real server WASM and client pages in Chromium, replacing
only the transport — a `BroadcastChannel` stand-in for the rendezvous socket
and for WebRTC, terminating the REAL worker-rendezvous registry inside the host
page (`tests/smoke/rendezvous-shim.js`, behind the `tests/smoke/transport-fixture.js`
seam). The normal project
checks message flow and DOM behavior without a GPU. The render project uses
SwiftShader and includes a pixel-level viewscreen check so a clean-console
render-graph failure cannot silently produce a blank scene.

## PASM

`uv run pasm validate`, `uv run pasm scan`, and `uv run pasm traceability`
check the repository-owned model under `pasm/spec/`. The PASM tool and its unit
suite live in Vellum; Phoenix does not have a local PASM pytest suite.

## CI gates

The ordinary Rust job runs `cargo test --workspace --features headless`.
The separate viewer job runs `cargo test --lib --features viewer viewer::`
and refuses zero matched tests. The library test binary still compiles in full,
but general tests are executed only by the ordinary suite and integration-test
binaries are not built for the viewer job.

Demo-build tests (`PHOENIX_DEMO_BUILD=true`, with the build flag, debug admission,
and absent-wire-route filters) and debug host/capture binary builds run in
independent `demo-test` and `tooling-build` jobs. Both remain deployment gates,
alongside `test`, `viewer-test`, `boundary`, `editor-test`, `build`, and `smoke`.
Each Rust job has its own cache; new jobs initially pay a cold-cache cost.

The WASM build runs independently; smoke depends on its artifact. PRs and main
pushes run the core smoke tier; nightly/manual runs and PRs labelled
`smoke-full` run the full suite. Native release builds, performance, and balance
keep their nightly/manual schedules. The Cruiser balance matrix gates its job;
regular performance comparisons report warnings rather than blocking deployment.
PASM retains its independent validation, scan and traceability job.

During implementation, use targeted tests. Run the documented final gates once
before pushing, including the additional native configurations when verifying
the full CI matrix. See AGENTS.md for commands and PowerShell demo environment
handling. Check viewer discovery as well as its exit status.

Prefer observable behaviour over source-text pins. The enabled logging filter
is exercised by the existing world-spawned duel's damage/death assertions;
there is no separate logging duel or claim that it captures emitted log text.

## Related

- [PASM Runtime](./pasm-runtime.md)
- [Build and Deployment](./build-and-deployment.md)
- [Codec Seam](./codec-seam.md)
