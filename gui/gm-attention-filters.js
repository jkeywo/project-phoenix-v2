/**
 * gui/gm-attention-filters.js — one operator's own attention filters and
 * snoozes (issue #1433, PRD #1419 M4).
 *
 * Presentation only, and *private*. Nothing here reaches a `GmAction`, the GM
 * roster, a snapshot, the digest or another operator's browser: two Game
 * Masters looking at the same queue may hide entirely different rows from
 * themselves and still have identical authority, identical ordering and
 * identical action availability. This is the same posture as
 * `gui/gm-role-presets.js`, and the same reason.
 *
 * It is a controller rather than panel-local state because #1442's authored
 * filtered-attention and workload widgets reuse it verbatim: they get the
 * operator's live filter/snooze decisions from here instead of growing a
 * second, silently divergent copy.
 *
 * # What a snooze is
 *
 * One click, exactly sixty REAL seconds, no picker. Real seconds because a
 * facilitator who pauses the world to talk to a table has not stopped waiting
 * — the minute keeps running, and `SimulationPaused` is not consulted anywhere
 * in this file. Two rules bound it:
 *
 *  - an occurrence that ESCALATES to Urgent while snoozed breaks the snooze
 *    immediately (it is new information, and it is the information the snooze
 *    was never about);
 *  - an occurrence that was ALREADY Urgent when it was snoozed waits out its
 *    minute like any other. A GM who deliberately snoozed an Urgent row meant
 *    it.
 *
 * # What a snooze is NOT
 *
 * It never suppresses a technical banner. Banners do not travel as
 * occurrences and never pass through [`visible`] — see the banner seam note in
 * `gui/gm-attention-panel.js`.
 *
 * # Persistence scope
 *
 * Filters and remaining snooze time survive a reconnect to THE SAME session
 * and reset for a new one. That is spelled as a scope key of
 * `session id` + `operator id`: a stored scope whose session is not the
 * current one is dropped on the next read, so a new session starts clean
 * without anything having to remember to clear it, and two operators sharing
 * one browser profile never inherit each other's snoozes.
 *
 * The session id is late — a GM knows it only once the fleet join resolves —
 * so the controller is built first and [`restore`] is called when it arrives,
 * exactly as `__hostGmRolePresetsRestore` is.
 */

/** Exactly one real minute. Not configurable, and not a picker (PRD #1419). */
export const GM_ATTENTION_SNOOZE_MS = 60000;

/** The closed band vocabulary, in queue order. Mirrors Rust's `GmAttentionBand`. */
export const GM_ATTENTION_BANDS = Object.freeze(['urgent', 'attention', 'background']);

/** The "no narrowing" value every filter defaults to. */
export const GM_ATTENTION_FILTER_ALL = 'all';

/** The three facets this controller owns. #1442 adds widgets, not facets. */
export const GM_ATTENTION_FILTER_KINDS = Object.freeze(['band', 'category', 'ship']);

/** Private browser storage key. Versioned so a shape change cannot be read as
 * the old one. */
export const GM_ATTENTION_STORAGE_KEY = 'phoenix.gm.attention.v1';

/** Keep stored scopes bounded; a browser is not an archive of past sessions. */
const MAX_STORED_SCOPES = 8;

/** Keep one operator's snooze set bounded for the same reason. */
const MAX_SNOOZES = 64;

const defaultFilters = () => ({
  band: GM_ATTENTION_FILTER_ALL,
  category: GM_ATTENTION_FILTER_ALL,
  ship: GM_ATTENTION_FILTER_ALL,
});

/** The ship facet of one occurrence, or `''` for fleet-wide traffic. */
export function attentionShipId(occurrence) {
  const id = occurrence && occurrence.target && occurrence.target.ship
    && occurrence.target.ship.entity_id;
  return typeof id === 'string' ? id : '';
}

