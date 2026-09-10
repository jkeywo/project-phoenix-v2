/**
 * gui/viewscreen-presentation.js — the Viewscreen's OWN presentation record
 * (issue #1427, PRD #1418 stories 4, 9, 11, 12, 16, 17).
 *
 * The Viewscreen is a television in a room. Its text size and contrast are a
 * property of THAT DISPLAY — the projector at the back of the hall, the 4K panel
 * two feet from the GM — and not of any person: whoever walks up to it changes
 * it for the room, and the room keeps the change next week. That is a different
 * fact from `gui/accessibility-profile.js`'s private operator profile, which is
 * one player's own choice travelling with them between devices, and the two must
 * not be able to overwrite each other.
 *
 * So this is a SECOND record, deliberately:
 *
 * | | private operator profile | this |
 * |---|---|---|
 * | whose | one player's | this endpoint's |
 * | where | `phoenix-operator-profile-v1`, portable/exportable | `phoenix-viewscreen-presentation-v1`, on the endpoint only |
 * | who edits it | the phone client's Accessibility tab | the Viewscreen settings cog (browser `gui/server-settings.js`, native `gui/native-settings.js`) |
 * | what it covers | text scale, contrast, motion, assistance, bindings, … | this display's text scale and contrast |
 *
 * Neither module reads or writes the other's key, so "the shared display was
 * turned up for the room" cannot follow a player home, and a private profile
 * import cannot silently re-scale the Viewscreen. That isolation is the whole
 * reason for a separate key rather than a section inside the operator profile,
 * and `tests/client/viewscreen-presentation.test.js` pins it in both directions.
 *
 * ## What is NOT duplicated
 *
 * The vocabulary, the clamps, the follow-system resolution and the
 * explicit/system/default status all come from `accessibility-profile.js` —
 * imported, not restated. A Viewscreen that clamped text scale differently from
 * a console would be a second contract, and the native bridge already mirrors
 * the one (`src/native_host/setup_accessibility.rs`). What is genuinely this
 * module's own is the SCOPE (which effects a shared display offers), the STORE
 * (endpoint-local, and on native a host-side file rather than a browser) and
 * the APPLICATION (this document's root only — never a console iframe, which
 * belongs to whoever is sitting at it).
 *
 * ## Scope: two effects, and why not three
 *
 * Text scale and contrast. Motion/shake/flash are the next slice's (#1428,
 * PRD #1418 stories 13-15), which wires the effect controls to their real
 * consumers on every settings surface at once — `server.html` already drives
 * `wasm_set_reduced_motion` from `matchMedia`, and putting a half of that lever
 * here would give the room two places to ask for the same thing. Adding it is
 * one entry in [`VIEWSCREEN_EFFECTS`] plus its rows, by design.
 *
 * DOM-free and window-free at import time, so vitest can import it in Node.
 */

import {
  FOLLOW_OS,
  EXPLICIT_ON,
  EXPLICIT_OFF,
  TEXT_SCALE_VAR,
  clampTextScale,
  osAccessibilityDefaults,
  presentationStatus,
  resolveTextScale,
  resolveTriState,
  unavailableOsPreferences,
} from './accessibility-profile.js';

export { FOLLOW_OS, EXPLICIT_ON, EXPLICIT_OFF };

/**
 * The endpoint-local storage key.
 *
 * Distinct from `OPERATOR_PROFILE_KEY` and never migrated from it: a browser
 * that is BOTH a viewscreen and a console (a developer's laptop with two tabs)
 * holds both records side by side under one origin, and each surface reads only
 * its own. The `-v1` suffix is the schema version in the name, as every other
 * persisted record in this repository spells it.
 */
export const VIEWSCREEN_PRESENTATION_KEY = 'phoenix-viewscreen-presentation-v1';

/** The record's self-declared identity, so a hand-edited or foreign JSON blob
 *  that happens to land on this key is recognisable rather than assumed. */
export const VIEWSCREEN_PRESENTATION_KIND = 'project-phoenix/viewscreen-presentation';

/** Schema version carried inside the record. */
export const VIEWSCREEN_PRESENTATION_VERSION = 1;

