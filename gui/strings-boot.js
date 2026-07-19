/**
 * gui/strings-boot.js — Loads assets/strings/strings.csv into gui/strings.js.
 *
 * Import this once, as early as possible, from any page that renders text:
 *
 *     import './strings-boot.js';        // side-effecting; must come first
 *     import { t } from './strings.js';
 *
 * The top-level `await` is load-bearing, not incidental. ES modules block their
 * importers until their top-level await settles, so every module downstream of
 * this one can treat `t()` as synchronous and always-ready. That matters here
 * more than usual because:
 *
 *   - the lobby renders text before the WebRTC connection exists, so we cannot
 *     hang string loading off the connection lifecycle; and
 *   - the ph-* web components build their shadow DOM in the constructor, which
 *     runs on first upgrade — far too early to await anything.
 *
 * Note this is the client's first data-file fetch. Every other asset either
 * ships inline or is fetched by server.html and pushed into WASM. If you add a
 * dynamic `import()` upstream of this module, you reintroduce the race this is
 * here to prevent, and consoles will render ⟨string.ids⟩.
 */

import { buildTable, setTable } from './strings.js';

// Resolved relative to the importing document, so this works from both the
// client root (dist/client/) and the console pages under gui/<ship>/.
const CSV_URL = new URL('../assets/strings/strings.csv', import.meta.url);

try {
  const response = await fetch(CSV_URL);
  if (!response.ok) {
    throw new Error(`HTTP ${response.status} ${response.statusText}`);
  }
  setTable(buildTable(await response.text()));
} catch (err) {
  // Leave the table empty rather than failing the module: t() degrades to
  // ⟨string.id⟩, which is a legible broken console instead of a blank page.
  console.error(`strings: failed to load ${CSV_URL} — UI will show raw ids`, err);
}
