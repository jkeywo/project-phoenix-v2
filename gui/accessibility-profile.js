/**
 * gui/accessibility-profile.js — the private, per-player Accessibility profile
 * (issue #1102).
 *
 * T1 (design: accessibility-private-effect-profile) owns ONE shared
 * Accessibility settings surface and a private per-player profile whose
 * settings name FUNCTIONAL EFFECTS — text scale, contrast, motion — and never
 * a diagnosis or an inferred reason. OS preferences supply the DEFAULT layer
 * where the browser exposes one (`matchMedia`); an explicit player choice
 * overrides it in BOTH directions — it can turn an effect on when the OS is
 * silent AND turn it back off when the OS asked for it (e.g. re-allow motion).
 *
 * The profile is CLIENT-LOCAL and stays that way. Since issue #1279 its current
 * persistence is the enclosing operator profile; the Accessibility-only key
 * below remains the supported migration/old-shell fallback. Live state is on
 * the client-only `simState.accessibilityProfile` field, exactly like
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
 *   2. A per-function ASSISTANCE schema (`ASSISTANCE_FUNCTIONS`) — declared but
 *      assistance-inert in T1 (AC4): storing a request does not run AI. Issue
 *      #1103 evaluates those requests locally into an anonymous eligibility
 *      result. The schema stays separate from Station Rating (server-side:
 *      src/ship/rating.rs) because assistance is a per-function override layered
 *      SEPARATELY from the rating (design:
 *      accessibility-station-eligibility-contract).
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
import {
  EFFECT_IDS,
  FOLLOW_PREFERENCE,
  applyEffectIntensitiesToRoot,
  applicableEffects,
  normalizeEffectLevel,
  reduceEffectsChoices,
  resolveEffectIntensities,
} from './visual-effects.js';

/** Pre-#1279 Accessibility-only localStorage key, retained for migration. */
export const ACCESSIBILITY_PROFILE_KEY = 'phoenix-accessibility-v1';

/** The CSS custom property the text-scale effect drives on every `:root`. */
export const TEXT_SCALE_VAR = '--a11y-text-scale';

// ── Presentation effect vocabulary ──────────────────────────────────────────

/** Text-scale slider bounds — whole-percent stops from 100% to **200%**
 *  (issue #1422, PRD #1418: "provide usable in-app text enlargement through
 *  200%"). The exposed ceiling was 150% until that PRD; raising it is a change
 *  to a CONTRACT, not just a number, because the native bridge reasons over the
 *  same range: `SUPPORTED_TEXT_SCALE_MIN`/`MAX` in
 *  `src/native_host/setup_accessibility.rs` mirror these two, and the drift
 *  guard `the_supported_extremes_match_the_client` pins the pair. A pane's
 *  reflow demand is `MIN_CONSOLE_LOGICAL_WIDTH_PX x scale`, so 200% asks a
 *  split pane for 640 logical px of width rather than 480.
 *
 *  `TEXT_SCALE_FLOOR`/`CEIL` remain the absolute clamp a hand-edited or
 *  host-injected record is coerced into. The ceiling is now the SAME number as
 *  the slider maximum — deliberately: 200% is the largest scale this client
 *  claims its consoles reflow at, so a record asking for 300% is clamped to the
 *  largest supported value rather than honoured into a layout nobody has
 *  verified. The floor still guards the other direction, where there is no
 *  exposed control at all. */
export const TEXT_SCALE_MIN = 1.0;
export const TEXT_SCALE_MAX = 2.0;
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
export const FOLLOW_OS = FOLLOW_PREFERENCE;
export const EXPLICIT_ON = 'on';
export const EXPLICIT_OFF = 'off';
const TRI_STATES = new Set([FOLLOW_OS, EXPLICIT_ON, EXPLICIT_OFF]);

/**
 * The presentation effects the profile carries.
 *
 * The first three are the T1 settings: a text-scale number, and two tri-states.
 * The last three are issue #1428's separate visual effects — camera/page shake,
 * flash/pulse and decorative motion — each an intensity in `0..=1` or
 * follow-the-preference, with their vocabulary in `gui/visual-effects.js`.
 *
 * They live in ONE list because every consumer here iterates it: the
 * normaliser, the writer, the scoped Reset all and the status projection. That
 * is what made adding three effects a change to this array rather than to four
 * places, and it is what keeps "Reset all is scoped to the presentation
 * settings" true of the new three without anybody remembering to add them.
 */
