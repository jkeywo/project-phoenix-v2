import { test, expect } from '@playwright/test';
import { DEVICE_MATRIX, TEXT_SCALES, BROWSER_ZOOMS } from '../fixtures/device-matrix.mjs';

/**
 * tests/smoke/phone-viewscreen.spec.js — issue #1429 (PRD #1418 stories 18-21).
 *
 * The phone Viewscreen surface `docs/acceptance/1421-device-matrix.md`
 * recorded as "does not exist in the repository yet": server.html run on a
 * phone-shaped screen offers only text scale, contrast and Reduce effects on
 * its Display tab, and level-3 AI-to-AI Coordination chatter stays compact
 * there no matter the chosen text scale while a tap opens the same message's
 * full text at that scale in a dismissible reading surface that survives a
 * new message arriving.
 *
 * `tests/client/viewscreen-presentation-panel.test.js` and
 * `tests/client/phone-chatter-reader.test.js` prove the logic in jsdom. What
 * only a real engine can answer is whether the phone media query and the real
 * DOM/focus/touch mechanics actually behave this way at the two starting
 * regression viewports (`phone-390x844-portrait`, `phone-844x390-landscape`,
 * per PRD #1418 and issue #1429's own brief).
 *
 * ── Which bundle ──────────────────────────────────────────────────────────
 * `/` is the Trunk host bundle. Nothing below waits for WASM — the landing,
 * the cog, the Display tab and the chatter widget are markup plus `gui/*`
 * modules; chatter bubbles are pushed through `window.__updateChatter`, the
 * SAME classic-script entry point the Rust bridge calls, with literal
 * (non-String-Table) title/body text so assertions can match it exactly.
 */

const COG = '#server-settings-btn';
const OVERLAY = '#server-settings-overlay';
const TAB = (id) => `${OVERLAY} [data-tab="${id}"]`;
const CONTROL = (id) => `${OVERLAY} [data-control="${id}"]`;

const PHONE_VIEWPORTS = ['phone-390x844-portrait', 'phone-844x390-landscape'];

const device = (id) => {
  const found = DEVICE_MATRIX.find((d) => d.id === id);
  if (!found) throw new Error(`device-matrix.mjs has no row '${id}'`);
  return found;
};

/**
 * Land on server.html and step around the pre-game landing/scenario panels.
 *
 * Both are opaque, full-viewport and VISIBLE BY DEFAULT (`#landing-panel`
 * z-index 205 display:block, `#scenario-panel` z-index 200 display:flex,
 * gui/host-landing.css / gui/host-scenarios.css) — deliberately, so an
 * operator always lands on a choice rather than a blank screen. They sit
 * ABOVE `#chatter-container` (z-index 10), which is a permanently-present
 * child of `#server-shell` regardless of session phase. This spec is about
 * that widget's own presentation and reachability, not the landing/lobby
 * flow, so it hides the two panels directly rather than driving a scenario
 * pick and a lobby session to reach a widget that was there the whole time.
 * The settings cog itself needs no such help — `#server-settings-btn` is
 * z-index 210, already above both.
 */
async function openShell(page) {
  await page.goto('/');
  await expect(page.locator(COG)).toBeVisible({ timeout: 30_000 });
  await page.evaluate(() => {
    for (const id of ['landing-panel', 'scenario-panel']) {
      const el = document.getElementById(id);
      if (el) el.style.display = 'none';
    }
  });
}

async function openDisplayTab(page) {
  await openShell(page);
  await page.locator(COG).click();
  await expect(page.locator(OVERLAY)).toBeVisible();
  await page.locator(TAB('presentation')).click();
  await expect(page.locator(CONTROL('viewscreen-text-scale'))).toBeVisible();
}

/**
 * Drive the real slider FROM THE KEYBOARD, the same technique
 * tests/smoke/viewscreen-settings-presentation.spec.js uses and for the same
 * reason: keyboard reachability of the control is itself part of what this
 * spec has to prove, not an assumption `fill()` would paper over.
 */
async function chooseTextScale(page, scale) {
  const slider = page.locator(CONTROL('viewscreen-text-scale'));
  await slider.focus();
  const { min, step } = await slider.evaluate((el) => ({
    min: Number(el.min),
    step: Number(el.step),
  }));
  await page.keyboard.press('Home');
  const presses = Math.round((scale - min) / step);
  for (let i = 0; i < presses; i += 1) await page.keyboard.press('ArrowRight');
  await expect
    .poll(async () => page.evaluate(
      () => getComputedStyle(document.documentElement).getPropertyValue('--a11y-text-scale').trim(),
    ))
    .toBe(String(scale));
}

/** Push one chatter bubble through the real window.__updateChatter entry
 *  point with literal title/body text (not a String-Table id), so the
 *  assertions below can match it exactly without depending on strings.csv
 *  content — normalizeCoordinationPresentation passes unrecognised text
 *  through unchanged (tests/client/coordination-popup.test.js pins that). */
