import { test, expect } from '@playwright/test';

// Actual shipped phone documents and semantic action-map dispatch. The fixture
// is an already-scoped phone publication, not a simulated Rust sensor producer.
// Native sibling cases prove alias admission/physical work. Transporter has no
// phone surface yet (dependency 20), so it is deliberately not invented here.
const alias = '12:sensor-radar:contact-7';
async function open(page, url) {
  await page.goto(url);
  await page.waitForFunction(() => typeof window.__updateConsole === 'function');
  await page.evaluate(async () => {
    const { dispatchConsoleAction } = await import('/gui/action-map.js');
    window.__actions = []; window.__wire = [];
    window.__sendAction = json => {
      const action = JSON.parse(json); window.__actions.push(action);
      dispatchConsoleAction(action, (type, data) => window.__wire.push({ type, ...data }));
    };
  });
}
const publish = (page, station, state) => page.evaluate(({ station, state }) => window.__updateConsole(station, JSON.stringify(state)), { station, state });

test('Dock uses the scoped measured partner and can release after contact identity disappears', { tag: '@core' }, async ({ page }) => {
  await open(page, '/gui/cruiser/helm.html');
  const state = { system_ids: ['dock'], system_families: { dock: 'helm' }, helm_auto: false,
    tracks: [], blips: [], dock: { system_id: 'dock', available: true, engaged: false, docked: false,
      available_target: alias, available_target_name: 'Recorded berth' } };
  await publish(page, 'helm', state);
  await expect(page.locator('#dock-status')).toContainText('Recorded berth');
  await page.locator('#dock-btn').click();
  await publish(page, 'helm', { ...state, dock: { system_id: 'dock', available: false, engaged: true, docked: true,
    available_target: null, available_target_name: null, docked_to: alias, docked_to_name: 'Recorded berth' } });
  await expect(page.locator('#dock-status')).toContainText('Recorded berth');
  // Loss removes only identifying references. The real own coupling still
  // keeps its ordinary Undock control available.
  await publish(page, 'helm', { ...state, dock: { system_id: 'dock', available: false, engaged: true, docked: true,
    available_target: null, available_target_name: null, docked_to: null, docked_to_name: null } });
  await expect(page.locator('#dock-status')).not.toContainText('Recorded berth');
  await page.locator('#dock-btn').focus(); await page.keyboard.press('Enter');
  expect(await page.evaluate(() => window.__wire)).toEqual([
    { type: 'ControlSystemCorrelated', correlation: expect.any(String), target: 'dock', payload: { type: 'Dock' } },
    { type: 'ControlSystemCorrelated', correlation: expect.any(String), target: 'dock', payload: { type: 'Undock' } },
  ]);
  expect(await page.evaluate(() => window.__actions.map(action => action.semantic_action))).toEqual(['helm.dock', 'helm.dock']);
});

test('Tractor renders only the held measured label and releases through its ordinary control after evidence loss', { tag: '@core' }, async ({ page }) => {
  await open(page, '/gui/cruiser/engineering.html');
  const state = { system_ids: ['tractor'], system_families: { tractor: 'tractor' },
    systems: { tractor: { system_id: 'tractor', tractor_auto: false, range: 500, engaged: false, coupled_target: null, coupled_target_name: null } } };
  await publish(page, 'engineering', state);
  const segment = page.locator('#engineering-seg-ops'); if (await segment.count()) await segment.click();
  await expect(page.locator('#tractor-btn')).toBeVisible(); await page.locator('#tractor-btn').click();
  await publish(page, 'engineering', { ...state, systems: { tractor: { ...state.systems.tractor, engaged: true,
    coupled_target: alias, coupled_target_name: 'Recorded tow contact' } } });
  await expect(page.locator('#tractor-status')).toContainText('Recorded tow contact');
  await publish(page, 'engineering', { ...state, systems: { tractor: { ...state.systems.tractor, engaged: true,
    coupled_target: null, coupled_target_name: null } } });
  await expect(page.locator('#tractor-status')).not.toContainText('Recorded tow contact');
  await page.locator('#tractor-btn').focus(); await page.keyboard.press('Enter');
  expect(await page.evaluate(() => window.__wire)).toEqual([
    { type: 'ControlSystemCorrelated', correlation: expect.any(String), target: 'tractor', payload: { type: 'EngageTractor' } },
    { type: 'ControlSystemCorrelated', correlation: expect.any(String), target: 'tractor', payload: { type: 'ReleaseTractor' } },
  ]);
  expect(await page.evaluate(() => window.__actions.map(action => action.semantic_action))).toEqual(['engineering.tractor', 'engineering.tractor']);
});
