import { test, expect } from './fixtures';
import { ts } from './strings';

const CONSOLE_URL = '/gui/battleship/sensors.html';

const NOMINAL_STATE = {
  scan_range: 1200,
  impulse_charge_progress: 0,
  blips: [
    { uuid: 'ksv-nemesis', name: 'KSV NEMESIS', kind: 'ship', radar_x: -0.24, radar_y: -0.12, scaled_radius: 0.03, color: [1.0, 0.5, 0.376], stance: 'hostile', faction: 'Klingon' },
    { uuid: 'ast-001', kind: 'asteroid', radar_x: -0.65, radar_y: 0.21, scaled_radius: 0.025, color: [0.48, 0.75, 1.0] },
  ],
  regions: [],
  target_uuid: null,
};

test('sensors console: __updateConsole renders scan range', async ({ page }) => {
  await page.goto(CONSOLE_URL);
  await page.evaluate((s) => window.__updateConsole('sensors', JSON.stringify(s)), NOMINAL_STATE);
  await expect(page.locator('#scan-range-val')).toHaveText('1200');
});

test('sensors console: contact summary shows blip count', async ({ page }) => {
  await page.goto(CONSOLE_URL);
  await page.evaluate((s) => window.__updateConsole('sensors', JSON.stringify(s)), NOMINAL_STATE);
  await expect(page.locator('#contact-sub')).toContainText(ts('console.common.contacts.other', { n: 2 }));
});

test('sensors console: sensor radar shows on-screen state when active', async ({ page }) => {
  await page.goto(CONSOLE_URL);
  await page.evaluate((s) => window.__updateConsole('sensors', JSON.stringify(s)), {
    ...NOMINAL_STATE,
    on_screen_active: true,
  });
  await expect(page.locator('ph-sensor-radar').locator('#on-screen-btn')).toHaveClass(/active/);
});

test('sensors console: target panel shows NO TARGET when no target set', async ({ page }) => {
  await page.goto(CONSOLE_URL);
  await page.evaluate((s) => window.__updateConsole('sensors', JSON.stringify(s)), NOMINAL_STATE);
  await expect(page.locator('#tgt-name')).toHaveText(ts('console.common.no_target'));
  await expect(page.locator('#tgt-kind-tag')).toHaveText(ts('console.common.no_contact'));
});

test('sensors console: target panel renders target name when target set', async ({ page }) => {
  await page.goto(CONSOLE_URL);
  await page.evaluate((s) => window.__updateConsole('sensors', JSON.stringify(s)), {
    ...NOMINAL_STATE,
    target_uuid: 'ksv-nemesis',
    target_name: 'KSV NEMESIS',
    target_kind: 'ship',
    target_stance: 'hostile',
    target_bearing: 243.4,
    target_range: 321,
  });
  await expect(page.locator('#tgt-name')).toHaveText('KSV NEMESIS');
  await expect(page.locator('#tgt-kind-tag')).toHaveText('SHIP');
});

test('sensors console: renders single shield bar when target_shield_fraction set', async ({ page }) => {
  await page.goto(CONSOLE_URL);
  await page.evaluate((s) => window.__updateConsole('sensors', JSON.stringify(s)), {
    ...NOMINAL_STATE,
    target_uuid: 'ksv-nemesis',
    target_name: 'KSV NEMESIS',
    target_kind: 'ship',
    target_shield_fraction: 0.5,
    target_shields: [],
  });
  await expect(page.locator('#shield-facings .s-facing')).toHaveCount(1);
  await expect(page.locator('#shield-facings .s-facing .lbl')).toHaveText(ts('console.shield.shld'));
  await expect(page.locator('#shield-facings .s-facing .pct')).toHaveText('50%');
  await expect(page.locator('#shields-tag')).toHaveText(ts('console.shield.online'));
});

test('sensors console: renders SHIELD DOWN when target_shield_fraction is zero', async ({ page }) => {
  await page.goto(CONSOLE_URL);
  await page.evaluate((s) => window.__updateConsole('sensors', JSON.stringify(s)), {
    ...NOMINAL_STATE,
    target_uuid: 'ksv-nemesis',
    target_name: 'KSV NEMESIS',
    target_kind: 'ship',
    target_shield_fraction: 0,
    target_shields: [],
  });
  await expect(page.locator('#shield-facings .s-facing .pct')).toHaveText(ts('console.shield.down'));
  await expect(page.locator('#shields-tag')).toHaveText(ts('console.shield.shield_down'));
});

test('sensors console: renders NO SHIELD DATA when target has no shield_fraction', async ({ page }) => {
  await page.goto(CONSOLE_URL);
  await page.evaluate((s) => window.__updateConsole('sensors', JSON.stringify(s)), {
    ...NOMINAL_STATE,
    target_uuid: 'ast-001',
    target_name: 'ASTEROID',
    target_kind: 'asteroid',
    target_shields: [],
  });
  await expect(page.locator('#shield-facings')).toContainText(ts('console.common.no_shield_data'));
});

