/**
 * gui/gm-attention-panel.js — the Game Master attention queue (issues #1433 and
 * #1434, PRD #1419 M4, presentation contract PRD #1418).
 *
 * Renders the `gm_attention` Host Channel projection: three bands, oldest
 * first, each row explaining itself in one short sentence and offering exactly
 * two verbs — open the thing that is already there, or snooze it for one real
 * minute.
 *
 * One list, several kinds of occurrence. A pending conversation opens the
 * authored Comms route that already speaks as its sender; an eligible beat
 * (#1434) opens that beat's existing row in the mission panel, where the Fire,
 * Pause and Skip its author declared already live. Neither verb acts: nothing
 * in this file fires a beat, sends a hail, or submits any GM action at all.
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
 * filter/hold path: it takes rows, it draws rows.
 *
 * Issue #1437 fills it. This panel still OWNS the region — that is what makes
 * "no filter, snooze or hold can reach it" a property of one place rather than
 * a promise every caller has to keep — but it does not decide what a technical
 * warning looks like. The `renderBanners` hook does that, and on the GM desk it
 * is `gui/gm-health-banner.js`, the same component the M5 live-restore surfaces
 * (#1446/#1447) mount elsewhere.
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
export const GM_ATTENTION_CATEGORIES = Object.freeze([
  'pending_comms',
  'eligible_beat',
  'idle_npc',
  'station_health',
  'quiet_time',
]);

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
        // The authored beat this row is about (issue #1434), and which levers
        // the mission panel is already offering for it. Dropped whole unless it
        // names an event, so a row can never claim controls it cannot point at.
        event: target.event && typeof target.event.id === 'string' && target.event.id.length > 0
          ? {
            id: target.event.id,
            label: typeof target.event.label === 'string' ? target.event.label : '',
            fire: target.event.fire === true,
            pause: target.event.pause === true,
            skip: target.event.skip === true,
          }
          : null,
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

/**
 * The banner renderer a panel with no host-supplied one falls back to: one
 * paragraph per row, from a String Table id and its parameters. Deliberately
 * minimal — it exists so the region is never blank when nobody has wired the
 * health component, not as a second design of what a warning looks like.
 */
export function defaultRenderBanners(rows, container, { doc = globalThis.document, t = (id) => id } = {}) {
  const list = rows.filter(
    (row) => row && typeof row.id === 'string' && typeof row.message_id === 'string',
  );
  container.replaceChildren(...list.map((row) => {
    const item = doc.createElement('p');
    item.dataset.bannerId = row.id;
    item.textContent = t(row.message_id, row.params && typeof row.params === 'object' ? row.params : {});
    return item;
  }));
  container.hidden = list.length === 0;
  return list.length;
}

