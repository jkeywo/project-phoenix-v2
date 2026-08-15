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

/** The contents of the `prefers-reduced-motion: reduce` media block. */
function reducedMotionBlock(source) {
  const index = source.indexOf('@media (prefers-reduced-motion: reduce)');
  if (index === -1) return null;
  // Balance braces from the media query's own opening brace.
  const open = source.indexOf('{', index);
  let depth = 0;
  for (let i = open; i < source.length; i += 1) {
    if (source[i] === '{') depth += 1;
    else if (source[i] === '}') {
      depth -= 1;
      if (depth === 0) return source.slice(open + 1, i);
    }
  }
  return null;
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
    const block = reducedMotionBlock(CLIENT_HTML);
    expect(block).not.toBeNull();
    expect(block).toContain('#phone-bezel.alert-on');
    expect(block).toMatch(/animation:\s*none/);
  });

  it('stills the pulse without hiding the alert', () => {
    // The accessibility answer is to stop the motion, not to drop the
    // information: a player on reduced motion must still be able to tell that
    // the ship is at red alert, so the bezel holds at the pulse's peak.
    const block = reducedMotionBlock(CLIENT_HTML);
    expect(block).toMatch(/border-color:\s*#ff5a44/);
    expect(block).toContain('box-shadow');
    expect(block).not.toMatch(/display:\s*none/);
  });
});
