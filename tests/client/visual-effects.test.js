// @vitest-environment jsdom
/**
 * tests/client/visual-effects.test.js — issue #1428 (PRD #1418 stories 13-17).
 *
 * Camera/page shake, flashes and decorative interface motion become three
 * preferences instead of one, on every FULL settings surface, wired to the
 * consumers that actually render them. The claims this file has to make true,
 * in the order the acceptance criteria ask them:
 *
 *   1. all three choices visibly work — each writes a value that reaches a real
 *      consumer, and turning one down leaves the other two alone;
 *   2. Reduce effects sets them conservatively and each stays individually
 *      adjustable afterwards;
 *   3. a zero-effect state keeps the information the effect was carrying:
 *      red alert is still readable with the pulsing off, a stopped spinner
 *      still says "working", hull damage is still reported with no shake;
 *   4. persistence and both reset scopes survive a restart;
 *   5. the private console profile, a GM session and the shared Viewscreen each
 *      change only their own presentation;
 *   6. every effect a surface cannot render is RECORDED and never offered as an
 *      inert control.
 *
 * Everything drives the REAL modules against a real jsdom document. The CSS and
 * cross-language claims are read off disk — `gui/tokens.css`, `client.html`,
 * `server.html`, `src/server/viewscreen_border.rs` — because a rule that is
 * only in a file nobody imports is not a rule. What jsdom cannot do is composite
 * an animation or run the WASM renderer; that half is
 * `tests/smoke/viewscreen-effects.render.spec.js`.
 */

import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import { JSDOM } from 'jsdom';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import { t } from '../../gui/strings.js';
import { TEXT_SCALES } from '../fixtures/device-matrix.mjs';
import {
  EFFECT_IDS,
  EFFECT_FULL,
  EFFECT_OFF,
  EFFECT_REDUCED,
  EFFECT_COPY,
  EFFECT_INVENTORY,
  EFFECT_SURFACES,
  FOLLOW_PREFERENCE,
  applicableEffects,
  applyEffectIntensitiesToRoot,
  effectApplies,
  effectBand,
  effectChoiceKey,
  effectChoices,
  effectSlug,
  inapplicableEffects,
  normalizeEffectLevel,
  reduceEffectsChoices,
  resolveEffectIntensities,
  effectsAreReduced,
} from '../../gui/visual-effects.js';
import {
  FOLLOW_OS,
  EXPLICIT_ON,
  emptyAccessibilityProfile,
  normalizeAccessibilityProfile,
  profileWithPresentation,
  profileWithPresentationDefaults,
  presentationStatus,
  resolveEffects,
  applyAccessibilityProfile,
} from '../../gui/accessibility-profile.js';
import {
  createViewscreenPresentation,
  browserViewscreenStore,
  loadViewscreenPresentation,
  publishViewscreenMotion,
  viewscreenPresentationRecordFields,
} from '../../gui/viewscreen-presentation.js';
import { VIEWSCREEN_PRESENTATION_CONTROLS } from '../../gui/viewscreen-presentation-panel.js';
import { mountSettings } from '../../gui/settings-panel.js';
import { mountServerSettings } from '../../gui/server-settings.js';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const read = (rel) => fs.readFileSync(path.join(HERE, '../../', rel), 'utf-8');
const TOKENS_CSS = read('gui/tokens.css');
const CLIENT_HTML = read('client.html');
const SERVER_HTML = read('server.html');
const CONSOLE_CSS = read('gui/console.css');
const LOBBY_CSS = read('gui/host-lobby.css');
const BORDER_RS = read('src/server/viewscreen_border.rs');
const BRIDGE_RS = read('src/server/bridge.rs');
const PRESENTATION_RS = read('src/native_host/viewscreen_presentation.rs');
const HUD_RS = read('src/native_host/panes/hud.rs');
const ULTRALIGHT_RS = read('src/native_host/panes/ultralight.rs');
const HUD_HTML = read('gui/viewscreen-hud.html');
const STRINGS_CSV = read('assets/strings/strings.csv');

/** Every id `assets/strings/strings.csv` actually carries a row for. */
const STRING_IDS = new Set(
  STRINGS_CSV.split(/\r?\n/).slice(1)
    .map((line) => line.split(',')[0].trim())
    .filter(Boolean),
);

/** A localStorage-shaped store a test can also read the raw bytes out of. */
function fakeStorage(seed = {}) {
  const map = new Map(Object.entries(seed));
  return {
    getItem: (key) => (map.has(key) ? map.get(key) : null),
    setItem: (key, value) => { map.set(key, String(value)); },
    removeItem: (key) => { map.delete(key); },
    keys: () => [...map.keys()].sort(),
    raw: (key) => (map.has(key) ? map.get(key) : null),
  };
}

// ── 1. The vocabulary and its resolution ────────────────────────────────────