async function pushChatter(page, { from = 'Sensors', to = null, title, body }) {
  await page.evaluate(({ from, to, title, body }) => {
    window.__updateChatter(JSON.stringify({
      from_label: from,
      to_label: to,
      presentation: { title, title_params: {}, body, body_params: {} },
    }));
  }, { from, to, title, body });
}

for (const id of PHONE_VIEWPORTS) {
  const entry = device(id);

  test(`the Display tab on ${id} offers only text scale, contrast and Reduce effects`, async ({ browser }) => {
    const context = await browser.newContext({
      viewport: { width: entry.width, height: entry.height },
    });
    const page = await context.newPage();
    try {
      await openDisplayTab(page);

      for (const control of ['viewscreen-text-scale', 'viewscreen-contrast-default', 'viewscreen-reduce-effects']) {
        await expect(page.locator(CONTROL(control)), control).toBeVisible();
      }
      // The individual camera-shake/flash/decorative-motion rows — and the
      // "not offered here" sentences that exist only to explain an omitted
      // INDIVIDUAL control — are both absent. PRD #1418 Out of Scope:
      // "Detailed individual effect controls on the phone Viewscreen."
      for (const effect of ['shake', 'flash', 'decorative-motion']) {
        await expect(page.locator(CONTROL(`viewscreen-${effect}-full`)), `${effect} row`).toHaveCount(0);
        await expect(page.locator(CONTROL(`viewscreen-${effect}-absent`)), `${effect} absent-sentence`).toHaveCount(0);
      }
    } finally {
      await context.close();
    }
  });

  test(`level-3 Coordination stays compact on ${id} at every text scale while other text keeps enlarging`, async ({ browser }) => {
    const context = await browser.newContext({
      viewport: { width: entry.width, height: entry.height },
    });
    const page = await context.newPage();
    try {
      await openDisplayTab(page);

      let previousBubblePx = null;
      let previousHeadingPx = 0;
      for (const scale of TEXT_SCALES) {
        await chooseTextScale(page, scale);
        await pushChatter(page, { title: `Scale ${scale}`, body: 'Coordination body text.' });
        const where = `${id} @ ${scale * 100}%`;
        const sizes = await page.evaluate(() => ({
          bubble: parseFloat(getComputedStyle(document.querySelector('.chatter-bubble')).fontSize),
          heading: parseFloat(getComputedStyle(document.querySelector('.server-settings-heading')).fontSize),
        }));
        // The bubble PRD #1418 exempts from the 200% requirement: fixed size
        // across every scale on this build.
        if (previousBubblePx !== null) {
          expect(sizes.bubble, `bubble ${where}`).toBeCloseTo(previousBubblePx, 1);
        }
        // Everything else on this same page, including the very control that
        // set the scale, keeps enlarging normally — the exemption names one
        // selector, not a category (PRD Out of Scope).
        expect(sizes.heading, `other text ${where}`).toBeGreaterThan(previousHeadingPx);
        previousBubblePx = sizes.bubble;
        previousHeadingPx = sizes.heading;
      }
    } finally {
      await context.close();
    }
  });

  test(`tapping a compact bubble on ${id} opens its full text at the chosen scale and survives an arriving message`, async ({ browser }) => {
    const context = await browser.newContext({
      viewport: { width: entry.width, height: entry.height },
      hasTouch: true,
    });
    const page = await context.newPage();
    try {
      await openDisplayTab(page);
      await chooseTextScale(page, 2);
      await page.keyboard.press('Escape');
      await expect(page.locator(OVERLAY)).toBeHidden();

      await pushChatter(page, { title: 'First contact', body: 'Pinned message body.' });
      const bubble = page.locator('.chatter-bubble').first();
      await expect(bubble).toBeVisible();
      await bubble.tap();

      const reader = page.locator('#chatter-reader');
      await expect(reader).toBeVisible();
      await expect(page.locator('#chatter-reader-title')).toHaveText('First contact');
      await expect(page.locator('#chatter-reader-body')).toHaveText('Pinned message body.');

      // The reader reads at the operator's CHOSEN scale — unlike the bubble
      // it was opened from, which stayed at its fixed compact size.
      const readerBodyPx = await page.locator('#chatter-reader-body')
        .evaluate((el) => parseFloat(getComputedStyle(el).fontSize));
      const bubblePx = await bubble.evaluate((el) => parseFloat(getComputedStyle(el).fontSize));
      expect(readerBodyPx).toBeGreaterThan(bubblePx);

      // A second message arrives while the reader is open. The live stream
      // advancing does not move what is pinned on screen.
      await pushChatter(page, { title: 'Second contact', body: 'A newer message.' });
      await expect(page.locator('#chatter-reader-title')).toHaveText('First contact');
      await expect(page.locator('#chatter-reader-body')).toHaveText('Pinned message body.');

      // Escape dismisses it and hands focus back to the bubble that was tapped.
      await page.keyboard.press('Escape');
      await expect(reader).toBeHidden();
      await expect(bubble).toBeFocused();
    } finally {
      await context.close();
    }
  });

  test(`the reading surface on ${id} also dismisses by its close button and by a backdrop tap`, async ({ browser }) => {
    const context = await browser.newContext({
      viewport: { width: entry.width, height: entry.height },
      hasTouch: true,
    });
    const page = await context.newPage();
    try {
      await openShell(page);

      await pushChatter(page, { title: 'Close-button case', body: 'Body A.' });
      const bubbleA = page.locator('.chatter-bubble').first();
      await expect(bubbleA).toBeVisible();
      await bubbleA.tap();
      await expect(page.locator('#chatter-reader')).toBeVisible();
      await page.locator('#chatter-reader-close').click();
      await expect(page.locator('#chatter-reader')).toBeHidden();

      await pushChatter(page, { title: 'Backdrop case', body: 'Body B.' });
      const bubbleB = page.locator('.chatter-bubble').first();
      await expect(bubbleB).toBeVisible();
      await bubbleB.tap();
      await expect(page.locator('#chatter-reader')).toBeVisible();
      // The backdrop, not the panel: the top-left corner of the full-viewport
      // overlay, well outside `.chatter-reader-panel`.
      await page.locator('#chatter-reader').tap({ position: { x: 2, y: 2 } });
      await expect(page.locator('#chatter-reader')).toBeHidden();
    } finally {
      await context.close();
    }
  });
}

