/**
 * gui/gm-attention-panel.js — the Game Master attention queue (issue #1433,
 * PRD #1419 M4, presentation contract PRD #1418).
 *
 * Renders the `gm_attention` Host Channel projection: three bands, oldest
 * first, each row explaining itself in one short sentence and offering exactly
 * two verbs — open the conversation that is already there, or snooze it for
 * one real minute.
 *
 * # Reading stability (PRD #1418 stories 23/24/26)
 *
 * A queue that reorders under a reading operator is worse than no queue. So
 * the panel has two modes:
 *
 *  - **live** — the rendered list is the current projection;
 *  - **held** — the operator is reading, focusing or has a row selected, so the
 *    rendered rows stay exactly where they are. New arrivals and band changes
 *    are counted, not applied; the panel says how many are waiting and that
 *    what is on screen is held rather than current; *Return to live* applies
 *    them on the operator's own word.
 *
 * Holding is presentation. It never pauses the simulation, never delays a
 * projection, and never changes what any other operator sees — the world keeps
 * running behind a held list, which is exactly why the list has to SAY it is
 * held.
 *
 * Nothing in this file submits a command. Opening a row hands the occurrence's
 * existing target to the host, which selects the authored Comms route already
 * on the desk; snoozing and filtering are private browser state
 * (`gui/gm-attention-filters.js`).
 *
 * # The banner seam (issue #1437)
 *
 * Technical connection/recovery failures are NOT occurrences and must never be
 * hidden by a filter, a snooze or a hold — a held list that can conceal "the
 * fleet lost a peer" is the failure PRD #1418 story 25 names. They render into
 * their own region through [`banners`], which is deliberately outside the
 * filter/hold path: it takes rows, it draws rows. #1437 fills it; this issue
 * establishes that it cannot be filtered.
 */

import {
  GM_ATTENTION_BANDS,
  GM_ATTENTION_FILTER_ALL,
  GM_ATTENTION_FILTER_KINDS,
  attentionShipId,
  createGmAttentionFilters,
} from './gm-attention-filters.js';

/** Categories this build draws. Unknown categories still render (the reason id
 * carries the meaning); this list only seeds the filter's option order. */
export const GM_ATTENTION_CATEGORIES = Object.freeze(['pending_comms']);

/** Ages repaint on this cadence. Slow enough to be free, fast enough that a
 * sixty-second snooze visibly expires. */
export const GM_ATTENTION_REFRESH_MS = 1000;

/**
 * Strictly validate one `gm_attention` payload.
 *
 * Same posture as `gm-activity-feed.js`: a malformed row is dropped rather
 * than rendered as `undefined`, and a payload that is not the expected shape
 * at all is rejected whole so the previous honest queue stays on screen.
 */
export function parseGmAttentionProjection(payload) {
  let value = payload;
  if (typeof value === 'string') {
    try { value = JSON.parse(value); } catch (_) { return null; }
  }
  if (!value || typeof value !== 'object' || !Array.isArray(value.occurrences)) return null;
  const seen = new Set();
  const occurrences = [];
  for (const row of value.occurrences) {
    if (!row || typeof row !== 'object') continue;
    if (typeof row.id !== 'string' || row.id.length === 0 || seen.has(row.id)) continue;
    if (!GM_ATTENTION_BANDS.includes(row.band)) continue;
    if (typeof row.category !== 'string' || row.category.length === 0) continue;
    if (!row.reason || typeof row.reason.id !== 'string' || row.reason.id.length === 0) continue;
    if (!Number.isSafeInteger(row.first_seen_tick) || row.first_seen_tick < 0) continue;
    if (!Number.isSafeInteger(row.age_ms) || row.age_ms < 0) continue;
    seen.add(row.id);
    const target = row.target && typeof row.target === 'object' ? row.target : {};
    occurrences.push({
      id: row.id,
      band: row.band,
      category: row.category,
      first_seen_tick: row.first_seen_tick,
      age_ms: row.age_ms,
      reason: {
        id: row.reason.id,
        params: row.reason.params && typeof row.reason.params === 'object' ? { ...row.reason.params } : {},
      },
      target: {
        route: typeof target.route === 'string' ? target.route : null,
        ship: target.ship && typeof target.ship.entity_id === 'string' ? { ...target.ship } : null,
        sender: target.sender && typeof target.sender.entity_id === 'string' ? { ...target.sender } : null,
        conversation: typeof target.conversation === 'string' ? target.conversation : null,
      },
    });
  }
  return { occurrences };
}

