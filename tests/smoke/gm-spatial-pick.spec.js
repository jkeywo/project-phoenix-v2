import { test, expect, waitForWasmReady, captureServerPageErrors } from './fixtures';
import { ts } from './strings';
import { revealGmPanel } from './dock-helpers.js';

/**
 * Directed spatial picking on the docked Live map (issue #1508).
 *
 * The three things this pins that no unit test can: that pick mode reaches a
 * REAL chart through the real dock, that the surface puts its other floating
 * panels away and brings them back, and that a click with no meaningful drag
 * places facing the contextual default the panel had been previewing — in words
 * as well as degrees, so a forced-colours browser and a screen reader get it
 * too.
 */
test('a GM picks a place and a direction on the docked map', { tag: '@core' }, async ({ page }) => {
  test.setTimeout(180_000);
  const errors = captureServerPageErrors(page);
  await page.goto('/');
  await page.locator('#landing-menu [data-landing-entry="host_gm"]').click();
  const worlds = page.locator('#world-list .world-btn[data-scenario-id]');
  await expect(worlds.first()).toBeVisible({ timeout: 60_000 });
  await worlds.first().click();
  const ship = page.locator('ph-ship-picker .ship-card').first();
  await Promise.race([ship.waitFor({ state: 'visible', timeout: 60_000 }),
    page.locator('#landing-panel').waitFor({ state: 'hidden', timeout: 60_000 })]);
  if (await ship.isVisible()) await ship.click();
  await waitForWasmReady(page);
  await page.waitForFunction(() => !!window.__hostLocalGm?.());
  await page.locator('#gm-session-start').click();
  await page.locator('#gm-action-confirmation [data-confirmation-accept]').click();
  await page.waitForFunction(() => window.__saveSlotsPhase === 'InProgress');

  // Spawn is a temporary action panel: it opens as a floating draft.
  await revealGmPanel(page, 'spawn');
  const spawn = page.locator('[data-panel="spawn"]');
  await expect(spawn).toHaveClass(/is-floating/);
  const row = page.locator('#gm-spawn-palette .gm-spawn-entry').first();
  await expect(row).toBeVisible({ timeout: 30_000 });

  // A second floating panel, to watch it go away and come back.
  await page.locator('#gm-live-layout [data-panel="journal"] [data-layout-control="float"]').click();
  const journal = page.locator('[data-panel="journal"].is-floating');
  await expect(journal).toBeVisible();

  await row.locator('button[data-role="place"]').click();

  // Pick mode names the control capturing the map, and the chart says it is armed.
  const pick = page.locator('#gm-spawn-pick');
  await expect(pick).toHaveAttribute('data-picking', /.+/);
  await expect(page.locator('#gm-entity-map')).toHaveAttribute('data-placement-armed', '');
  // Every OTHER floating panel is away; the draft that is picking stays, because
  // it is the only thing showing the preview.
  await expect(journal).toBeHidden();
  await expect(spawn).toBeVisible();
  await expect(page.locator('#gm-map-panel')).toBeVisible();

  // Escape changes nothing about the draft and hands the floats back.
  await page.keyboard.press('Escape');
  await expect(page.locator('#gm-entity-map')).not.toHaveAttribute('data-placement-armed', '');
  await expect(journal).toBeVisible();
  await expect(pick).toHaveText('');

  // Pick again, and commit with a click and no meaningful drag: the placement
  // takes the contextual default direction the preview was showing.
  await row.locator('button[data-role="place"]').click();
  const box = await page.locator('#gm-entity-map').boundingBox();
  await page.mouse.move(box.x + box.width * 0.7, box.y + box.height * 0.4);
  await page.mouse.down();
  await expect(pick).toHaveAttribute('data-contextual', 'true');
  const previewed = await pick.textContent();
  // Degrees AND a compass word, so the direction survives forced colours.
  expect(previewed).toMatch(/\d+/);
  expect(previewed).toContain(ts('server.gm.spawn.pick_preview_default').split('{')[0].trim());
  await page.mouse.up();

  await expect(page.locator('#gm-spawn-log [data-outcome="applied"]')).toHaveCount(1, { timeout: 30_000 });
  await expect(journal).toBeVisible();
  await expect(pick).toHaveText('');
  expect(errors).toEqual([]);
});


/**
 * A ghost contact is placed with the same gesture (issue #1510).
 *
 * What no unit test can pin: that the ghost draft reaches the REAL chart
 * through the real dock, that one chart never serves two gestures at once, and
 * that a place picked in metres arrives in the draft as the canonical integer
 * millimetres the field has always taken — refused, not clamped, if it cannot.
 */
