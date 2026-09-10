// @vitest-environment jsdom
/**
 * tests/client/accessibility-200-percent.test.js — issue #1422 (PRD #1418
 * stories 1, 2, 4, 9, 10, 16, 17).
 *
 * The exposed text-size ceiling moves from 150% to 200%, and one COMPLETE
 * console workflow — Power allocation on the Engineering/Power seat — is
 * carried through the shell, the console iframe and the allocation controls at
 * 100%, 150% and 200%. Alongside it: explicit-default semantics (is this value
 * mine, my system's, or the built-in default?), live preview, per-setting
 * reset, a Reset all scoped to presentation, and the settings overlay's
 * keyboard dismissal and focus retention.
 *
 * Everything here drives the REAL modules — the shipped profile resolver, the
 * shipped `mountConsoles` iframe mount, the shipped `mountSettings` overlay and
 * the shipped `<ph-power-controls>` custom element — under jsdom. The one thing
 * jsdom cannot do is lay out and measure real text, which is why the companion
 * Playwright spec `tests/smoke/text-scale-power-workflow.spec.js` exists; these
 * are the behavioural halves that do not need a renderer.
 */

import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import { JSDOM } from 'jsdom';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import { t } from '../../gui/strings.js';
import { TEXT_SCALES } from '../fixtures/device-matrix.mjs';
import {
  TEXT_SCALE_MIN,
  TEXT_SCALE_MAX,
  TEXT_SCALE_STEP,
  TEXT_SCALE_DEFAULT,
  TEXT_SCALE_VAR,
  ACCESSIBILITY_PROFILE_KEY,
  FOLLOW_OS,
  EXPLICIT_ON,
  EXPLICIT_OFF,
  emptyAccessibilityProfile,
  normalizeAccessibilityProfile,
  clampTextScale,
  profileWithPresentation,
  profileWithPresentationDefaults,
  profileWithAssistance,
  presentationStatus,
  resolveEffects,
  applyAccessibilityProfile,
  loadAccessibilityProfile,
  saveAccessibilityProfile,
} from '../../gui/accessibility-profile.js';
import {
  createOperatorProfileSnapshot,
  OPERATOR_PROFILE_KIND,
  OPERATOR_PROFILE_VERSION,
} from '../../gui/operator-profile.js';
import { mountConsoles, applyConsoleVisibility } from '../../gui/console-mount.js';
import { mountSettings } from '../../gui/settings-panel.js';
import {
  POWER_DECREASE_ACTION_ID,
  POWER_INCREASE_ACTION_ID,
} from '../../gui/stations/engineering-actions.js';
import '../../gui/components/ph-power-controls.js';

const repoFile = (rel) => fs.readFileSync(
  path.join(path.dirname(fileURLToPath(import.meta.url)), '../..', rel), 'utf-8');

// ── fixtures ────────────────────────────────────────────────────────────────

/** The Engineering/Power seat plus two neighbours, so "every console iframe"
 *  is a claim about more than one frame. */
const SHIP = {
  stations: [
    { id: 'power', name: 'Power', console: 'gui/battleship/power.html' },
    { id: 'engineering', name: 'Engineering', console: 'gui/cruiser/engineering.html' },
    { id: 'helm', name: 'Helm', console: 'gui/battleship/helm.html' },
  ],
};

/** A realistic Power allocation payload: three groups, one held below its
 *  commanded level by the battery floor, one cold. */
const POWER_GROUPS = [
  { id: 'helm', label: 'PROPULSION', level: 2, commanded_level: 2, min_level: 1, max_level: 4 },
  { id: 'weapons', label: 'WEAPONS', level: 1, commanded_level: 3, min_level: 0, max_level: 4 },
  { id: 'shields', label: 'SHIELDS', level: 0, commanded_level: 0, min_level: 0, max_level: 4 },
];