describe('one number per effect, and what it resolves to', () => {
  it('carries off, full and everything between, and coerces anything else', () => {
    expect(normalizeEffectLevel(0)).toBe(EFFECT_OFF);
    expect(normalizeEffectLevel(1)).toBe(EFFECT_FULL);
    expect(normalizeEffectLevel(0.3)).toBe(0.3);
    // Out of range is clamped rather than dropped: a hand-edited record asking
    // for 400% of an effect wants the loudest this build renders.
    expect(normalizeEffectLevel(4)).toBe(EFFECT_FULL);
    expect(normalizeEffectLevel(-2)).toBe(EFFECT_OFF);
    // Anything that is not a number is "follow the preference", which is the
    // value that behaves exactly as the build before this issue did.
    for (const junk of [undefined, null, 'off', {}, Number.NaN]) {
      expect(normalizeEffectLevel(junk)).toBe(FOLLOW_PREFERENCE);
    }
  });

  it('follows the resolved motion preference until something is chosen', () => {
    const unset = {};
    expect(resolveEffectIntensities(unset, false))
      .toEqual({ shake: 1, flash: 1, decorativeMotion: 1 });
    // Reduce means OFF for all three — precisely what shipped before they were
    // separable, so an operator who never opens these controls sees no change.
    expect(resolveEffectIntensities(unset, true))
      .toEqual({ shake: 0, flash: 0, decorativeMotion: 0 });
  });

  it('lets an explicit intensity override the preference in both directions', () => {
    // Keep the shake on a machine asking to reduce motion…
    expect(resolveEffectIntensities({ shake: 1 }, true).shake).toBe(1);
    // …and turn the flash off on a machine asking for nothing.
    expect(resolveEffectIntensities({ flash: 0 }, false).flash).toBe(0);
    // …while the effects NOT chosen keep following. That is what makes these
    // three separate preferences rather than one lever with three labels.
    const one = resolveEffectIntensities({ shake: 0 }, false);
    expect(one).toEqual({ shake: 0, flash: 1, decorativeMotion: 1 });
  });

  it('bands an intensity for the consumers that cannot take a number', () => {
    expect(effectBand(0)).toBe('off');
    expect(effectBand(0.4)).toBe('reduced');
    expect(effectBand(1)).toBe('full');
  });

  it('offers three named stops per effect and can say which one is pressed', () => {
    for (const effect of EFFECT_IDS) {
      const keys = effectChoices(effect).map((choice) => choice.key);
      expect(keys).toEqual(['default', 'full', 'reduced', 'off']);
      expect(effectChoiceKey(effect, EFFECT_OFF)).toBe('off');
      expect(effectChoiceKey(effect, EFFECT_REDUCED[effect])).toBe('reduced');
      expect(effectChoiceKey(effect, FOLLOW_PREFERENCE)).toBe('default');
      // A record hand-edited to sit between two stops presses none of them
      // rather than claiming the nearest.
      expect(effectChoiceKey(effect, 0.77)).toBeNull();
    }
  });
});

// ── 2. The surface-to-effect inventory ──────────────────────────────────────

describe('the inventory of where each effect is real', () => {
  it('names a consumer or a reason for every surface and every effect', () => {
    for (const surface of EFFECT_SURFACES) {
      for (const effect of EFFECT_IDS) {
        const entry = EFFECT_INVENTORY[surface][effect];
        expect(entry, `${surface}/${effect} is in the inventory`).toBeTruthy();
        // Exactly one of the two: a consumer, or a recorded reason there is
        // none. "Neither" would be an effect nobody has decided about.
        expect(Boolean(entry.consumer) !== Boolean(entry.absent)).toBe(true);
        if (entry.absent) expect(STRING_IDS.has(entry.absent)).toBe(true);
      }
    }
  });

  it('spells every String-Table id out, so the strings gate can see them', () => {
    // A COMPOSED id (`'settings.effects.' + effect`) is invisible to
    // scripts/check-strings.mjs, which would then never notice a missing row —
    // the failure mode issue #949 named. Every id below is a literal in
    // `EFFECT_COPY`, and this is the check that it has a row.
    for (const effect of EFFECT_IDS) {
      const copy = EFFECT_COPY[effect];
      expect(STRING_IDS.has(copy.labelId), copy.labelId).toBe(true);
      expect(STRING_IDS.has(copy.resetId), copy.resetId).toBe(true);
      for (const surface of Object.keys(copy.hintIds)) {
        // A hint exists exactly where the effect is offered — no orphan copy
        // for a surface that does not render it, and no missing sentence where
        // one does.
        expect(effectApplies(surface, effect), `${surface}/${effect}`).toBe(true);
        expect(STRING_IDS.has(copy.hintIds[surface]), copy.hintIds[surface]).toBe(true);
      }
      for (const surface of EFFECT_SURFACES) {
        if (effectApplies(surface, effect)) {
          expect(copy.hintIds[surface], `${surface}/${effect} hint`).toBeTruthy();
        }
      }
    }
    for (const choice of effectChoices('flash')) {
      expect(STRING_IDS.has(choice.labelId), choice.labelId).toBe(true);
    }
    for (const id of [
      'settings.effects.heading', 'settings.effects.reduce',
      'settings.effects.reduce_hint', 'settings.effects.intensity_value',
    ]) {
      expect(STRING_IDS.has(id), id).toBe(true);
    }
  });

  it('records the two effects a console has no path to, and offers neither', () => {
    // A console draws no camera and receives no page-shake offset: the `shake`
    // host channel is `server.html`'s alone.
    expect(applicableEffects('console')).toEqual(['flash', 'decorativeMotion']);
    expect(CLIENT_HTML.includes('__applyShake')).toBe(false);
    expect(SERVER_HTML.includes('__applyShake')).toBe(true);
    expect(inapplicableEffects('console').map((entry) => entry.effect)).toEqual(['shake']);
  });

  it('records what a Game Master session does not draw at all', () => {
    // The GM page hides the render surface outright, so neither the hull shake
    // nor the red-alert vignette exists there to turn down.
    expect(SERVER_HTML).toContain('html.phoenix-gm-page #canvas,');
    expect(SERVER_HTML).toContain('html.phoenix-gm-page #hud-overlay { display: none; }');
    expect(applicableEffects('gm')).toEqual(['decorativeMotion']);
    expect(inapplicableEffects('gm').map((entry) => entry.effect)).toEqual(['shake', 'flash']);
  });

  it('offers all three on the shared Viewscreen, each naming its consumer', () => {
    expect(applicableEffects('viewscreen')).toEqual(EFFECT_IDS);
    expect(EFFECT_INVENTORY.viewscreen.shake.consumer).toContain('apply_camera_shake');
    expect(EFFECT_INVENTORY.viewscreen.flash.consumer).toContain('drive_vignette_intensity');
    // …and those consumers are really in that file, under those names.
    expect(BORDER_RS).toContain('fn apply_camera_shake');
    expect(BORDER_RS).toContain('fn drive_vignette_intensity');
  });
});

// ── 3. Reduce effects ───────────────────────────────────────────────────────

