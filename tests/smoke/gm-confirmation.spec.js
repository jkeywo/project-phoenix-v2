import { readFile } from 'node:fs/promises';
import {
  captureServerPageErrors, createTestClient, expect, expectFixtureWorld,
  readHostPeerId, test, waitForJoinCode, waitForWasmReady,
} from './fixtures';
import { ts } from './strings';

const WORLD = `
[global]
seed = 1315
title = "GM confirmation smoke fixture"
description = "Private policies on equal GMs with ordinary damage results."
[[available_ships]]
template_path = "assets/entities/alliance_cruiser.toml"
[[entity]]
template_path = "assets/entities/alliance_cruiser.toml"
id = "player-ship"
spawn_on = "game_start"
[[entity]]
template_path = "assets/entities/alliance_courier.toml"
name = "Confirmation courier"
spawn_on = "game_start"
transform = { position = [400.0, 0.0, 0.0] }
overrides = { name = "Confirmation courier", display_name = "Confirmation courier" }
`;

async function settings(page, tab = 'controls') {
  await page.bringToFront();
  if (!await page.locator('#server-settings-overlay').isVisible()) {
    await page.locator('#server-settings-btn').click();
  }
  await page.locator(`.server-settings-tab[data-tab="${tab}"]`).click();
}
async function closeSettings(page) {
  await page.locator('#server-settings-btn').click();
  await expect(page.locator('#server-settings-overlay')).toBeHidden();
}
async function mode(page, category, value) {
  await settings(page);
  await page.locator(`[data-gm-confirmation-category="${category}"]`).selectOption(value);
  await closeSettings(page);
}
async function selectCourier(page) {
  const map = page.locator('#gm-entity-map');
  await map.scrollIntoViewIfNeeded();
  await map.focus();
  for (let i = 0; i < 20; i++) {
    await map.press('ArrowRight');
    if (await page.evaluate(() => {
      const map = document.getElementById('gm-entity-map');
      const id = window.__hostGmEffectState?.().selected;
      return map.state.blips.some(b => b.uuid === id && b.name === 'Confirmation courier');
    })) return;
  }
  throw new Error('The real map did not select the authored courier');
}

