// Content switcher for the client GUI shell. Pure function (issue #441).
//
// Maps the currently-active console to which HTML <section> should be
// visible. All nine consoles have HTML sections in client.html
// (captain -> #captain-ui, helm -> #helm-ui, tactical -> #weapons-ui,
// repair -> #repair-ui, power -> #power-ui, shields -> #shields-ui,
// sensors -> #sensors-ui, navigation -> #navigation-ui, comms -> #comms-ui).
//
// This module exports a pure function `consoleSections(activeConsole, inGame)`
// returning a visibility map keyed by section id. The inline `<script>` in
// client.html applies it by toggling `.active` on each section.
//
// The canvas itself is always in the DOM; visibility of the iframe is what
// changes here. When Tactical is active the iframe sits on top of the canvas
// at z-index 10.

import { REGISTRY } from './console-registry.js';

// Console name -> HTML section id. Derived from REGISTRY so there is a single
// source of truth for all HTML-panel consoles.
//
// Two-key map: the primary keys are the lowercase station ids (post issue
// #618, matching REGISTRY). Extra PascalCase Console-enum aliases are added
// so callers that still pass PascalCase names — chiefly
// `sectionForConsole(activeConsole)` where `activeConsole` comes from
// `player.consoles` (a PascalCase wire field retained pending issue #619) —
// resolve to the same section id without any per-caller translation.
const _LOWERCASE_SECTION = Object.fromEntries(
  Object.entries(REGISTRY).map(([k, v]) => [k, v.sectionId])
);
const _PASCAL_TO_STATION = Object.freeze({
  CaptainChair: 'captain',
  Helm: 'helm',
  Tactical: 'tactical',
  Repair: 'repair',
  Sensors: 'sensors',
  Shields: 'shields',
  Navigation: 'navigation',
  Power: 'power',
  Comms: 'comms',
});
const _PASCAL_SECTION = Object.fromEntries(
  Object.entries(_PASCAL_TO_STATION).map(([pascal, station]) => [pascal, _LOWERCASE_SECTION[station]])
);
export const CONSOLE_SECTION = Object.freeze({ ..._LOWERCASE_SECTION, ..._PASCAL_SECTION });

// Set of all known section ids that the switcher will reset.
export const HTML_SECTION_IDS = Object.freeze(
  Object.values(REGISTRY).map(v => v.sectionId)
);

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