describe('the Reduce effects preset', () => {
  it('writes conservative EXPLICIT values, scoped to what the surface renders', () => {
    expect(reduceEffectsChoices('viewscreen')).toEqual(EFFECT_REDUCED);
    // A console has no shake, so the preset cannot store one — which is what
    // stops the preset re-introducing the inert control the inventory removed.
    expect(Object.keys(reduceEffectsChoices('console'))).toEqual(['flash', 'decorativeMotion']);
    expect(Object.keys(reduceEffectsChoices('gm'))).toEqual(['decorativeMotion']);
  });

  it('is conservative: shake and flash off, decoration settled rather than dead', () => {
    // Flash is the effect with a photosensitivity cost, so its preset value is
    // off; decoration settles instead, because an interface that snaps between
    // states with no transition reads as broken.
    expect(EFFECT_REDUCED.flash).toBeLessThan(EFFECT_FULL);
    expect(effectBand(EFFECT_REDUCED.shake)).toBe('reduced');
    expect(effectBand(EFFECT_REDUCED.decorativeMotion)).toBe('reduced');
    expect(EFFECT_REDUCED.decorativeMotion).toBeGreaterThan(EFFECT_OFF);
  });

  it('leaves each effect individually adjustable afterwards', () => {
    let profile = emptyAccessibilityProfile();
    for (const [effect, value] of Object.entries(reduceEffectsChoices('console'))) {
      profile = profileWithPresentation(profile, effect, value);
    }
    expect(effectsAreReduced('console', normalizeAccessibilityProfile(profile).presentation))
      .toBe(true);
    // …and then ONE of them moves without disturbing the other.
    profile = profileWithPresentation(profile, 'decorativeMotion', EFFECT_FULL);
    const after = normalizeAccessibilityProfile(profile).presentation;
    expect(after.decorativeMotion).toBe(EFFECT_FULL);
    expect(after.flash).toBe(EFFECT_REDUCED.flash);
  });
});

// ── 4. What the profile applies to a document root ──────────────────────────

describe('the private profile stamps every effect on every root it reaches', () => {
  let dom;
  let doc;

  beforeEach(() => {
    dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'https://phoenix.test/' });
    doc = dom.window.document;
  });

  it('writes a band and a numeric scale for each effect', () => {
    applyAccessibilityProfile(
      normalizeAccessibilityProfile({ presentation: { flash: 0.3, decorativeMotion: 0 } }),
      { doc, win: dom.window },
    );
    const root = doc.documentElement;
    expect(root.getAttribute('data-flash')).toBe('reduced');
    expect(root.style.getPropertyValue('--a11y-flash-scale')).toBe('0.3');
    expect(root.getAttribute('data-decorative-motion')).toBe('off');
    expect(root.getAttribute('data-shake')).toBe('full');
  });

  it('reaches a console iframe root, like the text scale does', () => {
    const frame = { contentDocument: doc.implementation.createHTMLDocument('') };
    applyAccessibilityProfile(
      normalizeAccessibilityProfile({ presentation: { decorativeMotion: 0 } }),
      { doc, win: dom.window, iframes: [frame] },
    );
    expect(frame.contentDocument.documentElement.getAttribute('data-decorative-motion'))
      .toBe('off');
  });

  it('keeps stamping data-reduced-motion, which the uncovered rules still read', () => {
    // The older attribute is not replaced: it carries the OVERALL preference an
    // unset effect follows, and every animation that is not one of the three
    // named effects still hangs off it.
    applyAccessibilityProfile(
      normalizeAccessibilityProfile({ presentation: { reducedMotion: EXPLICIT_ON } }),
      { doc, win: dom.window },
    );
    expect(doc.documentElement.getAttribute('data-reduced-motion')).toBe('reduce');
    expect(doc.documentElement.getAttribute('data-decorative-motion')).toBe('off');
  });

  it('survives a root that throws rather than losing the rest', () => {
    expect(() => applyEffectIntensitiesToRoot({
      setAttribute() { throw new Error('detached'); },
    }, { shake: 1, flash: 1, decorativeMotion: 1 })).not.toThrow();
    expect(() => applyEffectIntensitiesToRoot(null, null)).not.toThrow();
  });

  it('reports where each effect’s live value came from', () => {
    const status = presentationStatus(
      normalizeAccessibilityProfile({ presentation: { flash: 0 } }),
      { reducedMotion: true },
    );
    // Chosen here, so the machine's preference is not what is showing.
    expect(status.flash).toMatchObject({ value: 0, source: 'explicit' });
    // Following, and the machine actually said something.
    expect(status.decorativeMotion).toMatchObject({ value: 0, source: 'system' });
    // Following, and the machine said nothing at all.
    expect(presentationStatus(emptyAccessibilityProfile(), {}).shake)
      .toMatchObject({ value: 1, source: 'default' });
  });

  it('inherits the motion read’s availability rather than inventing one', () => {
    // No OS reports a camera-shake preference, so a following effect's honesty
    // about "we could not ask" is the motion read's honesty.
    const status = presentationStatus(emptyAccessibilityProfile(), {}, ['reducedMotion']);
    expect(status.reducedMotion.available).toBe(false);
    for (const effect of EFFECT_IDS) expect(status[effect].available).toBe(false);
    // An explicit choice never depended on that read.
    const chosen = presentationStatus(
      normalizeAccessibilityProfile({ presentation: { shake: 0 } }), {}, ['reducedMotion'],
    );
    expect(chosen.shake.available).toBe(true);
  });
});

// ── 5. The CSS the bands actually drive ─────────────────────────────────────

