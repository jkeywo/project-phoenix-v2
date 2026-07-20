// Content switcher for the client GUI shell. Pure function (issues #441, #827).
//
// Maps the currently-active console to which HTML <section> should be
// visible. The section ids come from the server-supplied ship_stations
// roster (the stations this ship actually mounts), resolved through the
// canonical `${id}-ui` naming scheme in gui/mount-plan.js (which owns the
// one tactical → weapons-ui alias). The deleted gui/console-registry.js
// hardcoded list is gone: whatever the ship declares is what gets toggled.
//
// The inline `<script>` in client.html applies the returned visibility map
// by toggling `.active` on each section.

import { sectionIdFor } from './mount-plan.js';

// Returns the section id that should be visible for `activeConsole`, or null
// when there is no active console.
export function sectionForConsole(activeConsole) {
  if (!activeConsole) return null;
  return sectionIdFor(activeConsole);
}

// Returns a visibility map keyed by section id, covering exactly the
// sections of `stationIds` (the ship's mounted stations). `true` means
// "show"; at most one section is ever `true`. Lobby (`inGame === false`)
// returns all-false — client.html's `#lobby-ui` is a sibling section managed
// by the phase-toggle module, not here.
//
//   activeConsole — lowercase station id or null
//   inGame        — boolean
//   stationIds    — string[] of the ship's station ids (from
//                   uiState.shipStations.stations)
export function consoleSections(activeConsole, inGame, stationIds) {
  const out = {};
  const target = inGame ? sectionForConsole(activeConsole) : null;
  for (const id of stationIds || []) {
    const sectionId = sectionIdFor(id);
    if (sectionId) out[sectionId] = sectionId === target;
  }
  return out;
}

// Expose for non-module scripts in `client.html`.
if (typeof window !== 'undefined') {
  window.consoleSections = consoleSections;
  window.sectionForConsole = sectionForConsole;
}
