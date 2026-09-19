import { test, expect } from '@playwright/test';
import { createStoreZip } from '../../editor/mod-pack-export.js';
import { WORKSHOP_MANIFEST } from '../fixtures/workshop-pack.js';
import { ts } from './strings';
import { revealWorkshopPanel } from './dock-helpers.js';

const WORLD = 'assets/worlds/workshop.toml';
const SHIP = 'assets/entities/alliance_cruiser.toml';
const SCRIPT = 'assets/worlds/workshop.rhai';
const isTestFrame = frame => /\/workshop-test(?:\.html)?$/.test(new URL(frame.url() || 'about:blank').pathname);
const SOURCE = `script = "workshop.rhai"
[global]
title = "Captured Workshop Test"
seed = 9
[[available_ships]]
template_path = "${SHIP}"
[[entity]]
template_path = "${SHIP}"
id = "player-ship"
spawn_on = "game_start"
`;
const sourcePack = () => createStoreZip([
  { path: 'scenarios.toml', text: WORKSHOP_MANIFEST },
  { path: WORLD, text: SOURCE },
  { path: SCRIPT, text: '// Captured script\n' },
]);

async function sceneColours(page, frame) {
  const box = await frame.locator('#canvas').boundingBox();
  expect(box).toBeTruthy();
  // Sample only the centre: surrounding live HTML controls cannot make a
  // renderer which cleared its scene to one colour pass this assertion.
  const png = await page.screenshot({ clip: { x: Math.round(box.x + box.width * 0.3),
    y: Math.round(box.y + box.height * 0.3), width: Math.round(box.width * 0.4), height: Math.round(box.height * 0.4) } });
  return frame.evaluate(async value => {
    const image = new Image(); image.src = `data:image/png;base64,${value}`; await image.decode();
    const canvas = document.createElement('canvas'); canvas.width = image.width; canvas.height = image.height;
    const context = canvas.getContext('2d'); context.drawImage(image, 0, 0);
    const pixels = context.getImageData(0, 0, canvas.width, canvas.height).data, colours = new Set();
    for (let i = 0; i < pixels.length && colours.size < 64; i += 4) colours.add((pixels[i] << 16) | (pixels[i + 1] << 8) | pixels[i + 2]);
    return colours.size;
  }, png.toString('base64'));
}

