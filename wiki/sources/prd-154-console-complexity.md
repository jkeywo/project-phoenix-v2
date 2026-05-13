---
title: PRD #154 — Console Complexity: UI Hiding + AI Automation
type: source
tags: [prd, console, complexity, ai, ux, shipped]
source_url: https://github.com/jkeywo/project-phoenix-v2/issues/154
status: shipped
updated: 2026-05-13
---

# PRD #154 — Console Complexity: UI Hiding + AI Automation

Per-console `Low` / `Full` complexity presets. At `Low`, advanced controls are hidden from the UI and the server's `console_ai` module operates them silently. At `Full`, all controls are visible and human-driven.

## Status

Shipped (2026-05-12). Presets live in `assets/complexity/*.toml`. `SetComplexity` and `ComplexityChanged` are on the wire.

## Problem

Six consoles each carrying their own deep mechanic (phaser frequency match, torpedo loading, power overflow, repair shape, impulse charging) is too much for a casual three-player session. Hardcoding "easy mode" into every console individually would be brittle; cutting features for everyone would alienate experienced crews.

## Solution

- **Per-console complexity presets** in `assets/complexity/<console>.toml`. Each preset declares which UI elements are visible and which AI subroutines run server-side.
- **`SetComplexity { console, preset_name }`** — any player at that console may switch. **`ComplexityChanged`** broadcast so other clients re-render labels.
- **`console_ai` module** — server-side. Reads world state and emits the same `ClientMessage` types a human would (e.g. auto-fire torpedoes when a tube is loaded and a target is locked, auto-match phaser frequency when the bank fires, auto-shed power overflow). Shares its action vocabulary with PRD #142 NPCs.
- **Two presets per console** to start: `Low` and `Full`. Schema supports more.

## Key decisions

- **Server is the source of truth.** Client renders whatever the server's `ComplexityChanged` says.
- **AI emits player messages.** No "AI override" wire — the same paths a human uses.
- **Default per console** is read from each preset file's `default = true`.

## Schema additions

- `assets/complexity/<console>.toml` — visible controls + AI tuning per preset.
- `messages.rs`: `ClientMessage::SetComplexity { console, preset_name }`, `ServerMessage::ComplexityChanged { token, console, preset_name }`.
- `console_ai.rs` (server-only) — per-console AI subroutines.

## Out of scope

- Mid-mission complexity ramping (manual switch only).
- Per-mechanic granularity beyond the named presets.
- Tutorialisation of hidden controls.

## Cross-references

- [Draft 11 — Console Complexity](./design-11-console-complexity.md) (superseded by this PRD)
- Shares action vocabulary with [PRD #142 — AI and Behaviour](./prd-142-ai-and-behaviour.md)
- [Console](../entities/console.md)
- [Roadmap Overview](../roadmap/overview.md)
