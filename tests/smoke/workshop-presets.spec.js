import { test, expect } from '@playwright/test';
import { createStoreZip } from '../../editor/mod-pack-export.js';
import { WORKSHOP_MANIFEST } from '../fixtures/workshop-pack.js';
import { revealWorkshopPanel } from './dock-helpers.js';

// Issue #1477 through the REAL runtime: the presets panel reads a world's
// [[gm_role_preset]] blocks from wasm_workshop_presets, locates a rule the
// runtime itself refuses at load, repairs it as one exact-source edit, and
// appends a new preset from the runtime's own serialised block. Unit tests of
// this panel mock the runtime; this is the run that does not.
const WORLD = 'assets/worlds/workshop.toml';
const SHIP = 'assets/entities/alliance_cruiser.toml';
// A widget with an EMPTY label: GmRolePresetWidget::validate refuses it at world
// load, so it is a real rule with a real line, and the form can repair it.
const BROKEN_LABEL_LINE = 16;
const WORLD_TEXT = `# Keep this comment above the presets
[global]
title = "Presets in the Workshop"
seed = 7
[[available_ships]]
template_path = "${SHIP}"

[[gm_role_preset]]
id = "watch"
label = "world.workshop.preset.watch"
panels = ["gm-map-panel"]

[[gm_role_preset.widget]]
id = "seats"
type = "workload"
label = ""
`;
const sourcePack = () => createStoreZip([
  { path: 'scenarios.toml', text: WORKSHOP_MANIFEST },
  { path: WORLD, text: WORLD_TEXT },
]);
const byId = (page, id) => page.locator(`#workshop-presets-${id}`);

/** The exact source of one member. `#workshop-files` lives in the files panel,
 * which shares its dock group with the presets panel, so the files tab has to be
 * brought back to the front before the select is reachable. */
async function sourceOf(page, member) {
  await revealWorkshopPanel(page, 'files');
  await page.locator('#workshop-files').selectOption(member);
  await revealWorkshopPanel(page, 'source');
  return page.locator('#workshop-source').inputValue();
}

test('the presets panel locates a runtime preset rule, repairs it as one exact-source edit and appends a new preset',
  { tag: '@core' }, async ({ page }) => {
    test.setTimeout(120_000);
    const errors = [];
    page.on('pageerror', error => errors.push(error.message));
    await page.goto('/workshop.html');
    const chooser = page.waitForEvent('filechooser');
    await page.locator('#workshop-import').click();
    await (await chooser).setFiles({ name: 'presets.zip', mimeType: 'application/zip', buffer: Buffer.from(sourcePack()) });

    // A reading from the runtime, with the empty widget label located at its line.
    await revealWorkshopPanel(page, 'presets');
    await byId(page, 'world').selectOption(WORLD);
    await byId(page, 'refresh').click();
    await expect(byId(page, 'findings')).toContainText(`${WORLD}:${BROKEN_LABEL_LINE}`);
    // And the ordinary Check refuses the same draft for the same reason, so an
    // export would too: the world would not load with that widget in it.
    await page.locator('#workshop-check').click();
    await expect(page.locator('.workshop-findings')).toContainText('workshop.toml');

    // Repair it through the form: ONE runtime edit of the world's exact source.
    await revealWorkshopPanel(page, 'presets');
    await byId(page, 'preset').selectOption({ index: 0 });
    await byId(page, 'widget-0-label').fill('world.workshop.widget.seats');
    await byId(page, 'apply').click();
    await expect.poll(() => sourceOf(page, WORLD)).toContain('label = "world.workshop.widget.seats"');
    const repaired = await sourceOf(page, WORLD);
    // Every other byte survived, in order: only the empty label changed.
    expect(repaired).toBe(WORLD_TEXT.replace('label = ""', 'label = "world.workshop.widget.seats"'));
    // One press undoes the whole edit, back to the exact bytes.
    await page.locator('#workshop-undo').click();
    await expect.poll(() => sourceOf(page, WORLD)).toBe(WORLD_TEXT);
    await page.locator('#workshop-redo').click();
    await expect.poll(() => sourceOf(page, WORLD)).toBe(repaired);

    // The finding is gone from the panel and from Check.
    await revealWorkshopPanel(page, 'presets');
    await byId(page, 'refresh').click();
    await expect(byId(page, 'findings')).not.toContainText(`${WORLD}:${BROKEN_LABEL_LINE}`);
    await page.locator('#workshop-check').click();
    await expect(page.locator('.workshop-findings')).not.toContainText('workshop.toml');

    // A new preset is the runtime's own serialised block, appended to the world
    // it is authored in and undone by one press.
    await revealWorkshopPanel(page, 'presets');
    await byId(page, 'add-preset-id').fill('tactical');
    await byId(page, 'add-preset-label').fill('world.workshop.preset.tactical');
    await byId(page, 'add-preset-button').click();
    await expect.poll(() => sourceOf(page, WORLD)).toContain('id = "tactical"');
    const appended = await sourceOf(page, WORLD);
    expect(appended.startsWith(repaired)).toBe(true);
    expect((appended.match(/\[\[gm_role_preset\]\]/g) || []).length).toBe(2);
    // The reserved id is refused before the runtime is asked, and the draft is
    // left exactly as it was.
    await revealWorkshopPanel(page, 'presets');
    await byId(page, 'refresh').click();
    await byId(page, 'add-preset-id').fill('all');
    await byId(page, 'add-preset-label').fill('world.workshop.preset.all');
    await byId(page, 'add-preset-button').click();
    await expect(byId(page, 'status')).toHaveAttribute('role', 'alert');
    expect(await sourceOf(page, WORLD)).toBe(appended);
    await page.locator('#workshop-undo').click();
    await expect.poll(() => sourceOf(page, WORLD)).toBe(repaired);

    expect(errors).toEqual([]);
  });
