import { test, expect } from './fixtures';
import { ts } from './strings';

// Exercise the pure-client bundle built by scripts/build-client.mjs. The
// sibling /gui path belongs to the separately-built Trunk host bundle and may
// intentionally be absent/stale when this focused client smoke runs alone.
const CONSOLE_URL = '/client/gui/battleship/captain.html';

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

  await page.evaluate((s) => window.__updateConsole('captain', JSON.stringify(s)), state);

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

  await page.evaluate((s) => window.__updateConsole('captain', JSON.stringify(s)), state);

  await expect(page.locator('ph-red-alert').locator('#alert-btn')).toHaveText(ts('component.red_alert.standby'));
  await expect(page.locator('#footer-target')).toHaveText(ts('console.common.no_target'));
  await expect(page.locator('ph-objective-list .empty')).toHaveText(ts('component.objectives.empty'));
});

test('captain console: camera-select and red alert call __sendAction with correct envelopes', { tag: '@core' }, async ({ page }) => {
  await page.goto(CONSOLE_URL);

  await page.evaluate(() => {
    window.__sent = [];
    window.__sendAction = (json) => window.__sent.push(json);
  });

  await page.evaluate((s) => window.__updateConsole('captain', JSON.stringify(s)), {
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
  await page.locator('ph-red-alert').locator('#hold-btn').click();

  const sent = (await page.evaluate(() => window.__sent)).map(JSON.parse);
  expect(sent).toHaveLength(5);
  for (const envelope of sent) {
    expect(Number.isFinite(envelope.__input_ms)).toBe(true);
    delete envelope.__input_ms;
  }
  expect(sent[3].correlation).toMatch(/^[\x21-\x7e]{1,64}$/);
  expect(sent[3].semantic_action).toBe('captain.red-alert');
  delete sent[3].correlation;
  delete sent[3].semantic_action;
  expect(sent).toEqual([
    { action: 'set_view', console: 'captain', direction: 'Port' },
    { action: 'set_view', console: 'captain', direction: 'Starboard' },
    { action: 'set_view', console: 'captain', direction: 'Fore' },
    { action: 'set_red_alert', console: 'captain', active: true },
    { action: 'set_weapons_hold', console: 'captain', held: true },
  ]);
});

test('captain console: Red Alert feedback is correlated and never paints gameplay optimistically', async ({ page }) => {
  test.setTimeout(20_000);
  await page.goto(CONSOLE_URL);

  await page.evaluate(() => {
    window.__sent = [];
    window.__sendAction = (json) => window.__sent.push(JSON.parse(json));
  });
  const standby = {
    red_alert: false,
    camera_views: ['Fore', 'Port', 'Starboard', 'Aft'],
    view_direction: 'Fore',
    objectives: [],
    blips: [],
  };
  await page.evaluate((state) => window.__updateConsole('captain', JSON.stringify(state)), standby);

  const component = page.locator('ph-red-alert');
  const button = component.locator('#alert-btn');
  const status = component.locator('#feedback-status');
  await button.click();
  const refusedCorrelation = await page.evaluate(() => window.__sent.at(-1).correlation);

  await expect(status).toHaveText(ts('action_feedback.pending'));
  await expect(button).toHaveAttribute('aria-busy', 'true');
  await expect(button).toHaveClass(/standby/);
  await page.evaluate((correlation) => window.__updateActionFeedback({
    correlation,
    state: 'Refused',
  }), refusedCorrelation);
  await expect(status).toHaveText(ts('action_feedback.refused'));
  await expect(button).not.toHaveAttribute('aria-busy');
  await expect(button).toHaveClass(/standby/);

  await button.click();
  await page.evaluate(() => {
    const action = window.__sent.at(-1);
    const router = new window.ActionFeedbackRouter({
      timeoutMs: 25,
      deliver: (value) => window.__updateActionFeedback(value),
    });
    router.track({
      correlation: action.correlation,
      actionId: action.semantic_action,
      console: action.console,
      inputMs: action.__input_ms,
    });
    window.__testFeedbackRouter = router;
  });
  await expect(status).toHaveText(ts('action_feedback.pending'));
  await expect(status).toHaveText(ts('action_feedback.timed_out'));
  await expect(button).toHaveClass(/standby/);

  await page.evaluate((state) => window.__updateConsole(
    'captain',
    JSON.stringify({ ...state, red_alert: true }),
  ), standby);
  await expect(button).toHaveClass(/active/);
  await expect(button).toHaveText(ts('component.red_alert.active'));
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

  await page.evaluate((s) => window.__updateConsole('captain', JSON.stringify(s)), state);

  await expect(page.locator('ph-red-alert').locator('#alert-btn')).toBeDisabled();
  await expect(page.locator('ph-red-alert').locator('#auto-badge')).toBeVisible();
  await expect(page.locator('ph-red-alert').locator('#auto-badge')).toHaveText(ts('console.common.auto'));
});
