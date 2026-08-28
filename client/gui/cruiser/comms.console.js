/**
 * gui/cruiser/comms.console.js — the cruiser's Comms seat (issue #1235).
 *
 * The cruiser's Comms Station absorbs Navigation (issue #825's
 * SystemStationConsolePayload composes both under `s.systems`) — its own
 * seat elsewhere on the battleship. `commsView`/`navView` are both
 * metadata-selected family slices; the footer reads the Nav waypoint (not the hail,
 * unlike the battleship), and a message-count readout sits in `footer-right`.
 * The Nav map + its overlay clone, and the AUTO-badge conjunction of both
 * families' own auto flags (issue #825's controlSources composition), are
 * this hull's bespoke tail.
 */
import { makeCommsRender } from '../stations/comms-console.js';
import { familyView } from '../console-payload.js';

export const renderStation = makeCommsRender({
  commsView: (s) => familyView(s, 'comms'),
  ids: {
    contactList: 'comms-contact-list',
    hailList: 'comms-hail-list',
    currentMessage: 'comms-current-message',
    stationDamage: 'station-damage',
    autoBadge: 'comms-auto-badge',
  },
  // Station badge: the retired composite's comms_auto meant "station is
  // Backfill-rated". The per-system equivalent is every owned system
  // AI-run — the conjunction of the resolved views' *_auto flags. Both
  // flags come from controlSources on the generic composed path
  // (navigation_auto included, issue #825), which can lag a rating
  // change by one tick.
  autoState: (s, view) => !!(view.comms_auto && familyView(s, 'navigation').navigation_auto),
  tail: (s, view, doc, t) => {
    const nav = familyView(s, 'navigation');
    const navState = {
      blips: nav.blips || [], regions: nav.regions || [], range: nav.radar_range || 5000,
      ship_pos: { x: nav.ship_x || 0, z: nav.ship_z || 0 }, ship_heading: nav.ship_heading || 0,
      waypoint: nav.waypoint || null,
    };
    const nmEl = doc.getElementById('navigation-map');
    if (nmEl) nmEl.state = navState;
    const nmOvEl = doc.getElementById('nav-overlay-map');
    if (nmOvEl) nmOvEl.state = navState;

    const wp = nav.waypoint;
    const footerEl = doc.getElementById('footer-target');
    if (footerEl) footerEl.textContent = wp ? (wp.name || t('console.common.waypoint')) : t('console.common.no_waypoint');

    const msgCount = (view.messages || []).length;
    const footerRightEl = doc.getElementById('footer-right');
    if (footerRightEl) {
      footerRightEl.textContent = msgCount === 1
        ? t('console.comms.messages.one', { n: 1 })
        : t('console.comms.messages.other', { n: msgCount });
    }
  },
});
