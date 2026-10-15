import { test, expect } from '@playwright/test';

test('Cruiser weapon assignments keep primary play and route explicit overrides to owned Systems', { tag: '@core' }, async ({ page }) => {
  await page.goto('/client/gui/cruiser/tactical.html');
  await page.waitForFunction(() => typeof window.__updateConsole === 'function');
  await page.evaluate(() => { window.__assignmentActions = []; window.__sendAction = json => window.__assignmentActions.push(JSON.parse(json)); });
  const view = { banks: [], tubes: [], blasters: [], blips: [], weapon_targets: {
    rows: [{ system: 'custom-beam', controllable: true, assignment: { group: null, individual: null } },
      { system: 'delegated-tube', controllable: false, assignment: { group: null, individual: null } }],
    groups: [{ id: 'prepared', label: 'weapon.group.main_beams', members: ['custom-beam', 'delegated-tube'] }], contacts: [{ id: 'observed-contact', name: 'Known return' }],
  } };
  const payload = { system_ids: ['custom-beam'], system_families: { 'custom-beam': 'tactical' }, systems: { 'custom-beam': view } };
  await page.evaluate(value => window.__updateConsole('tactical', JSON.stringify(value)), payload);
  const details = page.locator('details').filter({ hasText: 'Weapon assignments' });
  await expect(details).not.toHaveAttribute('open'); await details.locator('summary').click();
  const row = details.locator('[data-assignment-key="custom-beam"]');
  await row.locator('select').selectOption('observed-contact'); await row.getByRole('button', { name: 'Assign', exact: true }).click();
  await expect.poll(() => page.evaluate(() => window.__assignmentActions.length)).toBe(1);
  expect(await page.evaluate(() => window.__assignmentActions[0])).toMatchObject({ action: 'set_weapon_target', control_system_id: 'custom-beam', uuid: 'observed-contact', correlation: expect.any(String) });
  await details.locator('[data-assignment-key="prepared"] button').click();
  expect(await page.evaluate(() => window.__assignmentActions.filter(row => row.action === 'set_weapon_group_target').map(row => row.control_system_id))).toEqual(['custom-beam']);
  await expect(details.locator('[data-assignment-key="delegated-tube"] select')).toBeDisabled();
  await row.getByRole('button', { name: 'Follow group', exact: true }).click();
  await row.getByRole('button', { name: 'Follow primary Combat Lock', exact: true }).click();
  expect(await page.evaluate(() => window.__assignmentActions.slice(-2).map(row => row.action))).toEqual(['follow_weapon_group', 'follow_primary_target']);
});
