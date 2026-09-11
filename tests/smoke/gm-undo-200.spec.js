import { test, expect, waitForWasmReady } from './fixtures';
import fs from 'node:fs';
import path from 'node:path';
import { DEVICE_MATRIX, TEXT_SCALES } from '../fixtures/device-matrix.mjs';

// The GM console's smallest supported landscape surface and the top of the
// enlargement range, both from #1421's shared matrix (PRD #1418).
const GM_VIEWPORT = DEVICE_MATRIX.find((row) => row.id === 'desktop-1280x720-gm');
const MAX_TEXT_SCALE = Math.max(...TEXT_SCALES);

// A real faction change and a real inverse of it, at 200% text on the smallest
// supported GM viewport (issues #1442 / #1418).
//
// Everything is driven through the ordinary controls: the faction select, the
// configurable confirmation modal, the journal row and its Undo button. The
// scale is set on the console root directly, which is what the presentation
// profile does; exposing 200% in the Settings slider is #1422's deliverable.
test('GM faction change and its undo stay readable and operable at 200% text on 1280x720',
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

    // The control names only authored factions the world actually loaded.
    const panel = page.locator('#gm-faction-panel');
    await expect(panel).toBeVisible();
    await page.waitForFunction(
      () => (window.__hostGmFactionState?.().factions.length || 0) >= 2);
    await page.selectOption('#gm-faction-source', 'Alliance');
    await page.selectOption('#gm-faction-enemy', 'Harrow');
    await expect(page.locator('#gm-faction-relation')).toHaveAttribute('data-hostile', 'false');

    // Panels wrap, stack and scroll; they never scroll sideways or shrink text.
    const geometry = await panel.evaluate(el => ({
      clientWidth: el.clientWidth,
      scrollWidth: el.scrollWidth,
      fontSize: parseFloat(getComputedStyle(el).fontSize),
    }));
    expect(geometry.scrollWidth).toBeLessThanOrEqual(geometry.clientWidth + 1);
    expect(geometry.fontSize).toBeGreaterThanOrEqual(14 * MAX_TEXT_SCALE);
    expect((await page.locator('#gm-faction-apply').boundingBox()).height)
      .toBeGreaterThanOrEqual(44);

    // `faction.relation` defaults to a plain confirmation.
    await page.locator('#gm-faction-apply').click();
    const dialog = page.locator('#gm-action-confirmation');
    await expect(dialog).toBeVisible();
    await dialog.locator('[data-confirmation-accept]').click();
    await page.waitForFunction(() => (window.__hostGmJournalState?.().entries || [])
      .some((entry) => entry.action_kind === 'faction-relation' && entry.outcome === 'applied'));
    await expect(page.locator('#gm-faction-relation')).toHaveAttribute('data-hostile', 'true');

    // The journal row for it offers an Undo, with the before/after pair spelled
    // out and the "already witnessed" sentence that is never suppressible.
    // The action log shares the centre region behind one tab strip (the
    // post-M5 screen); bring it to the front the way an operator does.
    await page.locator('#gm-log-tab-journal').click();
    const row = page.locator('.gm-journal-row[data-outcome="applied"]').last();
    await row.click();
    const eligibility = page.locator('#gm-journal-inverse .gm-inverse-eligibility');
    await expect(eligibility).toHaveAttribute('data-supported', 'true');
    const undo = page.locator('#gm-journal-undo');
    await expect(undo).toBeVisible();
    expect((await undo.boundingBox()).height).toBeGreaterThanOrEqual(44);

    // `action.undo` defaults to the preview mode, whose text distinguishes the
    // technical change from what crews already saw.
    await undo.click();
    await expect(dialog).toBeVisible();
    expect(await dialog.getAttribute('data-mode')).toBe('confirm-preview');
    const preview = await dialog.locator('[data-confirmation-preview]').textContent();
    expect(preview?.length).toBeGreaterThan(0);
    await dialog.locator('[data-confirmation-accept]').click();

    await page.waitForFunction(() => (window.__hostGmJournalState?.().entries || [])
      .some((entry) => entry.action_kind === 'action-undo' && entry.outcome === 'applied'));
    await expect(page.locator('#gm-faction-relation')).toHaveAttribute('data-hostile', 'false');
    // Both operators are on the one saved history, and the original now says it
    // has been reversed rather than disappearing from it.
    const history = await page.evaluate(() => window.__hostGmJournalState().entries);
    const inverse = history.find((entry) => entry.action_kind === 'action-undo');
    expect(inverse.undo_of.operator_id.length).toBeGreaterThan(0);
    expect(history.find((entry) => entry.correlation === inverse.undo_of.correlation).inverted)
      .toBe(true);

    // Keyboard reach on the history, with focus visibly where it moved to.
    await row.focus();
    await page.keyboard.press('ArrowUp');
    expect(await page.evaluate(() => document.activeElement?.className)).toContain('gm-journal-row');

    const screenshot = testInfo.outputPath('gm-undo-200.png');
    await page.screenshot({ path: screenshot, fullPage: false });
    await testInfo.attach('GM undo 1280×720 @200%', { path: screenshot, contentType: 'image/png' });
    expect(errors).toEqual([]);
  });
