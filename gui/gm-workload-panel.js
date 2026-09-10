/**
 * gui/gm-workload-panel.js — the Game Master Station-workload advisory (issue
 * #1438, PRD #1419 M4 stories 11/12/14, presentation contract PRD #1418).
 *
 * Renders the `gm_workload` Host Channel projection: one row per Station,
 * saying in WORDS how much its human operator is currently being asked for, and
 * expanding to the exact source demands that produced the number.
 *
 * # It is evidence, never a score
 *
 * The expanded list names the conversations, clearances and requests that are
 * outstanding right now. It is deliberately not a rating, a percentage or a
 * trend: a facilitator reading "Overloaded" has to be able to see the three
 * things behind it and disagree, and a number nobody can audit would be exactly
 * the opaque precision PRD #1419 refuses.
 *
 * # Reading stability (PRD #1418 stories 23, 26)
 *
 * Rows arrive in a fixed order — ship, then Station — and are rendered in that
 * order, so nothing reorders under a reading operator as counts change. Two
 * pieces of private state survive every repaint:
 *
 *  - which Stations the operator has expanded, keyed by ship + Station, so a
 *    row being read stays open when its count changes;
 *  - where the keyboard was, so a repaint returns focus to the same Station's
 *    own control rather than dropping it to the document.
 *
 * A Station that leaves the projection entirely takes its expansion with it,
 * and focus standing on it lands on the panel's status sentence — which is in
 * the region, is never in the tab ring, and is the sentence that just changed.
 *
 * Nothing here opens a dialog, switches a panel or submits a command. Workload
 * is advice.
 */

/** The levels this build draws, in the order a summary line reads them. */
export const GM_WORKLOAD_LEVELS = Object.freeze([
  'underused', 'engaged', 'overloaded', 'backfill', 'offline',
]);

/** Levels that are a statement about a person rather than about a seat. */
const COUNTED_LEVELS = Object.freeze(['underused', 'engaged', 'overloaded']);

/**
 * Strictly validate one `gm_workload` payload.
 *
 * Same posture as the attention queue: a malformed row is dropped rather than
 * drawn as `undefined`, and a payload that is not a summary at all is rejected
 * whole so the previous honest advisory stays on screen.
 */
export function parseGmWorkloadProjection(payload) {
  let value = payload;
  if (typeof value === 'string') {
    try { value = JSON.parse(value); } catch (_) { return null; }
  }
  if (!value || typeof value !== 'object' || !Array.isArray(value.stations)) return null;
  const seen = new Set();
  const stations = [];
  for (const row of value.stations) {
    if (!row || typeof row !== 'object') continue;
    if (!row.ship || typeof row.ship.entity_id !== 'string' || !row.ship.entity_id) continue;
    if (typeof row.station_id !== 'string' || !row.station_id) continue;
    if (!GM_WORKLOAD_LEVELS.includes(row.level)) continue;
    if (!Number.isSafeInteger(row.count) || row.count < 0) continue;
    const key = `${row.ship.entity_id}/${row.station_id}`;
    if (seen.has(key)) continue;
    seen.add(key);
    const demands = Array.isArray(row.demands) ? row.demands : [];
    stations.push({
      key,
      ship: { entity_id: row.ship.entity_id, name: typeof row.ship.name === 'string' ? row.ship.name : row.ship.entity_id },
      station_id: row.station_id,
      station_name: typeof row.station_name === 'string' && row.station_name ? row.station_name : row.station_id,
      level: row.level,
      count: row.count,
      sustained_secs: Number.isSafeInteger(row.sustained_secs) && row.sustained_secs >= 0 ? row.sustained_secs : 0,
      overload_count: Number.isSafeInteger(row.overload_count) && row.overload_count > 0 ? row.overload_count : 0,
      overload_secs: Number.isSafeInteger(row.overload_secs) && row.overload_secs > 0 ? row.overload_secs : 0,
      demands: demands
        .filter((demand) => demand && typeof demand.key === 'string' && demand.key
          && demand.reason && typeof demand.reason.id === 'string' && demand.reason.id)
        .map((demand) => ({
          key: demand.key,
          source: typeof demand.source === 'string' ? demand.source : '',
          reason: {
            id: demand.reason.id,
            params: demand.reason.params && typeof demand.reason.params === 'object'
              ? { ...demand.reason.params } : {},
          },
        })),
    });
  }
  return { stations };
}

/** Whether a level is a claim about a person's workload. */
export function workloadCountsPeople(level) {
  return COUNTED_LEVELS.includes(level);
}

