/**
 * tests/client/accessibility-profile.test.js — the private per-player
 * Accessibility profile (issue #1102).
 *
 * Pure-Node tests over gui/accessibility-profile.js: the presentation-effect
 * schema and its immutable reducers, the declared-but-inert per-function
 * assistance schema (AC4), the matchMedia OS-default resolver and the
 * explicit-override-wins-both-ways resolution (AC2), the versioned localStorage
 * round-trip and its private-mode safety, hydration into the sim-state
 * singleton, the reconnect-preserves-the-profile guarantee (AC2), and the
 * privacy invariant that the profile is never shaped into a wire message (AC5).
 */
import { describe, it, expect, vi } from 'vitest';
import {
  ACCESSIBILITY_PROFILE_KEY,
  TEXT_SCALE_VAR,
  TEXT_SCALE_DEFAULT,
  FOLLOW_OS,
  EXPLICIT_ON,
  EXPLICIT_OFF,
  ASSIST_OFF,
  ASSIST_REQUEST,
  ASSISTANCE_FUNCTIONS,
  emptyAccessibilityProfile,
  normalizeAccessibilityProfile,
  clampTextScale,
  profileWithPresentation,
  profileWithAssistance,
  osAccessibilityDefaults,
  resolveTriState,
  resolveTextScale,
  resolveEffects,
  applyEffectsToRoot,
  applyAccessibilityProfile,
  loadAccessibilityProfile,
  saveAccessibilityProfile,
  hydrateAccessibilityProfile,
} from '../../gui/accessibility-profile.js';
import * as profileModule from '../../gui/accessibility-profile.js';
import { ClientSimState } from '../../gui/sim-state.js';

// ── Fakes ────────────────────────────────────────────────────────────────────

function fakeStorage(initial = {}) {
  const map = new Map(Object.entries(initial));
  return {
    getItem: (k) => (map.has(k) ? map.get(k) : null),
    setItem: (k, v) => { map.set(k, String(v)); },
    _map: map,
  };
}

/** A window whose matchMedia reports the given queries as matching. */
function fakeWin(matches = {}) {
  return { matchMedia: (q) => ({ matches: !!matches[q] }) };
}

/** A document-root stand-in that records the var + attributes written to it. */
function fakeRoot() {
  const props = {};
  const attrs = {};
  return {
    style: { setProperty: (k, v) => { props[k] = v; } },
    setAttribute: (k, v) => { attrs[k] = v; },
    _props: props,
    _attrs: attrs,
  };
}

// ── Schema & reducers ────────────────────────────────────────────────────────

describe('profile schema', () => {
  it('a fresh profile follows the OS for every effect and offers no assistance', () => {
    expect(emptyAccessibilityProfile()).toEqual({
      presentation: { textScale: FOLLOW_OS, contrast: FOLLOW_OS, reducedMotion: FOLLOW_OS },
      assistance: {},
    });
  });

  it('normalizes junk into a valid profile without throwing', () => {
    expect(normalizeAccessibilityProfile(null)).toEqual(emptyAccessibilityProfile());
    expect(normalizeAccessibilityProfile('nope')).toEqual(emptyAccessibilityProfile());
    expect(normalizeAccessibilityProfile({
      presentation: { textScale: 'huge', contrast: 'sideways', reducedMotion: EXPLICIT_ON },
      assistance: { 'not.a.function': 'request', 'helm.course-keeping': 'request' },
    })).toEqual({
      presentation: { textScale: FOLLOW_OS, contrast: FOLLOW_OS, reducedMotion: EXPLICIT_ON },
      // Only a DECLARED function with a non-default state survives.
      assistance: { 'helm.course-keeping': ASSIST_REQUEST },
    });
  });

  it('clamps a text scale to the safe absolute range', () => {
    expect(clampTextScale(1.25)).toBeCloseTo(1.25);
    expect(clampTextScale(99)).toBe(2.0);
    expect(clampTextScale(0.01)).toBe(0.5);
    expect(clampTextScale('nonsense')).toBe(TEXT_SCALE_DEFAULT);
  });
});

