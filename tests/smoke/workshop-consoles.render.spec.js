import { test, expect } from '@playwright/test';
import { readFileSync } from 'node:fs';
import path from 'node:path';
import { parse } from 'smol-toml';
import { createStoreZip, readStoreZip } from '../../editor/mod-pack-export.js';
import { WORKSHOP_MANIFEST } from '../fixtures/workshop-pack.js';
import { revealWorkshopPanel } from './dock-helpers.js';
import { ts } from './strings';

const HULL = 'assets/entities/workshop_console_hull.toml';
const WORLD = 'assets/worlds/workshop.toml';
const CONSOLE = 'workshop-dock';
// Local source is intentional: inherited records must remain read-only. Shared
// includes still resolve from the shipped base at their ordinary relative paths.
const HULL_TEXT = readFileSync(path.join(__dirname, '../../assets/entities/alliance_cruiser.toml'), 'utf8');
const WORLD_TEXT = `[global]
title = "Console composition acceptance"
seed = 5
[[available_ships]]
template_path = "${HULL}"
[[entity]]
template_path = "${HULL}"
id = "player-ship"
spawn_on = "game_start"
`;
const byId = (page, id) => page.locator(`#workshop-ship-${id}`);
const optionValues = locator => locator.evaluate(select => [...select.options].map(option => option.value));
async function sourceOf(page) {
  await revealWorkshopPanel(page, 'files');
  await page.locator('#workshop-files').selectOption(HULL);
  await revealWorkshopPanel(page, 'source');
  return page.locator('#workshop-source').inputValue();
}
async function importPack(page, bytes) {
  const chooser = page.waitForEvent('filechooser');
  await page.locator('#workshop-import').click();
  await (await chooser).setFiles({ name: 'console-composition.zip', mimeType: 'application/zip', buffer: Buffer.from(bytes) });
  await expect(page.locator('.workshop-findings')).toContainText(ts('workshop.imported'));
  await revealWorkshopPanel(page, 'composition');
  await byId(page, 'template').selectOption(HULL);
  await expect(byId(page, 'console-add')).toBeEnabled();
}
async function applied(page, id) {
  await byId(page, id).click();
  await expect(byId(page, 'status')).toHaveText(ts('workshop.ship.applied'));
  await expect(byId(page, id)).toBeEnabled();
}

