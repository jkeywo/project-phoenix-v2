/**
 * gui/tutorial-state.js — Contextual tutorial trigger evaluation (issue #916).
 *
 * Pure module: evaluates TOML-authored overlay definitions (delivered on
 * `Welcome` as `ship_config.station_tutorials`, authored as
 * `[[station.tutorial]]` blocks) against the client-local tutorial progress
 * and the console payload the overlay would sit on. The server carries the
 * definitions verbatim and never interprets them — the trigger vocabulary
 * lives HERE, as data, so a new trigger kind is a TOML + JS change with no
 * Rust branch.
 *
 * Trigger vocabulary (`trigger.kind`):
 *   - `first_visit`     eligible until dismissed — the "first time on this
 *                       station" tip, persisted via the dismissal record.
 *   - `control_unused`  eligible until the named console action
 *                       (`trigger.control`) has been used once.
 *   - `state`           eligible while a field of the console's own payload
 *                       matches: `trigger.path` (dot-path), `trigger.op`
 *                       (truthy/falsy/eq/ne/gt/gte/lt/lte, default truthy),
 *                       `trigger.value` (operand for the binary ops).
 *
 * Two gates apply to EVERY overlay regardless of kind: a dismissed overlay
 * never shows again, and an overlay whose trigger names a `control` completes
 * as soon as that action is used (so a `state`-triggered "boost is ready" tip
 * retires itself the first time the player boosts). Unknown kinds and
 * unknown ops fail closed.
 *
 * Content fields (`title`/`text`) are strings.csv ids end to end; the
 * `ph-tutorial-overlay` component resolves them through t() at render time.
 * Nothing here touches display text.
 *
 * Progress is client-local presentation state — NOT server state. The shape
 * is `{ dismissed: { ['<hull>/<station>/<overlayId>']: true },
 *       used: { ['<hull>/<station>/<action>']: true } }` — every key is scoped
 * per hull and station (see scopedTutorialKey), so a tutorial action on one
 * hull can never retire a same-named tip on another. Persisted to
 * localStorage via the load/save helpers below.
 *
 * DOM-free; the ONE import-time side effect is hydrating
 * `simState.tutorialProgress` from localStorage (browser only — see the
 * hydration section near the bottom). Unit-tested in
 * tests/client/tutorial-state.test.js.
 */

// The sim-state singleton is an explicit ES import so the hydration at the
// bottom of this file runs AFTER sim-state.js evaluates, wherever this module
// enters the graph. Script-tag order in client.html is irrelevant — and must
// not be relied on: gui/console-state.js imports this module well before the
// sim-state.js script tag runs.
import { simState } from './sim-state.js';

/** Console action the overlay component sends to dismiss the active overlay.
 *  Handled entirely client-side by client.html — never forwarded to the host. */
export const TUTORIAL_DISMISS_ACTION = 'tutorial_dismiss';

/** localStorage key for hull-and-station-scoped tutorial progress. */
export const TUTORIAL_PROGRESS_KEY = 'phoenix-tutorial-progress-v3';
/** The station-scoped record written before hull identity was included. */
export const LEGACY_TUTORIAL_PROGRESS_KEY = 'phoenix-tutorial-progress-v2';
const LEGACY_TUTORIAL_MIGRATION_KEY = 'phoenix-tutorial-progress-v3-migrated';

/**
 * Progress-record key for one overlay id or control name on one station of
 * one hull: `'<hull>/<station>/<name>'`.
 */
export function scopedTutorialKey(hull, station, name) {
  // A pre-hull-identity host retains the v2 station key. Identified hulls
  // always use the three-part form, which is the normal runtime path.
  if (name === undefined) return `${hull}/${station}`;
  return hull ? `${hull}/${station}/${name}` : `${station}/${name}`;
}

/** A fresh, empty progress record. */
export function emptyTutorialProgress() {
  return { dismissed: {}, used: {} };
}

