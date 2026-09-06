/**
 * gui/cruiser/comms.console.js — the cruiser's Comms seat AND its auxiliary
 * Navigation seat, one document (issues #1235, #1379).
 *
 * alliance_cruiser.toml points TWO Stations at this file: "comms" (primary)
 * and the auxiliary "navigation" — two different crew members can hold
 * either one, and both load this same HTML. `buildConsoleStateInner`'s
 * single-family builder means the payload a given load of this document
 * receives carries exactly ONE of the two families' data, never both: a
 * `familyView(s, 'comms')`/`familyView(s, 'navigation')` call resolves
 * non-empty only for the family this particular Station actually owns (see
 * `gui/console-payload.js`). That is also how this document decides which
 * of its two full-panel `.view`s is on screen — see `tail` below — rather
 * than trusting the `console: 'comms'` name this file's own `initConsole`
 * call still hardcodes for outbound correlation (pre-existing; unaffected by
 * which Station actually mounted this load).
 *
 * The Navigation view reuses `makeNavigationRender` — the SAME renderer
 * `gui/battleship/navigation.console.js` drives, configured for this
 * document's ids — so both documents' charts, metrics and civilian-traffic
 * panel stay in lockstep by construction. The Comms view is the pre-#1379
 * `makeCommsRender` tail, unchanged pending #1380.
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

    // Which full-panel view is on screen (issue #1379): whichever family
    // view actually resolved non-empty says which real Station this load is
    // — never a client-local toggle (see the module doc comment above).
    const hasNav = Object.keys(nav).length > 0;
    const navViewEl = doc.getElementById('nav-view');
    const commsViewEl = doc.getElementById('comms-view');
    if (navViewEl) navViewEl.hidden = !hasNav;
    if (commsViewEl) commsViewEl.hidden = hasNav;
  },
});
