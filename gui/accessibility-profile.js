/**
 * gui/accessibility-profile.js — the private, per-player Accessibility profile
 * (issue #1102).
 *
 * A2 (design: accessibility-private-effect-profile) owns ONE shared
 * Accessibility settings surface and a private per-player profile whose
 * settings name FUNCTIONAL EFFECTS — text scale, contrast, motion — and never
 * a diagnosis or an inferred reason. OS preferences supply the DEFAULT layer
 * where the browser exposes one (`matchMedia`); an explicit player choice
 * overrides it in BOTH directions — it can turn an effect on when the OS is
 * silent AND turn it back off when the OS asked for it (e.g. re-allow motion).
 *
 * The profile is CLIENT-LOCAL and stays that way. It lives in localStorage and
 * on the client-only `simState.accessibilityProfile` field, exactly like
 * `tutorialProgress` — never a `ClientMessage`, never part of shared simulation
 * state, never sent to another player (issue #1102 AC5). `ClientSimState.reset()`
 * preserves the field, so it survives a Welcome / reconnect for free.
 *
 * Two schemas live here, deliberately apart:
 *
 *   1. PRESENTATION effects (this file's `applyAccessibilityProfile`) — cheap
 *      display settings that visibly change the player's OWN console surfaces.
 *      Text scale is proven end to end: the profile sets `--a11y-text-scale` on
 *      the shell `:root` and each same-origin console iframe `:root`, and the
 *      console root font-size multiplies by it (gui/console.css), so every
 *      rem-based string on every console scales at once (AC3).
 *
 *   2. A per-function ASSISTANCE schema (`ASSISTANCE_FUNCTIONS`) — DECLARED but
 *      INERT in A2 (AC4). It can carry a requested-assistance state per station
 *      function so a LATER band (#1103+) can evaluate it locally into an
 *      anonymous eligibility result. It is separate from Station Rating (which
 *      is server-side: src/ship/rating.rs) — assistance is a per-function
 *      override layered SEPARATELY from the station rating
 *      (design: accessibility-station-eligibility-contract). No AI runs here.
 *
 * DOM-free apart from `applyEffectsToRoot`/`applyAccessibilityProfile`, which
 * only write CSS vars/attributes. The ONE import-time side effect is hydrating
 * `simState.accessibilityProfile` from localStorage (browser only). Unit-tested
 * in tests/client/accessibility-profile.test.js and, for the observable effect,
 * tests/client/accessibility-presentation.test.js.
 */

// Explicit ES import so the hydration at the bottom runs AFTER sim-state.js has
// evaluated, wherever this module enters the graph — the same ordering
// guarantee gui/tutorial-state.js relies on.
import { simState } from './sim-state.js';

/** Versioned localStorage key. Bump the `-vN` suffix whenever the record shape
 *  changes so a stale record is discarded, not misread. */
export const ACCESSIBILITY_PROFILE_KEY = 'phoenix-accessibility-v1';

/** The CSS custom property the text-scale effect drives on every `:root`. */
export const TEXT_SCALE_VAR = '--a11y-text-scale';

// ── Presentation effect vocabulary ──────────────────────────────────────────

/** Text-scale slider bounds (whole-percent stops from 100% to 150%). The
 *  resolver clamps to a wider absolute range so a hand-edited record cannot
 *  push the root font-size somewhere unusable. */
export const TEXT_SCALE_MIN = 1.0;
export const TEXT_SCALE_MAX = 1.5;
export const TEXT_SCALE_STEP = 0.05;
export const TEXT_SCALE_DEFAULT = 1.0;
const TEXT_SCALE_FLOOR = 0.5;
const TEXT_SCALE_CEIL = 2.0;

/**
 * Tri-state for an effect the OS can default: `default` follows the OS
 * preference, `on` forces the effect regardless of the OS, `off` forces it OFF
 * even when the OS asked for it. This is what lets an explicit choice override
 * an OS default in either direction.
 */
export const FOLLOW_OS = 'default';
export const EXPLICIT_ON = 'on';
export const EXPLICIT_OFF = 'off';
const TRI_STATES = new Set([FOLLOW_OS, EXPLICIT_ON, EXPLICIT_OFF]);

/** The presentation effects the profile carries. */
export const PRESENTATION_EFFECTS = Object.freeze(['textScale', 'contrast', 'reducedMotion']);
const TRI_EFFECTS = new Set(['contrast', 'reducedMotion']);

