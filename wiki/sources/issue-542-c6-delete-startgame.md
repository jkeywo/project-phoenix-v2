---
title: Issue #542 C6 - Delete legacy StartGame
type: source
tags: [prd-519, c6, start-game, ready, lobby]
source_url: https://github.com/jkeywo/project-phoenix-v2/issues/542
status: shipped
updated: 2026-06-24
---

# Issue #542 C6 - Delete Legacy StartGame

## Status

Shipped. Parent: [PRD #519](./prd-519-player-station-ai-backfill.md).

## What Changed

`ClientMessage::StartGame` and the captain-only compat handler were removed. The only start path is now `ClientMessage::SetReady { ready }`; when all connected players are ready, the lobby transitions to `Loading` or `InProgress`.

Key code references:

- `src/core/messages.rs:1000` - `ClientMessage` contains `SetReady`, not `StartGame`.
- `src/lobby/handler.rs:291` - `SetReady` handling and auto-start.
- `src/lobby/session.rs:186` - `all_ready` over connected players.

## Post-Change Contract

UI should not send `StartGame`. Starting is collective readiness, with asset preloading represented by the `Loading` phase.