export function createGmWorkloadPanel({
  doc = globalThis.document,
  t = (id) => id,
  has = () => false,
} = {}) {
  const byId = (suffix) => doc && doc.getElementById(`gm-workload-${suffix}`);
  const root = byId('panel');
  const listEl = byId('list');
  const emptyEl = byId('empty');
  const statusEl = byId('status');
  // The status sentence doubles as the panel's focus landmark, exactly as the
  // attention queue's does. Never in the tab ring; it only catches focus
  // programmatically, when the Station an operator was standing on is gone.
  if (statusEl) statusEl.tabIndex = -1;

  /** The last honest projection. */
  let stations = [];
  /** Station keys the operator has expanded. Private, per browser, per desk. */
  const expanded = new Set();

  /** Resolve an id through the String Table only when the table actually holds
   * it — an authored ship or Station name that is literal text stays literal. */
  const label = (value) => (typeof value === 'string' && has(value) ? t(value) : (value || ''));

  const element = (tag, className, text) => {
    const node = doc.createElement(tag);
    if (className) node.className = className;
    if (text !== undefined) node.textContent = text;
    return node;
  };

  const cssEscape = (value) => (globalThis.CSS && globalThis.CSS.escape
    ? globalThis.CSS.escape(value) : value);

  /** Which Station row, if any, the keyboard is currently inside. */
  function focusedKey() {
    const active = doc && doc.activeElement;
    if (!active || !listEl || !listEl.contains(active)) return null;
    const item = active.closest && active.closest('li[data-station-key]');
    return item ? item.dataset.stationKey : null;
  }

  function restoreFocus(key) {
    if (key === null) return;
    const summary = listEl
      && listEl.querySelector(`li[data-station-key="${cssEscape(key)}"] summary`);
    if (summary) { summary.focus({ preventScroll: true }); return; }
    statusEl?.focus({ preventScroll: true });
  }

  /** One Station's summary sentence: identity, then the word, then the count.
   *  The word carries the meaning; nothing here depends on colour. */
  function summaryFor(row) {
    const summary = doc.createElement('summary');
    summary.append(element('span', 'gm-workload-name', t('server.gm.workload.row', {
      ship: label(row.ship.name),
      station: label(row.station_name),
    })));
    const state = element('span', 'gm-workload-state', t(`server.gm.workload.state.${row.level}`));
    state.dataset.level = row.level;
    summary.append(state);
    if (workloadCountsPeople(row.level)) {
      summary.append(element('span', 'gm-workload-count', t('server.gm.workload.count', {
        count: row.count,
      })));
    }
    return summary;
  }

  /** The body of one expanded Station: either why it is not a workload at all,
   *  or the exact demands that were counted. */
  function bodyFor(row) {
    const body = element('div', 'gm-workload-body');
    if (row.level === 'backfill') {
      body.append(element('p', 'gm-workload-note', t('server.gm.workload.backfill_note')));
      return body;
    }
    if (row.level === 'offline') {
      body.append(element('p', 'gm-workload-note', t('server.gm.workload.offline_note')));
      return body;
    }
    // The honest "nearly there" sentence: a Station at the count but not yet at
    // the duration reads Engaged, and this says why it is not Overloaded rather
    // than leaving the operator to wonder whether the rule fired.
    if (row.level === 'engaged' && row.overload_count && row.count >= row.overload_count) {
      body.append(element('p', 'gm-workload-building', t('server.gm.workload.building', {
        elapsed: row.sustained_secs,
        needed: row.overload_secs,
      })));
    }
    if (row.demands.length === 0) {
      body.append(element('p', 'gm-workload-note', t('server.gm.workload.no_evidence')));
      return body;
    }
    const list = element('ul', 'gm-workload-demands');
    list.setAttribute('aria-label', t('server.gm.workload.evidence'));
    for (const demand of row.demands) {
      const item = doc.createElement('li');
      item.dataset.demandKey = demand.key;
      if (demand.source) item.dataset.source = demand.source;
      const params = Object.fromEntries(
        Object.entries(demand.reason.params).map(([key, value]) => [key, label(value)]),
      );
      item.textContent = t(demand.reason.id, params);
      list.append(item);
    }
    body.append(list);
    return body;
  }

  function render() {
    if (!listEl) return;
    const keyboardKey = focusedKey();
    const live = new Set(stations.map((row) => row.key));
    // An expansion for a Station that has left the projection is retired with
    // it, so a hull that comes back is not silently pre-opened.
    for (const key of [...expanded]) if (!live.has(key)) expanded.delete(key);

    listEl.replaceChildren(...stations.map((row) => {
      const item = doc.createElement('li');
      item.dataset.stationKey = row.key;
      item.dataset.level = row.level;
      const details = doc.createElement('details');
      details.open = expanded.has(row.key);
      const summary = summaryFor(row);
      details.append(summary, bodyFor(row));
      // Two listeners for one fact, deliberately. `toggle` is the authoritative
      // one — it also catches a programmatic open and a browser's own find-in-
      // page expansion — but it is QUEUED as a task, so a projection arriving
      // in the same turn as the click would repaint from the old expansion set
      // and shut the row under the operator. The click handler records the
      // state the default action is about to produce, synchronously, so the
      // repaint cannot beat it.
      summary.addEventListener('click', () => {
        if (details.open) expanded.delete(row.key); else expanded.add(row.key);
      });
      details.addEventListener('toggle', () => {
        if (details.open) expanded.add(row.key); else expanded.delete(row.key);
      });
      item.append(details);
      return item;
    }));
    if (emptyEl) emptyEl.hidden = stations.length > 0;
    if (statusEl) {
      // Its own sentence, not the per-row `…count` one: this number is how many
      // STATIONS have a person at them, and borrowing the row's "N waiting"
      // copy told the operator the panel was waiting on things it was not.
      const crewed = stations.filter((row) => workloadCountsPeople(row.level)).length;
      statusEl.textContent = t('server.gm.workload.summary', { count: crewed });
    }
    if (keyboardKey !== null) restoreFocus(keyboardKey);
  }

  return {
    /** Apply one `gm_workload` payload. A payload that does not parse is
     *  ignored, leaving the last honest advisory on screen. */
    update(payload) {
      const parsed = parseGmWorkloadProjection(payload);
      if (!parsed) return;
      stations = parsed.stations;
      render();
    },
    /** Draw again from state already held. No projection is fetched and no
     *  authority is touched — the desk calls this when something else changed. */
    repaint() { render(); },
    /** The rows currently on screen, for the shell and for tests. */
    state() { return stations.map((row) => ({ ...row })); },
    /** Whether one Station is currently expanded. */
    isExpanded(key) { return expanded.has(key); },
    reset() {
      stations = [];
      expanded.clear();
      render();
    },
    dispose() {
      stations = [];
      expanded.clear();
      if (listEl) listEl.replaceChildren();
    },
    root,
  };
}