// ── Assistance schema (AC4 — declared but inert) ────────────────────────────

/** Inert-by-default assistance state for one station function. `off` is the
 *  default (no assistance); `request` is the "please assist this function"
 *  state a later band will evaluate. Nothing in A2 acts on either. */
export const ASSIST_OFF = 'off';
export const ASSIST_REQUEST = 'request';
const ASSIST_STATES = new Set([ASSIST_OFF, ASSIST_REQUEST]);

/**
 * The station functions a per-function assistance override may key onto. These
 * are machine identifiers (never display text), scoped `station.function`.
 * They exist so the profile can REPRESENT later AI assistance without any of it
 * being implemented in A2 — #1103 evaluates this schema locally, on the client,
 * into an anonymous eligible/ineligible result. Kept here, separate from the
 * server-side Station Rating, because assistance is layered separately from the
 * rating (design: accessibility-station-eligibility-contract).
 */
export const ASSISTANCE_FUNCTIONS = Object.freeze([
  'helm.course-keeping',
  'tactical.target-selection',
  'sensors.contact-triage',
  'comms.dialogue-timing',
]);

// ── Profile construction / normalisation ────────────────────────────────────

/** A fresh profile: every effect unset (follows the OS), no assistance. */
export function emptyAccessibilityProfile() {
  return {
    presentation: {
      textScale: FOLLOW_OS,
      contrast: FOLLOW_OS,
      reducedMotion: FOLLOW_OS,
    },
    assistance: {},
  };
}

/** Clamp a text-scale number to the safe absolute range. */
export function clampTextScale(value) {
  const n = Number(value);
  if (!Number.isFinite(n)) return TEXT_SCALE_DEFAULT;
  return Math.min(TEXT_SCALE_CEIL, Math.max(TEXT_SCALE_FLOOR, n));
}

function normalizeTri(value) {
  return TRI_STATES.has(value) ? value : FOLLOW_OS;
}

function normalizeTextScale(value) {
  if (value === FOLLOW_OS) return FOLLOW_OS;
  if (typeof value === 'number' && Number.isFinite(value)) return clampTextScale(value);
  return FOLLOW_OS;
}

/**
 * Coerce an untrusted value (parsed localStorage JSON, missing field, old
 * schema) into a valid profile. Never throws. Only assistance overrides that
 * differ from the `off` default are kept, so the stored record stays minimal.
 * @param {*} raw
 */
export function normalizeAccessibilityProfile(raw) {
  const p = emptyAccessibilityProfile();
  if (!raw || typeof raw !== 'object') return p;
  const pres = raw.presentation && typeof raw.presentation === 'object' ? raw.presentation : {};
  p.presentation.textScale = normalizeTextScale(pres.textScale);
  p.presentation.contrast = normalizeTri(pres.contrast);
  p.presentation.reducedMotion = normalizeTri(pres.reducedMotion);
  if (raw.assistance && typeof raw.assistance === 'object') {
    for (const id of ASSISTANCE_FUNCTIONS) {
      const v = raw.assistance[id];
      if (ASSIST_STATES.has(v) && v !== ASSIST_OFF) p.assistance[id] = v;
    }
  }
  return p;
}

/**
 * Profile with one presentation `effect` set to `value`. Returns the SAME input
 * reference when the effective value does not change, so a caller can skip a
 * persist / re-apply. An unknown effect is a no-op.
 *
 * @param {object} profile
 * @param {'textScale'|'contrast'|'reducedMotion'} effect
 * @param {number|string} value  a number or FOLLOW_OS for textScale; a tri-state otherwise
 */
export function profileWithPresentation(profile, effect, value) {
  if (!PRESENTATION_EFFECTS.includes(effect)) return profile;
  const next = effect === 'textScale' ? normalizeTextScale(value) : normalizeTri(value);
  const current = normalizeAccessibilityProfile(profile);
  if (current.presentation[effect] === next) return profile;
  current.presentation[effect] = next;
  return current;
}

/**
 * Profile with the assistance override for `funcId` set to `value`. `off`
 * removes the override entirely. Returns the SAME input when nothing changes.
 * Declared-but-inert: nothing in A2 reads the result.
 */
