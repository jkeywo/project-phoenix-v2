import { test, expect } from './fixtures';

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
  await page.evaluate((s) => (window as any).__updateConsole('sensors', JSON.stringify(s)), NOMINAL_STATE);
  await expect(page.locator('#scan-range-val')).toHaveText('1200');
});

test('sensors console: contact summary shows blip count', async ({ page }) => {
  await page.goto(CONSOLE_URL);
  await page.evaluate((s) => (window as any).__updateConsole('sensors', JSON.stringify(s)), NOMINAL_STATE);
  await expect(page.locator('#contact-sub')).toContainText('2 CONTACTS');
});

test('sensors console: sensor radar shows on-screen state when active', async ({ page }) => {
  await page.goto(CONSOLE_URL);
  await page.evaluate((s) => (window as any).__updateConsole('sensors', JSON.stringify(s)), {
    ...NOMINAL_STATE,
    on_screen_active: true,
  });
  await expect(page.locator('ph-sensor-radar').locator('#on-screen-btn')).toHaveClass(/active/);
});

test('sensors console: target panel shows NO TARGET when no target set', async ({ page }) => {
  await page.goto(CONSOLE_URL);
  await page.evaluate((s) => (window as any).__updateConsole('sensors', JSON.stringify(s)), NOMINAL_STATE);
  await expect(page.locator('#tgt-name')).toHaveText('NO TARGET');
  await expect(page.locator('#tgt-kind-tag')).toHaveText('NO CONTACT');
});

test('sensors console: target panel renders target name when target set', async ({ page }) => {
  await page.goto(CONSOLE_URL);
  await page.evaluate((s) => (window as any).__updateConsole('sensors', JSON.stringify(s)), {
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
  await page.evaluate((s) => (window as any).__updateConsole('sensors', JSON.stringify(s)), {
    ...NOMINAL_STATE,
    target_uuid: 'ksv-nemesis',
    target_name: 'KSV NEMESIS',
    target_kind: 'ship',
    target_shield_fraction: 0.5,
    target_shields: [],
  });
  await expect(page.locator('#shield-facings .s-facing')).toHaveCount(1);
  await expect(page.locator('#shield-facings .s-facing .lbl')).toHaveText('SHLD');
  await expect(page.locator('#shield-facings .s-facing .pct')).toHaveText('50%');
  await expect(page.locator('#shields-tag')).toHaveText('ONLINE');
});

test('sensors console: renders SHIELD DOWN when target_shield_fraction is zero', async ({ page }) => {
  await page.goto(CONSOLE_URL);
  await page.evaluate((s) => (window as any).__updateConsole('sensors', JSON.stringify(s)), {
    ...NOMINAL_STATE,
    target_uuid: 'ksv-nemesis',
    target_name: 'KSV NEMESIS',
    target_kind: 'ship',
    target_shield_fraction: 0,
    target_shields: [],
  });
  await expect(page.locator('#shield-facings .s-facing .pct')).toHaveText('DOWN');
  await expect(page.locator('#shields-tag')).toHaveText('SHIELD DOWN');
});

test('sensors console: renders NO SHIELD DATA when target has no shield_fraction', async ({ page }) => {
  await page.goto(CONSOLE_URL);
  await page.evaluate((s) => (window as any).__updateConsole('sensors', JSON.stringify(s)), {
    ...NOMINAL_STATE,
    target_uuid: 'ast-001',
    target_name: 'ASTEROID',
    target_kind: 'asteroid',
    target_shields: [],
  });
  await expect(page.locator('#shield-facings')).toContainText('NO SHIELD DATA');
});

test('sensors console: cancel impulse button hidden when charge=0', async ({ page }) => {
  await page.goto(CONSOLE_URL);
  await page.evaluate((s) => (window as any).__updateConsole('sensors', JSON.stringify(s)), NOMINAL_STATE);
  await expect(page.locator('#btn-cancel-impulse')).toHaveCSS('display', 'none');
});

test('sensors console: cancel impulse button visible when charging', async ({ page }) => {
  await page.goto(CONSOLE_URL);
  await page.evaluate((s) => (window as any).__updateConsole('sensors', JSON.stringify(s)), { ...NOMINAL_STATE, impulse_charge_progress: 0.5 });
  await expect(page.locator('#btn-cancel-impulse')).not.toHaveCSS('display', 'none');
});

test('sensors console: on-screen button calls __sendAction with set_view', async ({ page }) => {
  await page.goto(CONSOLE_URL);
  await page.evaluate(() => {
    (window as any).__sent = [];
    (window as any).__sendAction = (json: string) => (window as any).__sent.push(json);
  });
  await page.locator('ph-sensor-radar').locator('#on-screen-btn').click();
  const sent = await page.evaluate(() => (window as any).__sent);
  expect(sent).toHaveLength(1);
  const parsed = JSON.parse(sent[0]);
  expect(parsed.action).toBe('set_view');
  expect(parsed.console).toBe('sensors');
  expect(parsed.direction).toBe('SensorsRadar');
});

test('sensors console: cancel impulse button calls __sendAction with cancel_impulse', async ({ page }) => {
  await page.goto(CONSOLE_URL);
  await page.evaluate((s) => (window as any).__updateConsole('sensors', JSON.stringify(s)), { ...NOMINAL_STATE, impulse_charge_progress: 0.5 });
  await page.evaluate(() => {
    (window as any).__sent = [];
    (window as any).__sendAction = (json: string) => (window as any).__sent.push(json);
  });
  await page.locator('#btn-cancel-impulse').click();
  const sent = await page.evaluate(() => (window as any).__sent);
  expect(sent).toHaveLength(1);
  const parsed = JSON.parse(sent[0]);
  expect(parsed.action).toBe('cancel_impulse');
  expect(parsed.console).toBe('sensors');
});
