import { test, expect, waitForWasmReady } from './fixtures';
import fs from 'node:fs';
import path from 'node:path';
import { DEVICE_MATRIX, TEXT_SCALES } from '../fixtures/device-matrix.mjs';

// The GM console's smallest supported landscape surface and the top of the
// enlargement range, both taken from #1421's shared matrix rather than
// restated here (PRD #1418 acceptance-matrix decision).
const GM_VIEWPORT = DEVICE_MATRIX.find((row) => row.id === 'desktop-1280x720-gm');
const MAX_TEXT_SCALE = Math.max(...TEXT_SCALES);

// The saved action history at the GM console's smallest supported landscape
// viewport with text at 200% (issues #1441 / #1418).
//
// The scale is set on the console root directly, which is exactly what the
// presentation profile does (`accessibility-profile.js` TEXT_SCALE_VAR).
// Exposing 200% in the Settings slider itself is #1422's deliverable; this spec
// proves the JOURNAL survives the value, not how an operator picks it.
test('GM saved action history stays readable and operable at 200% text on 1280x720',
  async ({ context }, testInfo) => {
    test.setTimeout(90000);
    const world = fs.readFileSync(
      path.resolve(__dirname, '../fixtures/worlds/gm_npc_doctrine.toml'), 'utf8');
    await context.route('**/assets/worlds/default.toml',
      route => route.fulfill({ contentType: 'text/plain', body: world }));
    const page = await context.newPage();
    const errors = [];
    page.on('pageerror', error => errors.push(error.message));
    await page.setViewportSize({ width: GM_VIEWPORT.width, height: GM_VIEWPORT.height });
    await page.goto('/?gm=1&scenario=assets/worlds/default.toml');
    await waitForWasmReady(page);
    await page.evaluate(() => window.__hostFleetOpen());
    await page.waitForFunction(() => window.__hostGmStartState?.().localValidation);
    await page.evaluate(() => document.getElementById('gm-ready-btn').click());
    await page.waitForFunction(() => window.__saveSlotsPhase === 'InProgress');

    await page.evaluate((scale) => document.documentElement.style
      .setProperty('--a11y-text-scale', String(scale)), MAX_TEXT_SCALE);

    // A real canonical GM action, through the ordinary Pause control.
    await page.locator('#gm-session-pause').click();
    await page.waitForFunction(() => (window.__hostGmJournalState?.().entries.length || 0) > 0);

    const journal = page.locator('#gm-journal');
    await expect(journal).toBeVisible();
    // Panels wrap, stack and scroll; they never scroll sideways or shrink text.
    const geometry = await journal.evaluate(el => ({
      clientWidth: el.clientWidth,
      scrollWidth: el.scrollWidth,
      fontSize: parseFloat(getComputedStyle(el).fontSize),
    }));
    expect(geometry.scrollWidth).toBeLessThanOrEqual(geometry.clientWidth + 1);
    expect(geometry.fontSize).toBeGreaterThanOrEqual(14 * MAX_TEXT_SCALE);

    const row = journal.locator('.gm-journal-row').first();
    await expect(row).toBeVisible();
    const outcome = await row.locator('.gm-journal-outcome').textContent();
    expect(outcome?.trim().length).toBeGreaterThan(0);
    const box = await row.boundingBox();
    expect(box.height).toBeGreaterThanOrEqual(44);

    // The detail and its undo explanation remain reachable at this size.
    await row.click();
    await expect(page.locator('#gm-journal-detail')).toBeVisible();
    await expect(page.locator('#gm-journal-inverse .gm-inverse-eligibility')).toBeVisible();
    expect(await page.locator('#gm-journal-inverse .gm-inverse-eligibility')
      .getAttribute('data-supported')).toBe('false');

    // Keyboard reach, with focus visibly on the row the operator moved to.
    await row.focus();
    await page.keyboard.press('ArrowDown');
    expect(await page.evaluate(() => document.activeElement?.className)).toContain('gm-journal-row');

    const screenshot = testInfo.outputPath('gm-journal-200.png');
    await page.screenshot({ path: screenshot, fullPage: false });
    await testInfo.attach('GM journal 1280×720 @200%', { path: screenshot, contentType: 'image/png' });
    expect(errors).toEqual([]);
  });
