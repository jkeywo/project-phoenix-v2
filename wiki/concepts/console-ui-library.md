---
title: Console UI Authoring Library
kind: concept
status: decided
sources:
  - wiki/sources/issue-509.md
related:
  - wiki/entities/console.md
  - wiki/concepts/game-loop.md
---

## 1. Decision

The console GUI uses the **per-console HTML pattern**: each console has its own hand-authored HTML file that owns layout, aggregation, and render logic. There is no declarative rendering engine that generates HTML from a config file.

This pattern is supported by a shared authoring library (`gui/console-ui.js`) that provides reusable DOM primitives, eliminating the repetitive boilerplate currently copy-pasted across console files.

## 2. Rationale

- Each console's layout is meaningfully different (helm needs a radar widget; weapons needs a banks list and a tube list sharing magazine state; shields needs an SVG arc display). A declarative engine would need to be as expressive as HTML to cover all cases.
- The aggregation problem (torpedo tubes + magazine as one panel, phaser banks as a list) is solved by the console author in `_renderConsole`, not by the engine — the author decides which system states to combine and what the conditional logic is.
- The AUTO badge pattern (show when system is AI-controlled) is a small cross-cutting concern that `console-ui.js` handles via `setAutoState(button, badge, isAuto)`.

## 3. The `gui/console-ui.js` library

Six primitives are provided:

### `reconcileRows(container, newIds, cache, buildFn, updateFn)`

Stable-ref keyed reconciliation. Removes stale rows, builds new rows with `buildFn(id)` (returns `{ row, ...elements }`), re-appends in order, calls `updateFn(id, elements, datum)` for every row. Safe for mid-click updates — existing DOM nodes are never destroyed while visible. Pattern from `gui/weapons-console.html` tubes/banks.

### `keyedRebuild(container, newKey, buildFn)`

Full-rebuild guard. When `newKey` differs from the last key, wipes and rebuilds the subtree via `buildFn()`. Otherwise no-op. Pattern from `gui/power-console.html` and `gui/repair-console.html`.

### `setBtn(el, { enabled, active, variant })`

Unified button state. Sets `el.disabled`, applies variant class (`danger`, `armed`, `disabled`, etc.), toggles `active` class. Replaces the three inconsistent approaches currently used (native `disabled` in weapons, CSS class swap in power/helm, mixed in captain).

### `setBar(fillEl, fraction, { thresholds })`

Fill bar update. Sets `style.width` to `(fraction * 100).toFixed(1) + '%'` and applies a threshold class (`crit`/`warn`/`''`). Generalises `renderDamageBar` from `console-core.js` to any progress bar (cooldown, power, impulse, team progress).

### `setAutoState(button, badge, isAuto)`

AI-control indicator. Sets `button.disabled = isAuto`, `button.classList.toggle('readonly', isAuto)`, `badge.hidden = !isAuto`. Extracted from `gui/system-registry.js:26–32`. Will be used by every console that gains an AI-automatable rating.

### `setText(el, text, { cls, color })`

Text content + optional class/colour update. Extracted from repair console's private `_setTxt`.

## 4. Usage example

The torpedo tubes panel motivates the discussion. The console author owns what gets built and what conditions enable each button; the library owns the reconciliation mechanics.

```js
// In weapons-console.html _renderTorpedo(s):
import { reconcileRows, setBtn, setText } from './console-ui.js';

var _tubeCache = {};

function _renderTorpedo(s) {
  var mag = s.torpedo_magazine;
  setText(magCountEl, mag.current + ' / ' + mag.max);

  reconcileRows(tubeList, s.tubes.map(t => t.id), _tubeCache,
    function build(id) {
      var row = /* createElement ... */;
      var loadBtn   = row.querySelector('.load-btn');
      var unloadBtn = row.querySelector('.unload-btn');
      var fireBtn   = row.querySelector('.fire-btn');
      loadBtn.addEventListener('click',   () => sendAction('load_tube',    { tube: id }));
      unloadBtn.addEventListener('click', () => sendAction('unload_tube',  { tube: id }));
      fireBtn.addEventListener('click',   () => sendAction('fire_torpedo', { tube: id }));
      return { row, loadBtn, unloadBtn, fireBtn };
    },
    function update(id, els, tube) {
      setBtn(els.loadBtn,   { enabled: !tube.is_loaded && mag.current > 0 });
      setBtn(els.unloadBtn, { enabled:  tube.is_loaded });
      setBtn(els.fireBtn,   { enabled:  tube.is_loaded && s.helm_radar_valid_target, variant: 'danger' });
    }
  );
}
```

## 5. Migration strategy

- `gui/console-ui.js` is created as part of issue #510.
- Migration of existing consoles is incremental. New consoles (e.g. fine-system Helm from #511) use it from the start.
- `renderDamageBar` in `console-core.js` is superseded by `setBar` but kept for backward compat until all callers are migrated.

## 6. What this decision rules out

- A declarative config-file-driven renderer that generates HTML (too rigid for the layout diversity across consoles).
- A generalised fragment dispatch engine that auto-calls registered fragments (the per-console import-and-call pattern is simpler and already works for captain's AUTO badges).
