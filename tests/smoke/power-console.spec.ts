// Issue #425 — Smoke test: HTML power console tracer bullet.
//
// The power console page (gui/power-console.html) is a static HTML file
// copied into dist/gui/ by Trunk. It exercises the ADR-0001 bridge contract:
//   - window.__updateConsole(name, stateJson) renders PowerConsoleState into DOM.
//   - The +/- buttons call window.__sendAction(...) with the action envelope.

import { test, expect } from './fixtures';

const CONSOLE_URL = '/gui/power-console.html';

test('power console: __updateConsole renders power entries and battery', async ({ page }) => {
  await page.goto(CONSOLE_URL);

  const state = {
    consoles: [
      { id: 'Helm',     label: 'HELM',    level: 2, max_level: 4 },
      { id: 'Tactical', label: 'WEAPONS', level: 1, max_level: 4 },
      { id: 'Sensors',  label: 'SENSORS', level: 3, max_level: 4 },
    ],
    total:          6,
    total_max:      8,
    battery_charge: 75.0,
    battery_max:    100.0,
    locked:         false,
  };

  await page.evaluate((s) => (window as any).__updateConsole('Power', JSON.stringify(s)), state);

  // Three entries rendered in #power-data.
  await expect(page.locator('#power-data .power-entry')).toHaveCount(3);

  // Each entry carries correct data attrs.
  const helm = page.locator('.power-entry[data-id="Helm"]');
  await expect(helm).toHaveAttribute('data-level', '2');
  await expect(helm).toHaveAttribute('data-max-level', '4');

  const tactical = page.locator('.power-entry[data-id="Tactical"]');
  await expect(tactical).toHaveAttribute('data-level', '1');

  const sensors = page.locator('.power-entry[data-id="Sensors"]');
  await expect(sensors).toHaveAttribute('data-level', '3');

  // #power-data carries pool totals and lock flag.
  const pd = page.locator('#power-data');
  await expect(pd).toHaveAttribute('data-total', '6');
  await expect(pd).toHaveAttribute('data-total-max', '8');
  await expect(pd).toHaveAttribute('data-locked', 'false');

  // Battery percentage displayed.
  await expect(page.locator('#bat-val')).toHaveText('75%');
});

test('power console: __updateConsole reflects locked state', async ({ page }) => {
  await page.goto(CONSOLE_URL);

  const state = {
    consoles: [
      { id: 'Helm',     label: 'HELM',    level: 2, max_level: 4 },
      { id: 'Tactical', label: 'WEAPONS', level: 2, max_level: 4 },
      { id: 'Sensors',  label: 'SENSORS', level: 2, max_level: 4 },
    ],
    total:          6,
    total_max:      8,
    battery_charge: 40.0,
    battery_max:    100.0,
    locked:         true,
  };

  await page.evaluate((s) => (window as any).__updateConsole('Power', JSON.stringify(s)), state);

  await expect(page.locator('#power-data')).toHaveAttribute('data-locked', 'true');
  await expect(page.locator('#bat-val')).toHaveText('40%');
});

test('power console: Low complexity preset hides the overflow controls, Std shows them', async ({ page }) => {
  // Issue #461 — console-core applies gui/hideable-elements.js after render,
  // toggling .cpx-hidden on [data-hideable] per the active preset.
  await page.goto(CONSOLE_URL);

  const base = {
    consoles: [{ id: 'Helm', label: 'HELM', level: 2, max_level: 4 }],
    total: 6, total_max: 8, battery_charge: 50.0, battery_max: 100.0, locked: false,
  };
  const overflow = '[data-hideable="power_overflow_controls"]';

  await page.evaluate((s) => (window as any).__updateConsole('Power', JSON.stringify(s)),
    { ...base, complexityPreset: 'Low' });
  await expect(page.locator(overflow)).toHaveClass(/cpx-hidden/);
  await expect(page.locator(overflow)).toBeHidden();

  await page.evaluate((s) => (window as any).__updateConsole('Power', JSON.stringify(s)),
    { ...base, complexityPreset: 'Std' });
  await expect(page.locator(overflow)).not.toHaveClass(/cpx-hidden/);
  await expect(page.locator(overflow)).toBeVisible();
});

test('power console: +/- buttons call __sendAction with correct envelopes', async ({ page }) => {
  await page.goto(CONSOLE_URL);

  // Stub __sendAction to capture calls.
  await page.evaluate(() => {
    (window as any).__sent = [];
    (window as any).__sendAction = (json: string) => (window as any).__sent.push(json);
  });

  const state = {
    consoles: [
      { id: 'Helm',     label: 'HELM',    level: 2, max_level: 4 },
      { id: 'Tactical', label: 'WEAPONS', level: 2, max_level: 4 },
      { id: 'Sensors',  label: 'SENSORS', level: 2, max_level: 4 },
    ],
    total:          6,
    total_max:      8,
    battery_charge: 80.0,
    battery_max:    100.0,
    locked:         false,
  };

  await page.evaluate((s) => (window as any).__updateConsole('Power', JSON.stringify(s)), state);

  // Click the increment button for Helm.
  await page.locator('#inc-Helm').click();

  // Click the decrement button for Sensors.
  await page.locator('#dec-Sensors').click();

  const sent: string[] = await page.evaluate(() => (window as any).__sent);
  expect(sent).toHaveLength(2);

  expect(JSON.parse(sent[0])).toEqual({
    action: 'increase_power',
    console: 'Power',
    target: 'Helm',
  });
  expect(JSON.parse(sent[1])).toEqual({
    action: 'decrease_power',
    console: 'Power',
    target: 'Sensors',
  });
});
