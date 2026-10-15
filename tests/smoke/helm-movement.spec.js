import { test, expect } from '@playwright/test';

// Actual shipped phone page and normal semantic/wire path. Input is an already
// scoped publication; native owner tests separately cover paid physical motion.
test('Helm independently controls owned modes and dated Match Speed without stealing steering', { tag: '@core' }, async ({ page }) => {
  await page.goto('/gui/cruiser/helm.html');
  await page.waitForFunction(() => typeof window.__updateConsole === 'function');
  await page.evaluate(async () => {
    const { dispatchConsoleAction } = await import('/gui/action-map.js');
    window.__wire = []; window.__sendAction = json => dispatchConsoleAction(JSON.parse(json), (type, data) => window.__wire.push({ type, ...data }));
  });
  const publish = state => page.evaluate(state => window.__updateConsole('helm', JSON.stringify(state)), state);
  const axes = [{ role: 'thrust', system_id: 'main-drive', mode: 'assisted', available: true, controllable: true },
    { role: 'steering', system_id: 'rudder', mode: 'direct', available: true, controllable: false },
    { role: 'lateral', system_id: 'side-drive', mode: 'direct', available: false, controllable: true }];
  const contact = { contact: 'opaque-measured-contact', name: 'Recorded vessel', speed: 5, observed_tick: 8, age: 2 };
  const state = { system_ids: ['main-drive','rudder','side-drive'], system_families: { 'main-drive': 'helm', rudder: 'helm', 'side-drive': 'helm' },
    thrust_system_id: 'main-drive', steering_system_id: 'rudder', lateral_system_id: 'side-drive', helm_auto: true,
    helm_movement: { axes, contacts: [contact], match: null }, blips: [], tracks: [] };
  await publish(state);
  const widget = page.locator('ph-helm-movement'); await expect(widget).toBeVisible();
  await expect(widget.locator('[data-system-id="rudder"]')).toBeDisabled();
  await widget.locator('[data-system-id="main-drive"]').click();
  await widget.locator('#assist').evaluate(node => { node.open = true; });
  await widget.locator('#speed').fill('12'); await widget.locator('#set-speed').click();
  await widget.locator('#contact').selectOption(contact.contact); await widget.locator('#contact').focus();
  await publish({ ...state, helm_movement: structuredClone(state.helm_movement) });
  await expect(widget.locator('#contact')).toBeFocused(); await expect(widget.locator('#contact')).toHaveValue(contact.contact);
  await widget.locator('#match').click();
  await publish({ ...state, helm_movement: { axes, contacts: [], match: { contact: contact.contact, speed: 5, observed_tick: 8, age: 12, observation_available: false } } });
  await expect(widget.locator('#held')).toContainText('sample 8'); await expect(widget.locator('#held')).toContainText('age 12');
  await expect(widget.locator('#match')).toBeDisabled(); await widget.locator('#cancel').focus(); await page.keyboard.press('Enter');
  expect(await page.evaluate(() => window.__wire)).toEqual([
    { type: 'ControlSystemCorrelated', correlation: expect.any(String), target: 'main-drive', payload: { type: 'SetHelmMode', data: { mode: 'direct' } } },
    { type: 'ControlSystemCorrelated', correlation: expect.any(String), target: 'main-drive', payload: { type: 'SetDesiredSpeed', data: { speed: 12 } } },
    { type: 'ControlSystemCorrelated', correlation: expect.any(String), target: 'main-drive', payload: { type: 'SetMatchSpeed', data: { contact: contact.contact } } },
    { type: 'ControlSystemCorrelated', correlation: expect.any(String), target: 'main-drive', payload: { type: 'SetMatchSpeed', data: { contact: null } } },
  ]);
});
