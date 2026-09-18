import { test, expect } from '@playwright/test';
import { readFileSync } from 'node:fs';
import path from 'node:path';
import { createStoreZip } from '../../editor/mod-pack-export.js';
import { WORKSHOP_MANIFEST } from '../fixtures/workshop-pack.js';
import { revealWorkshopPanel } from './dock-helpers.js';

// Issue #1474 through the REAL runtime: the definitions panel reads its
// catalog from wasm_workshop_definitions, writes through wasm_workshop_edit
// and the findings come from the same validator that gates export. Every unit
// test of the panel mocks that runtime; this is the run that does not.
const WORLD = 'assets/worlds/workshop.toml';
const FACTION = 'assets/factions/workshop_only.toml';
const HULL = 'assets/entities/workshop_cruiser.toml';
const ALLIANCE = 'aaaaaaaa-1111-4111-8111-aaaaaaaaaaaa';
const UNKNOWN = '00000000-0000-4000-8000-000000000000';
// The faction starts by naming an enemy nobody declares: a real cross-file
// error, on line 4, that the panel must locate and the Check must refuse.
const FACTION_TEXT = `# Only this draft declares it\nuuid = "ffffffff-1474-4474-8474-ffffffffffff"\nname = "Workshop Only"\nenemies = ["${UNKNOWN}"]\n`;
// A complete shipped hull under a draft path, so its rungs are editable.
// Resolved from this file: playwright runs specs with tests/smoke as its cwd.
const HULL_TEXT = readFileSync(path.join(__dirname, '../../assets/entities/alliance_cruiser.toml'), 'utf8');
const sourcePack = () => createStoreZip([
  { path: 'scenarios.toml', text: WORKSHOP_MANIFEST },
  { path: WORLD, text: '[global]\n[anchors]\n' },
  { path: FACTION, text: FACTION_TEXT },
  { path: HULL, text: HULL_TEXT },
]);
const byId = (page, id) => page.locator(`#workshop-definitions-${id}`);

async function sourceOf(page, member) {
  await page.locator('#workshop-files').selectOption(member);
  await revealWorkshopPanel(page, 'source');
  return page.locator('#workshop-source').inputValue();
}

test('the definitions panel edits exact faction and rung source through the real runtime and locates cross-file errors',
  { tag: '@core' }, async ({ page }) => {
    test.setTimeout(120_000);
    const errors = [];
    page.on('pageerror', error => errors.push(error.message));
    await page.goto('/workshop.html');
    const chooser = page.waitForEvent('filechooser');
    await page.locator('#workshop-import').click();
    await (await chooser).setFiles({ name: 'definitions.zip', mimeType: 'application/zip', buffer: Buffer.from(sourcePack()) });

    // A reading from the runtime, with the unknown enemy located at its line.
    await revealWorkshopPanel(page, 'definitions');
    await byId(page, 'refresh').click();
    await expect(byId(page, 'status')).not.toHaveAttribute('role', 'alert');
    await expect(byId(page, 'findings')).toContainText(`${FACTION}:4`);
    // And the ordinary Check refuses the same draft for the same reason, so
    // export would too.
    await page.locator('#workshop-check').click();
    await expect(page.locator('.workshop-findings')).toContainText('workshop_only.toml');

    // Faction: drop the unknown enemy, add the shipped Alliance from the
    // runtime's choices, apply as ONE edit.
    await byId(page, 'faction').selectOption(FACTION);
    await expect(byId(page, 'name')).toHaveValue('Workshop Only');
    await byId(page, 'enemy-remove-0').click();
    await byId(page, 'add-enemy').selectOption(ALLIANCE);
    await byId(page, 'add-enemy-button').click();
    await expect(byId(page, 'enemies').locator('li')).toHaveCount(1);
    await byId(page, 'apply-faction').click();
    await expect.poll(() => sourceOf(page, FACTION)).toContain(`enemies = ["${ALLIANCE}"]`);
    const edited = await sourceOf(page, FACTION);
    // Every other byte survived: the comment, the uuid and the name lines.
    expect(edited.split('\n').slice(0, 3)).toEqual(FACTION_TEXT.split('\n').slice(0, 3));
    // One press undoes the whole grouped edit, back to the exact bytes.
    await page.locator('#workshop-undo').click();
    await expect.poll(() => sourceOf(page, FACTION)).toBe(FACTION_TEXT);
    await page.locator('#workshop-redo').click();
    await expect.poll(() => sourceOf(page, FACTION)).toBe(edited);

    // The finding is gone from the panel and from Check.
    await revealWorkshopPanel(page, 'definitions');
    await byId(page, 'refresh').click();
    await expect(byId(page, 'findings')).not.toContainText('workshop_only.toml');
    await page.locator('#workshop-check').click();
    await expect(page.locator('.workshop-findings')).not.toContainText('workshop_only.toml');

    // Complexity: append a rung to the first station of the draft hull. The
    // new [[station.rating]] must land after that station's existing rungs,
    // and every other byte of a 2000-line file must survive.
    await revealWorkshopPanel(page, 'definitions');
    await byId(page, 'hull').selectOption(HULL);
    // Stations are offered by their position in the hull; the first is "0".
    await byId(page, 'station').selectOption('0');
    const rungsBefore = (HULL_TEXT.match(/\[\[station\.rating\]\]/g) || []).length;
    await byId(page, 'new-rung').fill('Assisted');
    await byId(page, 'add-rung').click();
    await byId(page, 'apply-hull').click();
    await expect.poll(() => sourceOf(page, HULL)).toContain('name = "Assisted"');
    const hull = await sourceOf(page, HULL);
    expect((hull.match(/\[\[station\.rating\]\]/g) || []).length).toBe(rungsBefore + 1);
    // The new rung sits inside the first station: before the second
    // [[station]] header, after the first one.
    const firstStation = hull.indexOf('[[station]]');
    const secondStation = hull.indexOf('[[station]]', firstStation + 1);
    const assisted = hull.indexOf('name = "Assisted"');
    expect(assisted).toBeGreaterThan(firstStation);
    expect(assisted).toBeLessThan(secondStation);
    // Every original line survives, in order: take the three inserted lines
    // out of the result and what is left is the input. The emitter separates
    // the new table from its neighbours with a blank line, and one of those
    // may stand beside the blank line the input already had — that single
    // extra blank is the only difference allowed.
    const lines = hull.split('\n');
    const at = lines.indexOf('name = "Assisted"');
    expect(lines[at - 1]).toBe('[[station.rating]]');
    expect(lines[at + 1]).toBe('automated_systems = []');
    const rest = [...lines.slice(0, at - 1), ...lines.slice(at + 2)];
    const original = HULL_TEXT.split('\n');
    if (rest.length === original.length + 1) {
      const extra = rest.findIndex((line, index) => line === '' && original[index] !== '');
      expect(extra, 'the only surplus line is a blank one').toBeGreaterThanOrEqual(0);
      rest.splice(extra, 1);
    }
    expect(rest).toEqual(original);
    await page.locator('#workshop-undo').click();
    await expect.poll(() => sourceOf(page, HULL)).toBe(HULL_TEXT);

    expect(errors).toEqual([]);
  });