export function createGmAttentionFilters({
  storage = null,
  getSessionId = () => null,
  getOperatorId = () => null,
  now = () => Date.now(),
  onChange = () => {},
} = {}) {
  let filters = defaultFilters();
  /** id → { until: epoch ms, band: the band it was snoozed AT }. */
  let snoozes = new Map();
  let scopeKey = null;

  const sessionId = () => {
    const value = getSessionId();
    return typeof value === 'string' && value.length > 0 ? value : null;
  };
  const operatorId = () => {
    const value = getOperatorId();
    return typeof value === 'string' && value.length > 0 ? value : '';
  };
  const scope = () => {
    const session = sessionId();
    return session === null ? null : `${session}|${operatorId()}`;
  };

  function readStore() {
    if (!storage) return {};
    try {
      const raw = JSON.parse(storage.getItem(GM_ATTENTION_STORAGE_KEY) || 'null');
      return raw && typeof raw === 'object' && raw.scopes && typeof raw.scopes === 'object'
        ? raw.scopes : {};
    } catch (_) {
      return {};
    }
  }

  function persist() {
    const key = scope();
    if (!storage || key === null) return;
    const session = sessionId();
    const scopes = readStore();
    // A stored scope from ANOTHER session is exactly what "new sessions reset"
    // means: it is dropped here rather than resurrected later.
    for (const [name, value] of Object.entries(scopes)) {
      if (!value || value.session !== session) delete scopes[name];
    }
    scopes[key] = {
      session,
      filters: { ...filters },
      snoozes: [...snoozes.entries()].map(([id, row]) => ({ id, until: row.until, band: row.band })),
    };
    const names = Object.keys(scopes);
    for (const name of names.slice(0, Math.max(0, names.length - MAX_STORED_SCOPES))) {
      if (name !== key) delete scopes[name];
    }
    try { storage.setItem(GM_ATTENTION_STORAGE_KEY, JSON.stringify({ scopes })); } catch (_) { /* private mode */ }
  }

  function changed() {
    persist();
    onChange();
  }

  /**
   * Adopt the stored filters/snoozes for the CURRENT session and operator.
   *
   * Called once the session identity resolves. A different session (or no
   * stored scope) leaves this operator on defaults — that is the reset, and it
   * is deliberately not a separate "clear" call nobody would remember to make.
   * Remaining snooze time is restored from an absolute real-time deadline, so a
   * reconnect thirty seconds in leaves thirty seconds, not a fresh minute.
   */
  function restore() {
    const key = scope();
    scopeKey = key;
    filters = defaultFilters();
    snoozes = new Map();
    if (key === null) { onChange(); return; }
    const stored = readStore()[key];
    if (stored && stored.session === sessionId()) {
      if (stored.filters && typeof stored.filters === 'object') {
        for (const kind of GM_ATTENTION_FILTER_KINDS) {
          if (typeof stored.filters[kind] === 'string') filters[kind] = stored.filters[kind];
        }
      }
      for (const row of Array.isArray(stored.snoozes) ? stored.snoozes : []) {
        if (!row || typeof row.id !== 'string' || !Number.isFinite(row.until)) continue;
        if (row.until <= now()) continue;
        snoozes.set(row.id, {
          until: row.until,
          band: GM_ATTENTION_BANDS.includes(row.band) ? row.band : 'attention',
        });
      }
    }
    onChange();
  }

  function setFilter(kind, value) {
    if (!GM_ATTENTION_FILTER_KINDS.includes(kind)) return false;
    const next = typeof value === 'string' && value.length > 0 ? value : GM_ATTENTION_FILTER_ALL;
    if (filters[kind] === next) return false;
    filters[kind] = next;
    changed();
    return true;
  }

  /** One click, one minute, from the band the row is showing right now. */
  function snooze(id, band) {
    if (typeof id !== 'string' || !id) return false;
    snoozes.set(id, {
      until: now() + GM_ATTENTION_SNOOZE_MS,
      band: GM_ATTENTION_BANDS.includes(band) ? band : 'attention',
    });
    while (snoozes.size > MAX_SNOOZES) snoozes.delete(snoozes.keys().next().value);
    changed();
    return true;
  }

  function unsnooze(id) {
    if (!snoozes.delete(id)) return false;
    changed();
    return true;
  }

  function snoozeRemainingMs(id) {
    const row = snoozes.get(id);
    return row ? Math.max(0, row.until - now()) : 0;
  }

  function isSnoozed(id) {
    return snoozeRemainingMs(id) > 0;
  }

  /**
   * Reconcile the snooze set against the live queue.
   *
   * Three retirements, and the caller must run this on the UNFILTERED live
   * list — a snooze the operator can no longer see is still theirs:
   *  - expired (the minute is up);
   *  - the occurrence resolved or was withdrawn (it will never come back under
   *    this id, so keeping the snooze only wastes storage);
   *  - the occurrence escalated to Urgent from a lower band.
   *
   * Returns the ids whose snooze this call broke by escalation, so a panel can
   * say why a row it had hidden is suddenly back.
   */
  function sync(occurrences) {
    const live = new Map();
    for (const occurrence of Array.isArray(occurrences) ? occurrences : []) {
      if (occurrence && typeof occurrence.id === 'string') live.set(occurrence.id, occurrence);
    }
    const escalated = [];
    let dirty = false;
    for (const [id, row] of [...snoozes.entries()]) {
      const occurrence = live.get(id);
      if (row.until <= now() || !occurrence) {
        snoozes.delete(id); dirty = true; continue;
      }
      if (occurrence.band === 'urgent' && row.band !== 'urgent') {
        snoozes.delete(id); escalated.push(id); dirty = true;
      }
    }
    if (dirty) persist();
    return escalated;
  }

  /** Does this occurrence survive the operator's own narrowing? */
  function visible(occurrence) {
    if (!occurrence || typeof occurrence.id !== 'string') return false;
    if (isSnoozed(occurrence.id)) return false;
    if (filters.band !== GM_ATTENTION_FILTER_ALL && occurrence.band !== filters.band) return false;
    if (filters.category !== GM_ATTENTION_FILTER_ALL
      && occurrence.category !== filters.category) return false;
    if (filters.ship !== GM_ATTENTION_FILTER_ALL
      && attentionShipId(occurrence) !== filters.ship) return false;
    return true;
  }

  return {
    restore,
    filters: () => ({ ...filters }),
    setFilter,
    snooze,
    unsnooze,
    isSnoozed,
    snoozeRemainingMs,
    sync,
    visible,
    state: () => ({
      scope: scopeKey,
      filters: { ...filters },
      snoozes: [...snoozes.entries()].map(([id, row]) => ({ id, remaining_ms: Math.max(0, row.until - now()), band: row.band })),
    }),
  };
}
