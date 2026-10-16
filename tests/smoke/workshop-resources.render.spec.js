import { test, expect } from '@playwright/test';
import { readFileSync, readdirSync } from 'node:fs';
import { parse, stringify } from 'smol-toml';
import { resolveTemplate } from '../../editor/entity-includes.js';
import { createStoreZip, readStoreZip } from '../../editor/mod-pack-export.js';
import { WORKSHOP_MANIFEST } from '../fixtures/workshop-pack.js';
import { revealWorkshopPanel } from './dock-helpers.js';
import { ts } from './strings';

const HULL = 'assets/entities/workshop_resource_hull.toml';
const BASE = 'assets/entities/workshop_resource_base.toml';
const DEPOT = 'assets/entities/workshop_resource_depot.toml';
const WORLD = 'assets/worlds/workshop.toml';
const HULL_TEXT = 'includes=["workshop_resource_base.toml"]\n# Keep inherited ship equipment.\n';
// Retain every real inherited finite store when adding the fixture capacity.
// Capacity arrays replace as a whole; an old ammo-only override is not a hull.
function entitySources(directory = 'assets/entities') {
  return Object.fromEntries(readdirSync(directory, {withFileTypes: true}).flatMap(entry => {
    const path = `${directory}/${entry.name}`;
    return entry.isDirectory() ? Object.entries(entitySources(path))
      : entry.name.endsWith('.toml') ? [[path, readFileSync(path, 'utf8')]] : [];
  }));
}
const inherited = resolveTemplate('assets/entities/alliance_cruiser.toml', entitySources(), parse);
if (!inherited.ok) throw new Error('Actual Cruiser fixture failed include resolution');
const capacities = inherited.resolved.value.infrastructure.capacity;
const BASE_TEXT = `includes=["alliance_cruiser.toml"]
[umbilical]
capacity="torpedoes"
direction="collect"
[resource_services.torpedoes]
kind="capacity"
id="ammo"
[umbilical.link.provider]
kind="capacity"
id="ammo"
` + stringify({infrastructure: {capacity: [...capacities, {id: "ammo", amount: 4, ceiling: 20}]}});
const DEPOT_TEXT = `includes=["umbilical_berth.toml"]
[[infrastructure.capacity]]
id="ammo"
amount=50
ceiling=50
[resource_services.torpedoes]
kind="capacity"
id="ammo"
`;
const WORLD_TEXT = `[global]
title="Resource Workshop acceptance"
seed=6
[[available_ships]]
template_path="${HULL}"
[[entity]]
template_path="${HULL}"
id="player-ship"
spawn_on="game_start"
[[entity]]
template_path="${DEPOT}"
id="ammunition-depot"
spawn_on="game_start"
transform={position=[1000.0,0.0,0.0]}
`;
const byId = (page, id) => page.locator('#workshop-resource-' + id);
const options = locator => locator.evaluate(select => [...select.options].map(option => option.value));
async function sourceOf(page, member = HULL) {
  await revealWorkshopPanel(page, 'files'); await page.locator('#workshop-files').selectOption(member);
  await revealWorkshopPanel(page, 'source'); return page.locator('#workshop-source').inputValue();
}
async function resourceForm(page) {
  await revealWorkshopPanel(page, 'composition'); await byId(page, 'template').selectOption(HULL);
  await expect(byId(page, 'capacity-apply')).toBeEnabled();
}
async function importPack(page, bytes) {
  const chooser = page.waitForEvent('filechooser'); await page.locator('#workshop-import').click();
  await (await chooser).setFiles({ name: 'resource-workshop.zip', mimeType: 'application/zip', buffer: Buffer.from(bytes) });
  await expect(page.locator('.workshop-findings')).toContainText(ts('workshop.imported'));
  await resourceForm(page);
}
async function applied(page, id) {
  await byId(page, id).click(); await expect(byId(page, 'status')).toHaveText(ts('workshop.changed'));
  await expect(byId(page, id)).toBeEnabled();
}

