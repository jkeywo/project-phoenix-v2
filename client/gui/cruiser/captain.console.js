/**
 * gui/cruiser/captain.console.js — the cruiser's Captain seat (issue #1235).
 *
 * A single-family flat `captain` payload, same as the battleship, but with
 * no AUTO badge in this hull's markup and a contact-count footer that tints
 * by contact count (the shared core's `footer.colorize`).
 */
import { makeCaptainRender } from '../stations/captain-console.js';

export const renderStation = makeCaptainRender({
  // Flat `captain` family: the panels read fields straight off the payload.
  ids: {
    camera: 'camera-select',
    redAlert: 'red-alert',
    objectives: 'objective-list',
    stationDamage: 'station-damage',
  },
  footer: { id: 'footer-target', colorize: true },
});
