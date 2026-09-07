/**
 * gui/cruiser/comms.console.js — the cruiser's Comms seat AND its auxiliary
 * Navigation seat, one document (issues #1235, #1379).
 *
 * alliance_cruiser.toml points TWO Stations at this file: "comms" (primary)
 * and the auxiliary "navigation" — two different crew members can hold
 * either one, and both load this same HTML. `buildConsoleStateInner`'s
 * single-family builder identifies the owned family in `system_ids` and
 * `system_families`. A human Comms holder can also host visiting Navigation,
 * whose keyed data joins the payload without changing those authored ids.
 * The owned family decides which full-panel `.view` is on screen — see
 * `tail` below — rather
 * than trusting the `console: 'comms'` name this file's own `initConsole`
 * call still hardcodes for outbound correlation (pre-existing; unaffected by
 * which Station actually mounted this load).
 *
 * The Navigation view reuses `makeNavigationRender` — the SAME renderer
 * `gui/battleship/navigation.console.js` drives, configured for this
 * document's ids — so both documents' charts, metrics and civilian-traffic
 * panel stay in lockstep by construction. The Comms view is the shared
 * `makeCommsRender` core (issue #1380) — HAILS | CONTACTS over one list area,
 * one row per thread, and the open thread as a column at width or a local
 * `.overlay-panel` in phone portrait. This hull's `tail` therefore carries
 * only what that core does not: the Navigation view, the Comms message count,
 * and the switch deciding which of the two views is on screen.
 */
import { makeCommsRender } from '../stations/comms-console.js';
import { makeNavigationRender } from '../stations/navigation-console.js';
import { familyView } from '../console-payload.js';

/**
 * Resolve a Console Family view, shape-agnostic (issue #1379 review finding
 * 1). `buildConsoleStateInner`'s single-owned-system station returns a FLAT
 * payload — `system_ids`/`system_families` present, no `.systems` — so
 * `familyView` alone (which only reads `payload.systems[id]`) resolves empty
 * even when this payload's own single owned system IS `family`. Fall back to
 * the payload itself once `system_families` says so; a payload with no
 * projected metadata at all (pre-Welcome) still resolves to `{}` from
 * `familyView`, and `system_families` is then also empty, so the fallback
 * stays `{}` too — never invents a family from an absent projection.
 */
function resolveFamilyView(s, family) {
  const keyed = familyView(s, family);
  if (Object.keys(keyed).length > 0) return keyed;
  const families = (s && s.system_families) || {};
  return Object.values(families).includes(family) ? s : {};
}

const renderNavigationView = makeNavigationRender({
  ids: {
    map: 'navigation-map',
    civilianTraffic: 'civilian-traffic',
    objectiveList: 'objective-list',
    contactCount: 'nav-contact-count',
    waypointName: 'waypoint-name',
    onScreenBtn: 'btn-on-screen',
    autoBadge: 'navigation-auto-badge',
  },
});

export const renderStation = makeCommsRender({
  commsView: (s) => resolveFamilyView(s, 'comms'),
  ids: {
    contactList: 'comms-contact-list',
    hailList: 'comms-hail-list',
    currentMessage: 'comms-current-message',
    hailsUnread: 'comms-hails-unread',
    activeHail: 'footer-target',
    threadPanel: 'comms-thread-panel',
    autoBadge: 'comms-auto-badge',
  },
  // Station badge: the retired composite's comms_auto meant "station is
  // Backfill-rated". The per-system equivalent is every owned system
  // AI-run — the conjunction of the resolved views' *_auto flags. Both
  // flags come from controlSources on the generic composed path
  // (navigation_auto included, issue #825), which can lag a rating
  // change by one tick.
  autoState: (s, view) => !!(view.comms_auto && resolveFamilyView(s, 'navigation').navigation_auto),
  tail: (s, view, doc, t) => {
    const nav = resolveFamilyView(s, 'navigation');
    renderNavigationView(nav, doc);

    // `#footer-target` is the Comms view's own readout and the shared core
    // fills it with the open thread's channel (issue #1380). It used to carry
    // the WAYPOINT, from the era when this one seat was Comms and Navigation
    // at once; since #1379 split them into two full-panel views that readout
    // lived inside the Comms view and could only ever say NO WAYPOINT, because
    // a Comms load resolves no navigation family at all. The Navigation view
    // keeps its own `#waypoint-name` metric — each tab says what it shows.

    const msgCount = (view.messages || []).length;
    const footerRightEl = doc.getElementById('footer-right');
    if (footerRightEl) {
      footerRightEl.textContent = msgCount === 1
        ? t('console.comms.messages.one', { n: 1 })
        : t('console.comms.messages.other', { n: msgCount });
    }

    // A visiting Navigation view must not replace this Comms Station's view.
    // system_ids preserves authored ownership when withVisitingSystems adds
    // its keyed data; the separate Navigation tab owns its own authored ids.
    const hasNav = (s.system_ids || []).some(id => s.system_families?.[id] === 'navigation');
    const navViewEl = doc.getElementById('nav-view');
    const commsViewEl = doc.getElementById('comms-view');
    if (navViewEl) navViewEl.hidden = !hasNav;
    if (commsViewEl) commsViewEl.hidden = hasNav;
  },
});