export function profileWithAssistance(profile, funcId, value) {
  if (!ASSISTANCE_FUNCTIONS.includes(funcId)) return profile;
  const state = ASSIST_STATES.has(value) ? value : ASSIST_OFF;
  const current = normalizeAccessibilityProfile(profile);
  const had = current.assistance[funcId] || ASSIST_OFF;
  if (had === state) return profile;
  if (state === ASSIST_OFF) delete current.assistance[funcId];
  else current.assistance[funcId] = state;
  return current;
}

// ── Anonymous station/rating eligibility (issue #1103) ───────────────────────
//
// The client mirrors the RUST rule (`src/ship/eligibility.rs`) from a PROJECTED
// table the host sends on Welcome (`ShipClientConfig.station_assist_gaps`): per
// station → per rating → the assist-function ids that station would force its
// holder to operate MANUALLY at that rating. The player's PRIVATE profile is
// applied here, locally, and only the derived result ever leaves the device —
// the anonymous ineligible station-id list. The functional reason stays local,
// for the AC1 explanation shown to this player alone.

/** The assist-functions this profile requests help with (ASSIST_REQUEST). */
export function requestedAssistFunctions(profile) {
  const p = normalizeAccessibilityProfile(profile);
  return ASSISTANCE_FUNCTIONS.filter((id) => p.assistance[id] === ASSIST_REQUEST);
}

/**
 * Evaluate ONE complete station surface at `requiredRating` against the private
 * profile. Returns BOTH shapes:
 *   - `eligible`: the anonymous boolean the host is allowed to know.
 *   - `reason`: the PRIVATE functional explanation (the requested assist-function
 *     ids this station cannot cover) — `null` when eligible. NEVER sent to the
 *     host or another player; it drives only the local AC1 explanation.
 *
 * `stationGaps` is the projection entry for this station (`{ [rating]: [funcId] }`);
 * a missing station or rating means "no gaps ⇒ eligible" — the permissive default
 * that mirrors the host side-map's DEFAULT TRUE.
 *
 * @param {object} profile  the private accessibility profile
 * @param {Object<string,string[]>|null|undefined} stationGaps
 * @param {string} requiredRating
 * @returns {{ eligible: boolean, reason: { functions: string[] } | null }}
 */
export function deriveStationEligibility(profile, stationGaps, requiredRating) {
  const requested = requestedAssistFunctions(profile);
  if (requested.length === 0) {
    return { eligible: true, reason: null };
  }
  const gaps = (stationGaps && stationGaps[requiredRating]) || [];
  const blocked = requested.filter((id) => gaps.includes(id));
  if (blocked.length === 0) {
    return { eligible: true, reason: null };
  }
  return { eligible: false, reason: { functions: blocked } };
}

/**
 * The ANONYMOUS ineligible-station set to report to the host (issue #1103 §4):
 * the sorted list of station ids the profile is ineligible for, and NOTHING
 * else — no settings, no rating, no reason. Mirrors what the host stores.
 *
 * @param {object} profile
 * @param {Object<string,Object<string,string[]>>} allStationGaps
 *        the full `station_assist_gaps` projection (per station → per rating).
 * @param {(stationId: string) => string} ratingFor
 *        the required rating for each station (direct-claim rating for a
 *        claimable seat, visiting rating for a human-seeking station).
 * @param {string[]} stationIds  the stations to evaluate.
 * @returns {string[]} sorted ineligible station ids.
 */
export function computeIneligibleStations(profile, allStationGaps, ratingFor, stationIds) {
  const gaps = allStationGaps || {};
  const out = [];
  for (const id of stationIds || []) {
    const result = deriveStationEligibility(profile, gaps[id], ratingFor(id));
    if (!result.eligible) out.push(id);
  }
  return out.sort();
}

// ── OS-default resolver (matchMedia) ─────────────────────────────────────────

/**
 * The OS-derived DEFAULT layer. Reads the browser's accessibility media queries
 * where they exist; a host without `matchMedia` (or a query that throws) simply
 * yields `false`, i.e. "the OS states no preference". Never throws.
 *
 * @param {Window|null} [win]
 * @returns {{ reducedMotion: boolean, contrast: boolean, darkColorScheme: boolean }}
 */
