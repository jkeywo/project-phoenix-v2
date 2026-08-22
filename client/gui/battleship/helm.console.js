/**
 * gui/battleship/helm.console.js — the battleship's Helm seat (issue #1235).
 *
 * The plain case: no lateral-thrust joystick, a contact-count footer with
 * no zero-contacts fallback, no bespoke tail.
 */
import { makeHelmRender } from '../stations/helm-console.js';

export const renderStation = makeHelmRender({
  ids: {
    radar: 'helm-radar',
    joystick: 'helm-joystick',
    impulse: 'impulse-btn',
    boost: 'boost-btn',
    stationDamage: 'station-damage',
    autoBadge: 'helm-auto-badge',
  },
  footer: { id: 'footer-target' },
});