describe('the bands reach real rules, and a zero state keeps its meaning', () => {
  it('gives decorative motion its own settle and off blocks', () => {
    expect(TOKENS_CSS).toContain(':root[data-decorative-motion="reduced"] *,');
    expect(TOKENS_CSS).toContain(':root[data-decorative-motion="off"] *,');
    // Settle keeps the travel and stops the repeat; off collapses everything.
    const settle = TOKENS_CSS.slice(
      TOKENS_CSS.indexOf(':root[data-decorative-motion="reduced"] *,'),
      TOKENS_CSS.indexOf(':root[data-decorative-motion="off"] *,'),
    );
    expect(settle).toContain('animation-iteration-count: 1 !important;');
    expect(settle).not.toContain('transition-duration');
  });

  it('steps the old whole-interface rule aside once a band exists', () => {
    // Otherwise an operator who reduces motion but deliberately KEEPS
    // decorative animation would have it collapsed anyway by the rule that used
    // to be the only one.
    expect(TOKENS_CSS)
      .toContain(':root[data-reduced-motion="reduce"]:not([data-decorative-motion]) *,');
    expect(TOKENS_CSS)
      .toContain(':not([data-reduced-motion="no-preference"]):not([data-decorative-motion]) *,');
    // The bare, ungated form is gone from both drivers.
    expect(TOKENS_CSS).not.toContain('\n:root[data-reduced-motion="reduce"] *,');
  });

  it('holds red alert fully lit on a phone with flashes off', () => {
    // The state survives the effect: a full red frame at full glow, not moving.
    const rule = CLIENT_HTML.slice(CLIENT_HTML.indexOf(':root[data-flash="off"] #phone-bezel.alert-on'));
    expect(rule).toContain('animation: none;');
    expect(rule.slice(0, 400)).toContain('border-color: var(--fire-hot);');
  });

  it('holds the viewscreen’s red-alert glow at full opacity with flashes off', () => {
    const rule = SERVER_HTML.slice(
      SERVER_HTML.indexOf(':root[data-flash="off"] #hud-overlay.alert-on #hud-vignette'),
    );
    expect(rule.slice(0, 200)).toContain('animation: none;');
    expect(rule.slice(0, 200)).toContain('opacity: 1;');
  });

  it('slows a pulse in proportion to the flash intensity rather than only switching it', () => {
    // A gentler flash is a slower one as well as a dimmer one, and the floor
    // stops an intensity of zero dividing by zero. Each loop names its authored
    // period and resolves it once, beside the loop, so the page's own
    // declaration and the tokens.css override read the SAME number.
    const resolved = 'calc(var(--a11y-flash-period) / max(var(--a11y-flash-scale, 1), 0.05))';
    expect(SERVER_HTML).toContain('--a11y-flash-period: 1.3s;');
    expect(CLIENT_HTML).toContain('--a11y-flash-period: 2.8s;');
    expect(SERVER_HTML).toContain(`--a11y-flash-duration: ${resolved};`);
    expect(CLIENT_HTML).toContain(`--a11y-flash-duration: ${resolved};`);
    expect(SERVER_HTML).toContain('animation-duration: var(--a11y-flash-duration);');
    expect(CLIENT_HTML).toContain('animation-duration: var(--a11y-flash-duration);');
  });

  it('keeps every decorative hold reachable from the decorative band', () => {
    for (const source of [CLIENT_HTML, CONSOLE_CSS, LOBBY_CSS, SERVER_HTML]) {
      // Nothing may still hang a decorative hold off the motion attribute alone.
      const stale = source.match(/:root\[data-reduced-motion="reduce"\] [^:{,]/g) || [];
      expect(stale).toEqual([]);
    }
    expect(CLIENT_HTML).toContain(':root[data-decorative-motion="off"] #ready-pill.go');
    expect(CONSOLE_CSS).toContain(':root[data-decorative-motion="off"] .tutorial-highlight');
    expect(LOBBY_CSS).toContain(':root[data-decorative-motion="off"] #lobby-ready-badge.go');
    expect(SERVER_HTML).toContain(':root[data-decorative-motion="off"] .spinner-ring');
  });

  it('declares the three custom properties so an unset document renders as before', () => {
    expect(TOKENS_CSS).toContain('--a11y-shake-scale: 1;');
    expect(TOKENS_CSS).toContain('--a11y-flash-scale: 1;');
    expect(TOKENS_CSS).toContain('--a11y-decorative-scale: 1;');
  });
});

// ── 5b. The cascade the three bands actually resolve to ─────────────────────

/**
 * Every style rule in a stylesheet's SOURCE, flattened out of its at-rules.
 *
 * Read from the raw text rather than through `document.styleSheets`, because
 * jsdom's CSS object model silently drops `!important` from any declaration
 * whose value contains a function — `var(…)` and `calc(…)` both — which is
 * exactly the half of the cascade this section exists to check. At-rule bodies
 * are descended into rather than evaluated: the rules that matter here sit at
 * top level, and the ones inside `@media` are all gated on a band attribute
 * being ABSENT, so a document with the bands stamped matches none of them.
 */
function flattenCssRules(source, out = []) {
  const css = source.replace(/\/\*[\s\S]*?\*\//g, '');
  let i = 0;
  while (i < css.length) {
    const open = css.indexOf('{', i);
    if (open === -1) break;
    const prelude = css.slice(i, open).trim();
    let depth = 1;
    let j = open + 1;
    while (j < css.length && depth > 0) {
      if (css[j] === '{') depth += 1;
      else if (css[j] === '}') depth -= 1;
      j += 1;
    }
    const body = css.slice(open + 1, j - 1);
    if (prelude.startsWith('@')) flattenCssRules(body, out);
    else if (prelude) out.push({ selector: prelude, body });
    i = j;
  }
  return out;
}

/** One selector's `a, b, c` specificity, with `:not()` counted inside. */
function specificity(selector) {
  let ids = 0;
  let classes = 0;
  let elements = 0;
  const rest = selector.replace(/:not\(([^)]*)\)/g, (_, inner) => {
    const [a, b, c] = specificity(inner);
    ids += a; classes += b; elements += c;
    return ' ';
  });
  ids += (rest.match(/#[\w-]+/g) || []).length;
  classes += (rest.match(/\.[\w-]+/g) || []).length
    + (rest.match(/\[[^\]]*\]/g) || []).length
    + (rest.match(/:[\w-]+(\([^)]*\))?/g) || []).length;
  elements += (rest.match(/(^|[\s>+~])[a-zA-Z][\w-]*/g) || []).length;
  return [ids, classes, elements];
}

/**
 * Which declaration of `property` actually reaches `element`, decided the way a
 * browser decides it: importance first, then specificity, then source order.
 *
 * `element.matches` is jsdom's real selector engine, so the MATCHING half is
 * not a restatement of the rules under test; only the ordering is computed here.
 */
function winningDeclaration(element, sheets, property) {
  let best = null;
  let order = 0;
  const stronger = (a, b) => {
    for (let i = 0; i < a.length; i += 1) if (a[i] !== b[i]) return a[i] > b[i];
    return false;
  };
  for (const sheet of sheets) {
    for (const rule of flattenCssRules(sheet)) {
      order += 1;
      for (const declaration of rule.body.split(';')) {
        const parsed = declaration.match(/^\s*([\w-]+)\s*:\s*([\s\S]+)$/);
        if (!parsed || parsed[1] !== property) continue;
        const important = /!important\s*$/.test(parsed[2]);
        const value = parsed[2].replace(/!important\s*$/, '').trim();
        for (const selector of rule.selector.split(',')) {
          const one = selector.trim();
          let matched = false;
          try {
            matched = element.matches(one);
          } catch {
            matched = false;
          }
          if (!matched) continue;
          const rank = [important ? 1 : 0, ...specificity(one), order];
          if (!best || stronger(rank, best.rank)) best = { rank, value, selector: one };
        }
      }
    }
  }
  return best;
}

/**
 * The real page, with gui/tokens.css inlined exactly where its `<link>` sits.
 *
 * The href differs by page — `server.html` and `client.html` link it as
 * `gui/tokens.css`, and the native HUD overlay, which is itself served from
 * `/gui/`, links it as `tokens.css` — so the link is matched rather than
 * spelled, and a page that has stopped linking it at all fails loudly here
 * instead of quietly resolving half a cascade.
 */
function pageWithTokens(html) {
  const link = /<link rel="stylesheet" href="(?:gui\/)?tokens\.css"\s*\/?>/;
  expect(link.test(html), 'the page still links gui/tokens.css').toBe(true);
  const dom = new JSDOM(
    html.replace(link, `<style>${TOKENS_CSS}</style>`),
    { url: 'https://phoenix.test/' },
  );
  const doc = dom.window.document;
  return { doc, sheets: [...doc.querySelectorAll('style')].map((el) => el.textContent) };
}

/** The phone's red-alert bezel, mid-alert, with the three bands stamped. */
function alertingBezel(intensities) {
  const { doc, sheets } = pageWithTokens(CLIENT_HTML);
  const bezel = doc.getElementById('phone-bezel');
  bezel.classList.add('alert-on');
  applyEffectIntensitiesToRoot(doc.documentElement, intensities);
  return { element: bezel, sheets };
}

/** The viewscreen's red-alert vignette, mid-alert, with the bands stamped. */
function alertingVignette(intensities) {
  return alertingVignetteIn(SERVER_HTML, intensities);
}

/**
 * The same vignette on the NATIVE Viewscreen, which draws it in a document of
 * its own (`gui/viewscreen-hud.html`) rather than in `server.html`.
 *
 * Same markup, same rule, a different file — and until issue #1428 wired the
 * bands into it, the one the room is actually looking at when the host runs
 * native was the one no band reached.
 */
function alertingNativeVignette(intensities) {
  return alertingVignetteIn(HUD_HTML, intensities);
}

function alertingVignetteIn(html, intensities) {
  const { doc, sheets } = pageWithTokens(html);
  doc.getElementById('hud-overlay').classList.add('alert-on');
  applyEffectIntensitiesToRoot(doc.documentElement, intensities);
  return { element: doc.getElementById('hud-vignette'), sheets };
}

describe('one effect turned down leaves the others running, in the cascade', () => {
  // The regression this section exists for. `gui/tokens.css`'s decorative bands
  // sweep `*` with `!important`, and both red-alert loops are ordinary elements
  // caught by that `*`. Neither page declares its loop `!important`, so without
  // the `[data-effect="flash"]` re-assertion the decorative band silently wins:
  // Flashes = Full with Interface animation = Off kills the pulse outright, and
  // Reduce effects (which writes the `reduced` band) leaves it running exactly
  // one iteration however the flash control is moved afterwards. Story 13
  // promises the opposite, and a test that only reads the STORED record — which
  // is all the suite had — cannot see any of it.
  const flashKept = [
    ['interface animation off', { shake: 1, flash: 1, decorativeMotion: 0 }],
    ['Reduce effects, then flashes back to full', { shake: 0.3, flash: 1, decorativeMotion: 0.4 }],
  ];

  for (const [when, intensities] of flashKept) {
    it(`keeps the phone bezel pulsing with ${when}`, () => {
      const { element, sheets } = alertingBezel(intensities);
      expect(winningDeclaration(element, sheets, 'animation-iteration-count').value)
        .toBe('infinite');
      // …and at its own resolved period, not the sweep's near-zero collapse.
      expect(winningDeclaration(element, sheets, 'animation-duration').value)
        .toBe('var(--a11y-flash-duration)');
    });

    it(`keeps the viewscreen vignette pulsing with ${when}`, () => {
      const { element, sheets } = alertingVignette(intensities);
      expect(winningDeclaration(element, sheets, 'animation-iteration-count').value)
        .toBe('infinite');
      expect(winningDeclaration(element, sheets, 'animation-duration').value)
        .toBe('var(--a11y-flash-duration)');
    });

    it(`keeps the NATIVE viewscreen vignette pulsing with ${when}`, () => {
      const { element, sheets } = alertingNativeVignette(intensities);
      expect(winningDeclaration(element, sheets, 'animation-iteration-count').value)
        .toBe('infinite');
      expect(winningDeclaration(element, sheets, 'animation-duration').value)
        .toBe('var(--a11y-flash-duration)');
    });
  }

  it('still stops every loop when it is the FLASH that was turned off', () => {
    // The other direction of the same gate: the re-assertion must never hold a
    // pulse an operator explicitly asked to stop, whatever the other two say.
    // All three documents that draw a red-alert loop, including the native
    // Viewscreen's own — the surface whose flash control reached the shader but
    // not the glow until issue #1428's second pass.
    for (const alerting of [alertingBezel, alertingVignette, alertingNativeVignette]) {
      const { element, sheets } = alerting({ shake: 1, flash: 0, decorativeMotion: 1 });
      expect(winningDeclaration(element, sheets, 'animation').value).toBe('none');
    }
    // …and on both vignettes the state survives the effect: the glow is still
    // fully lit, it has simply stopped moving. (The phone's bezel says the same
    // thing in `border-color` rather than opacity, which §5 asserts.)
    for (const alerting of [alertingVignette, alertingNativeVignette]) {
      const { element, sheets } = alerting({ shake: 1, flash: 0, decorativeMotion: 1 });
      expect(winningDeclaration(element, sheets, 'opacity').value).toBe('1');
    }
  });

  it('leaves decorative motion collapsed while flashes stay full', () => {
    // The complement, and the reason the re-assertion is gated rather than
    // blanket: holding the flash must not buy the pulse back at the price of the
    // setting beside it. The phone's asset-loading spinner is the decorative
    // loop the console shows.
    const { doc, sheets } = pageWithTokens(CLIENT_HTML);
    applyEffectIntensitiesToRoot(doc.documentElement, {
      shake: 1, flash: 1, decorativeMotion: 0,
    });
    const spinner = doc.querySelector('#asset-loading .spinner-ring');
    expect(spinner, 'the phone really has a decorative spinner to still').not.toBeNull();
    expect(winningDeclaration(spinner, sheets, 'animation-duration').value).toBe('0.001ms');
  });
});

// ── 6. The private console settings surface ─────────────────────────────────

describe('the console Accessibility tab’s effect controls', () => {
  let dom;
  let doc;
  let live;
  let panel;

  const control = (id) => doc.querySelector(`[data-control="${id}"]`);

  function mount() {
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
        live.accessibilityProfile = normalizeAccessibilityProfile(
          profileWithPresentation(live.accessibilityProfile, effect, value),
        );
      },
      onAccessibilityResetPresentation: () => {
        live.accessibilityProfile = normalizeAccessibilityProfile(
          profileWithPresentationDefaults(live.accessibilityProfile),
        );
      },
      onAccessibilityReduceEffects: () => {
        let next = live.accessibilityProfile;
        for (const [effect, value] of Object.entries(reduceEffectsChoices('console'))) {
          next = profileWithPresentation(next, effect, value);
        }
        live.accessibilityProfile = normalizeAccessibilityProfile(next);
      },
    });
    const cog = doc.getElementById('settings-btn');
    cog.focus();
    cog.click();
    panel.selectTab('accessibility');
  }

  beforeEach(() => {
    dom = new JSDOM('<!doctype html><html><body></body></html>', {
      url: 'https://phoenix.test/', pretendToBeVisual: true,
    });
    doc = dom.window.document;
    live = { accessibilityProfile: emptyAccessibilityProfile() };
    mount();
  });

  afterEach(() => { panel.close(); });

  it('offers a control for each effect this console renders, and none it does not', () => {
    for (const effect of applicableEffects('console')) {
      for (const choice of effectChoices(effect)) {
        expect(control(`a11y-${effectSlug(effect)}-${choice.key}`),
          `${effect}/${choice.key}`).not.toBeNull();
      }
      expect(control(`a11y-${effectSlug(effect)}-reset`)).not.toBeNull();
      expect(control(`a11y-${effectSlug(effect)}-status`).getAttribute('role')).toBe('status');
    }
    // No inert shake control — and the reason is on the page instead.
    expect(control('a11y-shake-full')).toBeNull();
    expect(control('a11y-shake-off')).toBeNull();
    const absent = control('a11y-shake-absent');
    expect(absent).not.toBeNull();
    expect(absent.textContent).toBe(t('settings.effects.absent.console_shake'));
  });

  it('turns one effect off without touching the others', () => {
    control('a11y-flash-off').click();
    const chosen = live.accessibilityProfile.presentation;
    expect(chosen.flash).toBe(EFFECT_OFF);
    expect(chosen.decorativeMotion).toBe(FOLLOW_OS);
    expect(chosen.reducedMotion).toBe(FOLLOW_OS);
    // The pressed state moves with it, and the status line says what is live.
    expect(control('a11y-flash-off').getAttribute('aria-pressed')).toBe('true');
    expect(control('a11y-flash-full').getAttribute('aria-pressed')).toBe('false');
    expect(control('a11y-flash-status').textContent)
      .toContain(t('settings.effects.level_off'));
    expect(control('a11y-flash-status').textContent)
      .toContain(t('settings.accessibility.source_explicit'));
  });

  it('reads a gentler stop back as a percentage rather than as off or full', () => {
    control('a11y-decorative-motion-reduced').click();
    expect(live.accessibilityProfile.presentation.decorativeMotion)
      .toBe(EFFECT_REDUCED.decorativeMotion);
    expect(control('a11y-decorative-motion-status').textContent).toContain(
      t('settings.effects.intensity_value', {
        value: String(Math.round(EFFECT_REDUCED.decorativeMotion * 100)),
      }),
    );
  });

  it('applies Reduce effects in one press and leaves each control adjustable', () => {
    control('a11y-reduce-effects').click();
    const after = live.accessibilityProfile.presentation;
    expect(after.flash).toBe(EFFECT_REDUCED.flash);
    expect(after.decorativeMotion).toBe(EFFECT_REDUCED.decorativeMotion);
    // Scoped: it cannot store the effect this surface does not render, and it
    // does not reach the text size or the contrast beside it.
    expect(after.shake).toBe(FOLLOW_OS);
    expect(after.textScale).toBe(FOLLOW_OS);
    expect(after.contrast).toBe(FOLLOW_OS);
    // …and afterwards ONE of them still moves on its own.
    control('a11y-decorative-motion-full').click();
    expect(live.accessibilityProfile.presentation.decorativeMotion).toBe(EFFECT_FULL);
    expect(live.accessibilityProfile.presentation.flash).toBe(EFFECT_REDUCED.flash);
  });

  it('returns one effect to following the preference from its own reset', () => {
    control('a11y-flash-off').click();
    control('a11y-decorative-motion-off').click();
    control('a11y-flash-reset').click();
    expect(live.accessibilityProfile.presentation.flash).toBe(FOLLOW_OS);
    expect(live.accessibilityProfile.presentation.decorativeMotion).toBe(EFFECT_OFF);
  });

  it('sweeps every effect up in the scoped Reset all, and nothing else', () => {
    control('a11y-flash-off').click();
    control('a11y-decorative-motion-reduced').click();
    control('a11y-text-scale').value = '1.75';
    control('a11y-text-scale').dispatchEvent(new dom.window.Event('input'));
    live.accessibilityProfile = normalizeAccessibilityProfile(
      profileWithPresentation(live.accessibilityProfile, 'shake', EFFECT_OFF),
    );
    // An assistance override is not presentation and must ride through.
    live.accessibilityProfile.assistance['helm.course-keeping'] = 'request';

    control('a11y-reset-presentation').click();
    const after = live.accessibilityProfile;
    for (const effect of ['textScale', 'contrast', 'reducedMotion', ...EFFECT_IDS]) {
      expect(after.presentation[effect], effect).toBe(FOLLOW_OS);
    }
    expect(after.assistance['helm.course-keeping']).toBe('request');
  });

  it('keeps every effect control reachable at 200% text', () => {
    // The #1418 usability contract: settings must enlarge as reliably as the
    // rest, so the controls added here are checked at each supported scale.
    for (const scale of TEXT_SCALES) {
      applyAccessibilityProfile(
        normalizeAccessibilityProfile({ presentation: { textScale: scale } }),
        { doc, win: dom.window },
      );
      expect(doc.documentElement.style.getPropertyValue('--a11y-text-scale'))
        .toBe(String(scale));
      panel.selectTab('accessibility');
      for (const effect of applicableEffects('console')) {
        for (const choice of effectChoices(effect)) {
          const el = control(`a11y-${effectSlug(effect)}-${choice.key}`);
          expect(el, `${effect}/${choice.key} at ${scale}`).not.toBeNull();
          // A button, in the tab order, with its own accessible name — never a
          // div that a keyboard operator cannot reach.
          expect(el.tagName).toBe('BUTTON');
          expect(el.textContent.trim()).not.toBe('');
          expect(el.hasAttribute('disabled')).toBe(false);
        }
      }
      expect(control('a11y-reduce-effects')).not.toBeNull();
      // The panel's own root grows with the setting rather than the text being
      // shrunk to fit it (issue #1422's rule, re-checked for the new rows).
      expect(doc.documentElement.style.getPropertyValue('--a11y-text-scale'))
        .toBe(String(scale));
    }
  });
});

