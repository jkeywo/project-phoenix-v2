import { test, expect } from '@playwright/test';
import { createStoreZip } from '../../editor/mod-pack-export.js';
import { WORKSHOP_MANIFEST } from '../fixtures/workshop-pack.js';
import { ts } from './strings';
import { revealWorkshopPanel } from './dock-helpers.js';

// Issue #1476, criteria 1, 2, 3 and 5: a hull composed from fragments, read
// through the REAL wasm_workshop_entity; an inherited field shown with its owner
// and read-only until it is deliberately materialised; the materialised override
// written as NEW local text with every other byte of the document untouched and
// one undo reverting it; and the composed hull flown in the existing disposable
// Test. Unit tests of this panel mock the runtime; this is the run that does not.
const WORLD = 'assets/worlds/workshop.toml';
// The composed hull: two includes and nothing else of its own. It exists only in
// this unsaved draft, so a Test that flies it is flying the composition.
const HULL = 'assets/entities/workshop_hull.toml';
// The draft's own fragment. It merges AFTER the cruiser (the declaring template
// merges last, and includes merge in order), so it OWNS `hull_id`.
//
// FLAT, directly under assets/entities/, because that is the only shape a mod
// pack may carry: is_allowed_content_path admits a TOML member directly under an
// allowed prefix and nothing nested (shipped BASE content may nest, a pack may
// not). A nested fragment composes and validates here but refuses the pack at
// export and refuses to start a Test, so this fixture uses the shape an author
// can actually ship.
const FRAGMENT = 'assets/entities/workshop_1476.toml';
const CRUISER = 'assets/entities/alliance_cruiser.toml';
// The value the fragment authors, and the exact text materialising has to write.
const HULL_ID = 'WRK-1476';
// Stations the composed hull can only have through its include: they are
// authored as literal names in alliance_cruiser.toml, and the hull's own source
// declares nothing but `includes`. The roster prints them from the ship
// configuration the RUN resolved, so reading them there is composition observed
// in the simulation. The row's own label is deliberately not asserted: a Test's
// player ship does not take its name from the template it flies, and this spec
// has no business pinning a mechanism it did not establish.
const COMPOSED_STATIONS = ['Captain', 'Tactical', 'Command'];
const isTestFrame = frame => /\/workshop-test(?:\.html)?$/.test(new URL(frame.url() || 'about:blank').pathname);
const WORLD_TEXT = `[global]
title = "Composed hull in the Workshop"
seed = 4
[[available_ships]]
template_path = "${HULL}"
[[entity]]
template_path = "${HULL}"
id = "player-ship"
spawn_on = "game_start"
`;
// Everything this hull is comes from the two members beneath it. The comment and
// the array layout are what criterion 3 is judged on.
const HULL_TEXT = `# Keep this comment on the composed hull
includes = [
  "alliance_cruiser.toml",
  "workshop_1476.toml",
]
`;
const FRAGMENT_TEXT = `# Only this draft carries the fragment
hull_id = "${HULL_ID}"
`;
const sourcePack = () => createStoreZip([
  { path: 'scenarios.toml', text: WORKSHOP_MANIFEST },
  { path: WORLD, text: WORLD_TEXT },
  { path: HULL, text: HULL_TEXT },
  { path: FRAGMENT, text: FRAGMENT_TEXT },
]);
const byId = (page, id) => page.locator(`#workshop-entity-${id}`);
// The Materialise control for one field, addressed by the name a screen reader
// reads rather than by its positional id: the composed cruiser carries hundreds
// of fields and their order is the provenance map's, not this spec's.
const materialiseFor = (page, address) =>
  byId(page, 'fields').locator(`[aria-label="${ts('workshop.entity.materialise')}: ${address}"]`);
const includeRow = (authored, path, origin) =>
  ts('workshop.entity.include', { authored, path, origin });

/** The exact source of one member. `#workshop-files` lives in the files panel,
 * which the entity form does not share a dock group with — but the source view
 * does have to be brought forward to be read at all. */
async function sourceOf(page, member) {
  await revealWorkshopPanel(page, 'files');
  await page.locator('#workshop-files').selectOption(member);
  await revealWorkshopPanel(page, 'source');
  return page.locator('#workshop-source').inputValue();
}

