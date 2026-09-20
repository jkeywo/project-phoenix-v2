import { test, expect } from '@playwright/test';
import { createStoreZip } from '../../editor/mod-pack-export.js';
import { WORKSHOP_MANIFEST } from '../fixtures/workshop-pack.js';

// Issue #1474, criterion 4: "the exact unsaved definitions affect a disposable
// Test run without installation into Live content". The faction registry used
// to be compiled into the runtime, so no draft could change it; this proves,
// through a real browser Test, that the registry the run consults is the one
// the draft captured — a faction only the draft declares is there, and a base
// faction the draft overrides at its own path carries the draft's name.
const WORLD = 'assets/worlds/workshop.toml';
const SHIP = 'assets/entities/alliance_cruiser.toml';
const ALLIANCE = 'aaaaaaaa-1111-4111-8111-aaaaaaaaaaaa';
const DRAFT_ONLY = 'ffffffff-1474-4474-8474-ffffffffffff';
const isTestFrame = frame => /\/workshop-test(?:\.html)?$/.test(new URL(frame.url() || 'about:blank').pathname);
const SOURCE = `[global]
title = "Draft factions reach the Test"
seed = 3
[[available_ships]]
template_path = "${SHIP}"
[[entity]]
template_path = "${SHIP}"
id = "player-ship"
spawn_on = "game_start"
`;
// The draft REPLACES the shipped Alliance file at its own path and ADDS a
// faction the shipped content has never heard of. Neither is installed
// anywhere: they exist only as members of this unsaved draft.
const sourcePack = () => createStoreZip([
  { path: 'scenarios.toml', text: WORKSHOP_MANIFEST },
  { path: WORLD, text: SOURCE },
  { path: 'assets/factions/alliance.toml',
    text: `uuid = "${ALLIANCE}"\nname = "Alliance Draft"\nenemies = ["${DRAFT_ONLY}"]\n` },
  { path: 'assets/factions/workshop_only.toml',
    text: `# Only this draft declares it\nuuid = "${DRAFT_ONLY}"\nname = "Workshop Only"\nenemies = ["${ALLIANCE}"]\n` },
]);

test('a running browser Test consults the draft\'s factions, not the shipped registry',
  { tag: '@core' }, async ({ page }, testInfo) => {
    test.setTimeout(240_000);
    const errors = [], messages = [], factionRequests = [];
    page.on('console', message => { if (messages.length < 300) messages.push(`${message.type()}: ${message.text()}`); });
    page.on('pageerror', error => errors.push(error.message));
    // The Test page may read nothing outside its capture, so a faction file
    // fetched from the server would be a second registry source.
    page.on('request', request => {
      if (isTestFrame(request.frame()) && new URL(request.url()).pathname.startsWith('/assets/factions/')) {
        factionRequests.push(request.url());
      }
    });
    await page.addInitScript(() => {
      Object.defineProperty(navigator, 'webdriver', { get: () => false });
    });
    // A small declared base fixture keeps the shipped hull and shaders and
    // nothing else, exactly as the Test runtime spec does.
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
    await (await chooser).setFiles({ name: 'draft-factions.zip', mimeType: 'application/zip', buffer: Buffer.from(sourcePack()) });
    await page.locator('#workshop-open-test').click();
    await page.locator('#workshop-test-world').selectOption(WORLD);
    await page.locator('#workshop-test-ship').selectOption(SHIP);
    await page.locator('#workshop-test-seed').fill('3');
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
    const iframe = page.frames().find(isTestFrame);
    expect(iframe).toBeTruthy();

    // Watch the run through the omniscient view. Its faction panel lists
    // exactly the factions the running world loaded, read from the registry
    // the simulation consults — so what appears there is what the run uses.
    await page.locator('#workshop-test-view').selectOption('game-master');
    const listed = () => iframe.evaluate(() =>
      [...document.querySelectorAll('#gm-faction-source option')].map(option => option.value));
    await expect.poll(listed, { timeout: 60_000, message: 'the draft-only faction reaches the running Test' })
      .toContain('Workshop Only');
    const factions = await listed();
    // The draft's Alliance replaced the shipped one at its own path.
    expect(factions).toContain('Alliance Draft');
    expect(factions).not.toContain('Alliance');
    // The shipped factions the draft did not touch are still there: the
    // registry is the captured content, not the draft alone.
    expect(factions).toEqual(expect.arrayContaining(['Pirate', 'Harrow', 'Requiem']));

    await page.locator('#workshop-test-stop').click();
    await expect(page.locator('.workshop-test-viewscreen')).toHaveCount(0);
    expect(factionRequests).toEqual([]);
    expect(errors).toEqual([]);
  });
