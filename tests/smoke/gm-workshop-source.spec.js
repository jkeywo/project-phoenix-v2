import { test, expect, waitForWasmReady, captureServerPageErrors } from './fixtures';
import { workshopPack, WORKSHOP_WORLD, WORKSHOP_WORLD_TEXT } from '../fixtures/workshop-pack.js';
import { ts } from './strings';
import { revealGmPanel } from './dock-helpers.js';

// Requires the integrated host WASM: the DOM storage tests in workshop-handoff
// separately pin the one-use transfer and immutable source consumer.
test('a running GM opens retained authored source through the Workshop control', { tag: '@core' }, async ({ page }) => {
  test.setTimeout(180_000);
  const errors = captureServerPageErrors(page);
  await page.goto('/');
  await page.locator('#landing-menu [data-landing-entry="host_gm"]').click();
  const worlds = page.locator('#world-list .world-btn[data-scenario-id]');
  await expect(worlds.first()).toBeVisible({ timeout: 60_000 });
  await page.locator('#mod-pack-file').setInputFiles({ name: 'retained-source.zip', mimeType: 'application/zip', buffer: Buffer.from(workshopPack()) });
  await expect(page.locator('#mod-pack-status')).toContainText(ts('server.mod_pack_applied'));
  await page.locator('#world-list .world-btn[data-scenario-id]:not([data-scenario-id="workshop-test"])').first().click();
  const ship = page.locator('ph-ship-picker .ship-card').first();
  await Promise.race([ship.waitFor({ state: 'visible', timeout: 60_000 }),
    page.locator('#landing-panel').waitFor({ state: 'hidden', timeout: 60_000 })]);
  if (await ship.isVisible()) await ship.click();
  await waitForWasmReady(page);
  await page.waitForFunction(() => !!window.__hostLocalGm?.());
  await revealGmPanel(page, 'readiness');
  await page.locator('#gm-force-start-btn').click();
  await page.locator('#gm-action-confirmation [data-confirmation-accept]').click();
  await page.waitForFunction(() => window.__saveSlotsPhase === 'InProgress');
  // The handoff is a dock panel since issue #1505, and not the shown tab of its
  // group by default.
  await revealGmPanel(page, 'source-link');
  await page.locator('#gm-workshop-pack').selectOption('workshop-test');
  await page.locator('#gm-workshop-open').click();
  // The handoff URL carries the one-use transfer token in its fragment.
  await expect(page).toHaveURL(/\/workshop(?:\.html)?(?:#source=[^#]+)?$/);
  await page.locator('#workshop-files').selectOption(WORKSHOP_WORLD);
  await expect(page.locator('#workshop-source')).toHaveValue(WORKSHOP_WORLD_TEXT.replaceAll('\r\n', '\n'));
  await expect(page.locator('#workshop-undo')).toBeDisabled();
  expect(await page.evaluate(() => typeof window.__hostLocalGm)).toBe('undefined');
  expect(errors).toEqual([]);
});
