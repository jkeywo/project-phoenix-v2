---
title: Issue #440 — Lobby Integration + Phase Toggle
type: source
tags: [issue, client, lobby, phase-toggle, html, shipped]
source_url: https://github.com/jkeywo/project-phoenix-v2/issues/440
status: shipped
updated: 2026-06-08
---

# Issue #440 — Lobby Integration + Phase Toggle

Second vertical slice of the [HTML/JS Client GUI Shell PRD #438](./prd-438-html-client-gui-shell.md). Merges the standalone `gui/lobby-client.html` into `client.html` as a top-level `#lobby-ui` section and replaces the ad-hoc `s.phase === 'InProgress'` check in the render loop with a pure `sectionVisibility(phase)` function.

## Status

Shipped 2026-06-08. The bulk of the lobby HTML/CSS/JS merger landed earlier in commit `7d5f0d0` ("refactor(client): merge lobby UI into client.html"). This slice adds the phase-toggle module and wires it into `render()`.

## Problem

Two problems converged:

1. **Two lobby implementations.** The phone-facing lobby lived in a separate `gui/lobby-client.html` reached via iframe, while the in-flight game shell was rendered inline by `client.html`. Crossing the lobby → game boundary was a full page swap, which discarded the active PeerJS connection and forced re-handshake.
2. **Phase gating was hard-coded.** `render()` read `const inGame = s.phase === 'InProgress'`, which correctly hid the game shell during `Lobby` but *also* hid it during `GameOver` — the player would see a blank screen instead of the post-mission summary. Worse, the test had no test coverage because it was buried in DOM-touching code.

## Solution

- **Merge `gui/lobby-client.html` into `client.html`** as a `#lobby-ui` section sitting alongside `#game-ui` / `#weapons-ui` / `#repair-ui`. Both sections live in the same document and share the single PeerJS connection. `gui/lobby-client.html` becomes a meta-refresh redirect to `client.html` so any stale bookmarks still land somewhere sensible. (Landed in `7d5f0d0`.)
- **New `gui/phase-toggle.js`** ES module exporting:
  - `IN_GAME_PHASES = ['InProgress', 'GameOver']` (frozen).
  - `isInGame(phase) → boolean`.
  - `sectionVisibility(phase) → { lobby, game, bezel }`.
  - Attaches all three to `window` for use by the inline non-module `<script>` in `client.html`.
  - Unknown phases fail-safe to lobby visibility so a future enum variant doesn't black-screen the user.
- **Wire `sectionVisibility()` into `render()`** at `client.html:648`. Falls back to inline duplicated logic if `window.sectionVisibility` hasn't loaded yet (module load can race init on slow networks — keeps first paint correct).
- **`tests/client/phase-toggle.test.js`** — 10 Vitest unit tests covering Lobby / InProgress / GameOver / unknown-phase / undefined / null / empty-string inputs, plus the frozen-array contract.

## Schema additions

- New module: `gui/phase-toggle.js` (ES module, shared with Vitest tests).
- No new wire messages or `state` fields.
- No new DOM ids beyond what `7d5f0d0` already added (`#lobby-ui`, `#station-list`, etc.).

## Key decisions

- **`GameOver` is in-game.** PRD #438 AC #7 says "the game shell is shown for InProgress *and* GameOver". The lobby is shown only for `Lobby`. Anything else falls back to lobby (fail-safe).
- **Pure function via ES module.** Same pattern as `gui/phone-bezel.js` from issue #439. Testable in isolation by Vitest with zero DOM.
- **Fallback inline logic in `render()`.** If the module hasn't loaded, duplicate the truth table inline rather than crash. Module load is async, init is sync — we don't want a 200 ms race window where the first paint is blank.
- **`#lobby-ui` keeps its id** (no rename to `#lobby-section`). Structural equivalence is what matters; renaming would risk breaking unrelated CSS selectors and Bevy-side queries during the transitional window.
- **No standalone `gui/lobby-client.html`.** Page is now a meta-refresh stub. Lobby + game share one document, one PeerJS connection.

## Out of scope

- Tab bar (issue #441).
- Bevy `LobbyRoot` / `LobbyMaterial` cleanup (issue #442).
- Persistent save/load (PRD #116).

## Cross-references

- Parent: [PRD #438 — HTML/JS Client GUI Shell](./prd-438-html-client-gui-shell.md)
- Sibling: [Issue #439 — HTML Phone Bezel Frame](./issue-439-html-phone-bezel.md) — shipped.
- Test file: `tests/client/phase-toggle.test.js` (10 tests).
- Module: `gui/phase-toggle.js`.
- Render call site: `client.html:648` (`render()` entry).
- Predecessor commit: `7d5f0d0` (refactor: merge lobby UI into client.html).
