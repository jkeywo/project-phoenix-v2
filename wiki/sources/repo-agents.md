---
title: AGENTS.md (project root)
type: source
tags: [agents, operating-manual, conventions, llm]
source_path: AGENTS.md
status: shipped
updated: 2026-05-09
---

# AGENTS.md

The operating manual for any LLM coding agent working on the repo.

## What it covers

- TL;DR of the project (one paragraph + PRD links: #1, #22, #66).
- Prerequisites (`rustup`, `trunk`, node).
- Common commands (`cargo test`, `trunk serve`, smoke tests, CI).
- **"The Web Stack — For Game Devs"** section:
  - WASM ≠ Native (Bevy `App::run()` returns immediately on WASM).
  - The full message-flow diagram (phone → JS → WASM → Bevy → JS → broadcast).
  - Trunk basics (Vite-for-Rust analogy).
  - File layout table (now includes all client Rust modules).
- Detailed architecture overview: star topology, session tokens, Bevy 0.18 pull-based message system.
- The codec contract: `serde_json` lives only in `codec.rs`.
- Game-flow phases (Lobby, In-Progress, disconnect/reconnect).
- Module map with dependencies and Bevy involvement (server + client modules).
- Game-mechanic specs: ship physics, asteroid field, all four console authority rules.
- Testing strategy (Rust unit tests across all pure modules + Playwright smoke).
- Key constraints & rules (10 numbered, load-bearing).
- Cargo.toml notes: feature flags (`server`/`client`), WASM vs native targets.
- Deployed URLs.
- Quick reference for `client.html` JS patterns (now WASM-backed).
- Quick reference for `server.html` JS patterns.
- "Adding new message types" 8-step checklist.

## Why this matters for the wiki

`AGENTS.md` is a **raw source** but it overlaps heavily with the wiki's purpose: it's the briefing every new agent reads. The wiki's job is to *unpack* `AGENTS.md` into navigable, linked pages and integrate it with PRDs and design drafts.

When a future change makes `AGENTS.md` and a wiki page disagree, **`AGENTS.md` is the source of truth** and the wiki page must be updated.

## Cross-references

- Every concept page in the wiki traces back here.
- See [Architecture](../concepts/architecture.md), [Codec Seam](../concepts/codec-seam.md), [Message Flow](../concepts/message-flow.md), [Game Loop](../concepts/game-loop.md), [Testing Strategy](../concepts/testing-strategy.md).
