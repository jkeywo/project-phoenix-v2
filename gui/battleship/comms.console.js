/**
 * gui/battleship/comms.console.js — the Harrow battleship's Comms seat
 * (issue #1235). The reference hull: a single-family flat `comms` payload,
 * the contact/hail/message core, and a hail-name footer. No bespoke tail
 * beyond that footer text.
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
    stationDamage: 'station-damage',
    autoBadge: 'comms-auto-badge',
  },
  tail: (s, view, doc, t) => {
    const el = doc.getElementById('footer-target');
    if (!el) return;
    const msgs = view.messages || [];
    const threadMsg = msgs.find((m) => !m.is_read) || msgs[msgs.length - 1] || null;
    el.textContent = threadMsg ? (threadMsg.sender_name || t('console.common.active_hail')) : t('console.common.no_active_hail');
  },
});