export const PRESENTATION_EFFECTS = Object.freeze(
  ['textScale', 'contrast', 'reducedMotion'].concat(EFFECT_IDS),
);
const TRI_EFFECTS = new Set(['contrast', 'reducedMotion']);
const INTENSITY_EFFECTS = new Set(EFFECT_IDS);

// ── Assistance schema (AC4 — declared, with no AI side effect) ──────────────

/** Inert-by-default assistance state for one station function. `off` is the
 *  default (no assistance); `request` is the "please assist this function"
 *  state #1103 evaluates for anonymous eligibility. Neither state itself runs
 *  an AI behaviour. */
export const ASSIST_OFF = 'off';
export const ASSIST_REQUEST = 'request';
const ASSIST_STATES = new Set([ASSIST_OFF, ASSIST_REQUEST]);

/**
 * The station functions a per-function assistance override may key onto. These
 * are machine identifiers (never display text), scoped `station.function`.
 * They exist so the profile can REPRESENT later AI assistance without any of it
 * being implemented in T1 — #1103 evaluates this schema locally, on the client,
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
  const presentation = {};
  for (const effect of PRESENTATION_EFFECTS) presentation[effect] = FOLLOW_OS;
  return { presentation, assistance: {} };
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

/** One presentation value, coerced by the rule its effect is governed by: a
 *  clamped text-scale multiplier, a `0..=1` effect intensity, or a tri-state. */
function normalizePresentationValue(effect, value) {
  if (effect === 'textScale') return normalizeTextScale(value);
  if (INTENSITY_EFFECTS.has(effect)) return normalizeEffectLevel(value);
  return normalizeTri(value);
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
  // A profile written before issue #1428 carries none of these; each reads as
  // follow-the-preference, which is exactly the behaviour that record had.
  for (const effect of EFFECT_IDS) {
    p.presentation[effect] = normalizeEffectLevel(pres[effect]);
  }
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
  const next = normalizePresentationValue(effect, value);
  const current = normalizeAccessibilityProfile(profile);
  if (current.presentation[effect] === next) return profile;
  current.presentation[effect] = next;
  return current;
}

/**
 * Profile with EVERY presentation effect returned to its documented default —
 * follow-the-system for all three — and nothing else touched (issue #1422).
 *
 * This is the whole of "Reset all" on a presentation surface, and its SCOPE is
 * the point (PRD #1418: "Reset all is scoped to the current presentation
 * settings, not unrelated bindings, identity or save data"). It rebuilds the
 * `presentation` block only; the `assistance` overrides ride through untouched,
 * and everything outside this profile — semantic bindings, gamepad tuning,
 * feedback preferences, GM confirmation policy, scenario saves, the operator's
 * identity — is not reachable from here at all, by construction rather than by
 * a careful list.
 *
 * Returns the SAME input reference when every effect is already at its default,
 * so a caller can skip a persist / re-apply, exactly like
 * `profileWithPresentation`.
 *
 * @param {object} profile
 */
export function profileWithPresentationDefaults(profile) {
  const current = normalizeAccessibilityProfile(profile);
  const alreadyDefault = PRESENTATION_EFFECTS
    .every((effect) => current.presentation[effect] === FOLLOW_OS);
  if (alreadyDefault) return profile;
  for (const effect of PRESENTATION_EFFECTS) current.presentation[effect] = FOLLOW_OS;
  return current;
}

