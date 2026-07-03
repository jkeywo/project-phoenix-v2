---
title: AGENTS.md (project root)
type: source
tags: [agents, operating-manual, conventions, llm]
source_path: AGENTS.md
status: shipped
updated: 2026-07-03
---

# AGENTS.md

The operating manual for any LLM coding agent working on the repo. Slimmed in the 2026-07-03 docs audit: volatile state descriptions (console roster, feature inventory, open-PRD list) were removed in favour of wiki pointers, because they drift; AGENTS.md now carries only durable rules, commands, and orientation.

## What it covers

- TL;DR: host = Rust/Bevy WASM authoritative server (`server.html`); client = **pure HTML/CSS/JS** (`client.html`, no client-side WASM); PeerJS star topology. Current-state pointers go to `wiki/concepts/project-overview.md` and `wiki/roadmap/overview.md`.
- The wiki section: read `wiki/SCHEMA.md` at the start of non-trivial tasks; orient / ingest / file-back workflows; **run the lint pass when closing out a PRD or issue batch**.
- Common commands: `cargo test`, `cargo check`, `npx vitest run` (JS client tests), `trunk serve`, `node scripts/build-client.mjs`, Playwright smoke tests, CI shape.
- Message-flow diagram: phone JS → `wasm_receive_message` → `drain_inbound` → console plugins (`SimSet::Input`) → `flush_outbound` → `routeOutbound()` → client `handleMessage()` → `gui/sim-state.js` → `gui/console-state.js` → iframes.
- File layout: `src/` module map (server-side Rust) + top-level `gui/` (client JS + per-console HTML) + `assets/` + `tests/{client,smoke}/`.
- **Key Constraints & Rules (11 numbered, load-bearing):** codec seam (`serde_json` only in `codec.rs`); server authority; client is pure JS; captain authority; `Player.station` is authoritative ownership with `Backfill` on disconnect; human/AI symmetry via `ControlSystem` (never branch on source past admission); 10 Hz helm; deterministic asteroids; WebGL2 + PeerJS cloud broker; Bevy-free pure modules; **no hardcoded gameplay values — everything tunable lives in TOML**.
- Testing strategy: Rust inline unit tests, Vitest for `gui/*.js`, Playwright smoke; renderer untested.
- Cargo notes (`cdylib`+`rlib`, `server` feature only — no `client` feature since #463), deployed GitHub Pages URLs, and an updated "Adding new message types" checklist (prefer new `SystemControlPayload` variants; client steps go through `gui/sim-state.js` / `gui/action-map.js` / Vitest).

## Why this matters for the wiki

`AGENTS.md` is a **raw source** and deliberately delegates current-state description to the wiki. When a change makes `AGENTS.md` and a wiki page disagree, **`AGENTS.md` is the source of truth** for rules/conventions and the wiki page must be updated; for feature state, the code is the source of truth and both must follow it.

## Cross-references

- [Architecture](../concepts/architecture.md), [Client Architecture](../concepts/client-architecture.md), [Codec Seam](../concepts/codec-seam.md), [Message Flow](../concepts/message-flow.md), [Game Loop](../concepts/game-loop.md), [Testing Strategy](../concepts/testing-strategy.md), [Broadcaster Seam](../concepts/broadcaster-seam.md).
