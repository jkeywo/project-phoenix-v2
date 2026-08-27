---
title: Testing Strategy
type: concept
tags: [tests, rust, javascript, playwright, pasm, ci]
sources: [AGENTS.md, .github/workflows/ci.yml, tests/client/, tests/smoke/, tests/headless_runner.rs, src/core/codec_tests.rs]
updated: 2026-08-27
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

## Client JavaScript

Vitest under `tests/client/` covers pure state builders, routing, localization,
components, authoring scripts, and other browser-independent modules. Player
console behavior stays in pure HTML/CSS/JS; tests do not introduce a client
WASM layer.

## Browser smoke and rendering

Playwright boots the real server WASM and client pages in Chromium, replacing
only PeerJS transport with the `BroadcastChannel` shim. The normal project
checks message flow and DOM behavior without a GPU. The render project uses
SwiftShader and includes a pixel-level viewscreen check so a clean-console
render-graph failure cannot silently produce a blank scene.

## PASM

`uv run pasm validate`, `uv run pasm scan`, and `uv run pasm traceability`
check the repository-owned model under `pasm/spec/`. The PASM tool and its unit
suite live in Vellum; Phoenix does not have a local PASM pytest suite.

## CI gates

The workflow runs independent `pasm`, Rust `test`, and `editor-test` jobs. The
WASM build follows Rust tests; smoke follows the build; performance and balance
run on their declared dependencies. The asset performance capture and ratified
Cruiser balance matrix are gating, while other machine-sensitive performance
and balance results remain reports.

During implementation, use `cargo check` and targeted tests. Before a commit,
run the repository's documented fast gate set once. The build and smoke gates
are exercised between pushes as required by the issue workflow.

## Related

- [PASM Runtime](./pasm-runtime.md)
- [Build and Deployment](./build-and-deployment.md)
- [Codec Seam](./codec-seam.md)
