// Issue #457 — Smoke test: HTML sensors/science console tracer bullet.
//
// The sensors console page (gui/sensors-console.html) is a static HTML file
// copied into dist/gui/ by Trunk. It exercises the ADR-0001 bridge contract:
//   - window.__updateConsole(name, stateJson) renders SensorsConsoleState into DOM.
//   - Action buttons call window.__sendAction(...) with the action envelope.

import { test, expect } from './fixtures';

const CONSOLE_URL = '/gui/sensors-console.html';

const NOMINAL_STATE = {
  scan_range: 1200,
  complexity: 'full',
  impulse_charge_progress: 0,
  blips: [
    {
      uuid: 'ksv-nemesis', name: 'KSV NEMESIS', kind: 'ship',
      radar_x: -0.24, radar_y: -0.12, scaled_radius: 0.03,
      color: [1.0, 0.50, 0.376],
      stance: 'hostile', faction: 'Klingon',
    },
    {
      uuid: 'ast-001', kind: 'asteroid',
      radar_x: -0.65, radar_y: 0.21, scaled_radius: 0.025,
      color: [0.48, 0.75, 1.0],
    },
  ],
  regions: [],
  target_uuid: null,
};

test('sensors console: __updateConsole renders scan range', async ({ page }) => {
  await page.goto(CONSOLE_URL);

  await page.evaluate(
    (s) => (window as any).__updateConsole('Sensors', JSON.stringify(s)),
    NOMINAL_STATE,
  );

  await expect(page.locator('#scan-range-val')).toHaveText('1200');
});

test('sensors console: contact sub shows blip count', async ({ page }) => {
  await page.goto(CONSOLE_URL);

  await page.evaluate(
    (s) => (window as any).__updateConsole('Sensors', JSON.stringify(s)),
    NOMINAL_STATE,
  );

  await expect(page.locator('#contact-sub')).toContainText('2 CONTACTS');
});

test('sensors console: live region overlays survive the demo animation tick', async ({ page }) => {
  await page.goto(CONSOLE_URL);
  await page.evaluate(() => {
    const widgetCtor = (window as any).RadarWidget;
    const originalUpdate = widgetCtor.prototype.update;
    (window as any).__radarRegionCounts = [];
    widgetCtor.prototype.update = function(data: any) {
      (window as any).__radarRegionCounts.push(data?.regions?.length ?? 0);
      return originalUpdate.call(this, data);
    };
  });

  const withRegion = {
    ...NOMINAL_STATE,
    regions: [{
      uuid: 'field-1',
      shape: 'torus',
      radar_x: 0.1,
      radar_y: -0.2,
      scaled_outer_radius: 0.4,
      scaled_inner_radius: 0.2,
      color: [0.52, 0.32, 0.18],
    }],
  };

  await page.evaluate(
    (s) => (window as any).__updateConsole('Sensors', JSON.stringify(s)),
    withRegion,
  );
  await page.waitForTimeout(1100);

  const regionCounts = await page.evaluate(
    () => (window as any).__radarRegionCounts,
  );
  expect(regionCounts).toEqual([1]);
});

test('sensors console: target panel shows NO TARGET when no target set', async ({ page }) => {
  await page.goto(CONSOLE_URL);

  await page.evaluate(
    (s) => (window as any).__updateConsole('Sensors', JSON.stringify(s)),
    NOMINAL_STATE,
  );

  await expect(page.locator('#tgt-name')).toHaveText('NO TARGET');
  await expect(page.locator('#tgt-kind-tag')).toHaveText('NO CONTACT');
});

test('sensors console: target panel renders target name when target set', async ({ page }) => {
  await page.goto(CONSOLE_URL);

  const withTarget = {
    ...NOMINAL_STATE,
    target_uuid: 'ksv-nemesis',
    target_name: 'KSV NEMESIS',
    target_kind: 'ship',
    target_stance: 'hostile',
    target_bearing: 243.4,
    target_range: 321,
  };
  await page.evaluate(
    (s) => (window as any).__updateConsole('Sensors', JSON.stringify(s)),
    withTarget,
  );

  await expect(page.locator('#tgt-name')).toHaveText('KSV NEMESIS');
  await expect(page.locator('#tgt-kind-tag')).toHaveText('SHIP');
});

test('sensors console: cancel impulse button hidden when charge=0', async ({ page }) => {
  await page.goto(CONSOLE_URL);

  await page.evaluate(
    (s) => (window as any).__updateConsole('Sensors', JSON.stringify(s)),
    NOMINAL_STATE,
  );

  await expect(page.locator('#btn-cancel-impulse')).toHaveCSS('display', 'none');
});

test('sensors console: cancel impulse button visible when charging', async ({ page }) => {
  await page.goto(CONSOLE_URL);

  const charging = { ...NOMINAL_STATE, impulse_charge_progress: 0.5 };
  await page.evaluate(
    (s) => (window as any).__updateConsole('Sensors', JSON.stringify(s)),
    charging,
  );

  await expect(page.locator('#btn-cancel-impulse')).not.toHaveCSS('display', 'none');
});

test('sensors console: on-screen button calls __sendAction with set_view', async ({ page }) => {
  await page.goto(CONSOLE_URL);

  // Stub __sendAction to capture calls.
  await page.evaluate(() => {
    (window as any).__sent = [];
    (window as any).__sendAction = (json: string) => (window as any).__sent.push(json);
  });

  await page.locator('#btn-on-screen').click();

  const sent: string[] = await page.evaluate(() => (window as any).__sent);
  expect(sent).toHaveLength(1);
  const parsed = JSON.parse(sent[0]);
  expect(parsed.action).toBe('set_view');
  expect(parsed.console).toBe('Sensors');
  expect(parsed.direction).toBe('SensorsRadar');
});

test('sensors console: cancel impulse button calls __sendAction with cancel_impulse', async ({ page }) => {
  await page.goto(CONSOLE_URL);

  // Make button visible first.
  const charging = { ...NOMINAL_STATE, impulse_charge_progress: 0.5 };
  await page.evaluate(
    (s) => (window as any).__updateConsole('Sensors', JSON.stringify(s)),
    charging,
  );

  // Stub __sendAction to capture calls.
  await page.evaluate(() => {
    (window as any).__sent = [];
    (window as any).__sendAction = (json: string) => (window as any).__sent.push(json);
  });

  await page.locator('#btn-cancel-impulse').click();

  const sent: string[] = await page.evaluate(() => (window as any).__sent);
  expect(sent).toHaveLength(1);
  const parsed = JSON.parse(sent[0]);
  expect(parsed.action).toBe('cancel_impulse');
  expect(parsed.console).toBe('Sensors');
});
