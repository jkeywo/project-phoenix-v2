import { test, expect } from './fixtures';
import { ts } from './strings';

const CONSOLE_URL = '/gui/battleship/navigation.html';

test('navigation console: tapping the map sends set_navigation_waypoint', async ({ page }) => {
  await page.goto(CONSOLE_URL);
  await page.evaluate(() => {
    window.__sent = [];
    window.__sendAction = (json) => window.__sent.push(json);
  });
  await page.locator('ph-navigation-map').locator('canvas').click();
  const sent = await page.evaluate(() => window.__sent);
  expect(sent).toHaveLength(1);
  const parsed = JSON.parse(sent[0]);
  expect(parsed.action).toBe('set_navigation_waypoint');
  expect(parsed.console).toBe('navigation');
  expect(typeof parsed.x).toBe('number');
  expect(typeof parsed.z).toBe('number');
});

test('navigation console: clear waypoint sends clear_navigation_waypoint', async ({ page }) => {
  await page.goto(CONSOLE_URL);
  await page.evaluate(() => {
    window.__sent = [];
    window.__sendAction = (json) => window.__sent.push(json);
    window.__updateConsole('navigation', JSON.stringify({
      blips: [],
      waypoint: { x: 100, z: 200 },
      ship_x: 0,
      ship_z: 0,
      ship_heading: 0,
      ship_speed: 0,
      radar_range: 5000,
    }));
  });
  await page.locator('#btn-clear-waypoint').click();
  const sent = await page.evaluate(() => window.__sent);
  expect(sent).toHaveLength(1);
  const parsed = JSON.parse(sent[0]);
  expect(parsed.action).toBe('clear_navigation_waypoint');
  expect(parsed.console).toBe('navigation');
});

test('navigation console: selected entity name stays NONE until the operator taps a target', async ({ page }) => {
  await page.goto(CONSOLE_URL);
  await page.evaluate(() => {
    window.__updateConsole('navigation', JSON.stringify({
      blips: [
        { uuid: 'station-alpha', name: 'Alpha Station', kind: 'station', stance: 'friendly', radar_x: 0.2, radar_y: -0.1, world_x: 200, world_z: -100, selectable: true },
      ],
      waypoint: null,
      ship_x: 0,
      ship_z: 0,
      ship_heading: 0,
      ship_speed: 0,
      radar_range: 5000,
    }));
  });
  await expect(page.locator('#ent-name')).toHaveText(ts('console.navigation.none'));
});

test('navigation console: tapping a visible entity sends source_uuid for that target', async ({ page }) => {
  await page.goto(CONSOLE_URL);
  await page.evaluate(() => {
    window.__sent = [];
    window.__sendAction = (json) => window.__sent.push(json);
    window.__updateConsole('navigation', JSON.stringify({
      blips: [
        { uuid: 'station-alpha', name: 'Alpha Station', kind: 'station', stance: 'friendly', radar_x: 0.0, radar_y: 0.0, world_x: 0, world_z: 0, selectable: true },
      ],
      waypoint: null,
      ship_x: 0,
      ship_z: 0,
      ship_heading: 0,
      ship_speed: 0,
      radar_range: 5000,
    }));
  });
  await page.locator('ph-navigation-map').locator('canvas').click();
  const sent = await page.evaluate(() => window.__sent);
  expect(sent).toHaveLength(1);
  const parsed = JSON.parse(sent[0]);
  expect(parsed.action).toBe('set_navigation_waypoint');
  expect(parsed.console).toBe('navigation');
  expect(parsed.source_uuid).toBe('station-alpha');
  await expect(page.locator('#ent-name')).toHaveText('Alpha Station');
});

test('navigation console: waypoint state renders waypoint labels in the side panel and footer', async ({ page }) => {
  await page.goto(CONSOLE_URL);
  await page.evaluate(() => {
    window.__updateConsole('navigation', JSON.stringify({
      blips: [
        { uuid: 'station-bravo', name: 'Bravo Station', kind: 'station', stance: 'friendly', radar_x: 0.4, radar_y: -0.3, world_x: 300, world_z: -200, selectable: true },
      ],
      waypoint: { x: 300, z: -200, name: 'Bravo Station' },
      ship_x: 0,
      ship_z: 0,
      ship_heading: 0,
      ship_speed: 0,
      radar_range: 5000,
    }));
  });
  await expect(page.locator('#waypoint-name')).toHaveText('Bravo Station');
  await expect(page.locator('#footer-target')).toHaveText('Bravo Station');
});
