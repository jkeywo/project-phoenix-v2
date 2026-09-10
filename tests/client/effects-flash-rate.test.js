/**
 * tests/client/effects-flash-rate.test.js — issue #1431 (PRD #1418 stories
 * 5/6/13/14/15/19/33), the cheap automated half of the acceptance kit at
 * `docs/acceptance/1431-effects-readability.md`.
 *
 * This is NOT a flash-safety conformance check. The W3C "Three Flashes or
 * Below Threshold" method judges a flash on THREE things at once: how often
 * it repeats in any one-second window, how much of the viewing area it
 * covers, and whether the transition is a saturated-red opponent pair. Area
 * and red-saturation depend on the viewer's actual screen size and seating
 * distance — issue #1421's `docs/acceptance/1421-device-matrix.md` room-
 * distance field — which no source-reading test can know. What a source
 * read CAN answer honestly, without a GPU or a human in the room, is the
 * REPEAT-RATE third of the method: this repository's two looping flash
 * consumers (`#hud-vignette` on the shared Viewscreen, `#phone-bezel` on the
 * console) each pulse at one authored period, scaled by the operator's own
 * flash-intensity choice (`gui/visual-effects.js` `EFFECT_REDUCED.flash`).
 * That rate is exactly the number this file computes and checks against the
 * method's "no more than three flashes in any one-second period" figure, used
 * here as engineering guidance rather than a medical or full-conformance
 * verdict — the framing issue #1431 asks for.
 *
 * If the authored keyframe shape ever changes from one peak-trough cycle per
 * period to something with more flashes per loop, the shape guard below
 * fails FIRST, so this file's rate math is never silently wrong about how
 * many flashes one animation cycle actually contains.
 *
 * Reads real source text — same `read()`-from-disk pattern as
 * `tests/client/visual-effects.test.js`, which already pins the two
 * `--a11y-flash-period` declarations this file also reads. This file adds
 * the arithmetic and the pass/fail line; it does not re-derive the
 * declarations from nothing.
 */

import { describe, it, expect } from 'vitest';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import { EFFECT_FULL, EFFECT_REDUCED } from '../../gui/visual-effects.js';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const read = (rel) => fs.readFileSync(path.join(HERE, '../../', rel), 'utf-8');

/** Pull `--a11y-flash-period: <n>s;` out of a stylesheet-bearing source file.
 *  Throws (failing the test with a clear message) rather than returning
 *  `NaN` if the declaration has moved or been reworded. */
function flashPeriodSeconds(source, label) {
  const match = source.match(/--a11y-flash-period:\s*([\d.]+)s;/);
  if (!match) {
    throw new Error(`${label}: no --a11y-flash-period declaration found — has issue #1428's flash wiring moved?`);
  }
  return Number(match[1]);
}

/** The single-peak shape every consumer's `@keyframes` is checked against:
 *  exactly one 0%→peak→100% cycle, i.e. exactly one opposing luminance pair
 *  per authored period. Three percentage stops (0%, 50%, 100%) is that
 *  shape; any other stop count means the loop no longer flashes once per
 *  period and this file's rate arithmetic (flashes = intensity / period)
 *  would silently under- or over-count. */
function assertSinglePeakShape(source, keyframeName, label) {
  const block = source.match(new RegExp(`@keyframes\\s+${keyframeName}\\s*\\{([\\s\\S]*?)\\n\\s*\\}`));
  expect(block, `${label}: @keyframes ${keyframeName} block found`).not.toBeNull();
  const stops = block[1].match(/(\d+(?:\.\d+)?)%/g) || [];
  const uniqueStops = [...new Set(stops)];
  expect(
    uniqueStops.length,
    `${label}: @keyframes ${keyframeName} has a single peak-trough cycle (0%/50%/100%) — ` +
      `found stops ${JSON.stringify(uniqueStops)}. If this changed on purpose, the flashes-per-cycle ` +
      `assumption in this file's rate math must be re-derived, not just re-numbered.`,
  ).toBe(3);
}