function consoleHarness() {
  const dom = new JSDOM('<div id="console-container"></div>', { url: 'https://phoenix.test/' });
  const doc = dom.window.document;
  mountConsoles(doc, doc.getElementById('console-container'), SHIP);
  applyConsoleVisibility(doc, 'power', true, SHIP.stations.map((s) => s.id));
  // jsdom does not LOAD an iframe src, so point each at about:blank to
  // materialise the same-origin :root the shell writes onto. The mount and the
  // `.console-section iframe` discovery under test are the production ones.
  for (const st of SHIP.stations) {
    doc.getElementById(`${st.id}-iframe`).setAttribute('src', 'about:blank');
  }
  return { dom, doc };
}

const iframeScale = (doc, id) => {
  const root = doc.getElementById(id)?.contentDocument?.documentElement;
  return root ? root.style.getPropertyValue(TEXT_SCALE_VAR) : null;
};

/** A localStorage stand-in that cannot be confused with the real one. */
function memoryStorage(seed = {}) {
  const map = new Map(Object.entries(seed));
  return {
    getItem: (k) => (map.has(k) ? map.get(k) : null),
    setItem: (k, v) => map.set(k, String(v)),
    removeItem: (k) => map.delete(k),
    get size() { return map.size; },
  };
}

// ── 1. The ceiling itself ───────────────────────────────────────────────────

describe('the exposed text-size ceiling is 200% (issue #1422)', () => {
  it('offers whole-percent stops from 100% to 200%', () => {
    expect(TEXT_SCALE_MIN).toBe(1.0);
    expect(TEXT_SCALE_MAX).toBe(2.0);
    // A stop that does not divide the range evenly would put the ceiling out of
    // reach of the slider entirely.
    const stops = (TEXT_SCALE_MAX - TEXT_SCALE_MIN) / TEXT_SCALE_STEP;
    expect(Math.abs(stops - Math.round(stops))).toBeLessThan(1e-9);
  });

  it('accepts every scale the shared device-matrix fixture exercises', () => {
    // tests/fixtures/device-matrix.mjs is the acceptance kit's list; 200% must
    // now be selectable rather than merely aspirational.
    for (const scale of TEXT_SCALES) {
      expect(scale).toBeGreaterThanOrEqual(TEXT_SCALE_MIN);
      expect(scale).toBeLessThanOrEqual(TEXT_SCALE_MAX);
      const profile = profileWithPresentation(emptyAccessibilityProfile(), 'textScale', scale);
      expect(resolveEffects(profile).textScale).toBeCloseTo(scale);
    }
  });

  it('still clamps a record that asks for more than the supported maximum', () => {
    // Hand-edited or host-injected records are coerced into the range the
    // consoles are verified to reflow within, rather than honoured blindly.
    expect(clampTextScale(3.5)).toBe(2.0);
    expect(clampTextScale(0.1)).toBe(0.5);
    expect(clampTextScale('nonsense')).toBe(TEXT_SCALE_DEFAULT);
    const wild = normalizeAccessibilityProfile({ presentation: { textScale: 9 } });
    expect(wild.presentation.textScale).toBe(2.0);
  });
});

