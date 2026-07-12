import { test, expect } from './fixtures';

const CONSOLE_URL = '/gui/battleship/power.html';

test('power console: __updateConsole renders power entries and battery', async ({ page }) => {
  await page.goto(CONSOLE_URL);

  const state = {
    consoles: [
      { id: 'helm', label: 'HELM', level: 2, max_level: 4 },
      { id: 'weapons', label: 'WEAPONS', level: 1, max_level: 4 },
      { id: 'sensors', label: 'SENSORS', level: 3, max_level: 4 },
    ],
    total: 6,
    total_max: 8,
    battery_charge: 75.0,
    battery_max: 100.0,
    locked: false,
  };

  await page.evaluate((s) => (window as any).__updateConsole('power', JSON.stringify(s)), state);

  await expect(page.locator('#power-data .power-entry')).toHaveCount(3);
  await expect(page.locator('.power-entry[data-id="helm"]')).toHaveAttribute('data-level', '2');
  await expect(page.locator('.power-entry[data-id="weapons"]')).toHaveAttribute('data-level', '1');
  await expect(page.locator('.power-entry[data-id="sensors"]')).toHaveAttribute('data-level', '3');
  await expect(page.locator('#power-data')).toHaveAttribute('data-total', '6');
  await expect(page.locator('#power-data')).toHaveAttribute('data-total-max', '8');
  await expect(page.locator('#power-data')).toHaveAttribute('data-locked', 'false');
  await expect(page.locator('#bat-val')).toHaveText('75%');
});

test('power console: __updateConsole reflects locked state', async ({ page }) => {
  await page.goto(CONSOLE_URL);

  const state = {
    consoles: [
      { id: 'helm', label: 'HELM', level: 2, max_level: 4 },
      { id: 'weapons', label: 'WEAPONS', level: 2, max_level: 4 },
      { id: 'sensors', label: 'SENSORS', level: 2, max_level: 4 },
    ],
    total: 6,
    total_max: 8,
    battery_charge: 40.0,
    battery_max: 100.0,
    locked: true,
  };

  await page.evaluate((s) => (window as any).__updateConsole('power', JSON.stringify(s)), state);

  await expect(page.locator('#power-data')).toHaveAttribute('data-locked', 'true');
  await expect(page.locator('#bat-val')).toHaveText('40%');
});

test('power console: +/- buttons call __sendAction with correct envelopes', async ({ page }) => {
  await page.goto(CONSOLE_URL);

  await page.evaluate(() => {
    (window as any).__sent = [];
    (window as any).__sendAction = (json: string) => (window as any).__sent.push(json);
  });

  const state = {
    consoles: [
      { id: 'helm', label: 'HELM', level: 2, max_level: 4 },
      { id: 'weapons', label: 'WEAPONS', level: 2, max_level: 4 },
      { id: 'sensors', label: 'SENSORS', level: 2, max_level: 4 },
    ],
    total: 6,
    total_max: 8,
    battery_charge: 80.0,
    battery_max: 100.0,
    locked: false,
  };

  await page.evaluate((s) => (window as any).__updateConsole('power', JSON.stringify(s)), state);

  await page.locator('ph-power-controls [data-group-id="helm"] .mini-btn[data-action="incr"]').click();
  await page.locator('ph-power-controls [data-group-id="sensors"] .mini-btn[data-action="decr"]').click();

  const sent = await page.evaluate(() => (window as any).__sent);
  expect(sent).toHaveLength(2);
  expect(JSON.parse(sent[0])).toEqual({
    action: 'set_power',
    console: 'power',
    target: 'helm',
    level: expect.any(Number),
  });
  expect(JSON.parse(sent[1])).toEqual({
    action: 'set_power',
    console: 'power',
    target: 'sensors',
    level: expect.any(Number),
  });
});
