import { test, expect } from './fixtures';
import { ts } from './strings';

const CONSOLE_URL = '/gui/battleship/navigation.html';

test('navigation console: tapping the map alone never sends a waypoint action', async ({ page }) => {
  await page.goto(CONSOLE_URL);
  await page.evaluate(() => {
    window.__sent = [];
    window.__sendAction = (json) => window.__sent.push(json);
  });
  await page.locator('ph-navigation-map').locator('canvas').click();
  const sent = await page.evaluate(() => window.__sent);
  expect(sent).toHaveLength(0);
});

test('navigation console: Set Waypoint pick mode places a free waypoint on tap', async ({ page }) => {
  await page.goto(CONSOLE_URL);
  await page.evaluate(() => {
    window.__sent = [];
    window.__sendAction = (json) => window.__sent.push(json);
  });
  await page.locator('ph-navigation-map').locator('#btn-set-waypoint').click();
  await page.locator('ph-navigation-map').locator('canvas').click();
  const sent = await page.evaluate(() => window.__sent);
  expect(sent).toHaveLength(1);
  const parsed = JSON.parse(sent[0]);
  expect(parsed.action).toBe('set_navigation_waypoint');
  expect(parsed.console).toBe('navigation');
  expect(typeof parsed.x).toBe('number');
  expect(typeof parsed.z).toBe('number');
  expect(parsed.source_uuid).toBeUndefined();
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
  await page.locator('ph-navigation-map').locator('#btn-clear-waypoint').click();
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

test('navigation console: tapping a visible entity selects it and Set as Waypoint anchors it', async ({ page }) => {
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
  // Selecting alone does not set the waypoint.
  await expect(page.locator('#ent-name')).toHaveText('Alpha Station');
  let sent = await page.evaluate(() => window.__sent);
  expect(sent).toHaveLength(0);

  await page.locator('ph-navigation-map').locator('#btn-set-selected').click();
  sent = await page.evaluate(() => window.__sent);
  expect(sent).toHaveLength(1);
  const parsed = JSON.parse(sent[0]);
  expect(parsed.action).toBe('set_navigation_waypoint');
  expect(parsed.console).toBe('navigation');
  expect(parsed.source_uuid).toBe('station-alpha');
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