/**
 * Coerce an untrusted value (parsed localStorage JSON, missing field, old
 * schema) into a valid progress record. Never throws.
 * @param {*} raw
 */
export function normalizeTutorialProgress(raw) {
  const p = emptyTutorialProgress();
  if (!raw || typeof raw !== 'object') return p;
  if (raw.dismissed && typeof raw.dismissed === 'object') {
    for (const k of Object.keys(raw.dismissed)) {
      if (raw.dismissed[k]) p.dismissed[k] = true;
    }
  }
  if (raw.used && typeof raw.used === 'object') {
    for (const k of Object.keys(raw.used)) {
      if (raw.used[k]) p.used[k] = true;
    }
  }
  return p;
}

/**
 * Progress with the overlay key `key` (a scopedTutorialKey result) recorded
 * as dismissed. Returns the SAME object when nothing changes, so callers can
 * use reference equality to decide whether to persist / re-push.
 */
export function progressWithDismissed(progress, key) {
  if (!key) return progress;
  const p = normalizeTutorialProgress(progress);
  if (progress && progress.dismissed && progress.dismissed[key]) return progress;
  p.dismissed[key] = true;
  return p;
}

/**
 * Progress with the control key `key` (a scopedTutorialKey result over a
 * console action name) recorded as used. Returns the SAME object when
 * nothing changes.
 */
export function progressWithControlUsed(progress, key) {
  if (!key) return progress;
  const p = normalizeTutorialProgress(progress);
  if (progress && progress.used && progress.used[key]) return progress;
  p.used[key] = true;
  return p;
}

/**
 * Read a dot-path off an object (`'boost_enabled'`, `'own_hull.pct'`,
 * `'systems.helm-thrust.boost_enabled'`). Returns undefined on any miss.
 */
export function readPath(obj, path) {
  if (obj == null || typeof path !== 'string' || path === '') return undefined;
  let cur = obj;
  for (const seg of path.split('.')) {
    if (cur == null || typeof cur !== 'object') return undefined;
    cur = cur[seg];
  }
  return cur;
}

/** Evaluate a `state` trigger's comparison against the console payload. */
function stateConditionHolds(trigger, payload) {
  const actual = readPath(payload, trigger.path);
  const op = trigger.op || 'truthy';
  switch (op) {
    case 'truthy': return !!actual;
    case 'falsy':  return actual !== undefined && !actual;
    case 'eq':     return Number(actual) === Number(trigger.value);
    case 'ne':     return actual !== undefined && Number(actual) !== Number(trigger.value);
    case 'gt':     return Number(actual) > Number(trigger.value);
    case 'gte':    return Number(actual) >= Number(trigger.value);
    case 'lt':     return Number(actual) < Number(trigger.value);
    case 'lte':    return Number(actual) <= Number(trigger.value);
    default:       return false; // unknown op fails closed
  }
}

/**
 * Kind-level trigger condition (the dismissal and control-used gates are
 * applied per-overlay in eligibleOverlays, not here).
 *
 * @param {{ kind: string, control?: string, path?: string, op?: string,
 *           value?: number }} trigger
 * @param {object} payload  the console payload this overlay would sit on
 * @returns {boolean}
 */
export function evaluateTrigger(trigger, payload) {
  if (!trigger || typeof trigger.kind !== 'string') return false;
  switch (trigger.kind) {
    case 'first_visit':
      return true;
    case 'control_unused':
      // The used-gate is shared; the kind only demands a control be named.
      return typeof trigger.control === 'string' && trigger.control !== '';
    case 'state':
      return stateConditionHolds(trigger, payload);
    default:
      return false; // unknown kind fails closed
  }
}

/**
 * The overlays currently eligible to show, sorted by authored `priority`
 * (higher first; stable within equal priority so authored order is the
 * tiebreak).
 *
 * @param {Array} defs      overlay definitions for one station (wire shape)
 * @param {object} progress tutorial progress record (may be null/undefined)
 * @param {object} payload  the console payload the overlay merges into
 * @param {string} hull     current hull id, from Welcome ship_config
 * @param {string} station  station id the defs belong to
 * @returns {Array} eligible definitions
 */