/**
 * The effects this endpoint's menu offers, in display order.
 *
 * A list rather than three named fields, because every consumer below (the
 * normaliser, the scoped reset, the status projection, the two panels) iterates
 * it: adding motion in #1428 is an entry here and a row in the panel, never a
 * fourth place that has to be found.
 */
export const VIEWSCREEN_EFFECTS = Object.freeze(['textScale', 'contrast']);

/** A fresh record: both effects follow the system this display runs on. */
export function emptyViewscreenPresentation() {
  return { textScale: FOLLOW_OS, contrast: FOLLOW_OS };
}

function normalizeTri(value) {
  return value === EXPLICIT_ON || value === EXPLICIT_OFF ? value : FOLLOW_OS;
}

function normalizeScale(value) {
  if (value === FOLLOW_OS) return FOLLOW_OS;
  if (typeof value === 'number' && Number.isFinite(value)) return clampTextScale(value);
  return FOLLOW_OS;
}

/**
 * Coerce untrusted input — parsed storage JSON, a host injection, a
 * hand-edited file — into a valid record. Never throws.
 *
 * Accepts the wrapped `{kind, version, presentation:{…}}` form this module
 * writes AND a bare `{textScale, contrast}` object, because the native host
 * injects the bare shape (see [`readInjectedViewscreenPresentation`]) and a
 * second normaliser for it would be a second place for the clamp to drift.
 * Unknown fields are dropped rather than carried: this record is small and
 * endpoint-local, so there is nothing a future version needs preserved through
 * an older build.
 *
 * @param {*} raw
 */
export function normalizeViewscreenPresentation(raw) {
  const out = emptyViewscreenPresentation();
  if (!raw || typeof raw !== 'object') return out;
  const source = raw.presentation && typeof raw.presentation === 'object'
    ? raw.presentation
    : raw;
  out.textScale = normalizeScale(source.textScale);
  out.contrast = normalizeTri(source.contrast);
  return out;
}

/**
 * The record with one `effect` set to `value`. Returns the SAME input reference
 * when the effective value does not change, so a caller can skip a persist and
 * a re-apply — the contract `profileWithPresentation` has on the private side.
 *
 * An unknown effect is a no-op rather than a throw: the panel builds its rows
 * from [`VIEWSCREEN_EFFECTS`], so an unknown id can only arrive from a caller
 * that has already gone wrong, and losing the whole record is a worse answer
 * than ignoring the write.
 *
 * @param {object} record
 * @param {'textScale'|'contrast'} effect
 * @param {number|string} value
 */
export function presentationWithEffect(record, effect, value) {
  if (!VIEWSCREEN_EFFECTS.includes(effect)) return record;
  const next = effect === 'textScale' ? normalizeScale(value) : normalizeTri(value);
  const current = normalizeViewscreenPresentation(record);
  if (current[effect] === next) return record;
  current[effect] = next;
  return current;
}

/**
 * The record with EVERY effect returned to following the system — the whole of
 * "Reset all" on this surface, and its scope is the point.
 *
 * PRD #1418: "Reset all is scoped to the current presentation settings, not
 * unrelated bindings, identity or save data." Here that scope is structural
 * rather than careful: this record contains nothing else, and this module has
 * no path to the operator profile, to a scenario save, to the fleet code or to
 * the host's action bindings. The volume on the Audio tab beside it is a
 * different store again and is not touched.
 *
 * Returns the SAME input when everything is already at its default.
 *
 * @param {object} record
 */
export function presentationWithDefaults(record) {
  const current = normalizeViewscreenPresentation(record);
  if (VIEWSCREEN_EFFECTS.every((effect) => current[effect] === FOLLOW_OS)) return record;
  for (const effect of VIEWSCREEN_EFFECTS) current[effect] = FOLLOW_OS;
  return current;
}

/** True when nothing on this endpoint has been chosen explicitly. */
export function isDefaultViewscreenPresentation(record) {
  const current = normalizeViewscreenPresentation(record);
  return VIEWSCREEN_EFFECTS.every((effect) => current[effect] === FOLLOW_OS);
}

