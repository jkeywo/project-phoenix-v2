import { test, expect } from '@playwright/test';
import { DEVICE_MATRIX, TEXT_SCALES, BROWSER_ZOOMS } from '../fixtures/device-matrix.mjs';

/**
 * tests/smoke/viewscreen-settings-presentation.spec.js — issue #1427
 * (PRD #1418 stories 4, 9, 11, 12, 16, 17).
 *
 * The browser Viewscreen's own settings: an operator standing at the shared
 * screen opens the cog, enlarges the menus and turns contrast up, and the room
 * finds the screen that way again next week.
 *
 * `tests/client/viewscreen-presentation.test.js` proves the record, the store,
 * the isolation and both reset scopes in jsdom. What only a real engine can
 * answer is the half this file asserts:
 *
 *  * the `--text-*` ramp is `max(px, rem)`, so whether a chosen text size
 *    actually ENLARGES the menu — rather than being swallowed by a floor — is a
 *    layout question, not an arithmetic one;
 *  * whether the enlarged panel still fits, wraps and scrolls instead of
 *    clipping its own controls, at every viewport in the supported matrix;
 *  * whether `localStorage` really carries the choice across a page load;
 *  * whether browser zoom and this setting coexist, which is a separate
 *    verification PRD #1418 asks for by name.
 *
 * Viewports, scales and zoom steps come from `tests/fixtures/device-matrix.mjs`
 * (issue #1421), so this spec and the human acceptance kit exercise one matrix.
 *
 * ── Which bundle ──────────────────────────────────────────────────────────
 * `/` is the Trunk host bundle. Nothing below waits for WASM: the landing, the
 * cog and the settings overlay are markup plus `gui/*` modules, and the Display
 * tab reaches neither the simulation nor a `wasm_*` binding — which is itself
 * part of the claim, because a display's reading comfort must not depend on a
 * world having booted.
 */

const COG = '#server-settings-btn';
const OVERLAY = '#server-settings-overlay';
const POPUP = '.server-settings-popup';
const TAB = (id) => `${OVERLAY} [data-tab="${id}"]`;
const CONTROL = (id) => `${OVERLAY} [data-control="${id}"]`;

const SLIDER = CONTROL('viewscreen-text-scale');
const SLIDER_STATUS = CONTROL('viewscreen-text-scale-status');
const CONTRAST_MORE = CONTROL('viewscreen-contrast-on');
const CONTRAST_SYSTEM = CONTROL('viewscreen-contrast-default');
const CONTRAST_RESET = CONTROL('viewscreen-contrast-reset');
const RESET_ALL = CONTROL('viewscreen-reset-all');

/** Every control the Display tab must keep reachable at any size. */
const EVERY_CONTROL = [
  SLIDER,
  CONTROL('viewscreen-text-scale-reset'),
  CONTRAST_SYSTEM,
  CONTRAST_MORE,
  CONTROL('viewscreen-contrast-off'),
  CONTRAST_RESET,
  RESET_ALL,
];

// The viewscreen is a landscape surface by DESIGN — a television in a room —
// though no longer by contract: server.html's `data-phx-force-landscape` lock
// is gone, and a phone held upright now gets a portrait page rather than a
// rotated landscape one. The rows carried here are still the landscape ones PRD
// #1418 names as starting regression cases plus a landscape phone — a phone run
// as a secondary viewscreen, which the PRD supports without claiming it is a
// good room display.
const CARRIED_ON = ['tablet-1280x720-interim-landscape', 'phone-844x390-landscape'];

const device = (id) => {
  const found = DEVICE_MATRIX.find((d) => d.id === id);
  if (!found) throw new Error(`device-matrix.mjs has no row '${id}'`);
  return found;
};

/** Open the viewscreen and its settings cog on the Display tab. */
async function openDisplayTab(page) {
  await page.goto('/');
  await expect(page.locator(COG)).toBeVisible({ timeout: 30_000 });
  await page.locator(COG).click();
  await expect(page.locator(OVERLAY)).toBeVisible();
  await page.locator(TAB('presentation')).click();
  await expect(page.locator(SLIDER)).toBeVisible();
}

/**
 * Drive the real slider FROM THE KEYBOARD — Home to its floor, then one arrow
 * per authored step. Not `fill()`: keyboard reachability of every control is
 * itself one of the requirements, and a range input driven by arrow keys is the
 * only version of this interaction that proves it. It also exercises the live
 * preview the way an operator does, one `input` event at a time.
 */