export function osAccessibilityDefaults(win) {
  const w = win || (typeof window !== 'undefined' ? window : null);
  const query = (q) => {
    try {
      return !!(w && typeof w.matchMedia === 'function' && w.matchMedia(q).matches);
    } catch (_) {
      return false;
    }
  };
  return {
    reducedMotion: query('(prefers-reduced-motion: reduce)'),
    contrast: query('(prefers-contrast: more)'),
    darkColorScheme: query('(prefers-color-scheme: dark)'),
  };
}

/** Resolve a tri-state effect against its OS default: explicit wins both ways. */
export function resolveTriState(explicit, osDefault) {
  if (explicit === EXPLICIT_ON) return true;
  if (explicit === EXPLICIT_OFF) return false;
  return !!osDefault;
}

/** Resolve the stored text-scale value to a concrete multiplier. */
export function resolveTextScale(value) {
  if (value === FOLLOW_OS || value == null) return TEXT_SCALE_DEFAULT;
  return clampTextScale(value);
}

/**
 * The concrete effects to apply, folding the explicit profile over the OS
 * defaults.
 *
 * @param {object} profile
 * @param {{ reducedMotion?: boolean, contrast?: boolean }} [osDefaults]
 * @returns {{ textScale: number, contrast: boolean, reducedMotion: boolean }}
 */
export function resolveEffects(profile, osDefaults) {
  const p = normalizeAccessibilityProfile(profile);
  const os = osDefaults || {};
  return {
    textScale: resolveTextScale(p.presentation.textScale),
    contrast: resolveTriState(p.presentation.contrast, os.contrast),
    reducedMotion: resolveTriState(p.presentation.reducedMotion, os.reducedMotion),
  };
}

// ── Application onto document roots ──────────────────────────────────────────

/**
 * Write the resolved effects onto ONE document root (a `documentElement`).
 * Sets the text-scale CSS var and the motion/contrast attributes. Swallows any
 * DOM error (a detached or cross-origin root) so one bad target never stops the
 * rest.
 *
 * @param {Element|null} root
 * @param {{ textScale: number, contrast: boolean, reducedMotion: boolean }} effects
 */
export function applyEffectsToRoot(root, effects) {
  if (!root || !effects) return;
  try {
    if (root.style && typeof root.style.setProperty === 'function') {
      root.style.setProperty(TEXT_SCALE_VAR, String(effects.textScale));
    }
    if (typeof root.setAttribute === 'function') {
      root.setAttribute('data-reduced-motion', effects.reducedMotion ? 'reduce' : 'no-preference');
      root.setAttribute('data-contrast', effects.contrast ? 'more' : 'standard');
    }
  } catch (_) {
    /* detached / cross-origin root — best-effort */
  }
}

/** Every same-origin console iframe currently mounted under `doc`. */
function collectConsoleIframes(doc) {
  try {
    if (doc && typeof doc.querySelectorAll === 'function') {
      return Array.from(doc.querySelectorAll('.console-section iframe'));
    }
  } catch (_) {
    /* fall through */
  }
  return [];
}

/**
 * Resolve `profile` against the OS defaults and apply it to the shell `:root`
 * AND every same-origin console iframe `:root`. The consoles are same-origin
 * iframes, so their `contentDocument` is reachable — a not-yet-loaded or
 * cross-origin frame is skipped rather than throwing.
 *
 * @param {object} profile
 * @param {{ doc?: Document, win?: Window, iframes?: Array }} [opts]
 * @returns {{ textScale: number, contrast: boolean, reducedMotion: boolean }} the applied effects
 */
export function applyAccessibilityProfile(profile, opts = {}) {
  const win = opts.win || (typeof window !== 'undefined' ? window : null);
  const doc = opts.doc || (win && win.document) || (typeof document !== 'undefined' ? document : null);
  const effects = resolveEffects(profile, osAccessibilityDefaults(win));

  const roots = [];
  if (doc && doc.documentElement) roots.push(doc.documentElement);

  const iframes = opts.iframes || (doc ? collectConsoleIframes(doc) : []);
  for (const frame of iframes) {
    try {
      const idoc = frame && frame.contentDocument;
      if (idoc && idoc.documentElement) roots.push(idoc.documentElement);
    } catch (_) {
      /* cross-origin or not yet loaded — skip */
    }
  }

  for (const root of roots) applyEffectsToRoot(root, effects);
  return effects;
}

// ── Persistence (storage-object-injected so tests need no browser) ───────────

