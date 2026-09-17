import { test, expect, waitForWasmReady, captureServerPageErrors } from './fixtures';
import { ts } from './strings';
import { revealGmPanel } from './dock-helpers.js';

/**
 * Directed spatial picking on the docked Live map (issue #1508).
 *
 * The three things this pins that no unit test can: that pick mode reaches a
 * REAL chart through the real dock, that the surface puts its other floating
 * panels away and brings them back, and that a click with no meaningful drag
 * places facing the contextual default the panel had been previewing — in words
 * as well as degrees, so a forced-colours browser and a screen reader get it
 * too.
 */
test('a GM picks a place and a direction on the docked map', { tag: '@core' }, async ({ page }) => {
  test.setTimeout(180_000);
  const errors = captureServerPageErrors(page);
  await page.goto('/');
  await page.locator('#landing-menu [data-landing-entry="host_gm"]').click();
  const worlds = page.locator('#world-list .world-btn[data-scenario-id]');
  await expect(worlds.first()).toBeVisible({ timeout: 60_000 });
  await worlds.first().click();
  const ship = page.locator('ph-ship-picker .ship-card').first();
  await Promise.race([ship.waitFor({ state: 'visible', timeout: 60_000 }),
    page.locator('#landing-panel').waitFor({ state: 'hidden', timeout: 60_000 })]);
  if (await ship.isVisible()) await ship.click();
  await waitForWasmReady(page);
  await page.waitForFunction(() => !!window.__hostLocalGm?.());
  await page.locator('#gm-session-start').click();
  await page.locator('#gm-action-confirmation [data-confirmation-accept]').click();
  await page.waitForFunction(() => window.__saveSlotsPhase === 'InProgress');

  // Spawn is a temporary action panel: it opens as a floating draft.
  await revealGmPanel(page, 'spawn');
  const spawn = page.locator('[data-panel="spawn"]');
  await expect(spawn).toHaveClass(/is-floating/);
  const row = page.locator('#gm-spawn-palette .gm-spawn-entry').first();
  await expect(row).toBeVisible({ timeout: 30_000 });

  // A second floating panel, to watch it go away and come back.
  await page.locator('#gm-live-layout [data-panel="journal"] [data-layout-control="float"]').click();
  const journal = page.locator('[data-panel="journal"].is-floating');
  await expect(journal).toBeVisible();

  await row.locator('button[data-role="place"]').click();

  // Pick mode names the control capturing the map, and the chart says it is armed.
  const pick = page.locator('#gm-spawn-pick');
  await expect(pick).toHaveAttribute('data-picking', /.+/);
  await expect(page.locator('#gm-entity-map')).toHaveAttribute('data-placement-armed', '');
  // Every OTHER floating panel is away; the draft that is picking stays, because
  // it is the only thing showing the preview.
  await expect(journal).toBeHidden();
  await expect(spawn).toBeVisible();
  await expect(page.locator('#gm-map-panel')).toBeVisible();

  // Escape changes nothing about the draft and hands the floats back.
  await page.keyboard.press('Escape');
  await expect(page.locator('#gm-entity-map')).not.toHaveAttribute('data-placement-armed', '');
  await expect(journal).toBeVisible();
  await expect(pick).toHaveText('');

  // Pick again, and commit with a click and no meaningful drag: the placement
  // takes the contextual default direction the preview was showing.
  await row.locator('button[data-role="place"]').click();
  const box = await page.locator('#gm-entity-map').boundingBox();
  await page.mouse.move(box.x + box.width * 0.7, box.y + box.height * 0.4);
  await page.mouse.down();
  await expect(pick).toHaveAttribute('data-contextual', 'true');
  const previewed = await pick.textContent();
  // Degrees AND a compass word, so the direction survives forced colours.
  expect(previewed).toMatch(/\d+/);
  expect(previewed).toContain(ts('server.gm.spawn.pick_preview_default').split('{')[0].trim());
  await page.mouse.up();

  await expect(page.locator('#gm-spawn-log [data-outcome="applied"]')).toHaveCount(1, { timeout: 30_000 });
  await expect(journal).toBeVisible();
  await expect(pick).toHaveText('');
  expect(errors).toEqual([]);
});
