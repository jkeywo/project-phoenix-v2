---
title: PRD #438 — HTML/JS Client GUI Shell
type: source
tags: [prd, client, hud, html, shell, phase-toggle, tab-bar, shipped]
source_url: https://github.com/jkeywo/project-phoenix-v2/issues/438
status: shipped
updated: 2026-06-08
---

# PRD #438 — HTML/JS Client GUI Shell

Replaces the Bevy-rendered phone bezel, console-select tab bar, and lobby UI on `client.html` with equivalent HTML/CSS/JS. The Bevy WASM canvas shrinks from a full-viewport UI surface to a content-area hole that renders only per-console panels.

## Status

Shipped 2026-06-08. All four slices landed in series:

- [Issue #439 — HTML Phone Bezel Frame](./issue-439-html-phone-bezel.md) — shipped 2026-06-08.
- [Issue #440 — Lobby Integration + Phase Toggle](./issue-440-html-lobby-phase-toggle.md) — shipped 2026-06-08 (bulk landed in commit `7d5f0d0`; phase-toggle module + wiring landed in `becdffa`).
- [Issue #441 — Tab Bar + Content Switching](./issue-441-html-tab-bar-content-switching.md) — shipped 2026-06-08 (commit `7d45e09`).
- [Issue #442 — Bevy Cleanup (lobby, tab bar, border)](./issue-442-bevy-cleanup.md) — shipped 2026-06-08. Three Rust files (`console_shell.rs`, `phone_border/framing.rs`, `app.rs`) shrunk from ~2700 → ~900 lines; Bevy client now spawns only the UI camera + nine per-console panel roots.

## Problem

The Bevy-rendered phone bezel, tab bar, and lobby UI were rigid: every visual change required a Rust recompile and WASM deploy, and the lobby/console-shell code lived inside Bevy's widget tree where it couldn't be iterated on independently. The tactical console already proved the HTML approach works via its iframe (`gui/weapons-console.html`).

## Solution

Two top-level sections in `client.html`:

- **Lobby section** — borderless. Ship header, station list with claim/release, complexity controls, crew count, engage button. Sourced from `gui/lobby-client.html`.
- **Game section** — framed by the HTML phone-bezel. Embedded console-select tab bar. Content area shows either the Bevy canvas (for non-Tactical consoles) or the tactical iframe.

Single PeerJS connection serves both sections. Single `state` object drives all rendering. No new wire messages.

## Module breakdown (per PRD)

1. **Bezel Renderer** (`gui/phone-bezel.js`) — `bezelSrc(slot, alert) → URL`. Issue #439.
2. **Tab Bar Renderer** — pure function: `(consoles, active, orientation) → DOM`. Issue #441.
3. **Phase Toggle** — pure function: `(phase) → section visibility`. Issue #440.
4. **Content Switcher** — pure function: `(activeConsole) → canvas-vs-iframe visibility`. Issue #441.
5. **Red Alert Wire** — one-line addition reading `snap.red_alert`. Issue #439.
6. **Lobby Merge** — inline lobby HTML/CSS/JS into `client.html`. Issue #440.
7. **Bevy Cleanup** — remove `LobbyRoot`, `EmbeddedTabBar`, `PhoneBorderPlugin`, `BorderAssets`. Issue #442.
8. **Border Templates** — `gui/captain-{landscape,portrait}.html` reference files. (Optional design QA.)

## Key decisions

- **No new message types.** Existing `ClientMessage`/`ServerMessage` protocol is sufficient.
- **Tactical iframe keeps its `postMessage` bridge** (`gui/weapons-console.html` ↔ `client.html`).
- **Bevy canvas remains in the DOM at all times.** z-index management handles visibility.
- **Pure JS functions tested with Vitest** alongside the existing editor tests; new `tests/client/` directory.

## Out of scope

- Captain-specific buttons (view-selector cross and red-alert toggle) — stay in Bevy for now.
- Migrating other Bevy panels to HTML (except Tactical, already HTML).
- Server-side viewscreen border.
- Persistent save/load (PRD #116).
- Changing the wire protocol.

## Cross-references

- Child issues: [#439 (bezel)](./issue-439-html-phone-bezel.md), [#440 (lobby)](./issue-440-html-lobby-phase-toggle.md), [#441 (tab bar)](./issue-441-html-tab-bar-content-switching.md), [#442 (Bevy cleanup)](./issue-442-bevy-cleanup.md).
- Predecessor (Rust implementation, now superseded): [PRD #187 — Phone Console HUD](./prd-187-phone-console-hud.md).
- Sibling (server-side, unaffected): [PRD #180 — Viewscreen Frame](./prd-180-viewscreen-frame.md).
- ADR-0001 — `__sendAction` / `__updateConsole` bridge contract.
