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

test('rendererless GM maps and inspects stable local ship truth', async ({ context }) => {
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
  await page.waitForFunction(() => {
    const map = document.getElementById('gm-entity-map');
    return !map?.hidden && Array.isArray(map.state?.blips) && map.state.blips.length > 0;
  });

  const mapped = await page.evaluate(() => {
    const map = document.getElementById('gm-entity-map');
    return map.state.blips.map((blip) => ({
      uuid: blip.uuid,
      kind: blip.kind,
      destroyed: blip.destroyed,
    }));
  });
  expect(mapped.length).toBeGreaterThan(0);
  for (const blip of mapped) {
    expect(blip.uuid).toMatch(/^[0-9a-f-]{36}$/i);
    expect(['player_ship', 'npc_ship']).toContain(blip.kind);
    expect(typeof blip.destroyed).toBe('boolean');
  }

  // The real custom element is the browser interaction seam: keyboard picks
  // the first stable UUID, while wheel zoom remains a local map operation.
  const map = page.locator('#gm-entity-map');
  await map.focus();
  await map.press('ArrowRight');
  await page.waitForSelector('#gm-entity-card:not([hidden])');
  const canvas = map.locator('canvas');
  await canvas.hover();
  await page.mouse.wheel(0, -120);

  const first = await page.evaluate(() => {
    const card = document.getElementById('gm-entity-card');
    return {
      id: card.dataset.entityId,
      selected: document.getElementById('gm-entity-map').dataset.selectedEntityId,
      kind: card.dataset.kind,
      destroyed: card.dataset.destroyed,
      hull: document.getElementById('gm-entity-hull').value,
      status: document.getElementById('gm-entity-status').textContent,
      position: document.getElementById('gm-entity-position').textContent,
      legend: document.getElementById('gm-map-legend').textContent,
      tick: window.wasm_sim_tick(),
    };
  });
  expect(first.id).toMatch(/^[0-9a-f-]{36}$/i);
  expect(first.selected).toBe(first.id);
  expect(['player_ship', 'npc_ship']).toContain(first.kind);
  expect(Number(first.hull)).toBeGreaterThanOrEqual(0);
  expect(Number(first.hull)).toBeLessThanOrEqual(100);
  expect(first.status.length).toBeGreaterThan(0);
  expect(first.position.length).toBeGreaterThan(0);
  expect(first.legend).toContain('×');

  await page.waitForFunction((tick) => window.wasm_sim_tick() > tick + 10, first.tick);
  const second = await page.evaluate(() => {
    const card = document.getElementById('gm-entity-card');
    return {
      id: card.dataset.entityId,
      selected: document.getElementById('gm-entity-map').dataset.selectedEntityId,
      mapHasId: document.getElementById('gm-entity-map').state.blips
        .some((blip) => blip.uuid === card.dataset.entityId),
    };
  });
  expect(second).toEqual({ id: first.id, selected: first.id, mapHasId: true });
  expect(errors).toEqual([]);
});