describe('profileWithPresentation', () => {
  it('sets a numeric text scale and clamps it, without mutating the input', () => {
    const p0 = emptyAccessibilityProfile();
    const p1 = profileWithPresentation(p0, 'textScale', 1.3);
    expect(p1.presentation.textScale).toBeCloseTo(1.3);
    expect(p0.presentation.textScale).toBe(FOLLOW_OS); // input untouched
    expect(profileWithPresentation(p0, 'textScale', 5).presentation.textScale).toBe(2.0);
  });

  it('returns to the unset state when text scale is set back to default', () => {
    const p = profileWithPresentation(emptyAccessibilityProfile(), 'textScale', 1.4);
    expect(profileWithPresentation(p, 'textScale', FOLLOW_OS).presentation.textScale).toBe(FOLLOW_OS);
  });

  it('sets the tri-state effects and rejects an unknown value', () => {
    const p = profileWithPresentation(emptyAccessibilityProfile(), 'contrast', EXPLICIT_ON);
    expect(p.presentation.contrast).toBe(EXPLICIT_ON);
    const q = profileWithPresentation(p, 'reducedMotion', EXPLICIT_OFF);
    expect(q.presentation.reducedMotion).toBe(EXPLICIT_OFF);
    // An unknown value normalises to follow-OS.
    expect(profileWithPresentation(q, 'contrast', 'sideways').presentation.contrast).toBe(FOLLOW_OS);
  });

  it('returns the same reference on a no-op and ignores an unknown effect', () => {
    const p = profileWithPresentation(emptyAccessibilityProfile(), 'contrast', EXPLICIT_ON);
    expect(profileWithPresentation(p, 'contrast', EXPLICIT_ON)).toBe(p);
    expect(profileWithPresentation(p, 'nonsense', EXPLICIT_ON)).toBe(p);
  });
});

describe('profileWithAssistance (declared-but-inert, AC4)', () => {
  it('declares a non-empty per-function schema, keyed station.function', () => {
    expect(ASSISTANCE_FUNCTIONS.length).toBeGreaterThan(0);
    for (const id of ASSISTANCE_FUNCTIONS) expect(id).toMatch(/^[a-z]+\.[a-z-]+$/);
  });

  it('sets and clears an override, and ignores an undeclared function', () => {
    const fn = ASSISTANCE_FUNCTIONS[0];
    const p = profileWithAssistance(emptyAccessibilityProfile(), fn, ASSIST_REQUEST);
    expect(p.assistance[fn]).toBe(ASSIST_REQUEST);
    // Setting back to off removes the override entirely (minimal record).
    expect(profileWithAssistance(p, fn, ASSIST_OFF).assistance).toEqual({});
    // Unknown function is a no-op (same reference).
    expect(profileWithAssistance(p, 'sensors.telepathy', ASSIST_REQUEST)).toBe(p);
  });
});

// ── OS-default resolver (matchMedia) ─────────────────────────────────────────

describe('osAccessibilityDefaults', () => {
  it('reads the three accessibility media queries', () => {
    const win = fakeWin({
      '(prefers-reduced-motion: reduce)': true,
      '(prefers-contrast: more)': false,
      '(prefers-color-scheme: dark)': true,
    });
    expect(osAccessibilityDefaults(win)).toEqual({
      reducedMotion: true,
      contrast: false,
      darkColorScheme: true,
    });
  });

  it('reports no preference when matchMedia is missing or throws', () => {
    expect(osAccessibilityDefaults({})).toEqual({
      reducedMotion: false, contrast: false, darkColorScheme: false,
    });
    expect(osAccessibilityDefaults({ matchMedia() { throw new Error('nope'); } })).toEqual({
      reducedMotion: false, contrast: false, darkColorScheme: false,
    });
    expect(osAccessibilityDefaults(null)).toEqual({
      reducedMotion: false, contrast: false, darkColorScheme: false,
    });
  });
});

// ── Resolution: explicit overrides the OS default in BOTH directions (AC2) ───

