/**
 * gui/battleship/comms.console.js — the Harrow battleship's Comms seat
 * (issue #1235). The reference hull: a single-family flat `comms` payload and
 * nothing but the shared core — the HAILS | CONTACTS pair, the open thread and
 * the active-hail readout. It has no tail at all (issue #1380 moved the
 * readout into the core, where both hulls now spell it the same way).
 *
 * The `.html` imports `renderStation` and hands it to `initConsole`; a
 * vitest suite imports the same `renderStation`.
 */
import { makeCommsRender } from '../stations/comms-console.js';

export const renderStation = makeCommsRender({
  // Flat `comms` family: the panels read fields straight off the payload.
  ids: {
    contactList: 'comms-contact-list',
    hailList: 'comms-hail-list',
    currentMessage: 'comms-current-message',
    hailsUnread: 'comms-hails-unread',
    activeHail: 'footer-target',
    threadPanel: 'comms-thread-panel',
    autoBadge: 'comms-auto-badge',
  },
});
