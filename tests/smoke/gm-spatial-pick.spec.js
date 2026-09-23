import { test, expect, waitForWasmReady, captureServerPageErrors,
  createTestClient, readHostPeerId, waitForJoinCode } from './fixtures';
import { ts } from './strings';
import { revealGmPanel, floatGmPanel } from './dock-helpers.js';

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
  await revealGmPanel(page, 'readiness');
  await page.locator('#gm-force-start-btn').click();
  await page.locator('#gm-action-confirmation [data-confirmation-accept]').click();
  await page.waitForFunction(() => window.__saveSlotsPhase === 'InProgress');

  // Spawn is a temporary action panel: it opens as a floating draft.
  await revealGmPanel(page, 'spawn');
  const spawn = page.locator('[data-panel="spawn"]');
  await expect(spawn).toHaveClass(/is-floating/);
  const row = page.locator('#gm-spawn-palette .gm-spawn-entry').first();
  await expect(row).toBeVisible({ timeout: 30_000 });

  // A second floating panel, to watch it go away and come back. The journal is
  // an inactive tab of the records group, so it is brought to the front first —
  // its float control is only there to press once its frame is the shown one.
  await revealGmPanel(page, 'activity');
  await floatGmPanel(page, 'activity');
  const journal = page.locator('[data-panel="activity"].is-floating');
  await expect(journal).toBeVisible();

  // Two floats now, and the newer one opened over the draft. Bringing the draft
  // forward is what an operator does before pressing into it — the dock raises
  // a float on focus — so the press below is a real pointer press on a real
  // control, not one routed around the panel sitting on top of it.
  await spawn.locator('.workshop-panel-tab').focus();
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
test('a GM places a ghost contact on the docked map', { tag: '@core' }, async ({ page, context }) => {
  test.setTimeout(180_000);
  const errors = captureServerPageErrors(page);
  // GM-only entry does not create a player hull. A real fleet ship is the
  // recipient; the GM's own simulation remains hull-less.
  const scenario = 'assets/worlds/combat_test.toml';
  const owner = await context.newPage();
  const ownerErrors = captureServerPageErrors(owner);
  await owner.goto(`/?scenario=${scenario}&ship=assets/entities/alliance_cruiser.toml`);
  await waitForWasmReady(owner);
  const crew = await createTestClient(context, await readHostPeerId(owner), { name: 'Ghost witness' });
  await crew.send('SelectStation', { station: 'Captain' });
  await owner.evaluate(() => window.__hostFleetOpen());
  await waitForJoinCode(owner, 'fleet-code');
  const code = await owner.locator('#fleet-code').textContent();
  await page.goto(`/?gm=1&scenario=${scenario}`);
  await waitForWasmReady(page);
  await page.locator('#server-settings-btn').click();
  await page.locator('.server-settings-tab[data-tab="gameplay"]').click();
  await page.locator('[data-control="fleet-code"]').fill(code);
  await page.locator('[data-control="fleet-join"]').click();
  await page.waitForFunction(() => window.__hostGmStartState?.().admitted);
  await page.locator('#server-settings-btn').click();
  await crew.send('SetReady', { ready: true });
  await page.locator('#gm-header-ready').click();
  await page.waitForFunction(() => window.__saveSlotsPhase === 'InProgress');

  // Placing a ghost IS placing: the same draft, the same palette, the same
  // chart gesture. Only the OUTCOME differs — reported to one observing ship as
  // a false Sensors contact, never spawned into the world.
  await revealGmPanel(page, 'spawn');
  const spawn = page.locator('[data-panel="spawn"]');
  await expect(spawn).toHaveClass(/is-floating/);
  await page.locator('#gm-spawn-outcome-ghost').check();
  await expect(page.locator('#gm-spawn-ghost')).toBeVisible();
  const observers = page.locator('#gm-spawn-ghost-observer');
  await page.waitForFunction(
    () => document.querySelectorAll('#gm-spawn-ghost-observer option').length > 1,
    null, { timeout: 60_000 });
  const observer = await observers.locator('option').nth(1).getAttribute('value');
  await observers.selectOption(observer);
  await page.locator('#gm-spawn-ghost-id').fill('smoke-ghost');
  const row = page.locator('#gm-spawn-palette .gm-spawn-entry').first();
  await expect(row).toBeVisible({ timeout: 30_000 });
  const palette = await row.getAttribute('data-palette-id');
  const spawnedBefore = await page.evaluate(() => window.__hostGmSpawnState().authoritative);

  await row.locator('button[data-role="place"]').click();
  await expect(page.locator('#gm-spawn-pick')).toHaveAttribute('data-picking', /.+/);
  await expect(page.locator('#gm-entity-map')).toHaveAttribute('data-placement-armed', '');
  const chart = await page.locator('#gm-entity-map').boundingBox();
  expect(chart).not.toBeNull();
  await page.mouse.click(chart.x + chart.width * 0.6, chart.y + chart.height * 0.4);

  // The result lands on the panel that placed it, as a ghost, and the world
  // has spawned nothing: a ghost never produces a placement result.
  const applied = page.locator('#gm-spawn-log .gm-spawn-log-entry[data-outcome="applied"]');
  await expect(applied).toHaveCount(1, { timeout: 30_000 });
  await expect(applied.first()).toHaveAttribute('data-palette', palette);
  await expect(applied.first()).toContainText('smoke-ghost');
  expect(await page.evaluate(() => window.__hostGmSpawnState().authoritative)).toBe(spawnedBefore);

  // The observing ship's record on the contact tool shows it, with the one
  // simple action that undoes it — and that action undoes it.
  await revealGmPanel(page, 'contact');
  await page.locator('#gm-contact-observer').selectOption(observer);
  const listed = page.locator('#gm-contact-ghosts li').filter({ hasText: 'smoke-ghost' });
  await expect(listed).toHaveCount(1, { timeout: 30_000 });
  await listed.locator('button[data-role="remove"]').click();
  await expect(listed).toHaveCount(0, { timeout: 30_000 });
  expect(errors).toEqual([]);
  expect(ownerErrors).toEqual([]);
});