/**
 * Profile with the assistance override for `funcId` set to `value`. `off`
 * removes the override entirely. Returns the SAME input when nothing changes.
 * Declared-but-assistance-inert: eligibility reads the result, but changing it
 * does not itself run AI.
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
 * A host-injected OS default layer, when one is present.
 *
 * A browser reads its OS accessibility preferences through `matchMedia`. A
 * native Ultralight pane (issue #1127) loads the SAME page but has no OS-backed
 * `matchMedia`, so its host reads the machine's real preferences natively and
 * injects them as `window.PhoenixOsAccessibilityDefaults` — the documented seam,
 * exactly the way `gui/rendezvous-transport.js` reads
 * `window.PhoenixTransportFactories`. In an ordinary browser the global is
 * absent and this returns `{}`, so nothing about the matchMedia path changes.
 *
 * Only well-typed fields are honoured, so a malformed injection degrades to "no
 * preference" per field rather than breaking resolution. It is the DEFAULT layer
 * ONLY: an explicit player choice still overrides it in both directions (see
 * `resolveEffects`). Text scale is clamped to the safe absolute range here too,
 * so a hand-poked global cannot push a console's root font-size somewhere
 * unusable.
 *
 * @param {Window|object|null} w
 * @returns {{ reducedMotion?: boolean, contrast?: boolean, darkColorScheme?: boolean, textScale?: number }}
 */
export function readInjectedOsDefaults(w) {
  let raw = null;
  try {
    raw = w && w.PhoenixOsAccessibilityDefaults;
  } catch (_) {
    raw = null; // a getter that throws must not cost us the matchMedia layer
  }
  if (!raw || typeof raw !== 'object') return {};
  const out = {};
  for (const key of ['reducedMotion', 'contrast', 'darkColorScheme']) {
    if (typeof raw[key] === 'boolean') out[key] = raw[key];
  }
  if (typeof raw.textScale === 'number' && Number.isFinite(raw.textScale)) {
    out.textScale = clampTextScale(raw.textScale);
  }
  return out;
}

/** Failed native reads are distinct from a successfully read neutral value. */
export function unavailableOsPreferences(w) {
  try {
    const available = w?.PhoenixOsAccessibilityDefaults?.availability;
    return ['textScale', 'contrast', 'reducedMotion'].filter(key => available?.[key] === false);
  } catch (_) {
    return [];
  }
}

/**
 * The OS-derived DEFAULT layer. Reads the browser's accessibility media queries
 * where they exist; a host without `matchMedia` (or a query that throws) simply
 * yields `false`, i.e. "the OS states no preference". A native host may inject
 * `window.PhoenixOsAccessibilityDefaults`, which overlays the matchMedia result
 * per field (issue #1127) and is the ONLY source of a `textScale` default —
 * matchMedia has no text-scale query. Never throws.
 *
 * @param {Window|null} [win]
 * @returns {{ reducedMotion: boolean, contrast: boolean, darkColorScheme: boolean, textScale?: number }}
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
    // A native pane's OS preferences, where the host injected them. Absent in a
    // browser, so this spread is a no-op there and the shape is unchanged.
    ...readInjectedOsDefaults(w),
  };
}

/** Resolve a tri-state effect against its OS default: explicit wins both ways. */
export function resolveTriState(explicit, osDefault) {
  if (explicit === EXPLICIT_ON) return true;
  if (explicit === EXPLICIT_OFF) return false;
  return !!osDefault;
}

/**
 * Resolve the stored text-scale value to a concrete multiplier. An unset
 * (follow-OS) value takes the OS default where one is supplied — a native pane
 * imports the Windows text-size setting this way (issue #1127) — and otherwise
 * the identity. An explicit numeric choice always wins.
 *
 * @param {number|string} value
 * @param {number} [osTextScale]  the OS default multiplier, when supplied
 */
export function resolveTextScale(value, osTextScale) {
  if (value === FOLLOW_OS || value == null) {
    return osTextScale == null ? TEXT_SCALE_DEFAULT : clampTextScale(osTextScale);
  }
  return clampTextScale(value);
}

/**
 * The concrete effects to apply, folding the explicit profile over the OS
 * defaults. A native pane's `osDefaults` may carry a `textScale` (imported from
 * the Windows text-size setting, issue #1127); a browser's never does, so an
 * unset text scale stays the identity there.
 *
 * @param {object} profile
 * @param {{ reducedMotion?: boolean, contrast?: boolean, textScale?: number }} [osDefaults]
 * @returns {{ textScale: number, contrast: boolean, reducedMotion: boolean }}
 */
