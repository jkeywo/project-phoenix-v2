---
title: Issue #441 — Tab Bar + Content Switching
type: source
tags: [issue, client, hud, tab-bar, content-switcher, html, shipped]
source_url: https://github.com/jkeywo/project-phoenix-v2/issues/441
status: shipped
updated: 2026-06-08
---

# Issue #441 — Tab Bar + Content Switching

Third vertical slice of the [HTML/JS Client GUI Shell PRD #438](./prd-438-html-client-gui-shell.md). Replaces the Bevy `EmbeddedTabBar` (`src/client/console_shell.rs`) with an HTML/JS tab bar inside the bezel content area, plus a pure content-switcher that maps the active console to which HTML section is visible.

## Status

Shipped 2026-06-08. The Rust `console_shell.rs` is still loaded — it will be removed in [issue #442](./issue-442-bevy-cleanup.md). Both implementations co-exist during the transitional window.

## Problem

The console-select tab bar (the row of buttons letting a multi-console player switch between their stations) was rendered by Bevy WASM (`src/client/console_shell.rs`, 712 lines). Every visual change required a Rust recompile + WASM redeploy. Worse, the tab-bar widget was the parent container of *every* console panel (via `ConsoleShell::spawn()`), so removing it required reorganising the Bevy panel plugins.

Additionally, the content-section visibility logic in `client.html` `render()` was hand-rolled:

```js
$('game-ui').className = (inGame && activeConsole === 'CaptainChair') ? 'active' : '';
$('weapons-ui').className = (inGame && activeConsole === 'Tactical') ? 'active' : '';
$('repair-ui').className = (inGame && activeConsole === 'Repair') ? 'active' : '';
```

— three near-identical conditionals with no test coverage and three independent string comparisons that drifted from the canonical console name list.

## Solution

Two new pure JS modules + DOM root + CSS for the bezel-internal strip.

### `gui/tab-bar.js` (ES module)

Exports:

- `CONSOLE_LABEL` / `CONSOLE_INITIAL` — frozen full-name and short-form maps for all nine consoles.
- `INITIALS_THRESHOLD = 5` — `>= 5` consoles in portrait → use initials.
- `currentOrientation(win)` — `'portrait'` / `'landscape'` from `innerWidth > innerHeight`. Square ties go to portrait. Defaults to portrait when window is missing.
- `useInitials(consoles, orientation)` — true only when `orientation === 'portrait' && consoles.length >= INITIALS_THRESHOLD`. Landscape always returns false (vertical bar has unlimited room).
- `tabBarLayout(consoles, active, orientation, inGame)` — returns `{ hidden, orientation, useInitials, buttons: [{ console, label, active }] }`. `hidden` is true when not in-game or when `consoles.length < 2`.
- `renderTabBar(root, layout, options)` — DOM-mutating renderer. Rebuilds children from scratch on each call. Sets `root.dataset.orientation` (drives CSS), flips `style.display` and `aria-hidden`, wires `options.onPress(consoleName)` on each `<button class="tab-button">`.
- All also attached to `window.*` for the inline `<script>` in `client.html`.

### `gui/content-switcher.js` (ES module)

Exports:

- `CONSOLE_SECTION` — frozen map of the three consoles that have HTML sections (`CaptainChair → 'game-ui'`, `Tactical → 'weapons-ui'`, `Repair → 'repair-ui'`).
- `HTML_SECTION_IDS` — frozen list of the three section ids.
- `sectionForConsole(activeConsole)` — returns the section id, or `null` for Bevy-rendered consoles (Helm, Sensors, Shields, Navigation, Power, Comms).
- `consoleSections(activeConsole, inGame)` — returns `{ 'game-ui': bool, 'weapons-ui': bool, 'repair-ui': bool }`. All-false in lobby. All-false for Bevy-rendered consoles (canvas takes the bezel content area).
- `isBevyConsole(activeConsole)` — true when the console is rendered by Bevy.
- All also attached to `window.*`.

### `client.html` wiring

- `setActiveConsole(name)` — new single-source-of-truth helper near the `activeConsole` declaration. Idempotent (no-op when value is unchanged). Forwards to `wasm_client_set_active_console`. Called by the `render()` reconciler, the tab-bar onPress, and the swipe handler.
- New CSS for `#console-tab-bar` driven by `[data-orientation="portrait"|"landscape"]`. Portrait strip at the top inside the bezel inset (between the 120px-wide corners); landscape strip on the left (between the 72px-tall corners). z-index 16 (above bezel at 15, below top-bar at 20). `pointer-events: auto` (bezel itself is `pointer-events: none`).
- New DOM root `<div id="console-tab-bar" role="tablist" aria-label="Console tabs" aria-hidden="true">` between `#wasm-host` and `#console-container`.
- `render()` calls `window.consoleSections(activeConsole, inGame)` for HTML section visibility and `window.renderTabBar(tabRoot, layout, { onPress: setActiveConsole + scheduleRender })` for the tab bar.
- New `initOrientationWatch()` IIFE registers `resize` + `orientationchange` listeners that call `scheduleRender()` so the bar flips on rotation.
- Swipe handler refactored to call `setActiveConsole(mine[next])` instead of duplicating the WASM call.

### Tests

- `tests/client/tab-bar.test.js` — 36 Vitest unit tests:
  - 6 for `CONSOLE_LABEL` / `CONSOLE_INITIAL` shape + frozen-ness
  - 4 for `currentOrientation` (landscape, portrait, square, missing window)
  - 6 for `useInitials` (landscape always false, portrait threshold boundary at 5, non-arrays)
  - 5 for `tabBarLayout` hidden conditions (lobby, empty, single console, two consoles, unknown orientation default)
  - 4 for `tabBarLayout` labels (full names for 2/4, initials for 5, full names in landscape for 9)
  - 3 for `tabBarLayout` active highlight (active marks one, null active marks none, off-list active marks none)
  - 8 for `renderTabBar` DOM mutations (null root, hidden state, button construction, active class + aria, dataset.console, onPress wiring, omitted onPress, rebuild-from-scratch, hide clears buttons)
- `tests/client/content-switcher.test.js` — 20 Vitest unit tests:
  - 7 for `CONSOLE_SECTION` shape (only the three HTML consoles, correct mappings, no Bevy consoles, frozen)
  - 4 for `sectionForConsole` (three matches, Bevy consoles, null/empty, unknown strings)
  - 6 for `consoleSections` (lobby all-false, null active all-false, each of the three matches, Bevy consoles all-false)
  - 3 for `isBevyConsole` (six true, three false, null/empty/undefined)
- `vitest.config.js` already picks up `tests/client/**/*.test.js` from issue #439.

## Schema additions

- New modules: `gui/tab-bar.js`, `gui/content-switcher.js`.
- New CSS: `#console-tab-bar` (+ `[data-orientation]` selectors, `.tab-button`, `.tab-button.active`).
- New DOM root: `#console-tab-bar`.
- New JS helpers: `setActiveConsole(name)`, `initOrientationWatch()` IIFE.
- No new wire messages, no new state fields.

## Key decisions

- **Visibility driven by `aria-hidden`, not inline `style.display`.** Setting `style.display = ''` removes the inline declaration and lets the cascade re-apply — if the stylesheet has `display: none` as the default, the bar stays hidden forever. The fix is to drop the inline-style approach entirely and use `#console-tab-bar[aria-hidden="true"] { display: none; }`, then have `renderTabBar()` flip the attribute. This same latent bug existed on `#phone-bezel` from issue #439 and was fixed in the same commit (the bezel works in practice today only because no smoke test verifies the in-game phase, and the lobby path never set display in the first place).
- **`setActiveConsole(name)` as single source of truth.** Three callers (render-reconciler, swipe, tab-bar onPress) previously duplicated the `activeConsole = x; wasm_client_set_active_console(x)` pair. Now they all call one helper that's idempotent (no-ops when value is unchanged), preventing redundant WASM round-trips on no-change rerenders.
- **Tab bar inside bezel via z-index 16.** The bezel is `pointer-events: none` so the tab bar (pointer-events: auto) on top still receives taps. Sits between the bezel (15) and the top-bar (20).
- **Rebuild from scratch in `renderTabBar()`.** At most 9 buttons → diffing is overkill and risks stale-state bugs. The tab bar is unlikely to render more than once per frame thanks to `scheduleRender()`.
- **Landscape always shows full names.** Matches the Bevy implementation (`console_shell.rs:376`: `let use_initials = !embedded.is_vertical && my_consoles.len() >= 5`). Vertical strip has unlimited vertical room.
- **Portrait bar has `flex-wrap: wrap`.** Bevy parity is 9-console initials-mode at the bottom of the threshold, but a 375px iPhone with 9 initials × ~30px = 270px content versus 135px of available strip width can't fit on one line. Wrapping is a graceful fallback.
- **`role="tab"` + `aria-selected`, not `aria-pressed`.** The container has `role="tablist"`; the ARIA contract requires tab children with `aria-selected`. Mixing `aria-pressed` (toggle-button convention) inside a tablist is a contract violation that confuses screen readers.
- **`scheduleRender()` everywhere.** Both the tab-bar onPress and the swipe handler now call `scheduleRender()` instead of the synchronous `render(state)` — consistent with the message handlers, coalesces redundant repaints to one per RAF.
- **Inline fallback in `render()`.** Both `consoleSections` and `tabBarLayout` are accessed via `window.*` and the inline script has a fallback inline truth table for the section toggles (so the first paint still works if module load hasn't finished). The tab bar has a feature-test guard that simply skips rendering until the module is available — first paint without a tab bar is cosmetically acceptable since the WASM canvas is in the same place.

## Out of scope

- Bevy `ConsoleShell` / `EmbeddedTabBar` / `ConsoleShellPlugin` removal — issue #442. Every Bevy per-console panel plugin (`HelmPanelPlugin`, `WeaponsPanelPlugin`, etc.) currently calls `ConsoleShell::spawn()`; the cleanup needs to rewire those plugins to spawn into the bezel content area directly. Both implementations co-exist during the transitional window.
- Per-console-art QA inside the bar buttons (icons, colour theming per console role).
- Disabled-tab styling for consoles the player doesn't own (we just don't render them).
- Animated tab transitions.

## Cross-references

- Parent: [PRD #438 — HTML/JS Client GUI Shell](./prd-438-html-client-gui-shell.md)
- Siblings:
  - [Issue #439 — HTML Phone Bezel Frame](./issue-439-html-phone-bezel.md) — shipped.
  - [Issue #440 — Lobby Integration + Phase Toggle](./issue-440-html-lobby-phase-toggle.md) — shipped.
- Modules: `gui/tab-bar.js`, `gui/content-switcher.js`.
- Tests: `tests/client/tab-bar.test.js`, `tests/client/content-switcher.test.js`.
- Inline wiring: `client.html:646` (`setActiveConsole`), `client.html:752` (`render` reconciler + tab-bar render), `client.html:1172` (orientation watcher).
- Replaces: `src/client/console_shell.rs` (Bevy, 712 lines) — removal blocked on #442.
