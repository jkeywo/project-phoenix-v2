/**
 * tests/client/control-floors.test.js — module 3's contract (PRD #1023).
 *
 * Three floors, and the PRD's own words for why they are a TEST rather than a
 * one-off sweep: "so that a future component cannot silently reintroduce
 * 6-pixel text". Every one of these was already broken by shipped code, and
 * every one of them broke quietly — nothing crashes when a button is 16px, it
 * just stops being pressable while you are being shot at.
 *
 *   1. TOUCH. Every control a player taps is at least `--control-hit-min`
 *      (44px) of finger, or carries the documented escape.
 *   2. TYPE. No string renders below `--text-min`, including the two places
 *      the CSS ramp cannot reach: an inline `style=` attribute and a canvas
 *      font string.
 *   3. MOTION. Every indefinitely looping animation has a reduced-motion
 *      variant.
 *
 * ── How "a control" is decided ─────────────────────────────────────────────
 *
 * Not from a list of class names — a list is a thing to forget to add to, and
 * the next component's button would not be on it. From what the author already
 * wrote: a rule that declares `cursor: pointer` is the author saying "this is
 * clickable", and a `<button>` or `<select>` is one whether anyone said so or
 * not. Both are asked for the floor.
 *
 * That leaves a small EXEMPT list, each entry with a reason, because a few
 * things declare `cursor: pointer` and are not controls at all (a full-screen
 * scrim you tap to dismiss is already far larger than 44px) or cannot carry a
 * CSS height (an SVG path, a slider thumb).
 */
import { describe, it, expect } from 'vitest';
import fs from 'node:fs';
import path from 'node:path';
import {
  REPO_ROOT, GUI, TOKENS_CSS, readStripped, cssRules,
  consoleDocuments, componentFiles, rel,
} from './css-scan.js';

const TOKENS = fs.readFileSync(TOKENS_CSS, 'utf8');

/**
 * The PHONE surfaces. `--control-hit-min` is a thumb, and these are the
 * documents a thumb touches: the lobby the player joins on, the console
 * iframes, and the components inside them.
 *
 * server.html is deliberately absent from the TOUCH floor and present in the
 * MOTION one. It is the shared viewscreen — a television with a laptop beside
 * it, driven by a mouse, never tapped — so a 44px minimum there would enlarge
 * host chrome for a finger that never arrives. Motion sensitivity is not an
 * input device, so the reduced-motion sweep covers it like everything else.
 */
const TOUCH_SURFACES = [
  ...componentFiles(),
  ...consoleDocuments(),
  path.join(GUI, 'console.css'),
  path.join(REPO_ROOT, 'client.html'),
];

const MOTION_SURFACES = [
  ...TOUCH_SURFACES,
  path.join(REPO_ROOT, 'server.html'),
];

/**
 * Selectors that declare themselves clickable and are exempt from the size
 * floor, each with the reason.
 *
 * Keyed by `file :: selector`, so an exemption is granted to one rule in one
 * file rather than to a class name everywhere it might later appear.
 */
const EXEMPT = new Map([
  ['client.html :: .settings-overlay',
    'the full-screen scrim behind the settings modal; tap-anywhere to dismiss, '
    + 'so its target is the viewport'],
  ['client.html :: #coordination-popup',
    'a full-width popup dismissed by tapping it; already far above the floor'],
  ['client.html :: .settings-vol-row input[type="range"]::-webkit-slider-thumb',
    'a slider thumb cannot carry a min-height — the ROW it sits in takes the floor'],
  ['client.html :: .settings-vol-row input[type="range"]::-moz-range-thumb',
    'as above, the Firefox spelling of the same thumb'],
  ['gui/components/ph-shield-facings.js :: .arc-path',
    'an SVG path: no CSS box, so no min-height. Its target is .hit-path below'],
  ['gui/components/ph-shield-facings.js :: .hit-path',
    'the invisible fat stroke that IS the shield arc\'s target; asserted by '
    + 'stroke-width in its own test rather than by a CSS height'],
  ['gui/components/ph-power-controls.js :: .pip',
    'five to a row on a 375px phone: 44px each would be 220px of pip before '
    + 'the two steppers. Takes the documented escape — see the .hit-expand assertion'],
]);