/** Flashes per second at a given resolved effect intensity, for a consumer
 *  whose loop carries exactly one flash per `periodSeconds` at intensity 1
 *  (full). The resolved animation-duration is `period / intensity`
 *  (`gui/tokens.css`'s own formula, reasserted here as arithmetic rather
 *  than re-read from the `calc()` text, since vitest cannot evaluate CSS
 *  `calc()`); one flash per that duration is one flash per
 *  `period / intensity` seconds, i.e. `intensity / period` flashes/second. */
function flashesPerSecond(periodSeconds, intensity) {
  return intensity / periodSeconds;
}

// W3C "Three Flashes or Below Threshold" numeric figure, used here as the
// engineering guidance PRD #1418 and issue #1431 ask for — not a medical
// threshold and not, on its own, full conformance (area and red-saturation
// are the other two-thirds of that method and are NOT computed here; see
// docs/acceptance/1431-effects-readability.md §2 for why those need a human
// with the actual screen and seating distance).
const MAX_GUIDANCE_FLASHES_PER_SECOND = 3;

describe('flash repeat-rate — the source-derivable third of the W3C method (#1431)', () => {
  const SERVER_HTML = read('server.html');
  const CLIENT_HTML = read('client.html');
  const HUD_HTML = read('gui/viewscreen-hud.html');

  const consumers = [
    {
      label: 'Shared Viewscreen — #hud-vignette (server.html, browser)',
      source: SERVER_HTML,
      keyframe: 'hud-pulse',
    },
    {
      label: 'Console — #phone-bezel (client.html)',
      source: CLIENT_HTML,
      keyframe: 'bezel-pulse',
    },
    {
      label: 'Shared Viewscreen — native HUD overlay document (gui/viewscreen-hud.html)',
      source: HUD_HTML,
      keyframe: 'hud-pulse',
    },
  ];

  it.each(consumers)('$label: single flash per authored period', ({ source, keyframe, label }) => {
    assertSinglePeakShape(source, keyframe, label);
  });

  it.each(consumers)(
    '$label: rate stays within the guidance figure at Full and at the Reduce-effects stop',
    ({ source, label }) => {
      const period = flashPeriodSeconds(source, label);
      expect(period, `${label}: authored period is a positive number of seconds`).toBeGreaterThan(0);

      const atFull = flashesPerSecond(period, EFFECT_FULL);
      const atReduced = flashesPerSecond(period, EFFECT_REDUCED.flash);

      // eslint-disable-next-line no-console
      console.log(
        `[#1431 flash-rate] ${label}: period=${period}s → ` +
          `Full=${atFull.toFixed(3)}/s, Gentle=${atReduced.toFixed(3)}/s ` +
          `(guidance ceiling ${MAX_GUIDANCE_FLASHES_PER_SECOND}/s)`,
      );

      expect(
        atFull,
        `${label}: at Full (intensity 1) the authored loop must not exceed ` +
          `${MAX_GUIDANCE_FLASHES_PER_SECOND} flashes/second by rate alone. A failure here is a ` +
          `real finding for docs/acceptance/1431-effects-readability.md, not a false positive to ` +
          `silence — the human pass still judges area and red-saturation on top of this.`,
      ).toBeLessThanOrEqual(MAX_GUIDANCE_FLASHES_PER_SECOND);

      expect(atReduced, `${label}: the Gentle stop is strictly slower than Full`).toBeLessThan(atFull);
      expect(atReduced, `${label}: Gentle also stays within the guidance figure`).toBeLessThanOrEqual(
        MAX_GUIDANCE_FLASHES_PER_SECOND,
      );
    },
  );

  it('the shared Viewscreen vignette and the native HUD overlay stay pinned to the same period', () => {
    // Issue #1428's own note (docs/acceptance/1428-visual-effects.md §1a): the
    // native HUD overlay is a separate document that receives its effect
    // intensities over a host channel rather than this endpoint's settings
    // script. If a future edit changes one file's authored period without the
    // other, the two runtimes would flash at different rates for what the
    // operator believes is one Display-tab choice — this is the drift guard.
    const serverPeriod = flashPeriodSeconds(SERVER_HTML, 'server.html');
    const hudPeriod = flashPeriodSeconds(HUD_HTML, 'gui/viewscreen-hud.html');
    expect(hudPeriod, 'native HUD overlay period matches the browser Viewscreen vignette period').toBe(
      serverPeriod,
    );
  });
});
