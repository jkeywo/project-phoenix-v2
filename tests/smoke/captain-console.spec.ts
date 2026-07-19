import { test, expect } from './fixtures';
import { ts } from './strings';

const CONSOLE_URL = '/gui/battleship/captain.html';

// The battleship's Captain station does not own a sensors system (Sensors is
// its own dedicated station), so buildCaptainConsoleState — and the state
// pushed here — is flat, not nested under `captain`/`sensors` keys (that
// nesting is only used by the Destroyer's combined captain+sensors console).

test('captain console: __updateConsole renders objectives, target, and direction state', async ({ page }) => {
  await page.goto(CONSOLE_URL);

  const state = {
    red_alert: true,
    view_direction: 'Port',
    camera_views: ['Fore', 'Port', 'Starboard', 'Aft'],
    objectives: [
      { id: 'obj-1', text: 'Scan the anomaly', mandatory: true, status: 'Active' },
      { id: 'obj-2', text: 'Neutralise raiders', mandatory: false, status: 'Completed' },
    ],
    blips: [{ uuid: 'e1' }, { uuid: 'e2' }, { uuid: 'e3' }],
  };

  await page.evaluate((s) => (window as any).__updateConsole('captain', JSON.stringify(s)), state);

  await expect(page.locator('#objectives .objective-data')).toHaveCount(2);
  await expect(page.locator('.objective-data[data-id="obj-1"]')).toHaveAttribute('data-text', 'Scan the anomaly');
  await expect(page.locator('.objective-data[data-id="obj-2"]')).toHaveAttribute('data-status', 'Completed');
  await expect(page.locator('#dir')).toHaveAttribute('data-direction', 'Port');
  await expect(page.locator('#alert')).toHaveAttribute('data-red-alert', 'true');
  await expect(page.locator('#footer-target')).toContainText(ts('console.common.contacts.other', { n: 3 }));

  const camBtns = page.locator('ph-camera-select').locator('.cam-btn');
  await expect(camBtns).toHaveCount(4);
  await expect(camBtns.nth(1)).toHaveText('Port');
  await expect(camBtns.nth(1)).toHaveClass(/active/);

  const redAlertBtn = page.locator('ph-red-alert').locator('#alert-btn');
  await expect(redAlertBtn).toHaveClass(/active/);
  await expect(redAlertBtn).toHaveText(ts('component.red_alert.active'));
});

test('captain console: standard alert state renders correctly', async ({ page }) => {
  await page.goto(CONSOLE_URL);

  const state = {
    red_alert: false,
    view_direction: 'Fore',
    camera_views: ['Fore', 'Port', 'Starboard', 'Aft'],
    objectives: [],
    blips: [],
  };

  await page.evaluate((s) => (window as any).__updateConsole('captain', JSON.stringify(s)), state);

  await expect(page.locator('ph-red-alert').locator('#alert-btn')).toHaveText(ts('component.red_alert.standby'));
  await expect(page.locator('#footer-target')).toHaveText(ts('console.common.no_target'));
  await expect(page.locator('ph-objective-list .empty')).toHaveText(ts('component.objectives.empty'));
});

test('captain console: camera-select and red alert call __sendAction with correct envelopes', async ({ page }) => {
  await page.goto(CONSOLE_URL);

  await page.evaluate(() => {
    (window as any).__sent = [];
    (window as any).__sendAction = (json: string) => (window as any).__sent.push(json);
    (window as any).sendAction = (action: string, payload?: object) => {
      (window as any).__sendAction(JSON.stringify(Object.assign({ action, console: 'captain' }, payload)));
    };
  });

  await page.evaluate((s) => (window as any).__updateConsole('captain', JSON.stringify(s)), {
    red_alert: false,
    camera_views: ['Fore', 'Port', 'Starboard', 'Aft'],
    view_direction: 'Fore',
    objectives: [],
    blips: [],
  });

  const camBtns = page.locator('ph-camera-select').locator('.cam-btn');
  await camBtns.nth(1).click();
  await camBtns.nth(2).click();
  await camBtns.nth(0).click();
  await page.locator('ph-red-alert').locator('#alert-btn').click();

  const sent = await page.evaluate(() => (window as any).__sent);
  expect(sent).toHaveLength(4);
  expect(JSON.parse(sent[0])).toEqual({ action: 'set_view', console: 'captain', direction: 'Port' });
  expect(JSON.parse(sent[1])).toEqual({ action: 'set_view', console: 'captain', direction: 'Starboard' });
  expect(JSON.parse(sent[2])).toEqual({ action: 'set_view', console: 'captain', direction: 'Fore' });
  expect(JSON.parse(sent[3])).toEqual({ action: 'toggle_red_alert', console: 'captain' });
});

test('captain console: AI-run Red Alert renders read-only with AUTO badge', async ({ page }) => {
  await page.goto(CONSOLE_URL);

  const state = {
    red_alert: false,
    red_alert_auto: true,
    view_direction: 'Fore',
    camera_views: ['Fore', 'Port', 'Starboard', 'Aft'],
    objectives: [],
    blips: [],
  };

  await page.evaluate((s) => (window as any).__updateConsole('captain', JSON.stringify(s)), state);

  await expect(page.locator('ph-red-alert').locator('#alert-btn')).toBeDisabled();
  await expect(page.locator('ph-red-alert').locator('#auto-badge')).toBeVisible();
  await expect(page.locator('ph-red-alert').locator('#auto-badge')).toHaveText(ts('console.common.auto'));
});
