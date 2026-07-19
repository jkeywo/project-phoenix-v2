/**
 * gui/strings-boot.js — Loads assets/strings/strings.csv into gui/strings.js.
 *
 * Import this (side-effecting) before using t() from any module whose
 * *evaluation* renders text — web components building shadow templates in
 * their constructor, frozen label maps, applyToDom callers:
 *
 *     import './strings-boot.js';
 *     import { t } from './strings.js';
 *
 * The load is a SYNCHRONOUS XMLHttpRequest, on purpose. This page is plain ES
 * modules plus large classic inline scripts, and three constraints collide:
 *
 *   - ph-* components build their shadow DOM in the constructor, which runs
 *     the moment their module evaluates and the parsed elements upgrade;
 *   - console pages install `window.__updateConsole` during module evaluation,
 *     and the host/tests push state as soon as the load event fires;
 *   - a top-level `await fetch(...)` here delays every downstream module past
 *     the load event, so early state pushes hit un-upgraded elements whose
 *     `.state =` property assignment shadows the class accessor — silently
 *     dead consoles (caught by the smoke suite).
 *
 * Sync XHR keeps the whole module graph synchronous: by the time any importer
 * evaluates, the table is populated. The file is small (~90KB) and served
 * same-origin, so the one-time parse-blocking cost is a few milliseconds.
 * The deprecation warning in DevTools is the accepted price.
 *
 * In Node (vitest) there is no XMLHttpRequest; tests load the table from disk
 * via tests/client/setup-strings.js instead, so this module is a no-op there.
 */

import { buildTable, setTable } from './strings.js';

// Resolved relative to this module, so this works from both the client root
// (dist/client/) and the console pages under gui/<ship>/.
const CSV_URL = new URL('../assets/strings/strings.csv', import.meta.url);

if (typeof XMLHttpRequest !== 'undefined' && typeof document !== 'undefined') {
  try {
    const xhr = new XMLHttpRequest();
    xhr.open('GET', CSV_URL, false); // false = synchronous, see header comment
    xhr.send();
    if (xhr.status >= 200 && xhr.status < 300) {
      setTable(buildTable(xhr.responseText));
    } else {
      throw new Error(`HTTP ${xhr.status}`);
    }
  } catch (err) {
    // Leave the table empty rather than failing the module: t() degrades to
    // ⟨string.id⟩, which is a legible broken console instead of a blank page.
    console.error(`strings: failed to load ${CSV_URL} — UI will show raw ids`, err);
  }
}