export function resolveEffects(profile, osDefaults) {
  const p = normalizeAccessibilityProfile(profile);
  const os = osDefaults || {};
  const reducedMotion = resolveTriState(p.presentation.reducedMotion, os.reducedMotion);
  return {
    textScale: resolveTextScale(p.presentation.textScale, os.textScale),
    contrast: resolveTriState(p.presentation.contrast, os.contrast),
    reducedMotion,
    // The three separate effects (issue #1428) fold over the SAME resolved
    // motion preference, so an operator who has only ever used the Motion
    // control keeps exactly the behaviour they had: unset effects are 0 under
    // reduce and 1 otherwise. An explicit intensity overrides it in both
    // directions, like every other explicit value in this resolver.
    ...resolveEffectIntensities(p.presentation, reducedMotion),
  };
}

/**
 * Where each presentation effect's live value actually CAME FROM, alongside the
 * value itself — the honest answer to "is this my choice, my system's, or the
 * built-in default?" (issue #1422; PRD #1418 stories 9 and 16).
 *
 * The stored profile alone cannot answer that. `default` in the record means
 * *follow the system*, and the system may be saying something (Windows asked
 * for 150% text, the browser reports `prefers-reduced-motion: reduce`) or
 * saying nothing at all. Those are three different states behind one stored
 * value, and a settings surface that prints only the resolved number tells the
 * operator nothing about whether changing their OS would move it:
 *
 *   - `explicit`  — the operator chose this value here. It overrides the system
 *                   in BOTH directions (see `resolveTriState`).
 *   - `system`    — following the system, and the system supplied this value.
 *   - `default`   — following the system, and the system asked for nothing, so
 *                   the documented built-in default applies.
 *
 * `available` is `false` when the host tried to read this preference natively
 * and failed (`unavailableOsPreferences`). That is deliberately distinct from a
 * successful read of a neutral value: "we could not ask" is not "the answer was
 * no", and only the first of those is worth telling the operator about.
 *
 * Pure and never throws — it composes the same resolver the application path
 * uses, so a status line cannot disagree with what is actually on screen.
 *
 * @param {object} profile
 * @param {{ reducedMotion?: boolean, contrast?: boolean, textScale?: number }} [osDefaults]
 * @param {string[]} [unavailable] effect ids whose native read failed
 * @returns {{ [effect: string]: { value: number|boolean, source: 'explicit'|'system'|'default', available: boolean } }}
 */