test('two equal GMs keep private confirmation policies and resolve stale lethal previews normally', { tag: '@core' }, async ({ context }) => {
  test.setTimeout(180_000);
  await context.route('**/assets/worlds/default.toml', route => route.fulfill({ contentType: 'text/plain', body: WORLD }));
  const host = await context.newPage();
  const errors = [captureServerPageErrors(host)];
  await host.goto('/?scenario=assets/worlds/default.toml');
  await waitForWasmReady(host);
  const crew = await createTestClient(context, await readHostPeerId(host), { name: 'Confirmation witness' });
  await crew.send('SelectStation', { station: 'Captain' });
  await host.evaluate(() => window.__hostFleetOpen());
  await waitForJoinCode(host, 'fleet-code');
  const code = await host.locator('#fleet-code').textContent();
  const gms = [];
  for (let i = 0; i < 2; i++) {
    const page = await context.newPage();
    errors.push(captureServerPageErrors(page));
    // Separate live operators in the shared transport fixture's browser store.
    await page.addInitScript(() => localStorage.removeItem('phoenix.fleet.gm-identity.v1'));
    await page.goto('/?gm=1&scenario=assets/worlds/default.toml');
    await waitForWasmReady(page);
    await settings(page, 'gameplay');
    await page.locator('[data-control="fleet-code"]').fill(code);
    await page.locator('[data-control="fleet-join"]').click();
    await page.waitForFunction(() => {
      const state = window.__hostGmStartState?.();
      return state?.admitted && state.presentationReady && state.localValidation;
    });
    await closeSettings(page);
    gms.push(page);
  }
  const [one, two] = gms;
  const [oneId, twoId] = await Promise.all(gms.map(page => page.evaluate(() => window.__hostGmStartState().operatorId)));
  expect(oneId).not.toBe(twoId);
  await crew.send('SetReady', { ready: true });
  for (const page of gms) await page.locator('#gm-ready-btn').click();
  await Promise.all([host, ...gms].map(page => page.waitForFunction(() => window.__saveSlotsPhase === 'InProgress')));
  expectFixtureWorld(await crew.waitForMessage('WorldSetup'), WORLD);
  for (const page of gms) await selectCourier(page);
  const target = await one.locator('#gm-entity-card').getAttribute('data-entity-id');
  const ownRows = (page, id) => page.locator(`#gm-effect-log [data-operator-id="${id}"]`);
  const dialog = page => page.locator('#gm-action-confirmation');

  await mode(one, 'effect.damage', 'immediate');
  await settings(two);
  await expect(two.locator('[data-gm-confirmation-category="effect.damage"]')).toHaveValue('confirm');
  await closeSettings(two);
  await one.locator('#gm-effect-amount').fill('5');
  await one.locator('#gm-effect-damage').click();
  await expect(dialog(one)).toBeHidden();
  await expect(ownRows(one, oneId)).toHaveCount(1);
  await expect(ownRows(one, oneId).first()).toHaveAttribute('data-outcome', 'applied');
  await expect(ownRows(one, oneId).first()).toHaveAttribute('data-applied', '5000');

  await two.locator('#gm-effect-amount').fill('5');
  await two.locator('#gm-effect-damage').click();
  await expect(dialog(two)).toHaveAttribute('data-mode', 'confirm');
  expect(await two.evaluate(() => window.__hostGmEffectState().pending)).toBe(0);
  await two.locator('[data-confirmation-cancel]').click();
  await expect(ownRows(two, twoId)).toHaveCount(0);
  await two.locator('#gm-effect-damage').click();
  await two.locator('[data-confirmation-accept]').click();
  await expect(ownRows(two, twoId)).toHaveCount(1);
  await expect(ownRows(two, twoId).first()).toHaveAttribute('data-outcome', 'applied');

  // The actual file controls round-trip the private profile. Changing this
  // page's imported choice leaves the other live operator's choice alone.
  await settings(one);
  const downloadEvent = one.waitForEvent('download');
  await one.locator('[data-gm-profile-export]').click();
  const download = await downloadEvent;
  const exported = await readFile(await download.path(), 'utf8');
  await closeSettings(one);
  await settings(two);
  await two.locator('[data-gm-profile-import]').setInputFiles({
    name: 'operator-profile.json', mimeType: 'application/json', buffer: Buffer.from(exported),
  });
  await expect(two.locator('[data-gm-confirmation-category="effect.damage"]')).toHaveValue('immediate');
  await two.locator('[data-gm-confirmation-category="effect.damage"]').selectOption('confirm');
  await closeSettings(two);
  await settings(one);
  await expect(one.locator('[data-gm-confirmation-category="effect.damage"]')).toHaveValue('immediate');
  await closeSettings(one);

  await mode(one, 'effect.lethal', 'immediate');
  await two.locator('#gm-effect-amount').fill('99999');
  await two.locator('#gm-effect-damage').click();
  await expect(dialog(two)).toHaveAttribute('data-mode', 'confirm-preview');
  await expect(two.locator('[data-confirmation-preview]')).toContainText(ts('settings.gm.confirmation.destroys'));
  // Another equal GM removes the target while this captured intent is open.
  await one.locator('#gm-effect-amount').fill('99999');
  await one.locator('#gm-effect-damage').click();
  await expect(one.locator(`#gm-effect-log [data-operator-id="${oneId}"][data-destroyed="true"]`)).toHaveCount(1);
  await expect(two.locator('[data-confirmation-preview]')).toHaveText(ts('settings.gm.confirmation.preview_unavailable'));
  await two.locator('[data-confirmation-accept]').click();
  const stale = two.locator(`#gm-effect-log [data-operator-id="${twoId}"][data-outcome="refused"]`);
  await expect(stale).toHaveCount(1);
  await expect(stale).toHaveAttribute('data-entity', target);
  await expect(two.locator('#gm-effect-feedback')).toHaveAttribute('data-state', 'Refused');
  await expect(two.locator('#gm-activity-list [data-category="gm_action"]').filter({ hasText: target }).last()).toBeVisible();
  for (const captured of errors) expect(captured).toEqual([]);
  await crew.close();
});