async function previewColours(page) {
  const box = await page.frameLocator('.workshop-model-preview-frame').locator('#canvas').boundingBox();
  expect(box).toBeTruthy();
  const png = await page.screenshot({ clip: { x: Math.round(box.x + box.width * 0.25),
    y: Math.round(box.y + box.height * 0.25), width: Math.round(box.width * 0.5), height: Math.round(box.height * 0.5) } });
  return page.evaluate(async value => {
    const image = new Image(); image.src = `data:image/png;base64,${value}`; await image.decode();
    const canvas = document.createElement('canvas'); canvas.width = image.width; canvas.height = image.height;
    const context = canvas.getContext('2d'); context.drawImage(image, 0, 0);
    const pixels = context.getImageData(0, 0, canvas.width, canvas.height).data, colours = new Set();
    for (let i = 0; i < pixels.length && colours.size < 64; i += 4) colours.add((pixels[i] << 16) | (pixels[i + 1] << 8) | pixels[i + 2]);
    return colours.size;
  }, png.toString('base64'));
}

test('Workshop edits inherited stores and service links, then reloads and exercises the authored hull',
  { tag: '@core' }, async ({ page }, testInfo) => {
    test.setTimeout(360_000);
    const errors = []; page.on('pageerror', error => errors.push(error.message));
    await page.addInitScript(() => Object.defineProperty(navigator, 'webdriver', { get: () => false }));
    // Use the real providers and WASM validator. Bound only the asset preflight
    // to the two hulls used by this disposable fixture.
    await page.route('**/workshop-base.json', async route => {
      const response = await route.fetch(), source = await response.json(), all = source.base_asset_manifest;
      const selected = new Set(Object.keys(all).filter(name => name.endsWith('.wgsl')
        || /^assets\/(pfx|radar_icons|skybox)\//.test(name)
        || name.includes('alliance_cruiser_recreated') || name.includes('alliance_courier_recreated_dock_probe')));
      for (const name of selected) for (const dependency of all[name]?.requires || []) selected.add(dependency);
      source.base_asset_manifest = Object.fromEntries([...selected].map(name => [name, all[name]]));
      await route.fulfill({ response, json: source });
    });
    await page.goto('/workshop.html');
    await importPack(page, createStoreZip([
      { path: 'scenarios.toml', text: WORKSHOP_MANIFEST }, { path: WORLD, text: WORLD_TEXT },
      { path: HULL, text: HULL_TEXT }, { path: BASE, text: BASE_TEXT }, { path: DEPOT, text: DEPOT_TEXT },
    ]));
    await byId(page, 'capacities').selectOption('ammo');
    await byId(page, 'amount').fill('6'); await byId(page, 'ceiling').fill('24');
    await byId(page, 'capacity-apply').click();
    await expect(byId(page, 'status')).toContainText('materialise');
    expect(await sourceOf(page)).toBe(HULL_TEXT);
    await resourceForm(page); await byId(page, 'capacities').selectOption('ammo');
    await byId(page, 'materialise').check();
    await byId(page, 'amount').fill('6'); await byId(page, 'ceiling').fill('24');
    await applied(page, 'capacity-apply');
    const afterStock = await sourceOf(page);
    expect(parse(afterStock).infrastructure.capacity.find(row => row.id === 'ammo')).toEqual({ id: 'ammo', amount: 6, ceiling: 24 });
    await page.locator('#workshop-undo').click(); await expect.poll(() => sourceOf(page)).toBe(HULL_TEXT);
    await page.locator('#workshop-redo').click(); await expect.poll(() => sourceOf(page)).toBe(afterStock);

    // A named service can be switched between supported endpoint kinds. No
    // inherited capacity id may survive a switch to the magazine variant.
    await resourceForm(page); await byId(page, 'services').selectOption('torpedoes');
    await byId(page, 'materialise').check(); await byId(page, 'service-kind').selectOption('torpedo_magazine');
    await applied(page, 'service-apply');
    expect(parse(await sourceOf(page)).resource_services.torpedoes).toEqual({ kind: 'torpedo_magazine' });
    await resourceForm(page); await byId(page, 'materialise').check();
    await byId(page, 'link-kind').selectOption('service'); await byId(page, 'link-service').selectOption('torpedoes');
    await applied(page, 'link-apply');
    expect(parse(await sourceOf(page)).umbilical.link).toEqual({ service: 'torpedoes' });

    // The real runtime rejects a missing service reference; its refused source
    // must never replace the last accepted draft.
    const beforeRefusal = await sourceOf(page);
    await resourceForm(page);
    // Exercise the authoritative validator with an invalid requested service.
    // The ordinary select only offers known services, so inject this negative
    // test value without bypassing the panel's accepted-source route.
    await byId(page, 'link-kind').selectOption('service');
    await byId(page, 'link-service').evaluate(select => {
      const option = document.createElement('option'); option.value = 'absent-service'; option.textContent = 'Absent service';
      select.append(option); select.value = option.value;
    });
    await byId(page, 'link-apply').click();
    await expect(byId(page, 'status')).toContainText('not declared');
    expect(await sourceOf(page)).toBe(beforeRefusal);
    await resourceForm(page); await byId(page, 'count').fill('32'); await byId(page, 'materialise').check();
    await applied(page, 'magazine-apply');
    const beforeWeapon = await sourceOf(page);
    // Common equipment uses the same source, history and Rust validation as
    // stores. Select the first beam without coupling the journey to its id.
    await revealWorkshopPanel(page, 'composition');
    const weapon = id => page.locator('#workshop-weapon-' + id);
    await weapon('template').selectOption(HULL);
    await expect(weapon('save')).toBeEnabled();
    const beam = await weapon('existing').evaluate(select => [...select.options].find(option => /\(Beam\)$/.test(option.textContent))?.value);
    expect(beam).toBeDefined(); await weapon('existing').selectOption(beam);
    const weaponId = await weapon('id').inputValue(), range = Number(await weapon('range').inputValue());
    await weapon('range').fill(String(range + 1)); await weapon('save').click();
    await expect(weapon('status')).toContainText('materialise');
    expect(await sourceOf(page)).toBe(beforeWeapon);
    await revealWorkshopPanel(page, 'composition'); await weapon('materialise').check();
    await weapon('save').click(); await expect(weapon('status')).toHaveText(ts('workshop.changed'));
    const afterWeapon = await sourceOf(page);
    expect(parse(afterWeapon).weapon.find(row => row.id === weaponId).mount.range).toBe(range + 1);
    await page.locator('#workshop-undo').click(); await expect.poll(() => sourceOf(page)).toBe(beforeWeapon);
    await page.locator('#workshop-redo').click(); await expect.poll(() => sourceOf(page)).toBe(afterWeapon);
    await revealWorkshopPanel(page, 'composition'); await weapon('materialise').check();
    await weapon('fire_arc_deg').fill('-1'); await weapon('save').click();
    await expect(weapon('status')).toContainText('refused');
    expect(await sourceOf(page)).toBe(afterWeapon);
    // Local energy/thermal authoring is another accepted-source operation;
    // changing inherited hardware requires explicit profile materialisation.
    await revealWorkshopPanel(page, 'composition');
    const energy = id => page.locator('#workshop-energy-' + id);
    await energy('template').selectOption(HULL);
    await expect(energy('apply')).toBeEnabled();
    const consumer = await energy('subject').evaluate(select => [...select.options].find(option => option.value.startsWith('consumer:'))?.value);
    expect(consumer).toBeTruthy(); const consumerId = consumer.slice('consumer:'.length);
    await energy('subject').selectOption(consumer);
    const reserve = Number(await energy('reserve-capacity').inputValue());
    await energy('reserve-capacity').fill(String(reserve + 10)); await energy('apply').click();
    await expect(energy('status')).toContainText('materialise');
    expect(await sourceOf(page)).toBe(afterWeapon);
    await revealWorkshopPanel(page, 'composition'); await energy('materialise').check();
    await energy('apply').click(); await expect(energy('status')).toHaveText(ts('workshop.changed'));
    const afterEnergy = await sourceOf(page);
    expect(parse(afterEnergy).energy.consumer.find(row => row.system === consumerId).reserve.capacity).toBe(reserve + 10);
    await page.locator('#workshop-undo').click(); await expect.poll(() => sourceOf(page)).toBe(afterWeapon);
    await page.locator('#workshop-redo').click(); await expect.poll(() => sourceOf(page)).toBe(afterEnergy);
    await revealWorkshopPanel(page, 'composition'); await energy('materialise').check();
    await energy('reserve-trip_heat').fill('0'); await energy('apply').click();
    await expect(energy('status')).toContainText('refused');
    expect(await sourceOf(page)).toBe(afterEnergy);
    // The same actual validator accepts the redesigned shield, layout, remedy,
    // logistics and fabrication edits. Every accepted operation participates in
    // history before this exact combined source is exported and run below.
    const editedFields = [];
    async function editMechanism(family, selection, field, save, record) {
      await revealWorkshopPanel(page, 'composition');
      const control = id => page.locator(`#workshop-${family}-${id}`);
      await control('template').selectOption(HULL);
      await expect(control(save)).toBeEnabled();
      const selected = await control(selection).inputValue();
      const before = await sourceOf(page);
      await revealWorkshopPanel(page, 'composition');
      await control('materialise').check();
      const next = Number(await control(field).inputValue()) + 1;
      await control(field).fill(String(next));
      await control(save).click();
      await expect(control('status')).toHaveText(ts('workshop.changed'));
      const after = await sourceOf(page);
      expect(after).not.toBe(before);
      const binding = record(parse(after), selected);
      expect(binding.value).toBeCloseTo(next);
      editedFields.push({family, selection, field, save, binding});
      await page.locator('#workshop-undo').click();
      await expect.poll(() => sourceOf(page)).toBe(before);
      await page.locator('#workshop-redo').click();
      await expect.poll(() => sourceOf(page)).toBe(after);
    }
    await editMechanism('shield', 'existing', 'recharge_energy_per_hp', 'save', (value, id) => {
      const arc = value.shield_arc.find(arc => arc.id === id);
      return {id: arc.id, value: arc.energy.recharge_energy_per_hp};
    });
    await editMechanism('interior', 'rooms', 'max_hp', 'save', (value, id) => {
      const room = value.interior.compartment.find(room => room.id === id);
      return {id: room.id, value: room.max_hp};
    });
    await editMechanism('damage', 'remedies', 'work-secs', 'save-remedy', (value, id) => {
      const remedy = value.damage.remedies[id];
      return {id, value: remedy.work_secs};
    });
    await editMechanism('logistics', 'handling-system', 'units-per-sec', 'save-handling', (value, id) => {
      const handling = value.logistics.handling.find(handling => handling.system === id);
      return {id: handling.system, value: handling.units_per_sec};
    });
    await editMechanism('production', 'recipes', 'work-secs', 'save-recipe', (value, id) => {
      const recipe = value.production.recipe.find(recipe => recipe.id === id);
      return {id: recipe.id, value: recipe.work_secs};
    });
    const edited = await sourceOf(page);
    expect(parse(edited).torpedoes.count).toBe(32);
    expect(await sourceOf(page, BASE)).toBe(BASE_TEXT);

    const download = page.waitForEvent('download'); await page.locator('#workshop-export').click();
    const bytes = new Uint8Array(readFileSync(await (await download).path()));
    expect(readStoreZip(bytes)[HULL].replace(/\r\n/g, '\n')).toBe(edited);
    expect(readStoreZip(bytes)[BASE].replace(/\r\n/g, '\n')).toBe(BASE_TEXT);
    await importPack(page, bytes); expect(await sourceOf(page)).toBe(edited);
    // Reopened forms must read the saved coefficients, not their old DOM values.
    for (const {family, selection, field, save, binding} of editedFields) {
      await revealWorkshopPanel(page, 'composition');
      await page.locator(`#workshop-${family}-template`).selectOption(HULL);
      await expect(page.locator(`#workshop-${family}-${save}`)).toBeEnabled();
      await page.locator(`#workshop-${family}-${selection}`).selectOption(binding.id);
      await expect(page.locator(`#workshop-${family}-${field}`)).toHaveValue(String(binding.value));
    }
    await resourceForm(page); await expect(byId(page, 'count')).toHaveValue('32');
    await expect(byId(page, 'amount')).toHaveValue('6'); await expect(byId(page, 'ceiling')).toHaveValue('24');
    await expect(byId(page, 'link-service')).toHaveValue('torpedoes');

    // Preview reads the edited entity through the existing capture/viewer path.
    await revealWorkshopPanel(page, 'entity');
    await expect(page.locator('#workshop-entity-template')).toBeVisible();
    await page.locator('#workshop-entity-template').selectOption(HULL);
    await expect(page.locator('#workshop-entity-preview')).toBeEnabled();
    await page.locator('#workshop-entity-preview').click();
    await expect(page.locator('#workshop-preview-subject')).toHaveValue(HULL);
    await expect(page.locator('#workshop-model-preview-refresh')).toBeEnabled();
    await page.locator('#workshop-model-preview-refresh').click();
    // A starfield and rig bounds draw before the model settles. Require actual
    // measured mesh geometry before the pixel check and screenshot.
    await expect.poll(async () => {
      const stats = await page.locator('#workshop-model-preview-stats').textContent();
      return Number(stats.match(/Triangles: (\d+)/)?.[1] || 0);
    }, { timeout: 180_000 }).toBeGreaterThan(0);
    await expect(page.frameLocator('.workshop-model-preview-frame').locator('#canvas')).toBeVisible();
    await expect.poll(() => previewColours(page), { timeout: 60_000 }).toBeGreaterThan(1);
    await testInfo.attach('resource-preview', { body: await page.screenshot(), contentType: 'image/png' });

    // Disposable Test installs the same exported/reloaded source in ordinary
    // simulation; stopping it must leave every authored resource edit intact.
    await page.locator('#workshop-open-test').click();
    await page.locator('#workshop-test-world').selectOption(WORLD);
    await expect.poll(() => options(page.locator('#workshop-test-ship')), { timeout: 60_000 }).toContain(HULL);
    await page.locator('#workshop-test-ship').selectOption(HULL); await page.locator('#workshop-test-seed').fill('6');
    await page.locator('#workshop-test-start').click();
    try {
      await expect.poll(async () => await page.locator('#workshop-test-pause').isEnabled() ? 'running'
        : await page.locator('#workshop-test-status').getAttribute('role') === 'alert'
          ? await page.locator('#workshop-test-status').textContent() : 'starting', { timeout: 120_000 }).toBe('running');
      const frame = page.frames().find(item => /\/workshop-test(?:\.html)?$/.test(new URL(item.url() || 'about:blank').pathname));
      expect(frame).toBeTruthy();
      expect(await frame.evaluate(async () => JSON.parse((await import('/phoenix.js')).wasm_workshop_test_status())))
        .toMatchObject({ running: true, selection: { ship: HULL } });
    } finally { await page.locator('#workshop-test-stop').click(); }
    await expect(page.locator('.workshop-test-viewscreen')).toHaveCount(0);
    expect(await sourceOf(page)).toBe(edited); expect(errors).toEqual([]);
  });
