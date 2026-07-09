// Issue #428 — Smoke test: HTML captain console strategic overview panel.
//
// The captain console page (gui/captain-console.html) is a static HTML file
// copied into dist/gui/ by Trunk. It exercises the ADR-0001 bridge contract:
//   - window.__updateConsole(name, stateJson) renders state into DOM.
//   - Direction pad and red alert buttons call window.__sendAction(...).

import { test, expect } from './fixtures';

const CONSOLE_URL = '/gui/captain-console.html';

test('captain console: __updateConsole renders objectives, alert status, contacts and direction', async ({ page }) => {
  await page.goto(CONSOLE_URL);

  const state = {
    red_alert: true,
    view_mode: 'Camera',
    view_direction: 'Port',
    camera_views: ['Fore', 'Port', 'Starboard', 'Aft'],
    objectives: [
      { id: 'obj-1', text: 'Scan the anomaly', mandatory: true, status: 'Active' },
      { id: 'obj-2', text: 'Neutralise raiders', mandatory: false, status: 'Completed' },
    ],
    hull_integrity_pct: 78.5,
    game_status: 'RED ALERT — All hands to battlestations.',
    blips: [{ uuid: 'e1' }, { uuid: 'e2' }, { uuid: 'e3' }],
  };

  await page.evaluate((s) => (window as any).__updateConsole('CaptainChair', JSON.stringify(s)), state);

  // Hidden data-attr containers.
  await expect(page.locator('#objectives .objective-data')).toHaveCount(2);
  await expect(page.locator('.objective-data[data-id="obj-1"]')).toHaveAttribute('data-text', 'Scan the anomaly');
  await expect(page.locator('.objective-data[data-id="obj-2"]')).toHaveAttribute('data-status', 'Completed');

  // Direction pad data-attr (populated from camera_views).
  await expect(page.locator('#dir')).toHaveAttribute('data-direction', 'Port');

  // Red alert.
  await expect(page.locator('#alert')).toHaveAttribute('data-red-alert', 'true');

  // Status strip.
  await expect(page.locator('#alert-status')).toHaveText('RED ALERT');
  await expect(page.locator('#view-status')).toHaveText('PORT');
  await expect(page.locator('#contacts-status')).toHaveText('3');

  // Camera-select component renders camera buttons (shadow DOM).
  const camBtns = page.locator('ph-camera-select').shadow().locator('.cam-btn');
  await expect(camBtns).toHaveCount(4);
  await expect(camBtns.nth(0)).toHaveText('Fore');
  await expect(camBtns.nth(1)).toHaveText('Port');
  await expect(camBtns.nth(1)).toHaveClass(/active/);

  // Red alert LED and button styling.
  await expect(page.locator('#alert-led')).toHaveClass(/led fire/);
  await expect(page.locator('#red-alert-btn')).toHaveClass(/btn danger/);
  await expect(page.locator('#alert-label')).toHaveText('ALERT ACTIVE');
});

test('captain console: standard alert state renders correctly', async ({ page }) => {
  await page.goto(CONSOLE_URL);

  const state = {
    red_alert: false,
    view_mode: 'Camera',
    view_direction: 'Fore',
    camera_views: ['Fore', 'Port', 'Starboard', 'Aft'],
    objectives: [],
    hull_integrity_pct: 100,
    game_status: 'Standing by.',
    blips: [],
  };

  await page.evaluate((s) => (window as any).__updateConsole('CaptainChair', JSON.stringify(s)), state);

  await expect(page.locator('#alert-status')).toHaveText('STANDARD');
  await expect(page.locator('#alert-led')).not.toHaveClass(/fire/);
  await expect(page.locator('#alert-label')).toHaveText('RED ALERT');
  await expect(page.locator('#contacts-status')).toHaveText('0');
  await expect(page.locator('#obj-count-tag')).toHaveText('0');
});

test('captain console: camera-select and red alert call __sendAction with correct envelopes', async ({ page }) => {
  await page.goto(CONSOLE_URL);

  const sent: string[] = [];
  await page.evaluate(() => {
    (window as any).__sent = [];
    (window as any).__sendAction = (json: string) => (window as any).__sent.push(json);
    // ph-camera-select reads window.sendAction; wire it through __sendAction
    (window as any).sendAction = (action: string, payload?: object) => {
      (window as any).__sendAction(JSON.stringify(Object.assign({ action, console: 'captain' }, payload)));
    };
  });

  // Emit state to populate camera-select.
  await page.evaluate((s) => (window as any).__updateConsole('CaptainChair', JSON.stringify(s)), {
    red_alert: false,
    camera_views: ['Fore', 'Port', 'Starboard', 'Aft'],
    view_direction: 'Fore',
    objectives: [],
  });

  // Click camera buttons in the ph-camera-select shadow DOM.
  const camBtns = page.locator('ph-camera-select').shadow().locator('.cam-btn');
  await camBtns.nth(1).click(); // Port
  await camBtns.nth(2).click(); // Starboard
  await camBtns.nth(0).click(); // Fore

  await page.locator('#red-alert-btn').click();

  const getSent = () => page.evaluate(() => (window as any).__sent);
  const s = await getSent();
  expect(s).toHaveLength(4);

  expect(JSON.parse(s[0])).toEqual({ action: 'set_view', console: 'captain', direction: 'Port' });
  expect(JSON.parse(s[1])).toEqual({ action: 'set_view', console: 'captain', direction: 'Starboard' });
  expect(JSON.parse(s[2])).toEqual({ action: 'set_view', console: 'captain', direction: 'Fore' });
  expect(JSON.parse(s[3])).toEqual({ action: 'toggle_red_alert', console: 'captain' });
});

test('captain console: AI-run Red Alert renders read-only with AUTO badge', async ({ page }) => {
  await page.goto(CONSOLE_URL);

  const state = {
    red_alert: false,
    red_alert_system_id: 'red-alert',
    red_alert_auto: true,
    view_mode: 'Camera',
    view_direction: 'Fore',
    camera_views: ['Fore', 'Port', 'Starboard', 'Aft'],
    objectives: [],
    hull_integrity_pct: 100,
    game_status: 'Standing by.',
    blips: [],
  };

  await page.evaluate((s) => (window as any).__updateConsole('CaptainChair', JSON.stringify(s)), state);

  await expect(page.locator('#red-alert-btn')).toBeDisabled();
  await expect(page.locator('#red-alert-btn')).toHaveAttribute('data-system-id', 'red-alert');
  await expect(page.locator('#red-alert-btn')).toHaveAttribute('data-auto', 'true');
  await expect(page.locator('#red-alert-auto-badge')).toBeVisible();
  await expect(page.locator('#red-alert-auto-badge')).toHaveText('AUTO');
});