export function eligibleOverlays(defs, progress, payload, hull, station) {
  // Compatibility for callers that have no hull identity (pre-Welcome).
  if (station === undefined) {
    station = hull;
    hull = undefined;
  }
  const p = progress || emptyTutorialProgress();
  const dismissed = p.dismissed || {};
  const used = p.used || {};
  const eligible = (defs || []).filter(def => {
    if (!def || !def.id || !def.trigger) return false;
    if (dismissed[scopedTutorialKey(hull, station, def.id)]) return false;
    if (def.trigger.control && used[scopedTutorialKey(hull, station, def.trigger.control)]) return false;
    return evaluateTrigger(def.trigger, payload);
  });
  // Array.prototype.sort is stable, so equal priorities keep authored order.
  return eligible
    .slice()
    .sort((a, b) => (b.priority || 0) - (a.priority || 0));
}

/**
 * The `tutorial` block merged into every console payload (see
 * `withTutorialOverlay` in gui/console-state.js): the single active overlay
 * plus the count of overlays currently eligible, or null when nothing is.
 *
 * @param {Array} defs
 * @param {object} progress
 * @param {object} payload
 * @param {string} hull     current hull id, from Welcome ship_config
 * @param {string} station  station id the defs belong to
 * @returns {{ active: object, remaining: number }|null}
 */
export function buildTutorialState(defs, progress, payload, hull, station) {
  const eligible = eligibleOverlays(defs, progress, payload, hull, station);
  if (eligible.length === 0) return null;
  return { active: eligible[0], remaining: eligible.length };
}

// ── Persistence (storage-object-injected so tests need no browser) ──────────

/**
 * Load the progress record from a localStorage-like object. Corrupted JSON,
 * a missing key, or a throwing storage all yield a fresh empty record.
 *
 * @param {{ getItem: function }} storage
 * @param {string} [key]
 */
export function loadTutorialProgress(storage, key = TUTORIAL_PROGRESS_KEY) {
  try {
    const raw = storage && storage.getItem(key);
    if (!raw) return emptyTutorialProgress();
    return normalizeTutorialProgress(JSON.parse(raw));
  } catch (_) {
    return emptyTutorialProgress();
  }
}

/**
 * Persist the progress record. Storage errors (quota, privacy mode) are
 * swallowed — the tutorial then simply forgets across reloads.
 *
 * @param {{ setItem: function }} storage
 * @param {object} progress
 * @param {string} [key]
 */
export function saveTutorialProgress(storage, progress, key = TUTORIAL_PROGRESS_KEY) {
  try {
    if (storage) storage.setItem(key, JSON.stringify(normalizeTutorialProgress(progress)));
  } catch (_) { /* best-effort */ }
}

/**
 * Preserve a v2 record for the first identified hull a player visits. A v2
 * key has no hull identity, so applying it to every hull would recreate the
 * leak this version fixes. The migration marker makes this a one-time choice.
 */
export function migrateLegacyTutorialProgress(progress, storage, hull) {
  if (!storage || !hull) return progress;
  try {
    if (storage.getItem(LEGACY_TUTORIAL_MIGRATION_KEY)) return progress;
    const legacy = loadTutorialProgress(storage, LEGACY_TUTORIAL_PROGRESS_KEY);
    const next = normalizeTutorialProgress(progress);
    for (const field of ['dismissed', 'used']) {
      for (const key of Object.keys(legacy[field])) {
        const [station, name, ...extra] = key.split('/');
        if (station && name && extra.length === 0) next[field][scopedTutorialKey(hull, station, name)] = true;
      }
    }
    storage.setItem(LEGACY_TUTORIAL_MIGRATION_KEY, '1');
    return next;
  } catch (_) {
    return progress;
  }
}