/** `m:ss`, the shape a facilitator reads a wait in. */
export function formatAttentionAge(ms) {
  const total = Math.max(0, Math.floor(ms / 1000));
  return `${Math.floor(total / 60)}:${String(total % 60).padStart(2, '0')}`;
}

export function createGmAttentionPanel({
  doc = globalThis.document,
  t = (id) => id,
  has = () => false,
  filters = createGmAttentionFilters(),
  now = () => Date.now(),
  onOpen = () => {},
  schedule = (fn, ms) => setInterval(fn, ms),
  cancelSchedule = (handle) => clearInterval(handle),
} = {}) {
  const byId = (suffix) => doc && doc.getElementById(`gm-attention-${suffix}`);
  const root = byId('panel');
  const listEl = byId('list');
  const statusEl = byId('status');
  const emptyEl = byId('empty');
  const liveButton = byId('live');
  const bannerEl = byId('banners');
  const filterEls = Object.fromEntries(
    GM_ATTENTION_FILTER_KINDS.map((kind) => [kind, byId(`filter-${kind}`)]),
  );
  // The status sentence doubles as the panel's focus landmark. It is never in
  // the tab ring; it only catches focus programmatically, when the row the
  // operator was standing on stops existing and there is no row left to move to.
  if (statusEl) statusEl.tabIndex = -1;

  /** The live projection, exactly as Rust last published it. */
  let live = [];
  /** Wall-clock ms at which THIS browser decided each occurrence began. Taken
   * once, from the first payload that carried the id, so a row's age never
   * jitters as later payloads resample it. */
  const firstSeen = new Map();
  /** The wait a resolved-but-still-rendered row ended on. A held list keeps
   * showing rows the projection has already dropped (that is the whole point of
   * holding), and a row whose clock restarted at 0:00 would read as a brand new
   * arrival — the exact opposite of what happened. The last honest age is
   * frozen here until the row actually leaves the screen. */
  const frozenAge = new Map();
  /** The ids currently on screen, in screen order. */
  let rendered = [];
  let held = false;
  let selectedId = null;
  let pendingIds = new Set();
  const label = (value) => (typeof value === 'string' && has(value) ? t(value) : (value || ''));

  const usingList = () => {
    if (selectedId !== null) return true;
    const active = doc && doc.activeElement;
    return !!(active && listEl && listEl !== active && listEl.contains(active));
  };

  function ageMsOf(id) {
    if (frozenAge.has(id)) return frozenAge.get(id);
    const born = firstSeen.get(id);
    return born === undefined ? 0 : Math.max(0, now() - born);
  }

  /** Oldest first, stable-id tie-break — the same rule Rust orders by, applied
   * again here because filtering and band grouping re-slice the list. */
  function ordered(rows) {
    return [...rows].sort((a, b) => {
      const left = firstSeen.get(a.id) ?? 0;
      const right = firstSeen.get(b.id) ?? 0;
      if (left !== right) return left - right;
      return a.id < b.id ? -1 : a.id > b.id ? 1 : 0;
    });
  }

  const visibleLive = () => ordered(live.filter((row) => filters.visible(row)));

  const escapeId = (id) => (globalThis.CSS && globalThis.CSS.escape ? globalThis.CSS.escape(id) : id);

  /** One row's verb, if that row is on screen and the verb still does anything. */
  /**
   * The panel mutates its own filter controller (a snooze, a filter change),
   * and the controller reports every change to whoever is listening — on the
   * GM desk that listener is this panel's own repaint. A repaint fired from
   * inside a click handler would rebuild the list before the handler has
   * finished acting on the screen it read, so the panel suppresses the
   * courtesy repaint for changes it is making itself: it always renders
   * deliberately, once, afterwards.
   */
  let mutating = false;
  function ownMutation(mutate) {
    mutating = true;
    try { return mutate(); } finally { mutating = false; }
  }

  function rowControl(id, action) {
    if (!listEl || !id) return null;
    return listEl.querySelector(
      `li[data-occurrence-id="${escapeId(id)}"] button[data-action="${action}"]:not([disabled])`,
    );
  }

  /**
   * Focus must never leave the panel because a row went away under the
   * operator. Snoozing a row and returning to live both delete the element the
   * keyboard was standing on; without a landing place the browser drops focus
   * to `<body>`, which throws a keyboard GM back to the top of the desk after
   * every single snooze (PRD #1418 story 8, stable focus/target).
   *
   * Preference order: the same verb on a named neighbour, the same verb on the
   * first surviving row, then the panel's own status sentence — which is both
   * inside the region and the sentence that just changed.
   */
  function rescueFocus(candidateIds, action) {
    for (const id of candidateIds) {
      const control = rowControl(id, action);
      if (control) { control.focus({ preventScroll: true }); return; }
    }
    const first = listEl
      && listEl.querySelector(`li[data-occurrence-id] button[data-action="${action}"]:not([disabled])`);
    if (first) { first.focus({ preventScroll: true }); return; }
    statusEl?.focus({ preventScroll: true });
  }

  /** The ids on screen in SCREEN order (band groups first), which is the order
   * a keyboard walks them in — not the flat oldest-first order. */
  const domOrder = () => (listEl
    ? [...listEl.querySelectorAll('li[data-occurrence-id]')].map((item) => item.dataset.occurrenceId)
    : []);

  function paintFilterOptions() {
    const options = {
      band: GM_ATTENTION_BANDS.map((band) => [band, t(`server.gm.attention.band.${band}`)]),
      category: [...new Set([...GM_ATTENTION_CATEGORIES, ...live.map((row) => row.category)])]
        .map((category) => [category, t(`server.gm.attention.category.${category}`)]),
      ship: [...new Map(live.filter((row) => attentionShipId(row))
        .map((row) => [attentionShipId(row), label(row.target.ship.name)])).entries()],
    };
    for (const kind of GM_ATTENTION_FILTER_KINDS) {
      const select = filterEls[kind];
      if (!select) continue;
      const rows = [[GM_ATTENTION_FILTER_ALL, t('server.gm.attention.filter.all')], ...options[kind]];
      const signature = JSON.stringify(rows);
      if (select.dataset.options !== signature) {
        select.dataset.options = signature;
        select.replaceChildren(...rows.map(([value, text]) => {
          const option = doc.createElement('option');
          option.value = value; option.textContent = text;
          return option;
        }));
      }
      const chosen = filters.filters()[kind];
      // A filter naming something the queue no longer holds keeps its value —
      // silently widening an operator's narrowing is the same class of bug as
      // silently reordering their list.
      if (![...select.options].some((option) => option.value === chosen)) {
        const stale = doc.createElement('option');
        stale.value = chosen; stale.textContent = chosen; stale.disabled = true;
        select.appendChild(stale);
      }
      select.value = chosen;
    }
  }

  function rowFor(occurrence) {
    const item = doc.createElement('li');
    item.dataset.occurrenceId = occurrence.id;
    item.dataset.band = occurrence.band;
    item.dataset.category = occurrence.category;
    item.dataset.ship = attentionShipId(occurrence);
    if (occurrence.id === selectedId) item.dataset.selected = 'true';
    // Band as WORDS, not a colour: it has to survive high contrast, forced
    // colours and a monochrome projector (PRD #1418 story 6).
    const band = doc.createElement('span');
    band.className = 'gm-attention-band';
    band.textContent = t(`server.gm.attention.band.${occurrence.band}`);
    const age = doc.createElement('span');
    age.className = 'gm-attention-age';
    age.textContent = t('server.gm.attention.age', { clock: formatAttentionAge(ageMsOf(occurrence.id)) });
    const reason = doc.createElement('p');
    reason.className = 'gm-attention-reason';
    const params = Object.fromEntries(
      Object.entries(occurrence.reason.params || {}).map(([key, value]) => [key, label(value)]),
    );
    reason.textContent = t(occurrence.reason.id, params);
    const open = doc.createElement('button');
    open.type = 'button';
    open.dataset.action = 'open';
    open.textContent = t('server.gm.attention.open');
    const snooze = doc.createElement('button');
    snooze.type = 'button';
    snooze.dataset.action = 'snooze';
    snooze.textContent = t('server.gm.attention.snooze');
    item.append(band, age, reason, open, snooze);
    return item;
  }

  function paintList(rows) {
    if (!listEl) { rendered = rows.map((row) => row.id); return; }
    listEl.replaceChildren(...GM_ATTENTION_BANDS.map((band) => {
      const inBand = rows.filter((row) => row.band === band);
      const group = doc.createElement('section');
      group.className = 'gm-attention-band-group';
      group.dataset.band = band;
      group.hidden = inBand.length === 0;
      const heading = doc.createElement('h3');
      heading.id = `gm-attention-band-${band}`;
      heading.textContent = t('server.gm.attention.band_heading', {
        band: t(`server.gm.attention.band.${band}`), count: inBand.length,
      });
      const list = doc.createElement('ol');
      list.setAttribute('role', 'list');
      list.setAttribute('aria-labelledby', heading.id);
      list.append(...inBand.map(rowFor));
      group.append(heading, list);
      return group;
    }));
    rendered = rows.map((row) => row.id);
    for (const id of [...frozenAge.keys()]) {
      if (!rendered.includes(id)) frozenAge.delete(id);
    }
  }

  function paintStatus() {
    if (root) root.dataset.freshness = held ? 'held' : 'live';
    if (liveButton) liveButton.hidden = !held;
    if (emptyEl) emptyEl.hidden = rendered.length > 0;
    if (!statusEl) return;
    statusEl.dataset.state = held ? 'held' : 'live';
    statusEl.textContent = held
      ? t('server.gm.attention.held', { count: pendingIds.size })
      : t('server.gm.attention.live', { count: rendered.length });
  }

  /**
   * A held row whose occurrence has left the projection — the crew answered,
   * the hail was withdrawn, the ship is gone. It stays where the operator is
   * reading it, but it must SAY so in words (forced colours erase a border,
   * not a sentence) and it must stop offering verbs that would do nothing.
   */
  function markResolved(item) {
    item.dataset.stale = 'true';
    let note = item.querySelector('.gm-attention-resolved');
    if (!note) {
      note = doc.createElement('p');
      note.className = 'gm-attention-resolved';
      note.tabIndex = -1;
      note.textContent = t('server.gm.attention.resolved');
      item.append(note);
    }
    const focused = doc && doc.activeElement;
    let rescue = false;
    for (const button of item.querySelectorAll('button[data-action]')) {
      if (button === focused) rescue = true;
      button.disabled = true;
    }
    // Disabling the control under the operator's finger would drop focus to the
    // document; the sentence takes it instead, so the change is announced and
    // the list keeps the focus that holds it.
    if (rescue) note.focus({ preventScroll: true });
  }

  function markLive(item) {
    delete item.dataset.stale;
    item.querySelector('.gm-attention-resolved')?.remove();
    for (const button of item.querySelectorAll('button[data-action]')) button.disabled = false;
  }

  /** Update what a held row can honestly change: its age and its band. Never
   * its position, and never the membership around it. */
  function repaintHeldRows() {
    if (!listEl) return;
    for (const item of listEl.querySelectorAll('li[data-occurrence-id]')) {
      const id = item.dataset.occurrenceId;
      const occurrence = live.find((row) => row.id === id);
      const age = item.querySelector('.gm-attention-age');
      if (age) age.textContent = t('server.gm.attention.age', { clock: formatAttentionAge(ageMsOf(id)) });
      if (!occurrence) { markResolved(item); continue; }
      markLive(item);
      if (item.dataset.band !== occurrence.band) {
        item.dataset.band = occurrence.band;
        const band = item.querySelector('.gm-attention-band');
        if (band) band.textContent = t(`server.gm.attention.band.${occurrence.band}`);
      }
    }
  }

  function render() {
    const rows = visibleLive();
    paintFilterOptions();
    if (held) {
      pendingIds = new Set(rows.filter((row) => !rendered.includes(row.id)).map((row) => row.id));
      repaintHeldRows();
    } else {
      pendingIds = new Set();
      const focused = doc && doc.activeElement;
      const anchor = focused && focused.closest ? focused.closest('li[data-occurrence-id]') : null;
      const anchorId = anchor ? anchor.dataset.occurrenceId : null;
      const anchorAction = anchor && focused.dataset ? focused.dataset.action : null;
      paintList(rows);
      // A live repaint that happened to run while a control had focus (a
      // filter change, say) still puts focus back on the same control.
      if (anchorId && anchorAction && listEl) {
        rowControl(anchorId, anchorAction)?.focus({ preventScroll: true });
      }
    }
    paintStatus();
  }

  /** A new projection. Enters held mode if the operator is using the list. */
  function update(payload) {
    const parsed = parseGmAttentionProjection(payload);
    if (!parsed) return false;
    const sample = now();
    for (const row of parsed.occurrences) {
      frozenAge.delete(row.id);
      if (!firstSeen.has(row.id)) firstSeen.set(row.id, sample - row.age_ms);
    }
    for (const id of [...firstSeen.keys()]) {
      if (parsed.occurrences.some((row) => row.id === id)) continue;
      // Still on screen because the list is held: keep the wait it ended on.
      if (rendered.includes(id)) frozenAge.set(id, ageMsOf(id));
      firstSeen.delete(id);
    }
    for (const id of [...frozenAge.keys()]) {
      if (!rendered.includes(id)) frozenAge.delete(id);
    }
    live = parsed.occurrences;
    // Reconcile snoozes against the UNFILTERED queue: resolution retires one,
    // escalation to Urgent breaks one.
    filters.sync(live);
    if (selectedId !== null && !live.some((row) => row.id === selectedId)) selectedId = null;
    if (!held && usingList()) held = true;
    render();
    return true;
  }

  /** The operator's own word that the held presentation may catch up. */
  function returnToLive() {
    held = false;
    const focused = doc && doc.activeElement;
    const anchor = focused && focused.closest ? focused.closest('li[data-occurrence-id]') : null;
    const anchorId = anchor ? anchor.dataset.occurrenceId : selectedId;
    // *Return to live* ends the reading session, not just this one hold. A row
    // left marked as selected keeps `usingList()` true forever, so the next
    // projection would drop straight back into held mode with nobody reading
    // anything — a queue that quietly stops catching up after the first Open.
    selectedId = null;
    for (const other of listEl ? listEl.querySelectorAll('li[data-selected]') : []) delete other.dataset.selected;
    // *Return to live* is itself a control inside the panel, and it is the only
    // keyboard route out of a hold entered by focus alone. Catching up hides it
    // (and may retire the row the operator was reading), so focus has to be put
    // somewhere on purpose rather than left on a `display: none` button.
    const rescuing = !!(focused && root && root.contains(focused));
    render();
    if (rescuing) rescueFocus(anchorId ? [anchorId] : [], 'open');
  }

  /** Repaint ages and let an expired snooze bring its row back. */
  function refresh() {
    const before = visibleLive().map((row) => row.id).join('\n');
    if (!held && before !== rendered.join('\n')) render();
    else if (held) repaintHeldRows();
    else {
      for (const item of listEl ? listEl.querySelectorAll('li[data-occurrence-id]') : []) {
        const age = item.querySelector('.gm-attention-age');
        if (age) age.textContent = t('server.gm.attention.age', { clock: formatAttentionAge(ageMsOf(item.dataset.occurrenceId)) });
      }
      paintStatus();
    }
  }

  /**
   * The technical-banner seam (issue #1437). Rows given here are drawn
   * verbatim: no filter, no snooze, no hold. A GM cannot hide these from
   * themselves, deliberately or accidentally.
   */
  function banners(rows) {
    if (!bannerEl) return 0;
    const list = (Array.isArray(rows) ? rows : []).filter(
      (row) => row && typeof row.id === 'string' && typeof row.message_id === 'string',
    );
    bannerEl.replaceChildren(...list.map((row) => {
      const item = doc.createElement('p');
      item.dataset.bannerId = row.id;
      item.textContent = t(row.message_id, row.params && typeof row.params === 'object' ? row.params : {});
      return item;
    }));
    bannerEl.hidden = list.length === 0;
    return list.length;
  }

  function onListClick(event) {
    const button = event.target && event.target.closest ? event.target.closest('button[data-action]') : null;
    const item = button && button.closest('li[data-occurrence-id]');
    if (!button || !item) return;
    const id = item.dataset.occurrenceId;
    const occurrence = live.find((row) => row.id === id);
    // The occurrence resolved under a held list. Neither verb has anything left
    // to act on, so the row says so instead of failing silently.
    if (!occurrence) { markResolved(item); return; }
    if (button.dataset.action === 'snooze') {
      // The row the operator just dismissed is about to stop existing, in both
      // modes. Read the screen it is standing on FIRST — its neighbours and
      // whether the keyboard is in this row — because the snooze itself is a
      // change the controller reports onward (on the real desk, straight back
      // into this panel's repaint), and a live repaint rebuilds every element:
      // after it, this row is no longer in `domOrder()` and `item` no longer
      // contains anything, so both answers would come back wrong.
      const order = domOrder();
      const at = order.indexOf(id);
      const neighbours = [order[at + 1], order[at - 1]].filter((value) => typeof value === 'string');
      const keepFocus = !!(doc && doc.activeElement && item.contains(doc.activeElement));
      ownMutation(() => filters.snooze(id, occurrence.band));
      if (selectedId === id) selectedId = null;
      if (held) {
        // The operator asked for this row to go. Removing exactly it keeps
        // every other row where they left it.
        item.remove();
        rendered = rendered.filter((row) => row !== id);
      }
      render();
      if (keepFocus) rescueFocus(neighbours, 'snooze');
      return;
    }
    // Open: mark the row as the one being read (which holds the list), then
    // hand its EXISTING target to the host. No simulation command, no panel
    // switch, no dialog.
    selectedId = id;
    held = true;
    for (const other of listEl ? listEl.querySelectorAll('li[data-selected]') : []) delete other.dataset.selected;
    item.dataset.selected = 'true';
    paintStatus();
    onOpen(occurrence);
  }

  function reset() {
    live = [];
    firstSeen.clear();
    frozenAge.clear();
    rendered = [];
    pendingIds = new Set();
    held = false;
    selectedId = null;
    if (listEl) listEl.replaceChildren();
    if (bannerEl) { bannerEl.replaceChildren(); bannerEl.hidden = true; }
    render();
  }

  listEl?.addEventListener('click', onListClick);
  liveButton?.addEventListener('click', returnToLive);
  for (const kind of GM_ATTENTION_FILTER_KINDS) {
    filterEls[kind]?.addEventListener('change', () => {
      ownMutation(() => filters.setFilter(kind, filterEls[kind].value));
      // A deliberate filter change is not an ambush: it applies immediately,
      // even while held, because the operator asked for it.
      held = false;
      render();
    });
  }
  const timer = schedule(refresh, GM_ATTENTION_REFRESH_MS);
  render();

  return {
    update,
    refresh,
    reset,
    returnToLive,
    banners,
    /** Redraw from the state the panel already holds. The filter controller
     * calls this when something OTHER than a projection changed what the queue
     * shows — a restored session, or a snooze written by a sibling widget — so
     * the operator is not left looking at yesterday's filters until the world
     * happens to publish again. Presentation only: no projection is fetched
     * and no authority is touched. Changes the panel makes to the controller
     * itself are excluded: those render once, deliberately, from the handler
     * that made them. */
    repaint: () => { if (!mutating) render(); },
    dispose() { cancelSchedule(timer); },
    state: () => ({
      held,
      selectedId,
      newCount: pendingIds.size,
      rendered: [...rendered],
      occurrences: live.map((row) => ({ ...row })),
      filters: filters.state(),
    }),
  };
}
