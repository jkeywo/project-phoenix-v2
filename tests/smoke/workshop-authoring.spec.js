import { test, expect } from '@playwright/test';
import { readFileSync } from 'node:fs';
import { readStoreZip } from '../../editor/mod-pack-export.js';
import { workshopPack, WORKSHOP_WORLD, WORKSHOP_WORLD_TEXT } from '../fixtures/workshop-pack.js';
import { ts } from './strings';
import { OPERATOR_PROFILE_KEY, createOperatorProfileSnapshot } from '../../gui/operator-profile.js';

test('offline Workshop imports, edits, undoes and exports one source-preserving pack', { tag: '@core' }, async ({ page }, testInfo) => {
  const pageErrors = [];
  const externalRequests = [];
  const sockets = [];
  page.on('pageerror', error => pageErrors.push(error.message));
  page.on('websocket', socket => sockets.push(socket.url()));
  await page.route('**/*', route => {
    const request = new URL(route.request().url());
    const local = new URL(testInfo.project.use.baseURL);
    if (request.origin !== local.origin) {
      externalRequests.push(request.href);
      return route.abort();
    }
    return route.continue();
  });
  await page.goto('/workshop.html');
  await expect(page.getByRole('heading', { name: ts('workshop.title') })).toBeVisible();
  await expect(page.locator('#workshop-export')).toBeDisabled();
  const chooser = page.waitForEvent('filechooser');
  await page.locator('#workshop-import').click();
  await (await chooser).setFiles({ name: 'workshop.zip', mimeType: 'application/zip', buffer: Buffer.from(workshopPack()) });
  await expect(page.locator('#workshop-files')).toBeEnabled();
  await page.locator('#workshop-files').selectOption(WORKSHOP_WORLD);
  await page.locator('#workshop-source').fill(`${WORKSHOP_WORLD_TEXT}# Browser edit\n`);
  await page.locator('#workshop-files').selectOption('scenarios.toml');
  const manifest = await page.locator('#workshop-source').inputValue();
  await page.locator('#workshop-source').fill(manifest.replace('Workshop test', 'Browser pack'));
  await page.locator('#workshop-undo').click();
  await expect(page.locator('#workshop-files')).toHaveValue('scenarios.toml');
  await page.locator('#workshop-undo').click();
  await expect(page.locator('#workshop-files')).toHaveValue(WORKSHOP_WORLD);
  await expect(page.locator('#workshop-dirty')).toHaveText(ts('workshop.saved'));
  await page.locator('#workshop-redo').click();
  await page.locator('#workshop-source').fill('[global\n');
  await page.locator('#workshop-export').click();
  await expect(page.getByRole('alert')).toContainText(ts('workshop.check_refused'));
  await expect(page.getByRole('alert')).toBeFocused();
  await page.locator('#workshop-undo').click();
  const downloaded = page.waitForEvent('download');
  await page.locator('#workshop-export').click();
  const artifact = await downloaded;
  const files = readStoreZip(new Uint8Array(readFileSync(await artifact.path())));
  expect(files[WORKSHOP_WORLD]).toBe(`${WORKSHOP_WORLD_TEXT}# Browser edit\r\n`);
  expect(files['scenarios.toml']).toContain('# Keep this manifest note\r\n');
  expect(files['scenarios.toml']).toContain('custom_note = "retain extension"\r\n');
  await expect(page.locator('#workshop-dirty')).toHaveText(ts('workshop.saved'));
  await expect(page.locator('.workshop-findings')).toContainText(ts('workshop.exported'));
  expect(externalRequests).toEqual([]);
  expect(sockets).toEqual([]);
  expect(pageErrors).toEqual([]);
  await page.screenshot({ path: testInfo.outputPath('workshop-authoring.png'), fullPage: true });
});

test('Workshop applies the shared 200% text profile without horizontal page overflow', async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 720 });
  await page.addInitScript(({ key, profile }) => localStorage.setItem(key, JSON.stringify(profile)), {
    key: OPERATOR_PROFILE_KEY,
    profile: createOperatorProfileSnapshot({ accessibility: { presentation: { textScale: 2, contrast: 'on' } } }),
  });
  await page.goto('/workshop.html');
  await expect(page.getByRole('heading', { name: ts('workshop.title') })).toBeVisible();
  await expect(page.locator('html')).toHaveAttribute('data-contrast', 'more');
  expect(await page.evaluate(() => ({
    scale: document.documentElement.style.getPropertyValue('--a11y-text-scale'),
    overflow: document.documentElement.scrollWidth > document.documentElement.clientWidth,
  }))).toEqual({ scale: '2', overflow: false });
  await page.getByText(ts('editor.mod.settings.heading'), { exact: true }).click();
  await expect(page.getByRole('button').last()).toBeVisible();
});