/** The JSON this module writes, wrapped and versioned. */
export function serializeViewscreenPresentation(record) {
  return JSON.stringify({
    kind: VIEWSCREEN_PRESENTATION_KIND,
    version: VIEWSCREEN_PRESENTATION_VERSION,
    presentation: normalizeViewscreenPresentation(record),
  });
}

/**
 * Read the record from a `localStorage`-like object. A missing key, corrupt
 * JSON or a storage that throws (private mode, a browser with site data
 * blocked) all yield the follow-the-system default — this display then simply
 * does not remember, which is the honest degradation for a preference.
 *
 * @param {{ getItem: function }|null} storage
 * @param {string} [key]
 */
export function loadViewscreenPresentation(storage, key = VIEWSCREEN_PRESENTATION_KEY) {
  try {
    const raw = storage && storage.getItem(key);
    if (!raw) return emptyViewscreenPresentation();
    return normalizeViewscreenPresentation(JSON.parse(raw));
  } catch (_) {
    return emptyViewscreenPresentation();
  }
}

/**
 * Persist the record on THIS endpoint. Storage errors are swallowed for the
 * reason above. Writes exactly one key and reads none, so no other record on
 * this origin — the operator profile most of all — can be disturbed by it.
 *
 * @param {{ setItem: function }|null} storage
 * @param {object} record
 * @param {string} [key]
 */
export function saveViewscreenPresentation(storage, record, key = VIEWSCREEN_PRESENTATION_KEY) {
  try {
    if (storage) storage.setItem(key, serializeViewscreenPresentation(record));
  } catch (_) {
    /* best-effort: the display forgets across restarts rather than failing */
  }
}

/**
 * The store a BROWSER viewscreen uses: this browser's own `localStorage`, which
 * is what "on that host endpoint" means for `server.html`. Not a promise of
 * cross-browser synchronisation, and PRD #1418 says so in as many words.
 *
 * @param {{getItem: function, setItem: function}|null} storage
 * @param {string} [key]
 */
export function browserViewscreenStore(storage, key = VIEWSCREEN_PRESENTATION_KEY) {
  return {
    load: () => loadViewscreenPresentation(storage, key),
    save: (record) => saveViewscreenPresentation(storage, record, key),
  };
}

/**
 * The record a native host injected into its lobby document, if any.
 *
 * The native viewscreen runs in an Ultralight view whose storage session is
 * EPHEMERAL and never written to disk (`src/native_host/panes/ultralight.rs`),
 * so `localStorage` there forgets at the end of every session — which is
 * exactly what this issue must not do. The host therefore keeps the record in a
 * file of its own and seeds the page with it, the same documented seam
 * `window.PhoenixOsAccessibilityDefaults` uses for the Windows preference read.
 *
 * The injected shape is the HOST's, not this record's: `{textScale, contrast}`
 * with a number-or-null and a **boolean**-or-null, because that is what
 * `viewscreen_presentation::presentation_script` can emit safely without a JSON
 * string escaper (see its injection-safety note). `null` is follow-the-machine
 * in both, and the boolean is translated to this module's tri-state here —
 * once, at the seam — rather than by teaching the normaliser a second
 * vocabulary it would then accept from everywhere.
 *
 * @param {Window|object|null} w
 * @returns {object|null} a normalised record, or null when nothing was injected
 */
export function readInjectedViewscreenPresentation(w) {
  let raw = null;
  try {
    raw = w && w.PhoenixViewscreenPresentation;
  } catch (_) {
    return null; // a getter that throws is "nothing injected", not a crash
  }
  if (!raw || typeof raw !== 'object') return null;
  let contrast = FOLLOW_OS;
  if (raw.contrast === true) contrast = EXPLICIT_ON;
  else if (raw.contrast === false) contrast = EXPLICIT_OFF;
  return normalizeViewscreenPresentation({
    textScale: typeof raw.textScale === 'number' ? raw.textScale : FOLLOW_OS,
    contrast,
  });
}

/**
 * This record as the host's injected shape — the inverse of the reader above,
 * and the payload half of `HostLobbyRecord::SetPresentation`.
 *
 * Text size crosses as WHOLE PERCENT because that is the resolution the slider
 * offers and the number the operator is reading on screen; the Rust side stores
 * it that way for the same reason, and an integer cannot arrive as `NaN`.
 *
 * @param {object} record
 * @returns {{text_scale_percent: number|null, contrast: boolean|null}}
 */