// ── 7. The Viewscreen and Game Master surfaces ──────────────────────────────

describe('the Viewscreen cog’s effect controls', () => {
  const doc = document;
  let storage;
  let panel;

  const control = (id) => doc.querySelector(`[data-control="${id}"]`);

  function mount({ gm = false } = {}) {
    storage = storage || fakeStorage();
    panel = mountServerSettings({
      doc,
      bindings: { __getMasterVolume: () => 1, __hostLocalGm: () => gm },
      isDemo: () => false,
      autoRefresh: false,
      startGamepad: false,
      presentation: createViewscreenPresentation({
        doc, win: window, store: browserViewscreenStore(storage),
      }),
    });
    panel.open();
    panel.selectTab('presentation');
  }

  beforeEach(() => {
    document.body.innerHTML = '';
    document.documentElement.style.removeProperty('--a11y-text-scale');
    for (const attr of ['data-contrast', 'data-shake', 'data-flash', 'data-decorative-motion']) {
      document.documentElement.removeAttribute(attr);
    }
    storage = fakeStorage();
  });

  afterEach(() => {
    panel.destroy();
    document.body.innerHTML = '';
  });

  it('offers all three effects on a shared display', () => {
    mount();
    for (const effect of EFFECT_IDS) {
      for (const choice of effectChoices(effect)) {
        expect(control(VIEWSCREEN_PRESENTATION_CONTROLS.effect(effect, choice.key)),
          `${effect}/${choice.key}`).not.toBeNull();
      }
      expect(control(VIEWSCREEN_PRESENTATION_CONTROLS.effectReset(effect))).not.toBeNull();
    }
    expect(control(VIEWSCREEN_PRESENTATION_CONTROLS.reduceEffects)).not.toBeNull();
    // Nothing to record as missing on this surface.
    expect(control(VIEWSCREEN_PRESENTATION_CONTROLS.effectAbsent('shake'))).toBeNull();
  });

  it('previews a change on this document at once, and remembers it', () => {
    mount();
    control(VIEWSCREEN_PRESENTATION_CONTROLS.effect('shake', 'off')).click();
    expect(doc.documentElement.getAttribute('data-shake')).toBe('off');
    expect(doc.documentElement.style.getPropertyValue('--a11y-shake-scale')).toBe('0');
    // …and the endpoint's record carries it to the next launch.
    expect(loadViewscreenPresentation(storage).shake).toBe(EFFECT_OFF);
    expect(loadViewscreenPresentation(storage).flash).toBe(FOLLOW_OS);
  });

  it('applies Reduce effects and still resets one effect on its own', () => {
    mount();
    control(VIEWSCREEN_PRESENTATION_CONTROLS.reduceEffects).click();
    for (const effect of EFFECT_IDS) {
      expect(loadViewscreenPresentation(storage)[effect], effect).toBe(EFFECT_REDUCED[effect]);
    }
    control(VIEWSCREEN_PRESENTATION_CONTROLS.effectReset('flash')).click();
    expect(loadViewscreenPresentation(storage).flash).toBe(FOLLOW_OS);
    expect(loadViewscreenPresentation(storage).shake).toBe(EFFECT_REDUCED.shake);
    // Reset all is still scoped to this record alone.
    control(VIEWSCREEN_PRESENTATION_CONTROLS.resetAll).click();
    for (const effect of EFFECT_IDS) {
      expect(loadViewscreenPresentation(storage)[effect], effect).toBe(FOLLOW_OS);
    }
  });

  it('offers a Game Master only the effect that session actually draws', () => {
    mount({ gm: true });
    for (const choice of effectChoices('decorativeMotion')) {
      expect(control(VIEWSCREEN_PRESENTATION_CONTROLS.effect('decorativeMotion', choice.key)))
        .not.toBeNull();
    }
    for (const effect of ['shake', 'flash']) {
      expect(control(VIEWSCREEN_PRESENTATION_CONTROLS.effect(effect, 'off')),
        `${effect} is not offered`).toBeNull();
      const absent = control(VIEWSCREEN_PRESENTATION_CONTROLS.effectAbsent(effect));
      expect(absent, `${effect} is recorded`).not.toBeNull();
      expect(absent.textContent.trim()).not.toBe('');
    }
    // …and pressing the preset there cannot store a value for either of them.
    control(VIEWSCREEN_PRESENTATION_CONTROLS.reduceEffects).click();
    expect(loadViewscreenPresentation(storage).shake).toBe(FOLLOW_OS);
    expect(loadViewscreenPresentation(storage).decorativeMotion)
      .toBe(EFFECT_REDUCED.decorativeMotion);
  });

  it('changes this endpoint only — never a player’s private profile', () => {
    mount();
    control(VIEWSCREEN_PRESENTATION_CONTROLS.effect('flash', 'off')).click();
    expect(storage.keys()).toEqual(['phoenix-viewscreen-presentation-v1']);
    // The private profile's own resolution is untouched by anything above.
    expect(resolveEffects(emptyAccessibilityProfile(), {}).flash).toBe(EFFECT_FULL);
  });
});

