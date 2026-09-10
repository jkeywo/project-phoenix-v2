import { test, expect, waitForWasmReady } from './fixtures';
import fs from 'node:fs';
import path from 'node:path';
import { DEVICE_MATRIX, TEXT_SCALES } from '../fixtures/device-matrix.mjs';

// The GM console's smallest supported landscape surface and the top of the
// enlargement range, both taken from #1421's shared matrix rather than
// restated here (PRD #1418 acceptance-matrix decision).
const GM_VIEWPORT = DEVICE_MATRIX.find((row) => row.id === 'desktop-1280x720-gm');
const MAX_TEXT_SCALE = Math.max(...TEXT_SCALES);

// Named GM checkpoints at the GM console's smallest supported landscape
// viewport with text at 200% (issues #1445 / #1418).
//
// A REAL bookmark, through the real save machinery: the panel asks for the
// ordinary manual capture, the browser Store keeps it, and the capture tick in
// the confirmation comes back out of the catalogue rather than out of the
// request. That is the whole honesty rule of this feature, and it is worth
// proving in a browser and not only in jsdom, because the confirmation only
// arrives after a fixed-tick boundary the simulation actually reaches.
//
// The scale is set on the document root directly, exactly as the presentation
// profile does (`accessibility-profile.js` TEXT_SCALE_VAR); exposing 200% in
// the Settings slider is #1422's deliverable, and this spec proves the
// CHECKPOINT panel survives the value, not how an operator picks it.
test('GM named checkpoints stay readable and operable at 200% text on 1280x720',
  { tag: '@core' },
  async ({ context }, testInfo) => {
    test.setTimeout(120000);
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

    const panel = page.locator('#gm-checkpoint');
    await expect(panel).toBeVisible();

    // Touch/pointer reach at this size, before anything is typed.
    const bookmarkButton = page.locator('#gm-checkpoint-bookmark');
    await expect(bookmarkButton).toBeEnabled();
    expect((await bookmarkButton.boundingBox()).height).toBeGreaterThanOrEqual(44);

    const name = 'Before the ambush';
    await page.locator('#gm-checkpoint-name').fill(name);
    await bookmarkButton.click();

    // Routine attention does not interrupt: a bookmark settles in the panel's
    // own status line, and no confirmation dialog opens over the console.
    await expect(page.locator('#gm-action-confirmation')).toBeHidden();

    await page.waitForFunction(() => {
      const state = window.__hostGmCheckpointState?.();
      return state && !state.pending && state.statusTone !== '';
    }, null, { timeout: 60_000 });
    const settled = await page.evaluate(() => window.__hostGmCheckpointState());
    expect(settled.statusTone).toBe('ok');
    // The confirmation names the checkpoint and a tick it read back, and the
    // row it names really is selectable afterwards.
    expect(settled.statusText).toContain(name);
    const confirmed = settled.rows.find((row) => row.displayName === name);
    expect(confirmed).toBeTruthy();
    expect(Number(confirmed.captureTick)).toBeGreaterThan(0);
    expect(settled.statusText).toContain(String(confirmed.captureTick));

    // Panels wrap, stack and scroll; they never scroll sideways or shrink text.
    const geometry = await panel.evaluate(el => ({
      clientWidth: el.clientWidth,
      scrollWidth: el.scrollWidth,
      fontSize: parseFloat(getComputedStyle(el).fontSize),
    }));
    expect(geometry.scrollWidth).toBeLessThanOrEqual(geometry.clientWidth + 1);
    expect(geometry.fontSize).toBeGreaterThanOrEqual(14 * MAX_TEXT_SCALE);

    const row = page.locator(`#gm-checkpoint-list .gm-checkpoint-row[data-checkpoint-slot-id="${confirmed.slotId}"]`);
    await expect(row).toBeVisible();
    const box = await row.boundingBox();
    expect(box.height).toBeGreaterThanOrEqual(44);
    // Non-colour status: the verdict is a word on the row, not only the
    // `data-eligible` attribute and the border colour keyed off it.
    const verdict = await row.locator('.gm-checkpoint-verdict').textContent();
    expect(verdict?.trim().length).toBeGreaterThan(0);
    expect(await row.getAttribute('data-eligible')).toBe('true');

    // The candidate's own compatibility reasons remain reachable at this size.
    await row.click();
    await expect(page.locator('#gm-checkpoint-detail')).toBeVisible();
    expect(await page.locator('#gm-checkpoint-detail-preflight')
      .getAttribute('data-eligible')).toBe('true');

    // Keyboard reach, with focus visibly on the row the operator moved to.
    await row.focus();
    await page.keyboard.press('End');
    expect(await page.evaluate(() => document.activeElement?.className))
      .toContain('gm-checkpoint-row');

    const screenshot = testInfo.outputPath('gm-checkpoint-200.png');
    await page.screenshot({ path: screenshot, fullPage: false });
    await testInfo.attach('GM checkpoints 1280×720 @200%', { path: screenshot, contentType: 'image/png' });
    expect(errors).toEqual([]);
  });
