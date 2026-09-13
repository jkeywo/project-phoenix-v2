import { test, expect } from './fixtures';
import { workshopPack, WORKSHOP_WORLD, WORKSHOP_WORLD_TEXT } from '../fixtures/workshop-pack.js';
import { ts } from './strings';

const source = () => ({ selectedId: 'workshop-test', archives: [{ id: 'workshop-test', bytes: [...workshopPack()] }],
  base: { base_files: { 'assets/scenarios.toml': '[content]\nid="phoenix-base"\nepoch=1' }, base_asset_manifest: {} } });

test('Workshop source transfer is atomic and single-use in real browser storage', { tag: '@core' }, async ({ page }) => {
  await page.goto('/workshop.html');
  const result = await page.evaluate(async source => {
    const { createWorkshopHandoffStore } = await import('/editor/workshop-handoff.js');
    source.archives[0].bytes = Uint8Array.from(source.archives[0].bytes);
    const first = createWorkshopHandoffStore();
    const second = createWorkshopHandoffStore();
    const attempts = await Promise.allSettled([first.save(source), second.save(source)]);
    const accepted = attempts.find(result => result.status === 'fulfilled');
    const wrong = await second.take('a-different-token');
    const stored = await second.take(accepted.value);
    const duplicate = await first.take(accepted.value);
    return { states: attempts.map(result => result.status).sort(), wrong, duplicate,
      bytes: Array.from(stored.archives[0].bytes), selected: stored.selectedId };
  }, source());
  expect(result.states).toEqual(['fulfilled', 'rejected']);
  expect(result.wrong).toBeNull(); expect(result.duplicate).toBeNull();
  expect(result.bytes).toEqual([...workshopPack()]);
  expect(result.selected).toBe('workshop-test');
});

test('Workshop consumes retained source after navigation with a fresh Authoring history', { tag: '@core' }, async ({ page }) => {
  await page.goto('/workshop.html');
  const token = await page.evaluate(async source => {
    const { createWorkshopHandoffStore } = await import('/editor/workshop-handoff.js');
    source.archives[0].bytes = Uint8Array.from(source.archives[0].bytes);
    return createWorkshopHandoffStore().save(source);
  }, source());
  await page.goto(`/workshop.html#source=${token}`);
  // The smoke static server canonicalizes .html paths; both delivery forms
  // must discard the one-use token after consuming it.
  await expect(page).toHaveURL(/\/workshop(?:\.html)?$/);
  await page.locator('#workshop-files').selectOption(WORKSHOP_WORLD);
  await expect(page.locator('#workshop-source')).toHaveValue(WORKSHOP_WORLD_TEXT.replaceAll('\r\n', '\n'));
  await expect(page.locator('#workshop-undo')).toBeDisabled();
  await page.locator('#workshop-source').fill('# A new Authoring edit\n[global]\n');
  await expect(page.locator('#workshop-dirty')).toHaveText(ts('workshop.dirty'));
  await page.locator('#workshop-undo').click();
  await expect(page.locator('#workshop-source')).toHaveValue(WORKSHOP_WORLD_TEXT.replaceAll('\r\n', '\n'));
  expect(await page.evaluate(async token => (await import('/editor/workshop-handoff.js')).createWorkshopHandoffStore().take(token), token)).toBeNull();
  expect(await page.evaluate(() => typeof window.__hostLocalGm)).toBe('undefined');
});
