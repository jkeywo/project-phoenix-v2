/**
 * gui/gm-health-panel.js — the readable public peer/Station/tick-health panel
 * (issue #1437, PRD #1419 story 10, presentation contract PRD #1418).
 *
 * One place to answer "who is here, who has a human on their Station, and is
 * anything behind". It reads the `gm_health` Host Channel through the shared
 * adapter in `gui/gm-health-banner.js` and renders three groups — peers,
 * Stations, Game Masters — each row stating its condition as a translated
 * WORD, never as a colour alone.
 *
 * # This panel is a reader, not a control
 *
 * There is no button on a row. Nothing here disconnects a peer, frees a
 * Station, evicts a Session or ends a recovery: issue #1437 observes the
 * membership and recovery machinery the lockstep layer already owns, and the
 * automatic-removal policy a live restore needs is M5's to design. That is also
 * why every row is plain text: a repainting list with no focusable controls in
 * it cannot move the target out from under a keyboard operator (PRD #1418
 * story 23), and the one interactive surface — the technical banner's action —
 * lives in the banner region beside the attention queue, which reconciles
 * rather than rebuilds.
 *
 * # Freshness is stated in the barrier's own units
 *
 * A row says how many ticks behind a peer's declared watermark is and what the
 * fleet's agreed input delay is. Both come from the lockstep session; neither
 * is a wall-clock latency this page has no way to measure, and no row ever
 * claims a peer is "in sync" — only that it is inside the tolerance the barrier
 * itself applies.
 */

import {
  healthStateLabelId,
  parseGmHealthProjection,
  worstHealthState,
} from './gm-health-banner.js';

const EMPTY = Object.freeze({
  tick: 0, paused: false, input_delay_ticks: null, recovery: null,
  peers: [], stations: [], operators: [], alerts: [],
});

/**
 * @param {{
 *   doc?: Document,
 *   t?: (id: string, params?: object) => string,
 *   has?: (id: string) => boolean,
 *   banners?: (alerts: object[]) => number,
 * }} opts `banners` is the unfilterable region's entry point — on the GM desk
 *   that is the attention panel's own `banners()` seam (issue #1433), which is
 *   deliberately outside its filter, snooze and hold path.
 */
export function createGmHealthPanel({
  doc = globalThis.document,
  t = (id) => id,
  has = () => false,
  banners = () => 0,
} = {}) {
  const byId = (suffix) => doc && doc.getElementById(`gm-health-${suffix}`);
  const root = byId('panel');
  const summaryEl = byId('summary');
  const tickEl = byId('tick');
  const groupsEl = byId('groups');
  const emptyEl = byId('empty');
  // The summary doubles as the panel's focus landmark, exactly as the
  // attention queue's status sentence does: never in the tab ring, only ever
  // focused programmatically when a banner points here.
  if (summaryEl) summaryEl.tabIndex = -1;

  let projection = EMPTY;

  const label = (value) => (typeof value === 'string' && has(value) ? t(value) : value);

  function element(tag, className, text) {
    const node = doc.createElement(tag);
    if (className) node.className = className;
    if (text !== undefined) node.textContent = text;
    return node;
  }

  /** One `<li>`: what it is, what state it is in (as a word), and its detail. */
  function row(id, name, state, detail) {
    const item = element('li');
    item.dataset.rowId = id;
    item.dataset.state = state;
    item.append(element('span', 'gm-health-name', name));
    item.append(element('span', 'gm-health-state', t(healthStateLabelId(state))));
    if (detail) item.append(element('span', 'gm-health-detail', detail));
    return item;
  }

  function peerName(peer) {
    if (peer.ship) return label(peer.ship.name);
    if (peer.operators.length) return peer.operators.join(', ');
    // `peer:<n>` — a display ordinal the projection minted for a participant
    // with neither a hull nor a public operator. The fleet slot is deliberately
    // not on the wire, so this is all there is to say.
    const ordinal = peer.id.startsWith('peer:') ? peer.id.slice(5) : peer.id;
    return t('server.gm.health.peer', { ordinal });
  }

  function peerDetail(peer) {
    if (peer.local) return t('server.gm.health.this_desk');
    if (peer.behind_ticks === null) return '';
    return peer.behind_ticks === 0
      ? t('server.gm.health.current')
      : t('server.gm.health.behind', { ticks: peer.behind_ticks });
  }

  function group(headingId, rows) {
    if (!rows.length) return null;
    const section = element('div', 'gm-health-group');
    section.append(element('h3', null, t(headingId)), (() => {
      const list = element('ul');
      list.append(...rows);
      return list;
    })());
    return section;
  }

  function paint() {
    const worst = worstHealthState(projection);
    if (summaryEl) {
      summaryEl.dataset.state = worst;
      // Only the three slow-changing facts. The sample tick is deliberately NOT
      // in here: this element is the panel's `aria-live` region, and a sentence
      // carrying the tick would be a whole new announcement on every republish
      // - the intrusive ordinary update PRD #1418 rules out. The write is
      // guarded too, so an unchanged sentence never re-fires the region.
      const next = t('server.gm.health.summary', {
        state: t(healthStateLabelId(worst)),
        peers: projection.peers.length,
        warnings: projection.alerts.length,
      });
      if (summaryEl.textContent !== next) summaryEl.textContent = next;
    }
    // The observability basis every freshness number above is measured against,
    // beside the summary rather than inside it: read on demand, never announced.
    if (tickEl) {
      const sampled = t('server.gm.health.sample_tick', { tick: projection.tick });
      if (tickEl.textContent !== sampled) tickEl.textContent = sampled;
    }
    const sections = [
      group('server.gm.health.group.peers', projection.peers.map(
        (peer) => row(peer.id, peerName(peer), peer.state, peerDetail(peer)),
      )),
      group('server.gm.health.group.stations', projection.stations.map((station) => row(
        station.id,
        station.ship
          ? t('server.gm.health.station_on', { station: label(station.name), ship: label(station.ship.name) })
          : label(station.name),
        station.state,
        station.operator,
      ))),
      group('server.gm.health.group.operators', projection.operators.map(
        (operator) => row(operator.id, operator.name || operator.id, operator.state, ''),
      )),
    ].filter(Boolean);
    if (groupsEl) groupsEl.replaceChildren(...sections);
    if (emptyEl) emptyEl.hidden = sections.length > 0;
    // The unfilterable region. It is fed from the SAME parsed alerts the panel
    // counts in its summary, so the two can never disagree about whether
    // something is wrong.
    banners(projection.alerts);
  }

  paint();

  return {
    /** Fold one `gm_health` payload. Returns whether it was accepted. */
    update(payload) {
      const next = parseGmHealthProjection(payload);
      if (!next) return false;
      projection = next;
      paint();
      return true;
    },
    /** Bring this panel to hand and take the keyboard, for a banner with no
     * hull of its own to open. */
    focus() {
      root?.scrollIntoView?.({ block: 'nearest' });
      summaryEl?.focus({ preventScroll: true });
    },
    reset() {
      projection = EMPTY;
      paint();
    },
    state: () => ({
      worst: worstHealthState(projection),
      projection: JSON.parse(JSON.stringify(projection)),
    }),
  };
}