describe('the shell scales with the same setting its consoles do', () => {
  // The shell — the lobby, the Station Bar, the settings overlay itself, the
  // disconnect banner — had NO root font-size of its own before issue #1422 and
  // rode the browser's monospace default, so `--a11y-text-scale` reached every
  // console iframe and stopped at the page around them. That made settings,
  // errors and overlays the least readable parts of an enlarged interface,
  // which PRD #1418 story 4 exists to forbid. Pinned here as source text; the
  // rendered proof is tests/smoke/text-scale-power-workflow.spec.js, which
  // measures the Station Bar title and the connection label growing.
  const CLIENT_HTML = repoFile('client.html');
  const TOKENS_CSS = repoFile('gui/tokens.css');
  const CONSOLE_CSS = repoFile('gui/console.css');

  it('multiplies a named shell root size by the profile text scale', () => {
    expect(TOKENS_CSS).toMatch(/--root-size-shell:\s*\d+px/);
    expect(CLIENT_HTML).toMatch(
      /html\s*\{\s*font-size:\s*calc\(var\(--root-size-shell\)\s*\*\s*var\(--a11y-text-scale,\s*1\)\)/,
    );
  });

  it('uses the same multiplier the consoles use, so one choice drives both', () => {
    // Not two settings that happen to agree: the console root and the shell
    // root read the SAME custom property, which is the one the profile stamps.
    expect(CONSOLE_CSS).toMatch(/font-size:\s*calc\(var\(--root-size-\w+\)\s*\*\s*var\(--a11y-text-scale/);
    expect(TEXT_SCALE_VAR).toBe('--a11y-text-scale');
  });
});

// ── 2. Defaults, explicit override, explicit-default semantics ──────────────

describe('explicit choices, system defaults and the difference between them', () => {
  it('a fresh profile follows the system for every effect', () => {
    const p = emptyAccessibilityProfile();
    expect(p.presentation).toEqual({
      textScale: FOLLOW_OS, contrast: FOLLOW_OS, reducedMotion: FOLLOW_OS,
      shake: FOLLOW_OS, flash: FOLLOW_OS, decorativeMotion: FOLLOW_OS,
    });
    expect(resolveEffects(p, {})).toEqual({
      textScale: TEXT_SCALE_DEFAULT, contrast: false, reducedMotion: false,
      shake: 1, flash: 1, decorativeMotion: 1,
    });
  });

  it('an explicit choice overrides the system in both directions', () => {
    const os = { textScale: 1.25, contrast: true, reducedMotion: true };
    // Follow-system adopts all three.
    expect(resolveEffects(emptyAccessibilityProfile(), os)).toEqual({
      textScale: 1.25, contrast: true, reducedMotion: true,
      // …including the three effects issue #1428 split out, which follow it.
      shake: 0, flash: 0, decorativeMotion: 0,
    });
    // Explicit values win, including explicitly turning an effect OFF that the
    // system asked for.
    let p = profileWithPresentation(emptyAccessibilityProfile(), 'textScale', 2.0);
    p = profileWithPresentation(p, 'contrast', EXPLICIT_OFF);
    p = profileWithPresentation(p, 'reducedMotion', EXPLICIT_ON);
    expect(resolveEffects(p, os)).toEqual({
      textScale: 2.0, contrast: false, reducedMotion: true,
      // Explicitly asking to reduce motion turns the three effects off, exactly
      // as an OS asking for it does — one resolved preference, one answer.
      shake: 0, flash: 0, decorativeMotion: 0,
    });
  });

  it('reports whether each live value is the operator choice, the system, or the default', () => {
    const os = { textScale: 1.5, contrast: true };
    const p = profileWithPresentation(emptyAccessibilityProfile(), 'reducedMotion', EXPLICIT_ON);
    const status = presentationStatus(p, os);
    // Following the system, and the system said something.
    expect(status.textScale).toEqual({ value: 1.5, source: 'system', available: true });
    expect(status.contrast).toEqual({ value: true, source: 'system', available: true });
    // Chosen here.
    expect(status.reducedMotion).toEqual({ value: true, source: 'explicit', available: true });
  });

  it('a silent system is the documented default, not a system value', () => {
    const status = presentationStatus(emptyAccessibilityProfile(), {
      contrast: false, reducedMotion: false,
    });
    expect(status.textScale.source).toBe('default');
    expect(status.contrast.source).toBe('default');
    expect(status.reducedMotion.source).toBe('default');
  });

  it('an unreadable system preference is distinct from a neutral one, and never taints an explicit choice', () => {
    const p = profileWithPresentation(emptyAccessibilityProfile(), 'contrast', EXPLICIT_ON);
    const status = presentationStatus(p, {}, ['textScale', 'contrast', 'reducedMotion']);
    expect(status.textScale.available).toBe(false);
    expect(status.reducedMotion.available).toBe(false);
    // The explicit contrast choice never depended on the failed read.
    expect(status.contrast).toEqual({ value: true, source: 'explicit', available: true });
  });
});

// ── 3. Migration, persistence and reconnect ─────────────────────────────────

describe('migration and reconnect preserve the operator choice', () => {
  it('migrates a pre-#1279 bare Accessibility record and keeps its 150% choice exactly', () => {
    const storage = memoryStorage({
      [ACCESSIBILITY_PROFILE_KEY]: JSON.stringify({
        presentation: { textScale: 1.5, contrast: 'on', reducedMotion: 'default' },
        assistance: { 'helm.course-keeping': 'request' },
      }),
    });
    const loaded = loadAccessibilityProfile(storage);
    // A record written under the old 150% ceiling is still exactly 150% under
    // the new one — raising a ceiling must not move anybody's setting.
    expect(loaded.presentation.textScale).toBe(1.5);
    expect(loaded.presentation.contrast).toBe(EXPLICIT_ON);
    expect(loaded.assistance['helm.course-keeping']).toBe('request');
  });

  it('coerces a corrupt or legacy-shaped record instead of throwing', () => {
    expect(loadAccessibilityProfile(memoryStorage({
      [ACCESSIBILITY_PROFILE_KEY]: 'not json {',
    }))).toEqual(emptyAccessibilityProfile());
    expect(normalizeAccessibilityProfile({ presentation: { textScale: '1.5' } })
      .presentation.textScale).toBe(FOLLOW_OS);
    expect(normalizeAccessibilityProfile(null)).toEqual(emptyAccessibilityProfile());
  });

  it('survives a save/load round trip at the new ceiling', () => {
    // The "reconnect and restart" case: the record on disk is the only thing
    // that carries the choice across, and 200% has to come back as 200%.
    const storage = memoryStorage();
    const chosen = profileWithPresentation(emptyAccessibilityProfile(), 'textScale', 2.0);
    saveAccessibilityProfile(storage, chosen);
    expect(loadAccessibilityProfile(storage).presentation.textScale).toBe(2.0);
  });

  it('rides through the enclosing operator profile without leaving the device', () => {
    const snapshot = createOperatorProfileSnapshot({
      accessibility: profileWithPresentation(emptyAccessibilityProfile(), 'textScale', 2.0),
      bindings: {},
      feedback: { vibration: false, semanticCues: true },
    });
    expect(snapshot.kind).toBe(OPERATOR_PROFILE_KIND);
    expect(snapshot.version).toBe(OPERATOR_PROFILE_VERSION);
    expect(snapshot.accessibility.presentation.textScale).toBe(2.0);
    // The privacy boundary: no transport identity, no station ownership, no
    // save catalogue can reach the record a reconnect reloads.
    expect(Object.keys(snapshot).sort()).toEqual([
      'accessibility', 'bindings', 'feedback', 'gamepad', 'gmConfirmations',
      'kind', 'version',
    ]);
  });
});

// ── 4. Per-setting reset and the scoped Reset all ───────────────────────────

describe('resets are scoped (PRD #1418 story 17)', () => {
  const loaded = () => {
    let p = profileWithPresentation(emptyAccessibilityProfile(), 'textScale', 2.0);
    p = profileWithPresentation(p, 'contrast', EXPLICIT_ON);
    p = profileWithPresentation(p, 'reducedMotion', EXPLICIT_OFF);
    return profileWithAssistance(p, 'tactical.target-selection', 'request');
  };

  it('an individual reset returns one effect and preserves every other value', () => {
    const after = profileWithPresentation(loaded(), 'textScale', FOLLOW_OS);
    expect(after.presentation.textScale).toBe(FOLLOW_OS);
    expect(after.presentation.contrast).toBe(EXPLICIT_ON);
    expect(after.presentation.reducedMotion).toBe(EXPLICIT_OFF);
    expect(after.assistance['tactical.target-selection']).toBe('request');
  });

  it('Reset all returns every presentation effect and nothing else', () => {
    const after = profileWithPresentationDefaults(loaded());
    // Six effects since issue #1428, and the scope held without anybody adding
    // them to a list here: `profileWithPresentationDefaults` iterates
    // PRESENTATION_EFFECTS, which is where the three arrived.
    expect(after.presentation).toEqual({
      textScale: FOLLOW_OS, contrast: FOLLOW_OS, reducedMotion: FOLLOW_OS,
      shake: FOLLOW_OS, flash: FOLLOW_OS, decorativeMotion: FOLLOW_OS,
    });
    // Assistance overrides are not presentation and are not swept up.
    expect(after.assistance['tactical.target-selection']).toBe('request');
  });

  it('Reset all cannot reach bindings, gamepad tuning, feedback, GM policy or saves', () => {
    // The scope is structural: the reset rebuilds `presentation` only, and the
    // enclosing operator profile re-snapshots everything else from its own
    // owners. Prove it on the record that actually gets persisted.
    const bindings = { 'helm.throttle-up': [{ type: 'keyboard', code: 'KeyW', ctrlKey: false, shiftKey: false, altKey: false, metaKey: false }, null] };
    const before = createOperatorProfileSnapshot({
      accessibility: loaded(),
      bindings,
      preferredGamepadSlot: 2,
      tuning: { 'helm.throttle-up': { deadzone: 0.2, inverted: true } },
      feedback: { vibration: false, semanticCues: false },
      gmConfirmations: { 'gm.skip-event': 'confirm-preview' },
    });
    const after = createOperatorProfileSnapshot({
      accessibility: profileWithPresentationDefaults(loaded()),
      bindings,
      preferredGamepadSlot: 2,
      tuning: { 'helm.throttle-up': { deadzone: 0.2, inverted: true } },
      feedback: { vibration: false, semanticCues: false },
      gmConfirmations: { 'gm.skip-event': 'confirm-preview' },
    });
    expect(after.bindings).toEqual(before.bindings);
    expect(after.gamepad).toEqual(before.gamepad);
    expect(after.feedback).toEqual(before.feedback);
    expect(after.gmConfirmations).toEqual(before.gmConfirmations);
    expect(after.accessibility.presentation.textScale).toBe(FOLLOW_OS);
  });

  it('is a no-op reference when everything is already at its default', () => {
    const fresh = emptyAccessibilityProfile();
    expect(profileWithPresentationDefaults(fresh)).toBe(fresh);
  });
});

// ── 5. The choice reaches the shell AND every console iframe ────────────────

describe('one text choice reaches shell and console iframes at 100/150/200%', () => {
  it('writes the same multiplier onto the shell root and every mounted console root', () => {
    const { dom, doc } = consoleHarness();
    for (const scale of TEXT_SCALES) {
      const profile = profileWithPresentation(emptyAccessibilityProfile(), 'textScale', scale);
      const effects = applyAccessibilityProfile(profile, { doc, win: dom.window });
      expect(effects.textScale).toBeCloseTo(scale);
      expect(doc.documentElement.style.getPropertyValue(TEXT_SCALE_VAR)).toBe(String(scale));
      expect(iframeScale(doc, 'power-iframe')).toBe(String(scale));
      expect(iframeScale(doc, 'engineering-iframe')).toBe(String(scale));
      expect(iframeScale(doc, 'helm-iframe')).toBe(String(scale));
    }
  });

  it('nothing shrinks: the multiplier never falls below the identity as the choice rises', () => {
    const { dom, doc } = consoleHarness();
    let previous = 0;
    for (const scale of TEXT_SCALES) {
      const applied = applyAccessibilityProfile(
        profileWithPresentation(emptyAccessibilityProfile(), 'textScale', scale),
        { doc, win: dom.window },
      ).textScale;
      expect(applied).toBeGreaterThanOrEqual(TEXT_SCALE_DEFAULT);
      expect(applied).toBeGreaterThan(previous);
      previous = applied;
    }
  });

  it('stamps the contrast and motion decisions on the same roots', () => {
    const { dom, doc } = consoleHarness();
    let p = profileWithPresentation(emptyAccessibilityProfile(), 'contrast', EXPLICIT_ON);
    p = profileWithPresentation(p, 'reducedMotion', EXPLICIT_ON);
    applyAccessibilityProfile(p, { doc, win: dom.window });
    for (const root of [
      doc.documentElement,
      doc.getElementById('power-iframe').contentDocument.documentElement,
    ]) {
      expect(root.getAttribute('data-contrast')).toBe('more');
      expect(root.getAttribute('data-reduced-motion')).toBe('reduce');
    }
  });
});

// ── 6. The Power allocation workflow at every scale ─────────────────────────

describe('Power allocation stays operable and authoritative at 100/150/200%', () => {
  let commands;

  beforeEach(() => {
    document.body.innerHTML = '';
    commands = [];
    window.activateSemanticAction = (actionId, payload) => {
      commands.push({ actionId, ...payload });
      return true;
    };
  });

  afterEach(() => {
    document.body.innerHTML = '';
    delete window.activateSemanticAction;
    document.documentElement.style.removeProperty(TEXT_SCALE_VAR);
  });

  const mountPanel = (scale) => {
    document.documentElement.style.setProperty(TEXT_SCALE_VAR, String(scale));
    document.body.innerHTML = '<ph-power-controls id="power-controls"></ph-power-controls>';
    const el = document.getElementById('power-controls');
    el.state = { groups: POWER_GROUPS, auto: false };
    return el;
  };

  it('keeps every group, rung and stepper present and reachable at each scale', () => {
    for (const scale of TEXT_SCALES) {
      const el = mountPanel(scale);
      const where = `at ${scale}x`;
      // Three groups, each drawing its authored rungs — nothing is dropped to
      // make the enlarged layout fit.
      expect(el.shadowRoot.querySelectorAll('.group').length, where).toBe(3);
      expect(el.shadowRoot.querySelectorAll('.pip').length, where).toBe(12);
      // Both steppers per group, and the panel is ONE tab stop with arrow-key
      // roving between them, so keyboard reachability does not degrade as the
      // rows grow taller.
      const steppers = el.shadowRoot.querySelectorAll('.mini-btn');
      expect(steppers.length, where).toBe(6);
      expect(Array.from(steppers).filter((b) => b.getAttribute('tabindex') === '0').length, where)
        .toBe(1);
      expect(el.getAttribute('role'), where).toBe('toolbar');
      expect(el.getAttribute('aria-label'), where).toBe(t('component.power.title'));
    }
  });

  it('keeps the allocation STATUS readable at each scale — level, held and cold', () => {
    for (const scale of TEXT_SCALES) {
      const el = mountPanel(scale);
      const text = el.shadowRoot.textContent;
      const where = `at ${scale}x`;
      expect(text, where).toContain('PROPULSION');
      // Weapons is commanded to 3 but running at 1: the gap is stated in words,
      // not only in pip colour, so it survives contrast and forced-colour
      // changes as well as enlargement.
      expect(text, where).toContain(t('component.power.held', { n: 1, c: 3 }));
      // Shields is switched off, and says so.
      expect(text, where).toContain(t('component.power.cold'));
      const cold = el.shadowRoot.querySelector('[data-group-id="shields"] .cold-tag');
      expect(cold.hidden, where).toBe(false);
    }
  });

  it('sends the identical authoritative command at 100%, 150% and 200%', () => {
    // Enlarging text is a presentation choice. It must not change what the
    // console asks the ship to do, nor which action id carries the request.
    for (const scale of TEXT_SCALES) {
      commands = [];
      const el = mountPanel(scale);
      el.shadowRoot
        .querySelector('[data-group-id="helm"] .mini-btn[data-action="incr"]')
        .click();
      el.shadowRoot
        .querySelector('[data-group-id="helm"] .pip[data-level="4"]')
        .click();
      expect(commands.map((c) => [c.actionId, c.detail]), `at ${scale}x`).toEqual([
        [POWER_INCREASE_ACTION_ID, { target: 'helm', level: 3 }],
        [POWER_INCREASE_ACTION_ID, { target: 'helm', level: 4 }],
      ]);
      expect(commands.every((c) => c.source === 'control')).toBe(true);
    }
  });

  it('refuses the same orders at every scale — a floor and a ceiling do not move with text size', () => {
    for (const scale of TEXT_SCALES) {
      commands = [];
      const el = mountPanel(scale);
      // Shields sits at its authored floor of 0; − is disabled and sends nothing.
      const decr = el.shadowRoot.querySelector('[data-group-id="shields"] .mini-btn[data-action="decr"]');
      expect(decr.disabled, `at ${scale}x`).toBe(true);
      decr.click();
      expect(commands, `at ${scale}x`).toEqual([]);
      // And the decrease action id is still the one the console owns — the
      // refusal is a floor, not a missing binding.
      expect(POWER_DECREASE_ACTION_ID).toBeTruthy();
    }
  });

  it('draws no allocation string outside the shared type ramp, so the multiplier reaches all of them', () => {
    // The root multiplier scales `rem`, so a hard px font-size inside the
    // shadow root would be the one string that refuses to grow. Read the
    // shipped stylesheet rather than trusting jsdom to cascade `var()`.
    const el = mountPanel(2.0);
    // Every stylesheet the shadow root actually carries: the panel's own block
    // AND the shared control family adopted into it.
    const css = Array.from(el.shadowRoot.querySelectorAll('style'))
      .map((s) => s.textContent).join('\n');
    const fontSizes = Array.from(css.matchAll(/font-size:\s*([^;}]+)/g)).map((m) => m[1].trim());
    expect(fontSizes.length).toBeGreaterThan(0);
    for (const value of fontSizes) {
      // Every size is a token: --text-* off the shared ramp, or a --control-*
      // / --btn-* rung that itself resolves to one. A literal px here would be
      // the one string the root multiplier cannot reach.
      expect(value, `font-size: ${value}`).toMatch(/^var\(--(text|control|btn)-/);
    }
  });

  it('states the three pip states in forced colours, where every fill collapses to one ground', () => {
    // PRD #1418: forced-colour rendering is the browser's, and a pip that says
    // "this rung is lit" only by its fill says nothing at all in that mode.
    const el = mountPanel(1.0);
    const own = Array.from(el.shadowRoot.querySelectorAll('style'))
      .map((s) => s.textContent).find((text) => text.includes('.pip-cluster'));
    expect(own, 'the panel draws its own stylesheet').toBeTruthy();
    const block = own.slice(own.indexOf('@media (forced-colors: active)'));
    expect(block).toContain('.pip.active { background: Highlight');
    expect(block).toContain('.pip.held { background: Canvas; border-style: dashed');
    expect(block).toContain('.pip.disabled { opacity: 1; border-style: dotted');
  });
});

// ── 7. The settings surface: preview, resets, keyboard dismissal, focus ─────

describe('the Accessibility settings surface (issue #1422)', () => {
  let dom;
  let doc;
  let written;
  let resets;
  let panel;

  const control = (id) => doc.querySelector(`[data-control="${id}"]`);

  beforeEach(() => {
    dom = new JSDOM('<!doctype html><html><body></body></html>', {
      url: 'https://phoenix.test/', pretendToBeVisual: true,
    });
    doc = dom.window.document;
    written = [];
    resets = 0;
    // The live profile the panel paints from, updated exactly the way the shell
    // updates it: the write path hands the choice back into client state.
    const live = { accessibilityProfile: emptyAccessibilityProfile() };
    panel = mountSettings({
      doc,
      send() {},
      myToken: 'tok1',
      isDemo: () => false,
      getState: () => ({
        stations: [], stationRatings: {},
        accessibilityProfile: live.accessibilityProfile,
      }),
      onAccessibility: (effect, value) => {
        written.push([effect, value]);
        live.accessibilityProfile = normalizeAccessibilityProfile(
          profileWithPresentation(live.accessibilityProfile, effect, value),
        );
      },
      onAccessibilityResetPresentation: () => {
        resets += 1;
        live.accessibilityProfile = normalizeAccessibilityProfile(
          profileWithPresentationDefaults(live.accessibilityProfile),
        );
      },
    });
    // Opened the way a keyboard operator opens it — focus the cog, then
    // activate it — so the focus trap records the real opener to restore to.
    const cog = doc.getElementById('settings-btn');
    cog.focus();
    cog.click();
    panel.selectTab('accessibility');
  });

  afterEach(() => { panel.close(); });

  it('exposes a 100%–200% slider and its live status readout', () => {
    const slider = control('a11y-text-scale');
    expect(slider).not.toBeNull();
    expect(slider.min).toBe(String(TEXT_SCALE_MIN));
    expect(slider.max).toBe('2');
    expect(slider.step).toBe(String(TEXT_SCALE_STEP));
    const status = control('a11y-text-scale-status');
    expect(status.getAttribute('role')).toBe('status');
    expect(status.getAttribute('aria-live')).toBe('polite');
    // Nothing chosen yet, and jsdom's matchMedia says nothing either.
    expect(status.textContent).toContain(t('settings.accessibility.source_default'));
  });

  it('previews live on input — the choice is written and the readout moves with it', () => {
    const slider = control('a11y-text-scale');
    slider.value = '2';
    slider.dispatchEvent(new dom.window.Event('input'));
    // The write happens on `input`, not on `change`: the console text behind
    // the panel resizes under the finger.
    expect(written).toEqual([['textScale', 2]]);
    const status = control('a11y-text-scale-status');
    expect(status.textContent).toContain('200%');
    expect(status.textContent).toContain(t('settings.accessibility.source_explicit'));
    // The panel was NOT rebuilt, so the drag survives.
    expect(control('a11y-text-scale')).toBe(slider);
  });

  it('per-setting reset returns one effect and leaves the others alone', () => {
    control('a11y-text-scale').value = '1.75';
    control('a11y-text-scale').dispatchEvent(new dom.window.Event('input'));
    control('a11y-contrast-on').click();
    control('a11y-reducedMotion-on').click();
    written.length = 0;

    control('a11y-text-scale-reset').click();
    expect(written).toEqual([['textScale', 'default']]);
    // The other two are still explicitly chosen, and the tab still paints them
    // that way.
    expect(control('a11y-contrast-on').getAttribute('aria-pressed')).toBe('true');
    expect(control('a11y-reducedMotion-on').getAttribute('aria-pressed')).toBe('true');
    expect(control('a11y-text-scale').value).toBe('1');

    written.length = 0;
    control('a11y-contrast-reset').click();
    expect(written).toEqual([['contrast', 'default']]);
    expect(control('a11y-reducedMotion-on').getAttribute('aria-pressed')).toBe('true');
  });

  it('Reset all is scoped to presentation and says what it does not touch', () => {
    control('a11y-text-scale').value = '2';
    control('a11y-text-scale').dispatchEvent(new dom.window.Event('input'));
    control('a11y-contrast-on').click();
    written.length = 0;

    const resetAll = control('a11y-reset-presentation');
    expect(resetAll.textContent).toBe(t('settings.accessibility.reset_all'));
    resetAll.click();

    // ONE scoped operation, not three per-effect writes.
    expect(resets).toBe(1);
    expect(written).toEqual([]);
    expect(control('a11y-text-scale').value).toBe('1');
    expect(control('a11y-contrast-default').getAttribute('aria-pressed')).toBe('true');
    // The scope is stated to the operator, not merely honoured in code.
    expect(doc.getElementById('settings-overlay').textContent)
      .toContain(t('settings.accessibility.reset_all_scope_hint'));
  });

  it('keeps keyboard focus on a control that repaints the panel', () => {
    // Pressing a tri-state option rebuilds the whole overlay. Without focus
    // retention the operator is dumped onto the document body, outside the
    // modal, with no visible cursor.
    const more = control('a11y-contrast-on');
    more.focus();
    expect(doc.activeElement).toBe(more);
    more.click();
    const rebuilt = control('a11y-contrast-on');
    expect(rebuilt).not.toBe(more);           // it really was rebuilt
    expect(doc.activeElement).toBe(rebuilt);  // and focus came with it
  });

  it('Escape dismisses the panel and returns focus to the control that opened it', () => {
    const cog = doc.getElementById('settings-btn');
    expect(panel.isOpen()).toBe(true);
    control('a11y-text-scale').focus();
    doc.dispatchEvent(new dom.window.KeyboardEvent('keydown', {
      key: 'Escape', bubbles: true,
    }));
    expect(panel.isOpen()).toBe(false);
    expect(doc.getElementById('settings-overlay').hidden).toBe(true);
    expect(doc.activeElement).toBe(cog);
  });
});
