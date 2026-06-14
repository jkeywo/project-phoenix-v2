// Navigation console smoke tests.
//
// The map is a standalone iframe console page. This verifies that its
// waypoint placement path emits the ADR-0001 action envelope expected by
// client.html before the server turns it into ClientMessage::SetNavigationWaypoint.

import { test, expect } from './fixtures';

const CONSOLE_URL = '/gui/navigation-console.html';

test('navigation console: placing a waypoint sends set_navigation_waypoint', async ({ page }) => {
  await page.goto(CONSOLE_URL);

  await page.evaluate(() => {
    (window as any).__sent = [];
    (window as any).__sendAction = (json: string) => (window as any).__sent.push(json);
  });

  await page.locator('#btn-set-waypoint').click();
  await page.mouse.click(420, 260);

  const sent: string[] = await page.evaluate(() => (window as any).__sent);
  expect(sent).toHaveLength(1);

  const parsed = JSON.parse(sent[0]);
  expect(parsed.action).toBe('set_navigation_waypoint');
  expect(parsed.console).toBe('Navigation');
  expect(typeof parsed.x).toBe('number');
  expect(typeof parsed.z).toBe('number');
});

test('navigation console: clear waypoint sends clear_navigation_waypoint', async ({ page }) => {
  await page.goto(CONSOLE_URL);

  await page.evaluate(() => {
    (window as any).__sent = [];
    (window as any).__sendAction = (json: string) => (window as any).__sent.push(json);
    (window as any).__updateConsole('Navigation', JSON.stringify({
      blips: [],
      waypoint: { x: 100, z: 200 },
      ship_x: 0,
      ship_z: 0,
      ship_heading: 0,
      ship_speed: 0,
      impulse_charge_progress: 0,
      cancel_visible: false,
      on_screen: false,
      radar_range: 5000,
    }));
  });

  await page.locator('#btn-clear-waypoint').click();

  const sent: string[] = await page.evaluate(() => (window as any).__sent);
  expect(sent).toHaveLength(1);

  const parsed = JSON.parse(sent[0]);
  expect(parsed.action).toBe('clear_navigation_waypoint');
  expect(parsed.console).toBe('Navigation');
});
