// Issue #1291 — the explicit production browser GM profile boots the real
// authoritative WASM simulation without a renderer or a local player ship.

import {
  captureServerPageErrors,
  expect,
  test,
  waitForWasmReady,
} from './fixtures';

test('the public Fleet role control selects the explicit GM boot profile', async ({ context }) => {
  const page = await context.newPage();
  const errors = captureServerPageErrors(page);
  await page.goto('/?scenario=assets/worlds/default.toml');
  await waitForWasmReady(page);

  await page.click('#server-settings-btn');
  await page.click('.server-settings-tab[data-tab="gameplay"]');
  await Promise.all([
    page.waitForURL((url) => url.searchParams.get('gm') === '1'),
    page.click('[data-control="fleet-role-gm"]'),
  ]);
  await waitForWasmReady(page);

  expect(await page.evaluate(() => ({
    requested: document.documentElement.dataset.phoenixBootRequest,
    actual: window.wasm_boot_profile(),
    role: window.__hostFleetRole(),
  }))).toEqual({
    requested: 'browser-game-master',
    actual: 'browser-game-master',
    role: 'gm',
  });
  await expect(page.locator('#gm-console')).toBeVisible();
  await expect(page.locator('#canvas')).toBeHidden();
  expect(errors).toEqual([]);
});

test('explicit rendererless GM page advances and renders stable local truth', async ({ context }) => {
  const page = await context.newPage();
  const errors = captureServerPageErrors(page);
  await page.goto('/?gm=1&scenario=assets/worlds/default.toml');
  await waitForWasmReady(page);

  const boot = await page.evaluate(() => ({
    webdriver: navigator.webdriver,
    requested: document.documentElement.dataset.phoenixBootRequest,
    actual: window.wasm_boot_profile(),
    role: window.__hostFleetRole(),
  }));
  expect(boot.webdriver).toBe(true);
  expect(boot.requested).toBe('browser-game-master');
  expect(boot.actual).toBe('browser-game-master');
  expect(boot.role).toBe('gm');
  await expect(page.locator('#gm-console')).toBeVisible();
  await expect(page.locator('#canvas')).toBeHidden();

  const tick0 = await page.evaluate(() => window.wasm_sim_tick());
  await page.waitForFunction((tick) => window.wasm_sim_tick() > tick + 5, tick0);

  await page.evaluate(() => window.__hostFleetOpen());
  await page.waitForFunction(() => {
    const state = window.__hostGmStartState?.();
    return state?.admitted === true
      && state.presentationReady === true
      && state.localValidation === true;
  });
  expect(await page.evaluate(() => window.__hostFleetState().role)).toBe('gm');

  await page.evaluate(() => document.getElementById('gm-ready-btn').click());
  await page.waitForFunction(() => window.__saveSlotsPhase === 'InProgress');
  await page.waitForSelector('#gm-entity-card:not([hidden])');

  const first = await page.evaluate(() => {
    const card = document.getElementById('gm-entity-card');
    return {
      id: card.dataset.entityId,
      destroyed: card.dataset.destroyed,
      hull: document.getElementById('gm-entity-hull').value,
      status: document.getElementById('gm-entity-status').textContent,
      tick: window.wasm_sim_tick(),
    };
  });
  expect(first.id).toMatch(/^[0-9a-f-]{36}$/i);
  expect(Number(first.hull)).toBeGreaterThanOrEqual(0);
  expect(Number(first.hull)).toBeLessThanOrEqual(100);
  expect(first.status.length).toBeGreaterThan(0);

  await page.waitForFunction((tick) => window.wasm_sim_tick() > tick + 10, first.tick);
  const second = await page.evaluate(() => {
    const card = document.getElementById('gm-entity-card');
    return {
      id: card.dataset.entityId,
      destroyed: card.dataset.destroyed,
      hull: document.getElementById('gm-entity-hull').value,
      status: document.getElementById('gm-entity-status').textContent,
    };
  });
  expect(second).toEqual({
    id: first.id,
    destroyed: first.destroyed,
    hull: first.hull,
    status: first.status,
  });
  expect(errors).toEqual([]);
});