// ── 8. The seam to the renderer ─────────────────────────────────────────────

describe('the two effects the renderer owns cross the wasm seam', () => {
  it('publishes both intensities whenever the record is applied', () => {
    const calls = [];
    const win = {
      wasm_set_shake_intensity: (v) => calls.push(['shake', v]),
      wasm_set_flash_intensity: (v) => calls.push(['flash', v]),
    };
    publishViewscreenMotion(win, { shake: 0, flash: 0.3 });
    expect(calls).toEqual([['shake', 0], ['flash', 0.3]]);
  });

  it('is a no-op where there is no renderer, and survives one that throws', () => {
    expect(() => publishViewscreenMotion({}, { shake: 0, flash: 0 })).not.toThrow();
    expect(() => publishViewscreenMotion(null, null)).not.toThrow();
    expect(() => publishViewscreenMotion({
      wasm_set_shake_intensity() { throw new Error('tearing down'); },
    }, { shake: 1, flash: 1 })).not.toThrow();
  });

  it('names the setters the Rust side actually exports', () => {
    // A cross-language contract with no compiler behind it.
    for (const name of ['wasm_set_shake_intensity', 'wasm_set_flash_intensity']) {
      expect(BRIDGE_RS).toContain(`pub fn ${name}(intensity: f32)`);
      expect(SERVER_HTML).toContain(`window.${name} = wasmBindings.${name};`);
    }
    // …and the readbacks a smoke spec proves the crossing with.
    expect(BRIDGE_RS).toContain('pub fn wasm_shake_intensity() -> f32');
    expect(BRIDGE_RS).toContain('pub fn wasm_flash_intensity() -> f32');
  });

  it('folds the reduced-motion preference into the same two numbers in Rust', () => {
    // One number decides how far the picture moves, rather than a flag and a
    // scale that could disagree.
    expect(BORDER_RS).toContain('pub fn shake_magnitude(total_hull_damage: f32, intensity: f32)');
    expect(BORDER_RS).toContain('pub fn scaled_flash_intensity(raw: f32, intensity: f32)');
    expect(BORDER_RS).toContain('fn sync_viewscreen_motion(');
  });

  it('crosses to the native host as whole percent, with a chosen zero preserved', () => {
    const fields = viewscreenPresentationRecordFields({ shake: 0, flash: 0.3 });
    expect(fields.shake_percent).toBe(0);
    expect(fields.flash_percent).toBe(30);
    expect(fields.decorative_motion_percent).toBeNull();
    // …and the host reads those names, stores them, and pushes the two the
    // renderer owns into the live latch.
    for (const field of ['shake_percent', 'flash_percent', 'decorative_motion_percent']) {
      expect(PRESENTATION_RS).toContain(`pub ${field}: Option<u32>`);
    }
    expect(BRIDGE_RS).toContain('pub fn set_native_effect_intensities(');
  });

  it('carries the bands on to the native Viewscreen’s own HUD document', () => {
    // The native Viewscreen draws its frame, readout and red-alert vignette in
    // a THIRD document — gui/viewscreen-hud.html, a transparent Ultralight
    // surface over the 3-D view. It is not the lobby document, so the head
    // injection that seeds `window.PhoenixViewscreenPresentation` never reaches
    // it, and an Ultralight view answers no `prefers-reduced-motion` query: its
    // `@media` rule is dead on the runtime that needs it. Without this push the
    // Display tab's Flashes = Off reached the shield-flash uniform and left the
    // glow the room is looking at pulsing.
    expect(HUD_RS).toContain('pub(super) fn hud_effects_script(');
    expect(HUD_RS).toContain('window.__phoenixSetHudEffects(');
    // Resolved ONCE, by the same system the shader reads, so the overlay and
    // the renderer cannot disagree about what the room chose.
    expect(BORDER_RS).toContain('pub decorative_intensity: f32');
    expect(ULTRALIGHT_RS).toContain('motion.shake_intensity,');
    expect(ULTRALIGHT_RS).toContain('motion.decorative_intensity,');
    // And the page answers on the name the host calls.
    expect(HUD_HTML).toContain('window.__phoenixSetHudEffects = function');
  });

  it('gives that document the same two flash rules server.html carries', () => {
    // The pair that makes an off-state a HELD frame rather than a hidden one,
    // and keeps the pre-stamp @media fallback from outranking a stamped band.
    expect(HUD_HTML)
      .toContain(':root[data-flash="off"] #hud-overlay.alert-on #hud-vignette');
    expect(HUD_HTML)
      .toContain(':root:not([data-flash]) #hud-overlay.alert-on #hud-vignette');
    expect(HUD_HTML).toContain('<div id="hud-vignette" data-effect="flash">');
    expect(HUD_HTML).toContain('--a11y-flash-period: 1.3s;');
    expect(HUD_HTML).toContain('animation-duration: var(--a11y-flash-duration);');
    // The bare, ungated form of the old rule is gone: a stamped band must win.
    expect(HUD_HTML.replace(/\s+/g, ' '))
      .not.toContain('} #hud-overlay.alert-on #hud-vignette { animation: none;');
  });
});
