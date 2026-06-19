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

test('navigation console: ADD WAYPOINT button is hidden until an entity is selected', async ({ page }) => {
  await page.goto(CONSOLE_URL);

  // Push a state with a single selectable entity but no current selection.
  await page.evaluate(() => {
    (window as any).__updateConsole('Navigation', JSON.stringify({
      blips: [
        {
          uuid: 'station-alpha',
          name: 'Alpha Station',
          kind: 'station',
          stance: 'friendly',
          radar_x: 0.2,
          radar_y: -0.1,
          world_x: 200,
          world_z: -100,
          selectable: true,
        },
      ],
      waypoint: null,
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

  // No selection → button hidden.
  await expect(page.locator('#btn-add-waypoint')).toBeHidden();
});

test('navigation console: ADD WAYPOINT sends source_uuid for selected entity', async ({ page }) => {
  await page.goto(CONSOLE_URL);

  await page.evaluate(() => {
    (window as any).__sent = [];
    (window as any).__sendAction = (json: string) => (window as any).__sent.push(json);
    (window as any).__updateConsole('Navigation', JSON.stringify({
      blips: [
        {
          uuid: 'station-alpha',
          name: 'Alpha Station',
          kind: 'station',
          stance: 'friendly',
          radar_x: 0.0,
          radar_y: 0.0,
          world_x: 0,
          world_z: 0,
          selectable: true,
        },
      ],
      waypoint: null,
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

  // Select the entity by tapping near its world position (0, 0). With
  // pan/zoom defaults, the entity sits at the canvas centre (W/2, H/2).
  await page.evaluate(() => {
    const c = document.getElementById('map-canvas') as HTMLCanvasElement;
    const cx = c.width / 2;
    const cy = c.height / 2;
    // Drive the same code path as a real mouse click at (cx, cy).
    c.dispatchEvent(new MouseEvent('mousedown', { clientX: cx, clientY: cy, bubbles: true }));
    window.dispatchEvent(new MouseEvent('mouseup', { clientX: cx, clientY: cy, bubbles: true }));
  });

  await expect(page.locator('#btn-add-waypoint')).toBeVisible();

  await page.locator('#btn-add-waypoint').click();

  const sent: string[] = await page.evaluate(() => (window as any).__sent);
  expect(sent.length).toBeGreaterThanOrEqual(1);

  const parsed = JSON.parse(sent[sent.length - 1]);
  expect(parsed.action).toBe('set_navigation_waypoint');
  expect(parsed.console).toBe('Navigation');
  expect(parsed.source_uuid).toBe('station-alpha');
  expect(typeof parsed.x).toBe('number');
  expect(typeof parsed.z).toBe('number');
});

test('navigation console: tapping anchored waypoint forwards selection to parent entity', async ({ page }) => {
  await page.goto(CONSOLE_URL);

  // Push a state where the waypoint is anchored to an entity at world
  // coordinates (300, -200), and the entity itself is also visible.
  await page.evaluate(() => {
    (window as any).__sent = [];
    (window as any).__sendAction = (json: string) => (window as any).__sent.push(json);
    (window as any).__updateConsole('Navigation', JSON.stringify({
      blips: [
        {
          uuid: 'station-bravo',
          name: 'Bravo Station',
          kind: 'station',
          stance: 'friendly',
          radar_x: 0.4,
          radar_y: -0.3,
          world_x: 300,
          world_z: -200,
          selectable: true,
        },
      ],
      waypoint: { x: 300, z: -200, source_uuid: 'station-bravo' },
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

  // Compute screen coordinates for the waypoint at world (300, -200).
  // Default pan = (0,0), zoom = 0.15. toSX(300) = W/2 + 300 * 0.15.
  await page.evaluate(() => {
    const c = document.getElementById('map-canvas') as HTMLCanvasElement;
    const cx = c.width / 2 + 300 * 0.15;
    const cy = c.height / 2 + (-200) * 0.15;
    c.dispatchEvent(new MouseEvent('mousedown', { clientX: cx, clientY: cy, bubbles: true }));
    window.dispatchEvent(new MouseEvent('mouseup', { clientX: cx, clientY: cy, bubbles: true }));
  });

  // Selection forwarded to parent → bottom-overlay name reads the parent.
  await expect(page.locator('#ent-name')).toHaveText('Bravo Station');
  // ADD WAYPOINT button visible because an entity is selected.
  await expect(page.locator('#btn-add-waypoint')).toBeVisible();
});