/** Every rule in `file` that declares itself interactive. */
function interactiveRules(file) {
  const source = readStripped(file);
  return cssRules(source).filter(({ selector, body }) => {
    if (selector.startsWith('@')) return false;
    // A pure state variant (`:hover`, `:disabled`, `.active`) restyles a
    // control the base rule already sized; asking each variant for the floor
    // again would be noise.
    if (/:(hover|active|focus|disabled|checked)\b/.test(selector)) return false;
    const saysClickable = /cursor\s*:\s*pointer/.test(body);
    const isFormControl = /(^|[\s,>+~])(button|select)($|[\s,:.[])/.test(`${selector} `);
    return saysClickable || isFormControl;
  });
}

/** Members of the shared control family, which carries the floor for them. */
const FAMILY = new Set(['.btn', '.btn--sm', '.btn--md', '.btn--lg', '.mini-btn']);

const FLOOR_DECLARATION = /min-height\s*:\s*var\(--control-hit-min\)/;

/**
 * Does this control reach the floor, or legitimately escape it?
 *
 * Four ways, and each of them is a real mechanism rather than a way of not
 * looking:
 *
 *   - the floor is declared on this selector SOMEWHERE in the file. Not
 *     necessarily in the same block: a component may split one element's
 *     styling across several rules, and ph-objective-list's `.row` is sized in
 *     one and made clickable in another.
 *   - the selector is a member of the shared control family, which declares the
 *     floor once for all of them.
 *   - the selector is a STATE VARIANT of a control the file already floors
 *     (`button.mine` beside `button`) — the base rule is what sizes the box.
 *   - the markup puts `hit-expand` on it: the documented escape.
 */
function meetsHitFloor(rule, rules, source) {
  const declaresFloor = (selector) => rules
    .some((other) => other.selector === selector && FLOOR_DECLARATION.test(other.body));

  if (declaresFloor(rule.selector)) return true;

  const target = rule.selector.split(',')[0].trim().replace(/^.*[\s>+~]/, '');
  if (FAMILY.has(target)) return true;

  const base = target.replace(/(?!^)\.[-\w]+$/, '');
  if (base !== target && (FAMILY.has(base) || declaresFloor(base)
    || rules.some((other) => other.selector === base))) return true;

  const bare = target.replace(/^\./, '');
  return new RegExp(`class="[^"]*\\b${bare}\\b[^"]*\\bhit-expand\\b`).test(source)
    || new RegExp(`class="[^"]*\\bhit-expand\\b[^"]*\\b${bare}\\b`).test(source)
    || new RegExp(`'${bare} hit-expand'`).test(source);
}

// ── 1. Touch ────────────────────────────────────────────────────────────────

describe('the token vocabulary carries a touch floor', () => {
  it('names --control-hit-min at the 44px platform guidance', () => {
    expect(TOKENS).toMatch(/--control-hit-min:\s*44px/);
  });

  it('applies it in the shared control family rather than per component', () => {
    const family = fs.readFileSync(path.join(GUI, 'components', 'ph-console-styles.js'), 'utf8');
    expect(family).toMatch(/min-height:\s*var\(--control-hit-min\)/);
    // The square stepper needs both axes: a 44px-tall control 16px wide is
    // still a miss waiting to happen.
    expect(family).toMatch(/min-width:\s*var\(--control-hit-min\)/);
    // And the authored proportions survive — the floor raises the small ones,
    // it does not flatten the family into one size.
    expect(family).toMatch(/height:\s*var\(--btn-h\)/);
  });

  it('offers one documented escape, on both axes, opt-in per axis', () => {
    const family = fs.readFileSync(path.join(GUI, 'components', 'ph-console-styles.js'), 'utf8');
    expect(family).toMatch(/\.hit-expand::after/);
    expect(family).toMatch(/--hit-expand-w/);
    expect(family).toMatch(/--hit-expand-h/);
  });
});

describe('every interactive control meets the touch floor', () => {
  for (const file of TOUCH_SURFACES) {
    const name = rel(file);
    const rules = interactiveRules(file);
    if (rules.length === 0) continue;
    it(`${name} sizes its ${rules.length} control rule(s) for a thumb`, () => {
      const source = readStripped(file);
      const all = cssRules(source);
      const short = [];
      for (const rule of rules) {
        if (EXEMPT.has(`${name} :: ${rule.selector}`)) continue;
        if (!meetsHitFloor(rule, all, source)) short.push(rule.selector);
      }
      expect(short).toEqual([]);
    });
  }

  it('grants no exemption without a reason written next to it', () => {
    for (const [key, reason] of EXEMPT) {
      expect(reason.length, `${key} is exempt with no reason`).toBeGreaterThan(20);
    }
  });

  it('the power pips take the escape rather than a quieter floor', () => {
    // The one place in the codebase that cannot fit the floor, so it is the
    // one place the escape is used — and it is used on the axis that HAS room.
    const source = fs.readFileSync(path.join(GUI, 'components', 'ph-power-controls.js'), 'utf8');
    expect(source).toMatch(/class="pip hit-expand"|classList\.add\('hit-expand'\)|pip hit-expand/);
    // Vertical floor taken in full; horizontal held at the pip's own width so
    // a tap never lands on the neighbouring rung.
    expect(source).toMatch(/--hit-expand-w:\s*100%/);
  });
});

// ── 2. Type ─────────────────────────────────────────────────────────────────

describe('every string meets the type floor', () => {
  it('the ramp floors every rung against an absolute minimum', () => {
    const ramp = TOKENS.match(/--text-(?:xs|sm|md|lg|xl|2xl|display):\s*([^;]+);/g) || [];
    expect(ramp.length).toBeGreaterThanOrEqual(5);
    for (const rung of ramp) expect(rung).toMatch(/max\(/);
  });

  // The CSS ramp reaches every `font-size` declaration — design-tokens.test.js
  // holds that line. These are the two places it CANNOT reach, and both had
  // shipped with a bare number.
  //
  // Phone surfaces only, and for a sharper reason than the touch floor's. The
  // failure the type floor exists to prevent is a RELATIVE size against the
  // console root: `clamp(11px, 3vw, 15px)` makes `0.65rem` render at 7.2px on a
  // narrow phone. server.html has no such root — it is a television at the
  // browser's default 16px — so the mechanism cannot fire there, and its
  // handful of inline diagnostics sit well above the floor at that root.
  for (const file of TOUCH_SURFACES) {
    const name = rel(file);
    it(`${name} sets no inline font-size that dodges the ramp`, () => {
      const source = readStripped(file);
      const inline = [];
      const re = /style\s*=\s*(["'])([^"']*font-size[^"']*)\1/gi;
      let m;
      while ((m = re.exec(source)) !== null) {
        if (!/var\(--text/.test(m[2])) inline.push(m[2]);
      }
      // The JS spelling of the same thing.
      const assigned = /style\.fontSize\s*=\s*(['"`])([^'"`]*)\1/g;
      while ((m = assigned.exec(source)) !== null) {
        if (!/var\(--text/.test(m[2])) inline.push(`style.fontSize = ${m[2]}`);
      }
      expect(inline).toEqual([]);
    });
  }

  it('canvas text is sized from the token, not from a number in the draw call', () => {
    // A `<canvas>` font string is not a CSS declaration, so no ramp reaches it
    // — which is how the radar came to draw 11px labels into a buffer scaled by
    // the device pixel ratio, rendering them at 3.7 CSS px on a 3x phone.
    for (const file of componentFiles()) {
      const source = readStripped(file);
      const fonts = [];
      const re = /\.font\s*=\s*([^;\n]+)/g;
      let m;
      while ((m = re.exec(source)) !== null) {
        const value = m[1].trim();
        // A bare numeric literal in a font string is the failure: it is a
        // DEVICE pixel in a scaled buffer and it dodges --text-min entirely.
        if (/(['"`])\s*\d/.test(value)) fonts.push(`${rel(file)}: ctx.font = ${value}`);
      }
      expect(fonts).toEqual([]);
    }
  });
});

// ── 3. Motion ───────────────────────────────────────────────────────────────

/**
 * Every `animation:` shorthand that never stops, as `{selector, name, order}`.
 *
 * `order` is the rule's index in the file, and it is not bookkeeping — see
 * `stilledSelectors`.
 */
function loopingAnimations(file) {
  const source = readStripped(file);
  const out = [];
  cssRules(source).forEach((rule, order) => {
    if (rule.at && /prefers-reduced-motion/.test(rule.at)) return;
    const m = rule.body.match(/animation\s*:\s*([^;]+)/);
    if (!m || !/\binfinite\b/.test(m[1])) return;
    const name = m[1].trim().split(/\s+/).find((tok) => /^[a-z][-a-z0-9]*$/i.test(tok)
      && !/^(ease|linear|infinite|alternate|reverse|both|forwards|backwards|none|normal|running|paused)/.test(tok));
    out.push({ selector: rule.selector, name, order, important: /!important/.test(m[1]) });
  });
  return out;
}

/**
 * Where each selector is stilled inside a `prefers-reduced-motion` block, as
 * `selector -> {order, important}`.
 *
 * The ORDER matters as much as the existence, and this is the part that is easy
 * to get wrong: a media query adds NO specificity. `@media (prefers-reduced-
 * motion) { #ready-pill.go { animation: none } }` and the `#ready-pill.go` that
 * starts the pulse weigh exactly the same, so whichever is written later wins.
 * Four of the lobby's overrides were first written beside the red-alert bezel,
 * near the top of the stylesheet, above every loop but the bezel's own — and
 * every one of them silently lost. The page went on pulsing at a player who had
 * asked it not to, with a correct-looking reduced-motion block sitting in the
 * file. A rule that loses the cascade is indistinguishable from one nobody
 * wrote, which is exactly the kind of failure a test has to catch because
 * reading the source will not.
 */
function stilledSelectors(file) {
  const source = readStripped(file);
  const out = new Map();
  cssRules(source).forEach((rule, order) => {
    if (!rule.at || !/prefers-reduced-motion/.test(rule.at)) return;
    const m = rule.body.match(/animation\s*:\s*none([^;]*)/);
    if (!m) return;
    for (const part of rule.selector.split(',')) {
      out.set(part.trim(), { order, important: /!important/.test(m[1]) });
    }
  });
  return out;
}

describe('every looping animation respects reduced motion', () => {
  it('finds the loops to check', () => {
    const total = MOTION_SURFACES.reduce((n, f) => n + loopingAnimations(f).length, 0);
    // The red-alert bezel plus the spinners, pulses and glows the sweep found.
    expect(total).toBeGreaterThanOrEqual(8);
  });

  for (const file of MOTION_SURFACES) {
    const name = rel(file);
    const loops = loopingAnimations(file);
    if (loops.length === 0) continue;
    it(`${name} stills all ${loops.length} of its loops when asked`, () => {
      const stilled = stilledSelectors(file);
      const unanswered = loops
        .filter((loop) => !stilled.has(loop.selector))
        .map((loop) => `${loop.selector} (${loop.name})`);
      expect(unanswered).toEqual([]);
    });

    it(`${name} writes each override where it actually WINS`, () => {
      const stilled = stilledSelectors(file);
      const losing = loops
        .filter((loop) => {
          const override = stilled.get(loop.selector);
          if (!override) return false; // the test above already reports this
          if (override.important && !loop.important) return false;
          return override.order < loop.order;
        })
        .map((loop) => `${loop.selector} (${loop.name}) is overridden ABOVE the rule that starts it`);
      expect(losing).toEqual([]);
    });
  }

  it('stills the motion without dropping the information', () => {
    // The accessibility answer is to stop the movement, not to hide what the
    // movement was telling the crew. The red-alert bezel is the case that
    // makes this concrete: a player on reduced motion must still be able to
    // tell, from across the room, that the ship is at red alert.
    const lobby = readStripped(path.join(REPO_ROOT, 'client.html'));
    for (const rule of cssRules(lobby)) {
      if (!rule.at || !/prefers-reduced-motion/.test(rule.at)) continue;
      expect(rule.body).not.toMatch(/display\s*:\s*none/);
      expect(rule.body).not.toMatch(/visibility\s*:\s*hidden/);
      expect(rule.body).not.toMatch(/opacity\s*:\s*0\s*(;|$)/);
    }
  });
});