test('Workshop authors Console identity, feeds and ownership through real validation, history, reload and Test',
  { tag: '@core' }, async ({ page }, testInfo) => {
    test.setTimeout(300_000);
    const errors = [];
    page.on('pageerror', error => errors.push(error.message));
    await page.addInitScript(() => Object.defineProperty(navigator, 'webdriver', { get: () => false }));
    // Keep real content and runtime; only bound the render asset preflight like
    // the existing composed-hull acceptance test.
    await page.route('**/workshop-base.json', async route => {
      const response = await route.fetch(), source = await response.json();
      const all = source.base_asset_manifest;
      const selected = new Set(Object.keys(all).filter(name => name.endsWith('.wgsl')
        || /^assets\/(pfx|radar_icons|skybox)\//.test(name) || name.includes('alliance_cruiser_recreated')));
      for (const name of selected) for (const dependency of all[name]?.requires || []) selected.add(dependency);
      source.base_asset_manifest = Object.fromEntries([...selected].map(name => [name, all[name]]));
      await route.fulfill({ response, json: source });
    });
    await page.goto('/workshop.html');
    await importPack(page, createStoreZip([
      { path: 'scenarios.toml', text: WORKSHOP_MANIFEST },
      { path: WORLD, text: WORLD_TEXT }, { path: HULL, text: HULL_TEXT },
    ]));
    const seats = await optionValues(byId(page, 'stations'));
    const originalOrder = await optionValues(byId(page, 'consoles'));
    await byId(page, 'consoles').selectOption('helm');
    const helmScreen = await byId(page, 'console-screen').inputValue();
    await byId(page, 'console-id').fill(CONSOLE);
    await byId(page, 'console-name').fill('Workshop Dock');
    await applied(page, 'console-add');
    await byId(page, 'consoles').selectOption(CONSOLE);
    await expect(byId(page, 'console-home')).toHaveValue('helm');
    await expect(byId(page, 'console-screen')).toHaveValue(helmScreen);
    expect(await optionValues(byId(page, 'stations'))).toEqual(seats);
    expect(await optionValues(byId(page, 'consoles'))).toEqual([...originalOrder, CONSOLE]);
    await applied(page, 'console-up');
    const expectedOrder = [...originalOrder.slice(0, -1), CONSOLE, originalOrder.at(-1)];
    expect(await optionValues(byId(page, 'consoles'))).toEqual(expectedOrder);

    await byId(page, 'console-feed-source').selectOption('viewscreen');
    await byId(page, 'console-feed-kind').selectOption('Viewscreen');
    await applied(page, 'console-feed-add');
    await expect(byId(page, 'console-feeds')).toContainText('viewscreen: Viewscreen');
    const beforeOwnership = await sourceOf(page);
    await revealWorkshopPanel(page, 'composition');
    await byId(page, 'systems').selectOption('dock');
    await byId(page, 'system-station').selectOption(CONSOLE);
    await applied(page, 'system-apply');
    const edited = await sourceOf(page);
    const authored = parse(edited);
    expect(authored.system.find(row => row.id === 'dock').console).toBe(CONSOLE);
    expect(authored.console.find(row => row.id === CONSOLE)).toMatchObject({
      home_station: 'helm', screen: helmScreen, feed: [{ source: 'viewscreen', kind: 'Viewscreen' }],
    });
    expect(authored.station).toEqual(parse(HULL_TEXT).station);
    await page.locator('#workshop-undo').click();
    await expect.poll(() => sourceOf(page)).toBe(beforeOwnership);
    await page.locator('#workshop-redo').click();
    await expect.poll(() => sourceOf(page)).toBe(edited);

    // Export invokes authoritative validation; reload the bytes actually saved,
    // rather than rebuilding a new fixture which could mask a lost binding.
    const download = page.waitForEvent('download');
    await page.locator('#workshop-export').click();
    const bytes = new Uint8Array(readFileSync(await (await download).path()));
    expect(readStoreZip(bytes)[HULL]).toBe(edited);
    await expect(page.locator('#workshop-dirty')).toHaveText(ts('workshop.saved'));
    await importPack(page, bytes);
    expect(await sourceOf(page)).toBe(edited);
    await revealWorkshopPanel(page, 'composition');
    expect(await optionValues(byId(page, 'consoles'))).toEqual(expectedOrder);
    await byId(page, 'consoles').selectOption(CONSOLE);
    await expect(byId(page, 'console-feeds')).toContainText('viewscreen: Viewscreen');
    await byId(page, 'systems').selectOption('dock');
    await expect(byId(page, 'system-station')).toHaveValue(CONSOLE);

    await page.locator('#workshop-open-test').click();
    await page.locator('#workshop-test-world').selectOption(WORLD);
    await expect.poll(() => optionValues(page.locator('#workshop-test-ship')), { timeout: 60_000 }).toContain(HULL);
    await page.locator('#workshop-test-ship').selectOption(HULL);
    await page.locator('#workshop-test-seed').fill('5');
    await page.locator('#workshop-test-start').click();
    try {
      await expect.poll(async () => await page.locator('#workshop-test-pause').isEnabled()
        ? 'running' : await page.locator('#workshop-test-status').getAttribute('role') === 'alert'
          ? await page.locator('#workshop-test-status').textContent() : 'starting', { timeout: 120_000 }).toBe('running');
      const frame = page.frames().find(item => /\/workshop-test(?:\.html)?$/.test(new URL(item.url() || 'about:blank').pathname));
      expect(frame).toBeTruthy();
      expect(await frame.evaluate(async () => JSON.parse((await import('/phoenix.js')).wasm_workshop_test_status())))
        .toMatchObject({ running: true, selection: { ship: HULL } });
    } finally {
      await testInfo.attach('browser-errors', { contentType: 'application/json', body: JSON.stringify(errors) });
      await page.locator('#workshop-test-stop').click();
    }
    await expect(page.locator('.workshop-test-viewscreen')).toHaveCount(0);
    expect(await sourceOf(page)).toBe(edited);
    expect(errors).toEqual([]);
  });