test('a GM places a ghost contact on the docked map', { tag: '@core' }, async ({ page }) => {
  test.setTimeout(180_000);
  const errors = captureServerPageErrors(page);
  await page.goto('/');
  await page.locator('#landing-menu [data-landing-entry="host_gm"]').click();
  const worlds = page.locator('#world-list .world-btn[data-scenario-id]');
  await expect(worlds.first()).toBeVisible({ timeout: 60_000 });
  await worlds.first().click();
  const ship = page.locator('ph-ship-picker .ship-card').first();
  await Promise.race([ship.waitFor({ state: 'visible', timeout: 60_000 }),
    page.locator('#landing-panel').waitFor({ state: 'hidden', timeout: 60_000 })]);
  if (await ship.isVisible()) await ship.click();
  await waitForWasmReady(page);
  await page.waitForFunction(() => !!window.__hostLocalGm?.());
  await page.locator('#gm-session-start').click();
  await page.locator('#gm-action-confirmation [data-confirmation-accept]').click();
  await page.waitForFunction(() => window.__saveSlotsPhase === 'InProgress');

  // Creating a ghost is a complex action, so it opens as a floating draft.
  await revealGmPanel(page, 'contact');
  await page.waitForFunction(
    () => document.querySelectorAll('#gm-contact-observer option').length > 1,
    null, { timeout: 60_000 });
  const observer = await page.locator('#gm-contact-observer option').nth(1).getAttribute('value');
  await page.locator('#gm-contact-observer').selectOption(observer);
  await revealGmPanel(page, 'ghost');
  const ghost = page.locator('[data-panel="ghost"]');
  await expect(ghost).toHaveClass(/is-floating/);
  await page.locator('#gm-contact-ghost-id').fill('smoke-ghost');
  await page.locator('#gm-contact-ghost-palette')
    .selectOption({ index: 1 });

  const pick = page.locator('#gm-contact-ghost-pick');
  const status = page.locator('#gm-contact-ghost-pick-status');
  await pick.click();
  await expect(page.locator('#gm-entity-map')).toHaveAttribute('data-placement-armed', '');
  await expect(status).toHaveText(ts('server.gm.contact.ghost_pick_armed'));

  // One chart, one gesture: arming Spawn ends the ghost pick rather than
  // leaving two panels listening for the same click.
  await revealGmPanel(page, 'spawn');
  await page.locator('#gm-spawn-palette .gm-spawn-entry').first()
    .locator('button[data-role="place"]').click();
  await expect(status).toHaveText('');
  await expect(page.locator('#gm-spawn-pick')).toHaveAttribute('data-picking', /.+/);
  await page.keyboard.press('Escape');

  // Pick the ghost's place on the real chart.
  await revealGmPanel(page, 'ghost');
  await pick.click();
  const box = await page.locator('#gm-entity-map').boundingBox();
  await page.mouse.click(box.x + box.width * 0.65, box.y + box.height * 0.35);
  await expect(status).toHaveText('');
  // Metres in, canonical integer millimetres out, in the field that has always
  // carried those bounds.
  for (const axis of ['x', 'z']) {
    const field = page.locator(`#gm-contact-ghost-${axis}`);
    await expect(field).toHaveAttribute('max', '2147483647');
    expect(Number(await field.inputValue())).toEqual(Math.trunc(Number(await field.inputValue())));
  }
  await page.locator('#gm-contact-ghost-set').click();
  await expect(page.locator('#gm-contact-ghost-feedback'))
    .toHaveAttribute('data-state', 'applied', { timeout: 30_000 });
  // A landed draft closes, and what it created is a record on the tool that is
  // always there — not behind reopening a temporary panel.
  await expect(page.locator('[data-panel="ghost"]')).toHaveCount(0);
  await expect(page.locator('#gm-contact-ghosts li')).toHaveCount(1);

  // Removing one is a simple action: load the ghost into the draft and remove
  // it, and the draft stays open because nothing about it was composed.
  await revealGmPanel(page, 'ghost');
  await page.locator('#gm-contact-ghosts button').first().click();
  await expect(page.locator('#gm-contact-ghost-id')).toHaveValue('smoke-ghost');
  await page.locator('#gm-contact-ghost-remove').click();
  await expect(page.locator('#gm-contact-ghosts li')).toHaveCount(0, { timeout: 30_000 });
  await expect(page.locator('[data-panel="ghost"]')).toHaveCount(1);
  expect(errors).toEqual([]);
});
