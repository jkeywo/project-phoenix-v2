/**
 * gui/cruiser/captain.console.js — the cruiser's Captain seat (issue #1235).
 *
 * A single-family flat `captain` payload, same as the battleship, but with
 * no AUTO badge in this hull's markup and a contact-count footer that tints
 * by contact count (the shared core's `footer.colorize`). Issue #1392 adds
 * the Mission column's deadline clock: `s` is already the flat payload the
 * core reads objectives from, so the tail needs no `familyView` lookup —
 * unlike the destroyer, which is system-id-keyed.
 */
import { makeCaptainRender } from '../stations/captain-console.js';

export const renderStation = makeCaptainRender({
  // Flat `captain` family: the panels read fields straight off the payload.
  ids: {
    camera: 'camera-select',
    redAlert: 'red-alert',
    objectives: 'objective-list',
  },
  footer: { id: 'footer-target', colorize: true },
  tail: (s, view, doc) => {
    const deadlineEl = doc.getElementById('deadline-list');
    if (deadlineEl) deadlineEl.state = { deadlines: view.deadlines || [] };
  },
});