/**
 * Load the profile from a localStorage-like object. Corrupted JSON, a missing
 * key, or a throwing storage all yield a fresh empty profile.
 *
 * @param {{ getItem: function }} storage
 * @param {string} [key]
 */
export function loadAccessibilityProfile(storage, key = ACCESSIBILITY_PROFILE_KEY) {
  try {
    const raw = storage && storage.getItem(key);
    if (!raw) return emptyAccessibilityProfile();
    return normalizeAccessibilityProfile(JSON.parse(raw));
  } catch (_) {
    return emptyAccessibilityProfile();
  }
}

/**
 * Persist the profile PRIVATELY under the versioned key. Storage errors (quota,
 * private mode) are swallowed — the profile then simply forgets across reloads.
 * Writes nowhere else: the record never leaves the device.
 *
 * @param {{ setItem: function }} storage
 * @param {object} profile
 * @param {string} [key]
 */
export function saveAccessibilityProfile(storage, profile, key = ACCESSIBILITY_PROFILE_KEY) {
  try {
    if (storage) storage.setItem(key, JSON.stringify(normalizeAccessibilityProfile(profile)));
  } catch (_) {
    /* best-effort */
  }
}

// ── Hydration into the sim-state singleton ──────────────────────────────────

/**
 * Point `sim.accessibilityProfile` at the record persisted in `storage`.
 * Exported for tests; production use is the module-scope call below.
 *
 * @param {{ accessibilityProfile?: object }} sim
 * @param {{ getItem: function }|null} storage
 */
export function hydrateAccessibilityProfile(sim, storage) {
  if (!sim) return;
  sim.accessibilityProfile = loadAccessibilityProfile(storage);
}

// Hydrate the singleton once, at module evaluation. `reset()` preserves the
// field (like tutorialProgress), so a reconnect keeps the player's explicit
// choices — the profile is independent of the socket.
try {
  if (typeof localStorage !== 'undefined') {
    hydrateAccessibilityProfile(simState, localStorage);
  }
} catch (_) {
  /* privacy-mode storage access can throw; keep the default */
}

// ── Window exposure (for the inline shell + settings panel in client.html) ───

if (typeof window !== 'undefined') {
  /** Apply the CURRENT profile to the shell and any mounted console iframes. */
  window.applyAccessibilityProfile = function applyCurrent(opts) {
    const profile = (window.simState && window.simState.accessibilityProfile)
      || emptyAccessibilityProfile();
    return applyAccessibilityProfile(profile, opts || {});
  };

  /**
   * Update one presentation effect, persist it PRIVATELY, and re-apply. Never
   * touches the socket — the profile is client-local (issue #1102 AC5).
   */
  window.setAccessibilityPresentation = function setPresentation(effect, value) {
    const sim = window.simState;
    if (!sim) return undefined;
    sim.accessibilityProfile = normalizeAccessibilityProfile(
      profileWithPresentation(sim.accessibilityProfile, effect, value),
    );
    let storage = null;
    try { storage = window.localStorage; } catch (_) { /* privacy mode */ }
    saveAccessibilityProfile(storage, sim.accessibilityProfile);
    return window.applyAccessibilityProfile();
  };

  /**
   * Update one per-function assistance override, persisted privately. In A2 the
   * assistance itself is inert (no AI), but the CHANGE re-derives eligibility:
   * after persisting, fire the optional `onAccessibilityAssistanceChanged` hook
   * so the client re-reports its anonymous ineligible set (issue #1103 §4).
   */
  window.setAccessibilityAssistance = function setAssistance(funcId, value) {
    const sim = window.simState;
    if (!sim) return;
    sim.accessibilityProfile = normalizeAccessibilityProfile(
      profileWithAssistance(sim.accessibilityProfile, funcId, value),
    );
    let storage = null;
    try { storage = window.localStorage; } catch (_) { /* privacy mode */ }
    saveAccessibilityProfile(storage, sim.accessibilityProfile);
    if (typeof window.onAccessibilityAssistanceChanged === 'function') {
      try { window.onAccessibilityAssistanceChanged(); } catch (_) { /* best-effort */ }
    }
  };

  // Pure eligibility derivation for the inline lobby glue (issue #1103).
  window.deriveStationEligibility = deriveStationEligibility;
  window.computeIneligibleStations = computeIneligibleStations;
}