test('browser zoom is a separate lever from the phone Display tab, and the two stack', async ({ browser }) => {
  // Same claim tests/smoke/viewscreen-settings-presentation.spec.js makes for
  // the full Display tab, repeated here because the phone-limited control set
  // is a different DOM shape (no individual effect rows) and must not
  // silently lose reachability under the browser's own zoom.
  const entry = device('phone-390x844-portrait');
  for (const zoom of BROWSER_ZOOMS) {
    const context = await browser.newContext({
      viewport: {
        width: Math.round(entry.width / zoom),
        height: Math.round(entry.height / zoom),
      },
      deviceScaleFactor: zoom,
    });
    const page = await context.newPage();
    try {
      await openDisplayTab(page);
      const where = `browser zoom ${zoom * 100}%`;
      for (const control of ['viewscreen-text-scale', 'viewscreen-contrast-default', 'viewscreen-reduce-effects']) {
        await expect(page.locator(CONTROL(control)), `${where}: ${control}`).toBeVisible();
      }
      expect(
        await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth),
        `${where}: no sideways page scroll`,
      ).toBeLessThanOrEqual(1);

      // …and the in-app scale still stacks with it rather than one erasing
      // the other, exactly as the full Display tab's own spec proves.
      await chooseTextScale(page, 2);
      await expect(page.locator(CONTROL('viewscreen-text-scale')), `${where} + 200%`).toBeVisible();
    } finally {
      await context.close();
    }
  }
});

test('forced colours outrank the Phoenix contrast choice on a phone Viewscreen too', async ({ browser }) => {
  // PRD #1418: "A Phoenix contrast selection is not permission to defeat
  // browser-enforced colours." The phone-limited Display tab and the reading
  // surface both carry the same shared tokens (gui/tokens.css's
  // `forced-colors: active` block) as the full Display tab; nothing new was
  // introduced for either that could out-specify it — this proves the claim
  // holds on the reduced control set too, not only on the full one.
  const entry = device('phone-390x844-portrait');
  const context = await browser.newContext({
    viewport: { width: entry.width, height: entry.height },
    forcedColors: 'active',
    hasTouch: true,
  });
  const page = await context.newPage();
  try {
    await openDisplayTab(page);
    await page.locator(CONTROL('viewscreen-contrast-on')).click();
    await expect(page.locator('html')).toHaveAttribute('data-contrast', 'more');
    await expect(page.locator(CONTROL('viewscreen-contrast-on'))).toHaveAttribute('aria-pressed', 'true');
    await expect(page.locator(CONTROL('viewscreen-text-scale-status'))).not.toBeEmpty();

    // The reading surface, reached from the same forced-colours page.
    await page.keyboard.press('Escape');
    await expect(page.locator(OVERLAY)).toBeHidden();
    await pushChatter(page, { title: 'Forced colours case', body: 'Body under forced colours.' });
    const bubble = page.locator('.chatter-bubble').first();
    await expect(bubble).toBeVisible();
    await bubble.tap();
    await expect(page.locator('#chatter-reader')).toBeVisible();
    await expect(page.locator('#chatter-reader-close')).toBeVisible();
  } finally {
    await context.close();
  }
});