test('the entity panel materialises an inherited value through the real runtime and the Test flies the composed hull',
  { tag: '@core' }, async ({ page }, testInfo) => {
    test.setTimeout(240_000);
    const errors = [], messages = [];
    page.on('console', message => { if (messages.length < 300) messages.push(`${message.type()}: ${message.text()}`); });
    page.on('pageerror', error => errors.push(error.message));
    await page.addInitScript(() => {
      Object.defineProperty(navigator, 'webdriver', { get: () => false });
    });
    // A small declared base fixture keeps the shipped hull's own assets and the
    // shaders, as the other Workshop Test render specs do. The composed hull
    // draws the cruiser's mesh, because that is the template it includes.
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
    await (await chooser).setFiles({ name: 'composed-hull.zip', mimeType: 'application/zip', buffer: Buffer.from(sourcePack()) });

    // Criterion 1, through the real runtime: choosing the hull reads its
    // composition — both includes with the member each resolves to and where
    // that member is defined, and the merge order beneath it.
    await revealWorkshopPanel(page, 'entity');
    await byId(page, 'template').selectOption(HULL);
    await expect(byId(page, 'status')).toHaveText(ts('workshop.entity.refreshed'));
    await expect(byId(page, 'includes')).toContainText(includeRow('alliance_cruiser.toml', CRUISER, 'base'));
    await expect(byId(page, 'includes')).toContainText(includeRow('workshop_1476.toml', FRAGMENT, 'draft'));
    await expect(byId(page, 'origin')).toContainText(CRUISER);
    await expect(byId(page, 'origin')).toContainText(FRAGMENT);

    // Criterion 2: the value the fragment owns is named with its owner and is
    // read-only here until an author asks for it.
    const inherited = byId(page, 'fields').locator('label', { hasText: /^hull_id —/ }).first();
    await expect(inherited).toContainText(ts('workshop.entity.field_inherited', { source: FRAGMENT }));
    const value = byId(page, 'fields').locator(`#${await inherited.getAttribute('for')}`);
    await expect(value).toBeDisabled();
    await expect(value).toHaveValue(`"${HULL_ID}"`);
    expect(await sourceOf(page, HULL)).toBe(HULL_TEXT);

    // Materialise it. The runtime reads the resolved value and writes it into
    // the local document as NEW text: every byte the hull already had survives,
    // in order (criterion 3), and ONE undo reverts the whole thing — one history
    // entry, not one per edit the runtime happened to make.
    await revealWorkshopPanel(page, 'entity');
    await materialiseFor(page, 'hull_id').click();
    await expect.poll(() => sourceOf(page, HULL)).toContain(`hull_id = "${HULL_ID}"`);
    const materialised = await sourceOf(page, HULL);
    expect(materialised.startsWith(HULL_TEXT)).toBe(true);
    await page.locator('#workshop-undo').click();
    await expect.poll(() => sourceOf(page, HULL)).toBe(HULL_TEXT);
    await page.locator('#workshop-redo').click();
    await expect.poll(() => sourceOf(page, HULL)).toBe(materialised);
    // The fragment it came from was never touched.
    expect(await sourceOf(page, FRAGMENT)).toBe(FRAGMENT_TEXT);

    // Criterion 5: the panel's own Test control hands the composed hull to the
    // existing disposable Test rather than standing up a second runner. The
    // Test's own catalogue is what decides the hull is flyable, and it resolves
    // the composition to answer — so wait for it rather than racing it.
    await revealWorkshopPanel(page, 'entity');
    await byId(page, 'refresh').click();
    await expect(byId(page, 'status')).toHaveText(ts('workshop.entity.refreshed'));
    await expect.poll(() => page.locator('#workshop-test-ship')
      .evaluate(select => [...select.options].filter(option => !option.disabled).map(option => option.value)),
    { timeout: 60_000, message: 'the Test catalogue offers the composed hull' }).toContain(HULL);
    await byId(page, 'test').click();
    await expect(byId(page, 'status')).toHaveText(ts('workshop.entity.testing', { path: HULL }));
    await expect(page.locator('#workshop-test-ship')).toHaveValue(HULL);

    await page.locator('#workshop-test-world').selectOption(WORLD);
    await page.locator('#workshop-test-seed').fill('4');
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

    // The runtime's own statement of what it is flying: the composed hull, which
    // exists only in this unsaved draft.
    const status = await iframe.evaluate(async () => JSON.parse((await import('/phoenix.js')).wasm_workshop_test_status()));
    expect(status).toMatchObject({ running: true, selection: { ship: HULL } });

    // And the omniscient view's roster, the GM's own projection of the run, shows
    // that hull with the Stations only its include can have given it. Read through
    // the frame so a roster nobody can see cannot pass.
    await page.locator('#workshop-test-view').selectOption('game-master');
    await expect.poll(async () => await iframe.evaluate(() => {
      const gm = document.getElementById('test-gm'), roster = document.getElementById('gm-roster-ships');
      if (gm?.hidden || !roster || roster.offsetParent === null) return null;
      return roster.textContent;
    }), { timeout: 60_000, message: 'the composed hull is in the running Test with its included Stations' })
      .toContain(COMPOSED_STATIONS[0]);
    const roster = await iframe.evaluate(() => document.getElementById('gm-roster-ships').textContent);
    for (const station of COMPOSED_STATIONS) expect(roster, station).toContain(station);

    await page.locator('#workshop-test-stop').click();
    await expect(page.locator('.workshop-test-viewscreen')).toHaveCount(0);
    expect(errors).toEqual([]);
  });
