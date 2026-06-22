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

  // Direction pad.
  await expect(page.locator('#dir')).toHaveAttribute('data-direction', 'Port');

  // Red alert.
  await expect(page.locator('#alert')).toHaveAttribute('data-red-alert', 'true');

  // Status strip.
  await expect(page.locator('#alert-status')).toHaveText('RED ALERT');
  await expect(page.locator('#view-status')).toHaveText('PORT');
  await expect(page.locator('#contacts-status')).toHaveText('3');

  // Direction pad buttons — only Port should be active.
  await expect(page.locator('#dir-fore')).not.toHaveClass(/active/);
  await expect(page.locator('#dir-port')).toHaveClass(/active/);
  await expect(page.locator('#dir-stbd')).not.toHaveClass(/active/);
  await expect(page.locator('#dir-aft')).not.toHaveClass(/active/);

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
    objectives: [],
    hull_integrity_pct: 100,
    game_status: 'Standing by.',
    blips: [],
  };

  await page.evaluate((s) => (window as any).__updateConsole('CaptainChair', JSON.stringify(s)), state);

  await expect(page.locator('#alert-status')).toHaveText('STANDARD');
  await expect(page.locator('#alert-led')).not.toHaveClass(/fire/);
  await expect(page.locator('#alert-label')).toHaveText('RED ALERT');
  await expect(page.locator('#dir-fore')).toHaveClass(/active/);
  await expect(page.locator('#contacts-status')).toHaveText('0');
  await expect(page.locator('#obj-count-tag')).toHaveText('0');
});

test('captain console: dir pad buttons call __sendAction with correct envelopes', async ({ page }) => {
  await page.goto(CONSOLE_URL);

  await page.evaluate(() => {
    (window as any).__sent = [];
    (window as any).__sendAction = (json: string) => (window as any).__sent.push(json);
  });

  await page.locator('#dir-fore').click();
  await page.locator('#dir-port').click();
  await page.locator('#dir-stbd').click();
  await page.locator('#dir-aft').click();
  await page.locator('#red-alert-btn').click();

  const sent: string[] = await page.evaluate(() => (window as any).__sent);
  expect(sent).toHaveLength(5);

  expect(JSON.parse(sent[0])).toEqual({ action: 'set_view', console: 'CaptainChair', direction: 'Fore' });
  expect(JSON.parse(sent[1])).toEqual({ action: 'set_view', console: 'CaptainChair', direction: 'Port' });
  expect(JSON.parse(sent[2])).toEqual({ action: 'set_view', console: 'CaptainChair', direction: 'Starboard' });
  expect(JSON.parse(sent[3])).toEqual({ action: 'set_view', console: 'CaptainChair', direction: 'Aft' });
  expect(JSON.parse(sent[4])).toEqual({ action: 'toggle_red_alert', console: 'CaptainChair' });
});

test('captain console: AI-run Red Alert renders read-only with AUTO badge', async ({ page }) => {
  await page.goto(CONSOLE_URL);

  const state = {
    red_alert: false,
    red_alert_system_id: 'red-alert',
    red_alert_auto: true,
    view_mode: 'Camera',
    view_direction: 'Fore',
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