export function createGmAttentionPanel({
  doc = globalThis.document,
  t = (id) => id,
  has = () => false,
  filters = createGmAttentionFilters(),
  now = () => Date.now(),
  onOpen = () => {},
  // The queue's own age tick, offered to whoever else is drawing one of these
  // waits. The panel is the only thing on the page that knows how old a row
  // really is (see `state()`), and in a lull nothing republishes, so a
  // consumer with no tick of its own would simply stop counting.
  onAge = () => {},
  renderBanners = defaultRenderBanners,
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
  // The same landing place, one step out, for a host document that renders the
  // list without the status sentence.
  if (listEl) listEl.tabIndex = -1;

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

  /**
   * Is the operator actually reading a ROW right now?
   *
   * Deliberately narrower than "focus is somewhere in the panel": the panel
   * also contains the filter selects and *Return to live*, and standing on one
   * of those is not reading the list — it is operating the list. Counting them
   * made *Return to live* self-defeating in a real browser, where clicking a
   * button focuses it: the click ended the hold, focus stayed on a control
   * inside the panel, and the very next projection re-entered held mode with
   * nobody reading anything. Only focus inside an `li[data-occurrence-id]`
   * holds the list.
   */
  const usingList = () => {
    if (selectedId !== null) return true;
    const active = doc && doc.activeElement;
    if (!active || !listEl || active === listEl || !listEl.contains(active)) return false;
    return !!(active.closest && active.closest('li[data-occurrence-id]'));
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

  /**
   * Does this row name something the desk can navigate to?
   *
   * Structural, not a category list: every row that has an Open destination
   * carries it in `target` (a Comms row always names its speaker, a beat row
   * its event), and a row whose target names nothing at all is an advisory
   * about the session itself - the quiet-time row of issue #1436. A category
   * this build has never heard of therefore gets the right verb without this
   * file being taught about it.
   */
  const hasDestination = (occurrence) => Object
    .values(occurrence.target || {})
    .some((value) => value !== null && value !== undefined);

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
    // `btn` is the desk's own control class, and it is what carries
    // `min-height: var(--control-hit-min)` plus the wrap-don't-shrink rules
    // (PRD #1418 stories 5 and 9). Without it these verbs were the only
    // controls on the desk with no touch-target floor, which shows up first at
    // --a11y-text-scale: 2 where everything around them grows and they do not.
    const snooze = doc.createElement('button');
    snooze.type = 'button';
    snooze.className = 'btn';
    snooze.dataset.action = 'snooze';
    snooze.textContent = t('server.gm.attention.snooze');
    // A row with no target has nowhere to go - the quiet-time advisory
    // (issue #1436) is about the session, not about a conversation, a beat or a
    // ship. It gets no Open verb at all rather than a verb that would do
    // nothing: a disabled control the keyboard still has to walk past is not
    // kinder than an absent one, and a live control that does nothing is worse
    // than both (PRD #1418 story 31 - a press must never look like an effect).
    if (!hasDestination(occurrence)) {
      item.append(band, age, reason, snooze);
      return item;
    }
    const open = doc.createElement('button');
    open.type = 'button';
    open.className = 'btn';
    open.dataset.action = 'open';
    // One verb, two destinations, and the label says which: a conversation row
    // opens the conversation, a beat row opens the controls the author declared
    // for that beat. Neither one ACTS - see `onListClick`.
    open.textContent = t(occurrence.target.event
      ? 'server.gm.attention.open_beat'
      : 'server.gm.attention.open');
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
    //
    // It lands on the panel's own status sentence — the landmark, never a row
    // control. A row control would put focus straight back inside the list the
    // operator just asked to stop holding: in a real browser the click that
    // ended the hold focuses the button, the rescue then moves to a surviving
    // row's Open verb, and the next projection holds the list again with nobody
    // reading. The sentence is inside the region, is the line that just
    // changed, and holds nothing.
    const rescuing = !!(focused && root && root.contains(focused));
    render();
    if (rescuing) (statusEl || listEl)?.focus({ preventScroll: true });
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
    onAge();
  }

  /**
   * The technical-banner seam (issue #1437). Rows given here are drawn
   * verbatim: no filter, no snooze, no hold. A GM cannot hide these from
   * themselves, deliberately or accidentally.
   *
   * The region belongs to this panel; the drawing belongs to `renderBanners`.
   */
  function banners(rows) {
    if (!bannerEl) return 0;
    return renderBanners(Array.isArray(rows) ? rows : [], bannerEl, { doc, t });
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
    // hand its EXISTING target to the host — the authored Comms route for a
    // conversation, the authored beat's own mission-panel row for a beat. No
    // simulation command, no dialog, and above all no Fire: the row takes the
    // operator to the controls, and the operator presses them, where the
    // ordinary admission check and the ordinary apply-tick revalidation are.
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
    // Through the same seam, so a renderer that keeps its own bookkeeping
    // (the health component reconciles rather than rebuilds) is told the
    // region is empty instead of finding its nodes gone from under it.
    banners([]);
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
      // Aged HERE, not handed on as the projection sampled it. Rust
      // deliberately does not republish on age alone (`src/gm_attention.rs`:
      // "age alone is not a change ... let the page age its own rows from the
      // last honest sample"), and the quiet-time advisory holds one stable id
      // for a whole lull, so a consumer given the raw `age_ms` would freeze at
      // the age of the last payload while the row beside it counted up. The
      // bar's Quiet pill is exactly that consumer.
      occurrences: live.map((row) => ({ ...row, age_ms: ageMsOf(row.id) })),
      filters: filters.state(),
    }),
  };
}