/** Ensure the in-memory record has received its one safe v2 migration. */
export function migrateTutorialProgressForHull(sim, storage) {
  if (!sim || !sim.hullId) return false;
  const before = sim.tutorialProgress || emptyTutorialProgress();
  const next = migrateLegacyTutorialProgress(before, storage, sim.hullId);
  if (next === before) return false;
  sim.tutorialProgress = next;
  saveTutorialProgress(storage, next);
  return true;
}

/**
 * Fold one console action envelope into the progress record. The envelope's
 * `console` field (injected by every console's sendAction, gui/console-core.js)
 * scopes the bookkeeping keys; an envelope without one records nothing (fail
 * closed — never guess a station).
 *
 * - `tutorial_dismiss` records `<hull>/<console>/<overlay_id>` as dismissed and is
 *   HANDLED locally: the caller must not forward it to the host (it is
 *   presentation state, not a ship command).
 * - Every other action records `<console>/<action>` as used, completing any
 *   `control_unused` overlay (and any overlay whose trigger names it as
 *   `control`) on that station, then flows to the normal dispatch unchanged.
 *
 * @param {object} progress
 * @param {{ action?: string, console?: string, overlay_id?: string }} action
 * @returns {{ progress: object, changed: boolean, handled: boolean }}
 */
export function tutorialProgressAfterAction(progress, action, hull) {
  const name = action && action.action;
  const station = action && action.console;
  if (!name) return { progress, changed: false, handled: false };
  if (name === TUTORIAL_DISMISS_ACTION) {
    // Handled even when malformed — a dismiss must never reach the host.
    const next = (station && action.overlay_id)
      ? progressWithDismissed(progress, scopedTutorialKey(hull, station, action.overlay_id))
      : progress;
    return { progress: next, changed: next !== progress, handled: true };
  }
  if (!station) return { progress, changed: false, handled: false };
  const next = progressWithControlUsed(progress, scopedTutorialKey(hull, station, name));
  return { progress: next, changed: next !== progress, handled: false };
}

// ── Hydration into the sim-state singleton ──────────────────────────────────

/**
 * Point `sim.tutorialProgress` at the record persisted in `storage`.
 * Exported for tests; production use is the module-scope call below.
 *
 * @param {{ tutorialProgress?: object }} sim      a ClientSimState (or stand-in)
 * @param {{ getItem: function }|null}   storage   localStorage-like object
 */
export function hydrateTutorialProgress(sim, storage) {
  if (!sim) return;
  sim.tutorialProgress = loadTutorialProgress(storage);
}

// Hydrate the singleton once, at module evaluation. The explicit `simState`
// import guarantees sim-state.js has already evaluated, wherever this module
// enters the graph — in client.html that is via the gui/console-state.js
// script tag, long before the sim-state.js tag. Welcome's reset() preserves
// the field, so a reconnect never replays dismissed tips.
try {
  if (typeof localStorage !== 'undefined') {
    hydrateTutorialProgress(simState, localStorage);
  }
} catch (_) { /* privacy-mode storage access can throw; keep the default */ }

// ── Window exposure (for the non-module inline script in client.html) ───────

if (typeof window !== 'undefined') {
  /**
   * Apply one console action to `simState.tutorialProgress`, persisting on
   * change. Returns `{ changed, handled }` — `handled` means "do not forward
   * this action to the host" (tutorial dismissals are client-local).
   */
  window.applyTutorialAction = function applyTutorialAction(simState, action, storage) {
    if (!simState) return { changed: false, handled: false };
    const before = simState.tutorialProgress || emptyTutorialProgress();
    migrateTutorialProgressForHull(simState, storage);
    const current = simState.tutorialProgress || before;
    const { progress, changed, handled } = tutorialProgressAfterAction(current, action, simState.hullId);
    if (changed) {
      simState.tutorialProgress = progress;
      saveTutorialProgress(storage, progress);
    }
    return { changed, handled };
  };
}
