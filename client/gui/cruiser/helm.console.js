/**
 * gui/cruiser/helm.console.js — the cruiser's Helm seat (issue #1235).
 *
 * Adds the lateral-thrust joystick and the zero-contacts-fallback,
 * glyph-prefixed, tint-by-count footer the battleship's Helm does not
 * carry. No bespoke tail (Dock / under-tow-load are destroyer-only panels).
 */
import { makeHelmRender } from '../stations/helm-console.js';

export const renderStation = makeHelmRender({
  ids: {
    radar: 'helm-radar',
    joystick: 'helm-joystick',
    lateral: 'lateral-thrust-joystick',
    impulse: 'impulse-btn',
    boost: 'boost-btn',
    stationDamage: 'station-damage',
    autoBadge: 'helm-auto-badge',
  },
  footer: { id: 'footer-target', zeroFallback: true, glyph: true, colorize: true },
});
