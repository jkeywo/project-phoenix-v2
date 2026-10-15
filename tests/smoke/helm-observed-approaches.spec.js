import { test, expect } from './fixtures';

test('Helm shows conditional measured-ordnance corridors without steering commands', { tag: '@core' }, async ({ page }) => {
  await page.goto('/gui/cruiser/helm.html');
  const state = { range: 100, x: 0, z: 0, ship_heading: 0, speed: 0, blips: [],
    tracks: [{ id: 'opaque-contact', name: 'Measured return' }],
    observed_approaches: [{ contact: 'opaque-contact', observed_tick: 7, seconds: 10,
      own_position: [0, 0], contact_position: [0, 20], position_error: 5, separation: [15, 25] }],
  };
  await page.evaluate(payload => {
    window.__sent = [];
    window.__sendAction = value => window.__sent.push(value);
    window.__updateConsole('helm', JSON.stringify(payload));
  }, state);
  const radar = page.locator('ph-helm-radar');
  await expect(radar.locator('#approaches')).toBeVisible();
  await radar.locator('#approaches summary').click();
  await expect(radar.locator('#approaches li')).toContainText('Measured return');
  await expect(radar.locator('#approaches li')).toContainText('15–25');
  await expect(radar.locator('#approach-corridors circle')).toHaveAttribute('r', '2.5');
  expect(await page.evaluate(() => window.__sent)).toEqual([]);
  await page.evaluate(payload => window.__updateConsole('helm', JSON.stringify(payload)), {
    ...state, tracks: [], observed_approaches: [],
  });
  await expect(radar.locator('#approaches')).toBeHidden();
  await expect(radar.locator('#approach-corridors circle')).toHaveCount(0);
});
