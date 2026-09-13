import { test, expect } from '@playwright/test';
import { readFileSync } from 'node:fs';
import path from 'node:path';
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

test('Workshop uses the real runtime for Rhai, source-span fields and browser recovery', { tag: '@core' }, async ({ page }) => {
  test.setTimeout(120_000);
  page.on('dialog', dialog => dialog.accept());
  await page.goto('/workshop.html');
  const chooser = page.waitForEvent('filechooser');
  await page.locator('#workshop-import').click();
  await (await chooser).setFiles({ name: 'script.zip', mimeType: 'application/zip',
    buffer: readFileSync(path.join(__dirname, '../fixtures/mod-packs/script-valid.zip')) });
  const script = 'assets/worlds/script_valid.rhai';
  const world = 'assets/worlds/script_valid.toml';
  await page.locator('#workshop-files').selectOption(script);
  await page.locator('#workshop-source').fill('import "network" as unsafe;');
  await page.locator('#workshop-check').click();
  await expect(page.getByRole('alert')).toContainText(ts('workshop.check_refused'), { timeout: 90_000 });
  await expect(page.getByRole('alert')).toContainText(script);
  await page.locator('#workshop-undo').click();
  await page.locator('#workshop-files').selectOption(world);
  const original = await page.locator('#workshop-source').inputValue();
  await page.getByText(ts('workshop.inspector'), { exact: true }).click();
  await page.locator('#workshop-inspect').click();
  await page.locator('#workshop-field').selectOption({ label: 'global.title' });
  await page.locator('#workshop-field-value').fill('"Edited with runtime fields"');
  await page.locator('#workshop-apply-field').click();
  await expect(page.locator('#workshop-source')).toHaveValue(original.replace('"Script Valid"', '"Edited with runtime fields"'));
  await expect(page.locator('#workshop-recovery-status')).toHaveText(ts('workshop.recovery_saved'));
  await page.reload();
  await expect(page.locator('#workshop-restore')).toBeVisible();
  await page.locator('#workshop-restore').click();
  await expect(page.locator('#workshop-files')).toHaveValue(world);
  await expect(page.locator('#workshop-dirty')).toHaveText(ts('workshop.dirty'));
  await expect(page.locator('#workshop-source')).toHaveValue(original.replace('"Script Valid"', '"Edited with runtime fields"'));
  await page.locator('#workshop-undo').click();
  await expect(page.locator('#workshop-source')).toHaveValue(original);
  await page.locator('#workshop-redo').click();
  const downloaded = page.waitForEvent('download', { timeout: 90_000 });
  await page.locator('#workshop-export').click();
  const artifact = await downloaded;
  const files = readStoreZip(new Uint8Array(readFileSync(await artifact.path())));
  expect(files[world]).toBe(original.replace('"Script Valid"', '"Edited with runtime fields"'));
  expect(files[script]).toContain('fn on_alarm(ctx)');
});

test('Workshop creates a pack, previews read-only dependencies and preserves MP3 import through recovery and undo', { tag: '@core' }, async ({ page }) => {
  test.setTimeout(120_000);
  await page.goto('/workshop.html');
  await page.locator('#workshop-new').click();
  await expect(page.locator('#workshop-files option')).toHaveCount(2, { timeout: 90_000 });
  await page.getByText(ts('workshop.dependencies'), { exact: true }).click();
  await page.locator('#workshop-dependencies-load').click();
  await expect(page.locator('#workshop-dependency-source')).toHaveAttribute('readonly', '');
  await expect(page.locator('#workshop-dependency option')).not.toHaveCount(0);
  await page.locator('#workshop-add-path').fill('assets/sounds/music.mp3');
  const chooser = page.waitForEvent('filechooser');
  await page.locator('#workshop-add-asset').click();
  await (await chooser).setFiles({ name: 'music.mp3', mimeType: 'audio/mpeg', buffer: Buffer.from([73, 68, 51, 255, 128, 0]) });
  await expect(page.locator('#workshop-source')).toBeDisabled();
  await expect(page.locator('#workshop-files')).toHaveValue('assets/sounds/music.mp3');
  await expect(page.locator('#workshop-recovery-status')).toHaveText(ts('workshop.recovery_saved'));
  await page.reload();
  await page.locator('#workshop-restore').click();
  await expect(page.locator('#workshop-files')).toHaveValue('assets/sounds/music.mp3');
  await page.locator('#workshop-check').click();
  await expect(page.getByRole('alert')).toContainText(ts('workshop.check_refused'));
  await page.locator('#workshop-undo').click();
  await expect(page.locator('#workshop-files option')).toHaveCount(2);
  const downloaded = page.waitForEvent('download');
  await page.locator('#workshop-export').click();
  const artifact = await downloaded;
  const files = readStoreZip(new Uint8Array(readFileSync(await artifact.path())));
  expect(Object.keys(files)).toEqual(['scenarios.toml', 'assets/worlds/workshop.toml']);
  expect(files['scenarios.toml']).toContain('workshop-pack');
});
