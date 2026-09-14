import { test, expect } from '@playwright/test';
import { readStoreZip } from '../../editor/mod-pack-export.js';
import { workshopPack, WORKSHOP_WORLD, WORKSHOP_WORLD_TEXT } from '../fixtures/workshop-pack.js';
import { ts } from './strings';

// The native engine/process is covered by its native integration tests. This
// check uses the built native adapter over a deterministic private IPC peer to
// exercise the actual shared markup/CSS/module graph and responsive controls.
test('built Workshop Test controls keep Authoring exclusive and fit 200% text', { tag: '@core' }, async ({ page }, testInfo) => {
  const errors = [], sockets = [];
  page.on('pageerror', error => errors.push(error.message));
  page.on('websocket', socket => sockets.push(socket.url()));
  await page.route('**/workshop.html', async route => {
    const response = await route.fetch();
    const html = await response.text();
    // Same one-entry replacement the native document owner performs. All
    // styles, modules and dependency imports still come from the built tree.
    await route.fulfill({ response, body: html.replace('<script type="module" src="gui/workshop-boot.js"></script>', '') });
  });
  await page.goto('/workshop.html');
  await page.evaluate(async ({ files, world }) => {
    await import('/gui/strings-boot.js');
    const { mountNativeWorkshop } = await import('/gui/native-workshop.js');
    let workspace, run = null;
    window.__testRequests = [];
    workspace = mountNativeWorkshop({ root: document.getElementById('workshop'), send(record) {
      const request = JSON.parse(record);
      window.__testRequests.push(request);
      let response = { status: 'done' };
      if (request.op === 'load-sources') response = { status: 'sources', kind: 'mod', revision: 'fixture', files };
      if (request.op === 'recovery-load') response = { status: 'recovery', recovery: null };
      if (request.op === 'test-catalog') response = { status: 'test-catalog', catalog: { worlds: [world], ships: ['assets/entities/read-only-base-hull.toml'] } };
      if (request.op === 'test-start') {
        run = { running: true, starting: false, paused: false, tick: 0, multiplier: 1, selection: request.selection };
        response = { status: 'test', run };
      }
      if (request.op === 'test-control') {
        if (request.control.command === 'pause') run = { ...run, paused: true };
        if (request.control.command === 'step') run = { ...run, tick: run.tick + 1 };
        response = { status: 'test', run };
      }
      if (request.op === 'test-stop') { run = null; response = { status: 'test', run }; }
      if (request.op === 'test-status') response = { status: 'test', run };
      queueMicrotask(() => workspace.receive({ id: request.id, ...response }));
    } });
    window.__testWorkspace = workspace;
    await workspace.ready;
  }, { files: readStoreZip(workshopPack()), world: WORKSHOP_WORLD });
  await expect(page.locator('#workshop-test-start')).toBeEnabled();
  await page.locator('#workshop-files').selectOption(WORKSHOP_WORLD);
  await page.locator('#workshop-source').fill(`${WORKSHOP_WORLD_TEXT}# exact unsaved run\n`);
  await page.locator('#workshop-test-start').click();
  await expect(page.locator('.workshop-layout')).toBeHidden();
  await expect(page.locator('#workshop > .workshop-toolbar')).toBeHidden();
  await expect(page.locator('#workshop-models')).toBeHidden();
  await expect(page.locator('#workshop-test-authoring')).toBeVisible();
  await expect(page.locator('#workshop-test-world')).toBeDisabled();
  await page.locator('#workshop-test-pause').click();
  await page.locator('#workshop-test-step').click();
  await expect(page.locator('#workshop-test-status')).toContainText(ts('workshop.test_held', { tick: '1' }));
  await page.setViewportSize({ width: 390, height: 844 });
  await page.evaluate(() => document.documentElement.style.setProperty('--a11y-text-scale', '2'));
  expect(await page.evaluate(() => document.documentElement.scrollWidth > document.documentElement.clientWidth)).toBe(false);
  await page.screenshot({ path: testInfo.outputPath('workshop-test-390px-200pct.png'), fullPage: true });
  await page.locator('#workshop-test-authoring').click();
  await expect(page.locator('#workshop-source')).toBeVisible();
  await expect(page.locator('#workshop-source')).toHaveValue(`${WORKSHOP_WORLD_TEXT}# exact unsaved run\n`);
  await page.locator('#workshop-source').fill(`${WORKSHOP_WORLD_TEXT}# edited after Test\n`);
  await expect(page.locator('#workshop-test-status')).toContainText(ts('workshop.test_stale'));
  const started = await page.evaluate(() => window.__testRequests.filter(request => request.op === 'test-start'));
  expect(started).toHaveLength(1);
  expect(started[0].files[WORKSHOP_WORLD]).toContain('# exact unsaved run');
  expect(started[0].files[WORKSHOP_WORLD]).not.toContain('edited after');
  expect(sockets).toEqual([]);
  expect(errors).toEqual([]);
});
