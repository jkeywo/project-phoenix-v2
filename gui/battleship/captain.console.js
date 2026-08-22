/**
 * gui/battleship/captain.console.js — the Harrow battleship's Captain seat
 * (issue #1235). The reference hull: a single-family flat `captain` payload
 * — the battleship's Captain station does not own a sensors system (Sensors
 * is its own dedicated Station) — the shared core, and a plain contact-count
 * footer with no zero-contacts fallback text tint. The bespoke tail drives
 * three hidden `#objectives`/`#dir`/`#alert` elements that mirror render
 * output as plain `data-*` attributes — a Playwright smoke-test seam no
 * other hull's markup carries.
 *
 * The `.html` imports `renderStation` and hands it to `initConsole`; a
 * vitest suite imports the same `renderStation`.
 */
import { makeCaptainRender } from '../stations/captain-console.js';

export const renderStation = makeCaptainRender({
  // Flat `captain` family: the panels read fields straight off the payload.
  ids: {
    camera: 'camera-select',
    redAlert: 'red-alert',
    objectives: 'objective-list',
    stationDamage: 'station-damage',
    autoBadge: 'captain-auto-badge',
  },
  footer: { id: 'footer-target' },
  tail: (s, view, doc) => {
    // Playwright smoke-test seam (predates this refactor): a plain
    // `data-*`-attribute mirror of the objectives/view-direction/red-alert
    // state, readable without reaching into the ph-objective-list shadow root.
    const objectivesEl = doc.getElementById('objectives');
    if (objectivesEl) {
      objectivesEl.replaceChildren();
      (view.objectives || []).forEach((o) => {
        const row = doc.createElement('div');
        row.className = 'objective-data';
        row.dataset.id = o.id || '';
        row.dataset.text = o.text || '';
        row.dataset.status = o.status || '';
        objectivesEl.appendChild(row);
      });
    }
    const dirEl = doc.getElementById('dir');
    if (dirEl) dirEl.dataset.direction = view.view_direction || '';
    const alertEl = doc.getElementById('alert');
    if (alertEl) alertEl.dataset.redAlert = String(!!view.red_alert);
  },
});
