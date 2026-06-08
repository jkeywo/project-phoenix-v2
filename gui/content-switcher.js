// Content switcher for the client GUI shell. Pure function (issue #441).
//
// Maps the currently-active console to which HTML <section> should be
// visible. Three consoles have HTML sections in client.html
// (CaptainChair -> #game-ui, Tactical -> #weapons-ui, Repair -> #repair-ui);
// the other six are rendered by the Bevy WASM canvas, so when one of those
// is active no HTML section is shown — the canvas takes the whole bezel
// content area.
//
// This module exports a pure function `consoleSections(activeConsole, inGame)`
// returning a visibility map keyed by section id. The inline `<script>` in
// client.html applies it by toggling `.active` on each section.
//
// The canvas itself is always in the DOM; visibility of the iframe is what
// changes here. When Tactical is active the iframe sits on top of the canvas
// at z-index 10.

// Console name -> HTML section id. Only consoles with HTML panels are keyed.
// Other consoles (Helm, Sensors, Shields, Navigation, Power, Comms) are
// rendered by Bevy and have no section in the map.
export const CONSOLE_SECTION = Object.freeze({
  CaptainChair: 'game-ui',
  Tactical: 'weapons-ui',
  Repair: 'repair-ui',
});

// Set of all known section ids that the switcher will reset.
export const HTML_SECTION_IDS = Object.freeze(['game-ui', 'weapons-ui', 'repair-ui']);

// Returns the section id that should be visible for `activeConsole`, or null
// if no HTML section maps to this console (Bevy renders it).
export function sectionForConsole(activeConsole) {
  if (!activeConsole) return null;
  return CONSOLE_SECTION[activeConsole] || null;
}

// Returns a visibility map keyed by section id. `true` means "show".
// Only one section is ever `true` at a time; rest are `false`. Lobby (when
// `inGame === false`) returns all-false — `client.html`'s `#lobby-ui` is a
// sibling section managed by the phase-toggle module, not here.
export function consoleSections(activeConsole, inGame) {
  const out = {};
  const target = inGame ? sectionForConsole(activeConsole) : null;
  for (const id of HTML_SECTION_IDS) out[id] = id === target;
  return out;
}

// True when the active console is rendered by Bevy (no HTML section).
// Useful for the inline script to decide whether the canvas should be
// raised above the (hidden) HTML sections.
export function isBevyConsole(activeConsole) {
  if (!activeConsole) return false;
  return !(activeConsole in CONSOLE_SECTION);
}

// Expose for non-module scripts in `client.html`.
if (typeof window !== 'undefined') {
  window.consoleSections = consoleSections;
  window.sectionForConsole = sectionForConsole;
  window.isBevyConsole = isBevyConsole;
  window.CONSOLE_SECTION = CONSOLE_SECTION;
  window.HTML_SECTION_IDS = HTML_SECTION_IDS;
}