describe('resolution folds explicit choice over the OS default', () => {
  it('follows the OS when unset', () => {
    expect(resolveTriState(FOLLOW_OS, true)).toBe(true);
    expect(resolveTriState(FOLLOW_OS, false)).toBe(false);
  });

  it('an explicit choice wins both ways', () => {
    // Enable an effect the OS is silent about…
    expect(resolveTriState(EXPLICIT_ON, false)).toBe(true);
    // …and DISABLE one the OS asked for (re-allow motion despite prefers-reduce).
    expect(resolveTriState(EXPLICIT_OFF, true)).toBe(false);
  });

  it('resolves text scale, treating unset as the identity', () => {
    expect(resolveTextScale(FOLLOW_OS)).toBe(TEXT_SCALE_DEFAULT);
    expect(resolveTextScale(1.4)).toBeCloseTo(1.4);
    expect(resolveTextScale(9)).toBe(2.0);
  });

  it('resolveEffects combines the profile with the OS defaults', () => {
    const profile = normalizeAccessibilityProfile({
      presentation: { textScale: 1.2, contrast: EXPLICIT_ON, reducedMotion: EXPLICIT_OFF },
    });
    const os = { reducedMotion: true, contrast: false };
    expect(resolveEffects(profile, os)).toEqual({
      textScale: 1.2,
      contrast: true,        // explicit on, despite OS off
      reducedMotion: false,  // explicit off, despite OS reduce
    });
    // With everything unset, the OS defaults come through.
    expect(resolveEffects(emptyAccessibilityProfile(), os)).toEqual({
      textScale: TEXT_SCALE_DEFAULT, contrast: false, reducedMotion: true,
    });
  });
});

// ── Application onto document roots (observable effect, AC3) ──────────────────

describe('application', () => {
  it('writes the text-scale var and the motion/contrast attributes onto a root', () => {
    const root = fakeRoot();
    applyEffectsToRoot(root, { textScale: 1.3, contrast: true, reducedMotion: false });
    expect(root._props[TEXT_SCALE_VAR]).toBe('1.3');
    expect(root._attrs['data-reduced-motion']).toBe('no-preference');
    expect(root._attrs['data-contrast']).toBe('more');
  });

  it('applies to the shell root AND every same-origin console iframe root', () => {
    const shell = fakeRoot();
    const iframeRoot = fakeRoot();
    const doc = { documentElement: shell };
    const iframes = [{ contentDocument: { documentElement: iframeRoot } }];
    const win = fakeWin(); // OS silent
    const profile = profileWithPresentation(emptyAccessibilityProfile(), 'textScale', 1.25);

    const effects = applyAccessibilityProfile(profile, { doc, win, iframes });

    expect(effects.textScale).toBeCloseTo(1.25);
    expect(shell._props[TEXT_SCALE_VAR]).toBe('1.25');
    expect(iframeRoot._props[TEXT_SCALE_VAR]).toBe('1.25');
  });

  it('skips a not-yet-loaded or cross-origin iframe without throwing', () => {
    const shell = fakeRoot();
    const doc = { documentElement: shell };
    const throwing = { get contentDocument() { throw new Error('cross-origin'); } };
    const iframes = [{ contentDocument: null }, throwing];
    expect(() => applyAccessibilityProfile(emptyAccessibilityProfile(), { doc, win: fakeWin(), iframes }))
      .not.toThrow();
    expect(shell._props[TEXT_SCALE_VAR]).toBe(String(TEXT_SCALE_DEFAULT));
  });
});

// ── Persistence (versioned, private-mode safe) ───────────────────────────────

describe('persistence', () => {
  it('round-trips the profile through the versioned key', () => {
    const storage = fakeStorage();
    const p = profileWithAssistance(
      profileWithPresentation(emptyAccessibilityProfile(), 'textScale', 1.35),
      ASSISTANCE_FUNCTIONS[0], ASSIST_REQUEST,
    );
    saveAccessibilityProfile(storage, p);
    expect(storage._map.has(ACCESSIBILITY_PROFILE_KEY)).toBe(true);
    expect(loadAccessibilityProfile(storage)).toEqual(normalizeAccessibilityProfile(p));
  });

  it('missing key, corrupt JSON, and throwing storage all yield an empty profile', () => {
    expect(loadAccessibilityProfile(fakeStorage())).toEqual(emptyAccessibilityProfile());
    expect(loadAccessibilityProfile(fakeStorage({ [ACCESSIBILITY_PROFILE_KEY]: '{oops' })))
      .toEqual(emptyAccessibilityProfile());
    expect(loadAccessibilityProfile({ getItem() { throw new Error('denied'); } }))
      .toEqual(emptyAccessibilityProfile());
    expect(loadAccessibilityProfile(null)).toEqual(emptyAccessibilityProfile());
    expect(() => saveAccessibilityProfile({ setItem() { throw new Error('quota'); } }, emptyAccessibilityProfile()))
      .not.toThrow();
  });
});