export function viewscreenPresentationRecordFields(record) {
  const current = normalizeViewscreenPresentation(record);
  return {
    text_scale_percent: current.textScale === FOLLOW_OS
      ? null
      : Math.round(Number(current.textScale) * 100),
    contrast: current.contrast === FOLLOW_OS ? null : current.contrast === EXPLICIT_ON,
  };
}

/**
 * Resolve the stored record to the concrete effects this display should show,
 * folding the explicit choice over what the system said.
 *
 * The two resolvers are `accessibility-profile.js`'s, so "explicit wins in both
 * directions" and "a follow-system text scale takes the OS scale where the host
 * supplies one" mean the same thing on this surface as on a console.
 *
 * @param {object} record
 * @param {{contrast?: boolean, textScale?: number}} [osDefaults]
 * @returns {{textScale: number, contrast: boolean}}
 */
export function resolveViewscreenEffects(record, osDefaults) {
  const current = normalizeViewscreenPresentation(record);
  const os = osDefaults || {};
  return {
    textScale: resolveTextScale(current.textScale, os.textScale),
    contrast: resolveTriState(current.contrast, os.contrast),
  };
}

/**
 * Where each live effect came from — `explicit` / `system` / `default`, plus
 * whether a native read failed — for the status line under each control
 * (PRD #1418 stories 9 and 16).
 *
 * Delegates to `presentationStatus`, feeding it a profile-shaped view of this
 * record. That indirection is deliberate: the status vocabulary and the rule
 * for what counts as "the system said something" are one decision, and a second
 * copy here would be free to disagree with the console's.
 *
 * @param {object} record
 * @param {{contrast?: boolean, textScale?: number}} [osDefaults]
 * @param {string[]} [unavailable] effect ids whose native read failed
 */
export function viewscreenPresentationStatus(record, osDefaults, unavailable) {
  const current = normalizeViewscreenPresentation(record);
  const full = presentationStatus(
    { presentation: { ...current, reducedMotion: FOLLOW_OS } },
    osDefaults,
    unavailable,
  );
  const out = {};
  for (const effect of VIEWSCREEN_EFFECTS) out[effect] = full[effect];
  return out;
}

/**
 * Write the resolved effects onto ONE document root.
 *
 * Text scale becomes `--a11y-text-scale`, which `server.html` and the native
 * lobby document multiply their root font-size by, so every `--text-*` rung on
 * the menus, the overlays and the lobby chrome grows together. The 3D scene does
 * not: `#canvas` is sized in percentages of its shell and the simulation draws
 * in world units, so enlarging text cannot shrink or crop the picture the room
 * is watching.
 *
 * Contrast becomes `data-contrast`, the attribute `gui/tokens.css` already
 * carries a palette for. `data-reduced-motion` is deliberately NOT written:
 * this endpoint does not own motion yet (see the module note), and stamping the
 * attribute would out-specify the `prefers-reduced-motion` fallback that is
 * driving the viewscreen's shake and flash today.
 *
 * Swallows DOM errors so one detached root never stops the rest.
 *
 * @param {Element|null} root
 * @param {{textScale: number, contrast: boolean}} effects
 */
export function applyViewscreenEffectsToRoot(root, effects) {
  if (!root || !effects) return;
  try {
    if (root.style && typeof root.style.setProperty === 'function') {
      root.style.setProperty(TEXT_SCALE_VAR, String(effects.textScale));
    }
    if (typeof root.setAttribute === 'function') {
      root.setAttribute('data-contrast', effects.contrast ? 'more' : 'standard');
    }
  } catch (_) {
    /* detached root — best effort */
  }
}

/**
 * Resolve `record` against this window's system defaults and apply it to
 * `doc`'s root. Returns the effects it applied.
 *
 * @param {object} record
 * @param {{doc?: Document, win?: Window}} [opts]
 */