test('a real disposable browser Test boots unsaved captured source, steps, returns and retires without Live storage or delivery', { tag: '@core' }, async ({ page }, testInfo) => {
  test.setTimeout(240_000);
  const errors = [], sockets = [], runtimeAssetRequests = [], messages = [];
  page.on('console', message => { if (messages.length < 300) messages.push(`${message.type()}: ${message.text()}`); });
  page.on('pageerror', error => errors.push(error.message));
  page.on('websocket', socket => sockets.push(socket.url()));
  page.on('request', request => {
    if (isTestFrame(request.frame()) && new URL(request.url()).pathname.startsWith('/assets/')) {
      runtimeAssetRequests.push(request.url());
    }
  });
  await page.addInitScript(() => {
    Object.defineProperty(navigator, 'webdriver', { get: () => false });
    if (!/^\/workshop-test(?:\.html)?$/.test(location.pathname)) return;
    window.__testStorageWrites = [];
    const setItem = Storage.prototype.setItem;
    Storage.prototype.setItem = function (key, value) { window.__testStorageWrites.push(key); return setItem.call(this, key, value); };
    const open = IDBFactory.prototype.open;
    IDBFactory.prototype.open = function (...args) { window.__testStorageWrites.push(`indexedDB:${args[0]}`); return open.apply(this, args); };
  });
  // A small declared base fixture retains the actual shipped hull and shaders.
  // Dependency descriptors still come from the real builder and its hashes.
  await page.route('**/workshop-base.json', async route => {
    const response = await route.fetch(), source = await response.json();
    const all = source.base_asset_manifest;
    const selected = new Set(Object.keys(all).filter(path => path.endsWith('.wgsl')
      || /^assets\/(pfx|radar_icons|skybox)\//.test(path)
      || path.includes('alliance_cruiser_recreated')));
    for (const path of selected) for (const dependency of all[path]?.requires || []) selected.add(dependency);
    source.base_asset_manifest = Object.fromEntries([...selected].map(path => [path, all[path]]));
    await route.fulfill({ response, json: source });
  });
  await page.goto('/workshop.html');
  const chooser = page.waitForEvent('filechooser');
  await page.locator('#workshop-import').click();
  await (await chooser).setFiles({ name: 'captured-test.zip', mimeType: 'application/zip', buffer: Buffer.from(sourcePack()) });
  await page.locator('#workshop-files').selectOption(WORLD);
  await revealWorkshopPanel(page, 'source');
  await page.locator('#workshop-source').fill(SOURCE.replace('Captured Workshop Test', 'Unsaved source reaches the runtime'));
  await page.locator('#workshop-open-test').click();
  await page.locator('#workshop-test-world').selectOption(WORLD);
  await page.locator('#workshop-test-ship').selectOption(SHIP);
  await page.locator('#workshop-test-seed').fill('41');
  await expect(page.locator('#workshop-test-start')).toBeEnabled();
  await page.locator('#workshop-test-start').click();
  try {
    await expect.poll(async () => await page.locator('#workshop-test-pause').isEnabled()
      ? 'running' : await page.locator('#workshop-test-status').getAttribute('role') === 'alert'
        ? await page.locator('#workshop-test-status').textContent() : 'starting', { timeout: 120_000 }).toBe('running');
  } catch (error) {
    await testInfo.attach('browser-diagnostics', { contentType: 'application/json', body: JSON.stringify({ errors, messages }, null, 2) });
    throw error;
  }
  await page.locator('#workshop-test-pause').click();
  const iframe = page.frames().find(isTestFrame);
  expect(iframe).toBeTruthy();
  const state = () => iframe.evaluate(async () => JSON.parse((await import('/phoenix.js')).wasm_workshop_test_status()));
  await expect.poll(async () => (await state()).paused).toBe(true);
  const held = await state();
  expect(held).toMatchObject({ running: true, starting: false, paused: true, selection: { world: WORLD, ship: SHIP, seed: 41 } });
  expect(await iframe.evaluate(async () => (await import('/phoenix.js')).wasm_boot_profile())).toBe('browser-workshop-test');
  expect(await iframe.evaluate(async world => (await import('/phoenix.js')).wasm_preload_world_source(world), WORLD))
    .toBe(SOURCE.replace('Captured Workshop Test', 'Unsaved source reaches the runtime'));
  await expect.poll(() => sceneColours(page, iframe), { message: 'The captured Test scene actually draws' }).toBeGreaterThan(4);
  expect(await iframe.evaluate(async () => {
    const runtime = await import('/phoenix.js');
    let refused = false;
    try { runtime.wasm_list_save_slots(); } catch (_) { refused = true; }
    return { refused, slot: runtime.wasm_create_save_slot('Must not persist'),
      imported: runtime.wasm_import_save_slot('not a stored run', 'Must not persist'),
      deleted: runtime.wasm_delete_save_slot('unavailable', true),
      resumed: runtime.wasm_prepare_resume('unavailable') };
  })).toEqual({ refused: true, slot: '', imported: 'Saved runs are unavailable in disposable Workshop Test',
    deleted: 'Saved runs are unavailable in disposable Workshop Test', resumed: 'Saved runs are unavailable in disposable Workshop Test' });
  await page.locator('#workshop-test-step').click();
  await expect.poll(async () => (await state()).tick).toBe(held.tick + 1);
  await expect.poll(async () => (await state()).paused).toBe(true);
  await page.waitForTimeout(250);
  expect((await state()).tick).toBe(held.tick + 1);
  await expect(page.locator('.workshop-layout')).toBeHidden();
  await page.screenshot({ path: testInfo.outputPath('workshop-browser-test.png'), fullPage: true });
  await page.locator('#workshop-test-authoring').click();
  await expect(page.locator('#workshop-source')).toBeVisible();
  await page.locator('#workshop-files').selectOption(SCRIPT);
  await page.locator('#workshop-source').fill('import "uncaptured" as forbidden;');
  await expect(page.locator('#workshop-test-status')).toContainText(ts('workshop.test_stale'));
  await page.locator('#workshop-open-test').click();
  await expect(page.locator('#workshop-test-start')).toBeEnabled();
  await page.locator('#workshop-test-start').click();
  await expect(page.locator('#workshop-test-status')).toHaveAttribute('role', 'alert');
  expect(page.frames().filter(isTestFrame)).toEqual([iframe]);
  expect((await state()).tick).toBe(held.tick + 1);
  await expect(page.locator('#workshop-test-step')).toBeEnabled();
  await page.locator('#workshop-test-step').click();
  await expect.poll(async () => (await state()).tick).toBe(held.tick + 2);
  expect(await iframe.evaluate(() => window.__testStorageWrites)).toEqual([]);
  await page.locator('#workshop-test-stop').click();
  await expect(page.locator('.workshop-test-viewscreen')).toHaveCount(0);
  await expect(page.locator('#workshop-source')).toBeVisible();
  expect(sockets).toEqual([]);
  expect(runtimeAssetRequests.filter(url => !url.endsWith('/assets/strings/strings.csv'))).toEqual([]);
  expect(errors).toEqual([]);
});