// ── Hydration + reconnect survival (AC2) ─────────────────────────────────────

describe('hydration and reconnect', () => {
  it('points sim.accessibilityProfile at the stored record', () => {
    const sim = { accessibilityProfile: emptyAccessibilityProfile() };
    const stored = normalizeAccessibilityProfile({ presentation: { textScale: 1.4 } });
    const storage = fakeStorage({ [ACCESSIBILITY_PROFILE_KEY]: JSON.stringify(stored) });
    hydrateAccessibilityProfile(sim, storage);
    expect(sim.accessibilityProfile).toEqual(stored);
  });

  it('a broken or absent storage hydrates a fresh empty profile, never a throw', () => {
    const sim = {};
    hydrateAccessibilityProfile(sim, { getItem() { throw new Error('denied'); } });
    expect(sim.accessibilityProfile).toEqual(emptyAccessibilityProfile());
    expect(() => hydrateAccessibilityProfile(null, fakeStorage())).not.toThrow();
  });

  it('ClientSimState.reset() PRESERVES the explicit profile — a reconnect keeps it', () => {
    const sim = new ClientSimState();
    sim.accessibilityProfile = normalizeAccessibilityProfile({
      presentation: { textScale: 1.3, reducedMotion: EXPLICIT_OFF },
    });
    const before = JSON.parse(JSON.stringify(sim.accessibilityProfile));
    // A bare reset and an InProgress Welcome are the two reconnect paths.
    sim.reset();
    expect(sim.accessibilityProfile).toEqual(before);
    sim.apply({ type: 'Welcome', data: { state: { phase: 'InProgress' } } });
    expect(sim.accessibilityProfile).toEqual(before);
  });

  it('hydrates the REAL simState singleton at module evaluation, regardless of load order', async () => {
    // Same regression shape as tutorial-state: a static importer may evaluate
    // this module before client.html's sim-state.js script tag runs, so
    // hydration must not depend on window.simState already existing — the
    // explicit `import { simState }` guarantees ordering.
    const stored = normalizeAccessibilityProfile({ presentation: { textScale: 1.45, contrast: EXPLICIT_ON } });
    vi.stubGlobal('localStorage', fakeStorage({ [ACCESSIBILITY_PROFILE_KEY]: JSON.stringify(stored) }));
    vi.resetModules();
    try {
      await import('../../gui/accessibility-profile.js');
      const { simState } = await import('../../gui/sim-state.js');
      expect(simState.accessibilityProfile).toEqual(stored);
      simState.reset();
      expect(simState.accessibilityProfile).toEqual(stored);
    } finally {
      vi.unstubAllGlobals();
      vi.resetModules();
    }
  });
});

// ── Privacy: the profile is never a wire message (AC5) ───────────────────────

describe('privacy', () => {
  it('exposes no message builder — the module cannot shape a ClientMessage', () => {
    // The whole module surface is reducers, resolvers, DOM application and
    // storage. Nothing here builds a `{ type, data }` wire envelope, so there
    // is no path by which a setting reaches the host.
    const messageish = Object.keys(profileModule).filter((k) => /message/i.test(k));
    expect(messageish).toEqual([]);
  });

  it('the stored record carries only presentation + assistance, no identity or reason', () => {
    const storage = fakeStorage();
    const p = profileWithPresentation(emptyAccessibilityProfile(), 'reducedMotion', EXPLICIT_ON);
    saveAccessibilityProfile(storage, p);
    const raw = JSON.parse(storage._map.get(ACCESSIBILITY_PROFILE_KEY));
    expect(Object.keys(raw).sort()).toEqual(['assistance', 'presentation']);
    // No diagnosis / reason / player-identity field is ever serialised.
    expect(JSON.stringify(raw)).not.toMatch(/diagnos|reason|token|player|name/i);
  });
});