export function applyViewscreenPresentation(record, opts = {}) {
  const win = opts.win || (typeof window !== 'undefined' ? window : null);
  const doc = opts.doc || (win && win.document)
    || (typeof document !== 'undefined' ? document : null);
  const effects = resolveViewscreenEffects(record, osAccessibilityDefaults(win));
  applyViewscreenEffectsToRoot(doc && doc.documentElement, effects);
  return effects;
}

/**
 * The live controller both Viewscreen cogs drive.
 *
 * One object rather than four loose functions because the three things a
 * settings surface does with a preference — read it to paint, change it with an
 * immediate preview, put it back — must happen in one order (normalise, persist,
 * apply) on both surfaces, and a panel that only *some* of the time re-applied
 * would be the live-preview requirement failing in exactly the case nobody
 * tests. The panel calls `set`; everything else follows from it.
 *
 * `store` is the endpoint seam: `browserViewscreenStore(localStorage)` for
 * `server.html`, and a host-record sender for the native lobby, whose page
 * cannot write a file. A store whose `save` throws is caught here, so a failed
 * write costs the persistence and not the preview.
 *
 * @param {{
 *   store?: {load: () => object, save: (record: object) => void},
 *   doc?: Document, win?: Window,
 *   onChange?: (record: object, effects: object) => void,
 * }} [opts]
 */
export function createViewscreenPresentation(opts = {}) {
  const win = opts.win || (typeof window !== 'undefined' ? window : null);
  const doc = opts.doc || (win && win.document)
    || (typeof document !== 'undefined' ? document : null);
  const store = opts.store || browserViewscreenStore(safeLocalStorage(win));

  let record = emptyViewscreenPresentation();
  try {
    record = normalizeViewscreenPresentation(store.load());
  } catch (_) {
    record = emptyViewscreenPresentation();
  }

  const osDefaults = () => osAccessibilityDefaults(win);

  function apply() {
    const effects = resolveViewscreenEffects(record, osDefaults());
    applyViewscreenEffectsToRoot(doc && doc.documentElement, effects);
    return effects;
  }

  function commit(next) {
    record = normalizeViewscreenPresentation(next);
    try {
      store.save(record);
    } catch (_) {
      /* a full disk or a refusing host costs persistence, never the preview */
    }
    const effects = apply();
    if (typeof opts.onChange === 'function') {
      try {
        opts.onChange(record, effects);
      } catch (_) { /* a listener must not undo the change it was told about */ }
    }
    return effects;
  }

  return {
    /** The explicit choices as stored — a copy, so a panel cannot mutate it. */
    record: () => ({ ...record }),
    /** The concrete values in force right now. */
    effects: () => resolveViewscreenEffects(record, osDefaults()),
    /** Per-effect value + where it came from, for the status lines. */
    status: () => viewscreenPresentationStatus(
      record, osDefaults(), unavailableOsPreferences(win),
    ),
    /** Effect ids whose native read failed, for the "unavailable" hint. */
    unavailable: () => unavailableOsPreferences(win),
    /** Change one effect: persisted on this endpoint and previewed at once. */
    set: (effect, value) => commit(presentationWithEffect(record, effect, value)),
    /** Per-setting reset — this effect only; the others keep their value. */
    reset: (effect) => commit(presentationWithEffect(record, effect, FOLLOW_OS)),
    /** Reset all, scoped to this record and nothing else. */
    resetAll: () => commit(presentationWithDefaults(record)),
    /** Re-apply without changing anything (boot, or a document swap). */
    apply,
  };
}

/** `win.localStorage`, or null where reading it throws or it does not exist. */
function safeLocalStorage(win) {
  try {
    return (win && win.localStorage) || null;
  } catch (_) {
    return null;
  }
}

// Expose for a classic-script consumer, the pattern `window.nativeSettings` and
// `window.hostLanding` already use. `server.html` imports the module, but a
// surface that can only be reached one way is one the next surface has to edit.
if (typeof window !== 'undefined') {
  window.viewscreenPresentation = {
    VIEWSCREEN_PRESENTATION_KEY,
    VIEWSCREEN_EFFECTS,
    createViewscreenPresentation,
    browserViewscreenStore,
    readInjectedViewscreenPresentation,
    viewscreenPresentationRecordFields,
    applyViewscreenPresentation,
  };
}