async function chooseTextScale(page, scale) {
  const slider = page.locator(SLIDER);
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

/** What the enlarged panel actually looks like on screen. */
async function panelState(page) {
  return page.evaluate(({ popup, controls }) => {
    const root = document.documentElement;
    const box = document.querySelector(popup);
    const rect = box.getBoundingClientRect();
    const style = getComputedStyle(box);
    return {
      rootFontPx: parseFloat(getComputedStyle(root).fontSize),
      headingPx: parseFloat(getComputedStyle(box.querySelector('.server-settings-heading')).fontSize),
      // The HEADING is prose; these three are the things an operator has to
      // read and press to get back out again — a contrast option, the tab
      // label above it, and the percentage readout beside the slider they are
      // dragging. PRD #1418 story 4 is specifically about help and recovery
      // becoming the least readable parts of an enlarged screen, so each is
      // measured on its own rather than trusted to share the heading's rung.
      controlPx: parseFloat(getComputedStyle(
        box.querySelector('[data-control="viewscreen-contrast-on"]'),
      ).fontSize),
      tabPx: parseFloat(getComputedStyle(box.querySelector('.server-settings-tab')).fontSize),
      readoutPx: parseFloat(getComputedStyle(
        box.querySelector('[data-control="viewscreen-text-scale"]')
          .parentElement.querySelector('.server-settings-readout'),
      ).fontSize),
      // Wrap, stack and scroll — never a panel wider than the screen it is on.
      overflowRight: Math.max(0, rect.right - document.documentElement.clientWidth),
      pageOverflow: root.scrollWidth - root.clientWidth,
      scrolls: style.overflowY === 'auto' || style.overflowY === 'scroll',
      clipped: box.scrollHeight > box.clientHeight && !(
        style.overflowY === 'auto' || style.overflowY === 'scroll'
      ),
      // Every control still laid out, still on screen, still pressable.
      reachable: controls.every((selector) => {
        const el = document.querySelector(selector);
        if (!el || el.disabled) return false;
        const r = el.getBoundingClientRect();
        return r.width > 0 && r.height > 0
          && r.left >= rect.left - 1 && r.right <= rect.right + 1;
      }),
    };
  }, { popup: POPUP, controls: EVERY_CONTROL.map((s) => s.replace(`${OVERLAY} `, '')) });
}

for (const id of CARRIED_ON) {
  const entry = device(id);

  test(`the Viewscreen menu enlarges and stays operable on ${id}`, async ({ browser }) => {
    const context = await browser.newContext({
      viewport: { width: entry.width, height: entry.height },
    });
    const page = await context.newPage();
    try {
      await openDisplayTab(page);

      let previousRoot = 0;
      const previousText = { headingPx: 0, controlPx: 0, tabPx: 0, readoutPx: 0 };
      for (const scale of TEXT_SCALES) {
        await chooseTextScale(page, scale);
        const state = await panelState(page);
        const where = `${id} @ ${scale * 100}%`;

        // It ENLARGES. The `--text-*` rungs are `max(px, rem)`, so the only
        // honest proof that a chosen size reached the menu is that the rendered
        // text is bigger than it was one rung down — never smaller, which is
        // the "do not silently shrink text" half of the same requirement.
        expect(state.rootFontPx, where).toBeGreaterThan(previousRoot);
        previousRoot = state.rootFontPx;
        // Prose AND the controls: a rung pinned at the ramp's absolute floor
        // (`--text-min`, a bare 11px) would leave the buttons, the tab labels
        // and the readout exactly as they were while the headings around them
        // doubled, which is the failure this measurement exists to catch.
        for (const part of ['headingPx', 'controlPx', 'tabPx', 'readoutPx']) {
          expect(state[part], `${where}: ${part}`).toBeGreaterThan(previousText[part]);
          previousText[part] = state[part];
        }

        // …and it stays usable: bounded, scrolling, nothing pushed off the side
        // of a panel or of the page.
        expect(state.reachable, `${where}: every control reachable`).toBe(true);
        expect(state.scrolls, `${where}: the panel scrolls rather than clipping`).toBe(true);
        expect(state.clipped, where).toBe(false);
        expect(state.overflowRight, where).toBeLessThanOrEqual(1);
        expect(state.pageOverflow, `${where}: no sideways page scroll`).toBeLessThanOrEqual(1);
      }

      // Keyboard dismissal at the largest size, with focus handed back to the
      // control that opened the panel.
      await page.keyboard.press('Escape');
      await expect(page.locator(OVERLAY)).toBeHidden();
      expect(await page.evaluate(() => document.activeElement?.id)).toBe('server-settings-btn');
    } finally {
      await context.close();
    }
  });
}

test('the choice is this endpoint’s, and it survives a reload', async ({ browser }) => {
  const entry = device('tablet-1280x720-interim-landscape');
  const context = await browser.newContext({
    viewport: { width: entry.width, height: entry.height },
  });
  const page = await context.newPage();
  try {
    await openDisplayTab(page);
    await chooseTextScale(page, 2);
    await page.locator(CONTRAST_MORE).click();
    await expect(page.locator('html')).toHaveAttribute('data-contrast', 'more');

    // Contrast is not a class the panel paints on itself: it re-points the
    // palette every surface on this page reads.
    const contrasted = await page.evaluate(
      () => getComputedStyle(document.documentElement).getPropertyValue('--ink').trim(),
    );

    // A RELOAD is the next session on this machine. Nothing here re-opens the
    // menu first: the setting has to be in force on the page it comes back to.
    await page.reload();
    await expect(page.locator(COG)).toBeVisible({ timeout: 30_000 });
    await expect
      .poll(async () => page.evaluate(
        () => getComputedStyle(document.documentElement).getPropertyValue('--a11y-text-scale').trim(),
      ))
      .toBe('2');
    await expect(page.locator('html')).toHaveAttribute('data-contrast', 'more');
    expect(await page.evaluate(
      () => getComputedStyle(document.documentElement).getPropertyValue('--ink').trim(),
    )).toBe(contrasted);

    // It is stored under the ENDPOINT's own key and nowhere else — in
    // particular not in the private operator profile a console keeps here.
    const stored = await page.evaluate(() => ({
      viewscreen: localStorage.getItem('phoenix-viewscreen-presentation-v1'),
      operator: localStorage.getItem('phoenix-operator-profile-v1'),
    }));
    expect(stored.viewscreen).toContain('"textScale":2');
    expect(stored.operator ?? '').not.toContain('viewscreen');

    // Per-setting reset, then Reset all — each scoped to what it names.
    await page.locator(COG).click();
    await page.locator(TAB('presentation')).click();
    await page.locator(CONTRAST_RESET).click();
    await expect(page.locator('html')).toHaveAttribute('data-contrast', 'standard');
    await expect
      .poll(async () => page.evaluate(
        () => getComputedStyle(document.documentElement).getPropertyValue('--a11y-text-scale').trim(),
      ))
      .toBe('2');

    await page.locator(RESET_ALL).click();
    await expect
      .poll(async () => page.evaluate(
        () => getComputedStyle(document.documentElement).getPropertyValue('--a11y-text-scale').trim(),
      ))
      .toBe('1');
    // A scoped reset does not reach the rest of this endpoint's data.
    expect(await page.evaluate(() => localStorage.getItem('phoenix-operator-profile-v1')))
      .toBe(stored.operator);
  } finally {
    await context.close();
  }
});

test('browser zoom is a separate lever, and the two are additive', async ({ browser }) => {
  const entry = device('tablet-1280x720-interim-landscape');
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
      // Phoenix's own setting stays at its default: the point is that the
      // browser's zoom works on its own, with Phoenix compensating for nothing.
      const plain = await panelState(page);
      expect(plain.reachable, where).toBe(true);
      expect(plain.pageOverflow, where).toBeLessThanOrEqual(1);

      // …and the two stack rather than one cancelling the other.
      await chooseTextScale(page, 2);
      const enlarged = await panelState(page);
      expect(enlarged.rootFontPx, `${where} + 200%`).toBeGreaterThan(plain.rootFontPx);
      expect(enlarged.reachable, `${where} + 200%`).toBe(true);
      expect(enlarged.pageOverflow, `${where} + 200%`).toBeLessThanOrEqual(1);
    } finally {
      await context.close();
    }
  }
});

test('a forced-colour palette outranks the Phoenix contrast choice', async ({ browser }) => {
  // PRD #1418: "A Phoenix contrast selection is not permission to defeat
  // browser-enforced colours." The Display tab's own status line and its
  // pressed option must still be legible when the browser is drawing the page.
  const entry = device('tablet-1280x720-interim-landscape');
  const context = await browser.newContext({
    viewport: { width: entry.width, height: entry.height },
    forcedColors: 'active',
  });
  const page = await context.newPage();
  try {
    await openDisplayTab(page);
    await page.locator(CONTRAST_MORE).click();
    await expect(page.locator('html')).toHaveAttribute('data-contrast', 'more');
    // The selection survives without depending on the colour Phoenix wanted:
    // `aria-pressed` carries it for a reader, and the option still has a box.
    await expect(page.locator(CONTRAST_MORE)).toHaveAttribute('aria-pressed', 'true');
    await expect(page.locator(CONTRAST_SYSTEM)).toHaveAttribute('aria-pressed', 'false');
    const state = await panelState(page);
    expect(state.reachable).toBe(true);
    // The status line is words, not a colour: it says what is in force.
    await expect(page.locator(SLIDER_STATUS)).not.toBeEmpty();
  } finally {
    await context.close();
  }
});
