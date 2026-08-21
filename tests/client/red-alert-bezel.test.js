/**
 * tests/client/red-alert-bezel.test.js — the phone bezel's red-alert pulse.
 *
 * The bezel is the game's one piece of chrome that is read from across a room:
 * a crew tells the ship is at red alert by glancing at each other's phones. It
 * therefore has to keep pulsing for as long as the alert stands. It shipped
 * with `animation: … forwards` and no iteration count, so it pulsed once — a
 * few seconds at the moment the captain raised the alert — and then held a
 * static frame for the rest of the engagement (PRD #1023's defect list).
 *
 * These read client.html's stylesheet as source. There is no DOM to inspect:
 * the rule is a media-query-dependent declaration on an element that only
 * exists on a live phone, and the failure mode is a missing keyword, which is
 * exactly what source assertions are good at.
 */
import { describe, it, expect } from 'vitest';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const REPO_ROOT = path.join(path.dirname(fileURLToPath(import.meta.url)), '..', '..');
const CLIENT_HTML = fs.readFileSync(path.join(REPO_ROOT, 'client.html'), 'utf8');

/** The declarations of the first rule whose selector matches, as one string. */
function ruleBody(source, selector) {
  const index = source.indexOf(selector + ' {');
  if (index === -1) return null;
  const open = source.indexOf('{', index);
  const close = source.indexOf('}', open);
  return source.slice(open + 1, close);
}

/**
 * The bezel's reduced-motion held-frame, as `{selector, body}`.
 *
 * Since #1172 the shell no longer stills the bezel with a bare `@media`
 * block — the global layer in gui/tokens.css collapses the motion off both the
 * OS query and the stamped attribute, and this rule only HOLDS the peak. It is
 * attribute-driven (`:root[data-reduced-motion="reduce"] #phone-bezel.alert-on`)
 * so an explicit choice resolves both ways, which a bare `@media` cannot do.
 */
function reducedMotionBezelRule(source) {
  const selector = ':root[data-reduced-motion="reduce"] #phone-bezel.alert-on';
  const index = source.indexOf(selector + ' {');
  if (index === -1) return null;
  const open = source.indexOf('{', index);
  const close = source.indexOf('}', open);
  if (open === -1 || close === -1) return null;
  return { selector, body: source.slice(open + 1, close) };
}

describe('red-alert bezel', () => {
  it('pulses for as long as the alert stands', () => {
    const body = ruleBody(CLIENT_HTML, '#phone-bezel.alert-on');
    expect(body).not.toBeNull();
    const animation = body.match(/animation:\s*([^;]+);/);
    expect(animation).not.toBeNull();
    expect(animation[1]).toContain('bezel-pulse');
    // The whole defect in one assertion: a one-shot fill mode where an
    // iteration count belongs.
    expect(animation[1]).toContain('infinite');
    expect(animation[1]).not.toContain('forwards');
  });

  it('keeps a loop worth looping at — the keyframes return to their start', () => {
    // `infinite` on keyframes that end somewhere other than they began is a
    // visible jump once a second rather than a pulse.
    const keyframes = CLIENT_HTML.slice(CLIENT_HTML.indexOf('@keyframes bezel-pulse'));
    const first = keyframes.match(/0%\s*\{([^}]*)\}/);
    const last = keyframes.match(/100%\s*\{([^}]*)\}/);
    expect(first).not.toBeNull();
    expect(last).not.toBeNull();
    expect(first[1].trim()).toBe(last[1].trim());
  });
});

describe('reduced motion', () => {
  it('stills the bezel pulse when the player asks for reduced motion', () => {
    const rule = reducedMotionBezelRule(CLIENT_HTML);
    expect(rule).not.toBeNull();
    expect(rule.selector).toContain('#phone-bezel.alert-on');
    expect(rule.body).toMatch(/animation:\s*none/);
    // #1172: driven by the stamped attribute, not a bare @media — so an
    // explicit "allow motion" resolves the tri-state and this steps aside.
    expect(rule.selector).toContain('data-reduced-motion="reduce"');
  });

  it('stills the pulse without hiding the alert', () => {
    // The accessibility answer is to stop the motion, not to drop the
    // information: a player on reduced motion must still be able to tell that
    // the ship is at red alert, so the bezel holds at the pulse's peak.
    const rule = reducedMotionBezelRule(CLIENT_HTML);
    expect(rule.body).toMatch(/border-color:\s*var\(--fire-hot\)/);
    expect(rule.body).toContain('box-shadow');
    expect(rule.body).not.toMatch(/display:\s*none/);
  });
});
