// Phone bezel image URL mapping. Pure function (issue #439).
//
// Maps a bezel slot identifier and a redAlert flag to the PNG path
// inside the shipped `gui/borders/` directory.
//
// Slots (8 total):
//   corner-tl, corner-tr, corner-bl, corner-br
//   edge-top, edge-bottom, edge-left, edge-right
//
// URL pattern:
//   alert=false → "gui/borders/phone-{slot}.png"
//   alert=true  → "gui/borders/phone-{slot}-alert.png"
//
// This module is loaded as an ES module by `client.html` (for use in the
// browser via `window.bezelSrc`) and imported by Vitest tests
// (`tests/client/phone-bezel.test.js`).

export const BEZEL_SLOTS = Object.freeze([
  'corner-tl',
  'corner-tr',
  'corner-bl',
  'corner-br',
  'edge-top',
  'edge-bottom',
  'edge-left',
  'edge-right',
]);

export function bezelSrc(slot, alert) {
  const suffix = alert ? '-alert' : '';
  return `gui/borders/phone-${slot}${suffix}.png`;
}

// Expose for non-module scripts in `client.html`.
if (typeof window !== 'undefined') {
  window.bezelSrc = bezelSrc;
  window.BEZEL_SLOTS = BEZEL_SLOTS;
}