export function presentationStatus(profile, osDefaults, unavailable) {
  const p = normalizeAccessibilityProfile(profile);
  const os = osDefaults || {};
  const effects = resolveEffects(p, os);
  const missing = new Set(Array.isArray(unavailable) ? unavailable : []);
  const sourceOf = (effect) => {
    if (p.presentation[effect] !== FOLLOW_OS) return 'explicit';
    // Following the system: `system` only when the system actually SAID
    // something. A silent OS is the documented default, not a system value.
    //
    // The three separate effects (issue #1428) have no system query of their
    // own — no OS reports a camera-shake preference — so the system signal they
    // follow is the motion preference, exactly the value they resolve against.
    // An explicit Motion choice is the operator's, not the system's, so it does
    // NOT make a following effect read as `system`.
    if (INTENSITY_EFFECTS.has(effect)) return os.reducedMotion === true ? 'system' : 'default';
    const signal = effect === 'textScale' ? os.textScale != null : os[effect] === true;
    return signal ? 'system' : 'default';
  };
  const out = {};
  for (const effect of PRESENTATION_EFFECTS) {
    const source = sourceOf(effect);
    out[effect] = {
      value: effects[effect],
      source,
      // An unreadable OS preference cannot invalidate an explicit choice — that
      // value never depended on the read. A following EFFECT depends on the
      // motion read rather than on one of its own, so it inherits that answer.
      available: source === 'explicit'
        ? true
        : !missing.has(INTENSITY_EFFECTS.has(effect) ? 'reducedMotion' : effect),
    };
  }
  return out;
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
  // The per-effect bands and intensities (issue #1428). Written alongside
  // `data-reduced-motion` rather than instead of it: the older attribute still
  // carries the OVERALL preference that unset effects follow, and the rules in
  // gui/tokens.css that are not one of the three named effects still read it.
  applyEffectIntensitiesToRoot(root, effects);
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

// ── Legacy persistence (migration/old-shell fallback) ────────────────────────

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
    if (typeof window.persistOperatorProfile === 'function') {
      window.persistOperatorProfile();
    } else {
      // The compatibility path is used by the standalone module tests and by
      // an old cached shell that has not loaded operator-profile.js yet.
      let storage = null;
      try { storage = window.localStorage; } catch (_) { /* privacy mode */ }
      saveAccessibilityProfile(storage, sim.accessibilityProfile);
    }
    return window.applyAccessibilityProfile();
  };

  /**
   * Return EVERY presentation effect to its documented default, persist, and
   * re-apply (issue #1422). The scoped "Reset all" the settings surface offers.
   *
   * It goes through the same `persistOperatorProfile()` hook a single-effect
   * change does, which is what keeps the scope honest: that hook snapshots the
   * live bindings, gamepad tuning, feedback preferences and GM confirmation
   * policy from their own owners and writes them back unchanged. This function
   * never reads or names them, so it cannot clear one by accident, and it has
   * no path at all to scenario saves or to the operator's identity.
   */
  window.resetAccessibilityPresentation = function resetPresentation() {
    const sim = window.simState;
    if (!sim) return undefined;
    sim.accessibilityProfile = normalizeAccessibilityProfile(
      profileWithPresentationDefaults(sim.accessibilityProfile),
    );
    if (typeof window.persistOperatorProfile === 'function') {
      window.persistOperatorProfile();
    } else {
      let storage = null;
      try { storage = window.localStorage; } catch (_) { /* privacy mode */ }
      saveAccessibilityProfile(storage, sim.accessibilityProfile);
    }
    return window.applyAccessibilityProfile();
  };

  /**
   * The **Reduce effects** preset (issue #1428, PRD #1418 story 14): one press
   * writing conservative values for every effect the CONSOLE surface can
   * actually render, persisted and previewed through the same path a single
   * control uses.
   *
   * It writes EXPLICIT intensities rather than turning the Motion tri-state on,
   * which is the difference between a preset and a second master switch: after
   * pressing it each control shows a chosen value, each can be moved on its own,
   * and a per-setting reset returns just that one to following the preference.
   *
   * Scoped to `applicableEffects('console')`, so it cannot quietly store a
   * camera-shake value on a surface that has no camera — see the inventory in
   * `gui/visual-effects.js`.
   */
  window.reduceAccessibilityEffects = function reduceEffects() {
    const sim = window.simState;
    if (!sim) return undefined;
    let profile = sim.accessibilityProfile;
    const choices = reduceEffectsChoices('console');
    for (const effect of Object.keys(choices)) {
      profile = profileWithPresentation(profile, effect, choices[effect]);
    }
    sim.accessibilityProfile = normalizeAccessibilityProfile(profile);
    if (typeof window.persistOperatorProfile === 'function') {
      window.persistOperatorProfile();
    } else {
      let storage = null;
      try { storage = window.localStorage; } catch (_) { /* privacy mode */ }
      saveAccessibilityProfile(storage, sim.accessibilityProfile);
    }
    return window.applyAccessibilityProfile();
  };

  /** The effects the private console surface offers controls for, for the
   *  inline shell (issue #1428). */
  window.applicableAccessibilityEffects = function applicable() {
    return applicableEffects('console');
  };

  /**
   * Update one per-function assistance override, persisted privately. In T1 the
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
    if (typeof window.persistOperatorProfile === 'function') {
      window.persistOperatorProfile();
    } else {
      let storage = null;
      try { storage = window.localStorage; } catch (_) { /* privacy mode */ }
      saveAccessibilityProfile(storage, sim.accessibilityProfile);
    }
    if (typeof window.onAccessibilityAssistanceChanged === 'function') {
      try { window.onAccessibilityAssistanceChanged(); } catch (_) { /* best-effort */ }
    }
  };

  // Pure eligibility derivation for the inline lobby glue (issue #1103).
  window.deriveStationEligibility = deriveStationEligibility;
  window.computeIneligibleStations = computeIneligibleStations;
}
