import { test, expect } from './fixtures';
import { energyConsolePayload } from '../fixtures/energy-console.mjs';
import { ts } from './strings';

const URL = '/client/gui/battleship/power.html';
async function open(page, state = energyConsolePayload()) {
  await page.goto(URL);
  await page.waitForFunction(() => typeof window.__updateConsole === 'function');
  await page.evaluate(() => { window.__sent = []; window.__sendAction = json => window.__sent.push(JSON.parse(json)); });
  await page.evaluate(value => window.__updateConsole('power', JSON.stringify(value)), state);
  await expect(page.locator('ph-energy-controls')).toBeVisible();
}
const sent = page => page.evaluate(() => window.__sent);
const numeric = async (page, id, value) => page.locator(`ph-energy-controls #${id}`).evaluate((input, next) => {
  input.value = String(next); input.dispatchEvent(new Event('input', { bubbles: true })); input.dispatchEvent(new Event('change', { bubbles: true }));
}, value);

test('Power renders real desired/effective flow, local reserves, cuts and shared reactor battery', async ({ page }) => {
  await open(page);
  await expect(page.locator('#bat-val')).toHaveText('72%');
  await expect(page.locator('ph-energy-controls [data-energy-group="weapons"] .stats')).toContainText(ts('energy.controls.flow_cut'));
  await expect(page.locator('ph-energy-controls #reactor-status')).toContainText('72/100');
  await expect(page.locator('ph-energy-controls #coolant')).toContainText('20');
  await expect(page.locator('ph-power-controls .group')).toHaveCount(0);
  await page.locator('ph-energy-controls #depth').selectOption('detail');
  await page.locator('ph-energy-controls #consumer-section').evaluate(node => { node.open = true; });
  await expect(page.locator('ph-energy-controls [data-energy-consumer="drive"] .stats')).toContainText('30/40');
});

test('Power controls use exact-owner correlated energy actions at each visible complexity', { tag: '@core' }, async ({ page }) => {
  await open(page);
  await numeric(page, 'flow-helm', 6);
  expect(await sent(page)).toContainEqual(expect.objectContaining({ action: 'set_energy_group', control_system_id: 'main-reactor',
    group: 'helm', flow_per_sec: 6, cooling_per_sec: 1, correlation: expect.any(String) }));
  await page.locator('ph-energy-controls #depth').selectOption('min');
  await page.locator('ph-energy-controls [data-energy-group="helm"] .row button:last-child').click();
  expect(await sent(page)).toContainEqual(expect.objectContaining({ action: 'set_energy_priority', group: 'helm', priority: 1 }));
  await page.locator('ph-energy-controls #mode').selectOption('minimal');
  expect(await sent(page)).toContainEqual(expect.objectContaining({ action: 'set_energy_mode', mode: 'minimal', cooling_usage: 'low' }));
  await page.locator('ph-energy-controls #depth').selectOption('detail');
  await page.locator('ph-energy-controls #consumer-section').evaluate(node => { node.open = true; });
  await numeric(page, 'system-flow-drive', 2);
  expect(await sent(page)).toContainEqual(expect.objectContaining({ action: 'set_energy_system', system: 'drive', flow_per_sec: 2 }));
  const updated = energyConsolePayload(); updated.energy.consumers[0].flow_override_per_sec = 2;
  await page.evaluate(value => window.__updateConsole('power', JSON.stringify(value)), updated);
  await page.locator('ph-energy-controls [data-energy-consumer="drive"] button').click();
  expect(await sent(page)).toContainEqual(expect.objectContaining({ action: 'set_energy_system', system: 'drive', flow_per_sec: null }));
});

test('ordinary delegation disables machinery while complexity stays a Console working view', async ({ page }) => {
  const state = energyConsolePayload(); state.power_auto = true;
  await open(page, state);
  await expect(page.locator('ph-energy-controls #flow-helm')).toBeDisabled();
  await expect(page.locator('ph-energy-controls #output')).toBeDisabled();
  await page.locator('ph-energy-controls #depth').selectOption('min');
  const actions = await sent(page);
  expect(actions.filter(action => action.action.startsWith('set_energy_'))).toEqual([]);
  expect(actions).toContainEqual(expect.objectContaining({ action: 'set_console_view', view: expect.objectContaining({ energy_depth: 'min' }) }));
});

test('keyboard input adjusts native flow without reviving retired rung shortcuts', async ({ page }) => {
  await open(page);
  await page.locator('ph-energy-controls #flow-helm').focus();
  await page.keyboard.press('ArrowRight');
  expect(await sent(page)).toContainEqual(expect.objectContaining({ action: 'set_energy_group', group: 'helm', flow_per_sec: 5 }));
  await page.keyboard.press('KeyQ'); await page.keyboard.press('KeyE');
  expect((await sent(page)).filter(action => action.action === 'set_power')).toEqual([]);
});