test('sensors console: cancel impulse button hidden when charge=0', async ({ page }) => {
  await page.goto(CONSOLE_URL);
  await page.evaluate((s) => window.__updateConsole('sensors', JSON.stringify(s)), NOMINAL_STATE);
  await expect(page.locator('#btn-cancel-impulse')).toHaveCSS('display', 'none');
});

test('sensors console: cancel impulse button visible when charging', async ({ page }) => {
  await page.goto(CONSOLE_URL);
  await page.evaluate((s) => window.__updateConsole('sensors', JSON.stringify(s)), { ...NOMINAL_STATE, impulse_charge_progress: 0.5 });
  await expect(page.locator('#btn-cancel-impulse')).not.toHaveCSS('display', 'none');
});

test('sensors console: on-screen button calls __sendAction with set_view', { tag: '@core' }, async ({ page }) => {
  await page.goto(CONSOLE_URL);
  await page.evaluate(() => {
    window.__sent = [];
    window.__sendAction = (json) => window.__sent.push(json);
  });
  // #1287 routes this through the shared semantic action, and the handler
  // resolves its view from console state — with no payload pushed it refuses
  // and sends nothing at all.
  await page.evaluate((s) => window.__updateConsole('sensors', JSON.stringify(s)), NOMINAL_STATE);
  await page.locator('ph-sensor-radar').locator('#on-screen-btn').click();
  const sent = await page.evaluate(() => window.__sent);
  expect(sent).toHaveLength(1);
  const parsed = JSON.parse(sent[0]);
  expect(parsed.action).toBe('set_view');
  expect(parsed.console).toBe('sensors');
  expect(parsed.direction).toBe('SensorsRadar');
});

test('sensors console: cancel impulse button calls __sendAction with cancel_impulse', async ({ page }) => {
  await page.goto(CONSOLE_URL);
  await page.evaluate((s) => window.__updateConsole('sensors', JSON.stringify(s)), { ...NOMINAL_STATE, impulse_charge_progress: 0.5 });
  await page.evaluate(() => {
    window.__sent = [];
    window.__sendAction = (json) => window.__sent.push(json);
  });
  await page.locator('#btn-cancel-impulse').click();
  const sent = await page.evaluate(() => window.__sent);
  expect(sent).toHaveLength(1);
  const parsed = JSON.parse(sent[0]);
  expect(parsed.action).toBe('cancel_impulse');
  expect(parsed.console).toBe('sensors');
});

test('sensors console: independent focused instruments retain their owner and working view', { tag: '@core' }, async ({ page }) => {
  await page.goto(CONSOLE_URL);
  const state = {
    ...NOMINAL_STATE,
    target_uuid: 'public-contact',
    scans: [
      { system_id: 'focus-one', capable: true, controllable: true, history: [], focused: { subject: 'public-contact', progress: 250, waiting: '' } },
      { system_id: 'focus-two', capable: true, controllable: true, history: [], focused: null,
        reading: { subject_uuid: 'public-contact', subject_name: '', band: 'detailed', band_label: '',
          taken_at_tick: 200, condition_fraction: 0.6, condition_step: 0.1, mass: 10,
          forecast: { sample_ticks: [100, 200], horizon_secs: 120,
            assumptions: ['component.scan.forecast.linear', 'component.scan.forecast.quantisation'],
            ranges: [{ quantity: { kind: 'condition' }, lower: 0.25, upper: 0.55 }] } } },
    ],
  };
  await page.evaluate((payload) => {
    window.__sent = [];
    window.__sendAction = (json) => window.__sent.push(JSON.parse(json));
    window.__updateConsole('sensors', JSON.stringify(payload));
  }, state);
  const readout = page.locator('ph-scan-readout');
  await expect(readout.locator('#progress')).toContainText('25%');
  await readout.locator('#instrument').selectOption('focus-two');
  await expect(readout.locator('.forecast')).toContainText('120');
  await expect(readout.locator('.forecast')).toContainText('25–55%');
  await readout.locator('#action').click();
  const sent = await page.evaluate(() => window.__sent);
  expect(sent).toHaveLength(1);
  expect(sent[0]).toMatchObject({ action: 'scan_target', control_system_id: 'focus-two', uuid: 'public-contact' });
  expect(sent[0].correlation).toEqual(expect.any(String));

  await page.evaluate((payload) => window.__updateConsole('sensors', JSON.stringify(payload)), {
    ...state,
    scans: state.scans.map((scan) => scan.system_id === 'focus-two' ? { ...scan, controllable: false } : scan),
  });
  await expect(readout.locator('#instrument')).toHaveValue('focus-two');
  await expect(readout.locator('#action')).toBeHidden();
  await readout.locator('#instrument').selectOption('focus-one');
  await expect(readout.locator('.forecast')).toHaveCount(0);
  await expect(readout.locator('#progress')).toContainText('25%');
  await expect(readout.locator('#cancel')).toBeVisible();
});
