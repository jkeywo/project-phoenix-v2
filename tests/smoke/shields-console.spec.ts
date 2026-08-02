import { test, expect } from './fixtures';
import { ts } from './strings';

const CONSOLE_URL = '/gui/battleship/shields.html';

const NOMINAL_STATE = {
  facings: [
    { arc_id: 'fore', label: 'Fore', hp: 100, max_hp: 100, online: true, offline_remaining: 0, is_focused: true, center_deg: 0, width_deg: 90 },
    { arc_id: 'port', label: 'Port', hp: 72, max_hp: 100, online: true, offline_remaining: 0, is_focused: false, center_deg: 270, width_deg: 90 },
    { arc_id: 'aft', label: 'Aft', hp: 0, max_hp: 100, online: false, offline_remaining: 8, is_focused: false, center_deg: 180, width_deg: 90 },
    { arc_id: 'starboard', label: 'Starboard', hp: 88, max_hp: 100, online: true, offline_remaining: 0, is_focused: false, center_deg: 90, width_deg: 90 },
  ],
  hull_integrity_pct: 78,
  focused_facing: 'Fore',
  threat_bearing: 272,
  grid_status: 'GRID NOMINAL',
};

test('shields console: __updateConsole renders one facing entry per arc', async ({ page }) => {
  await page.goto(CONSOLE_URL);
  await page.evaluate((s) => (window as any).__updateConsole('shields', JSON.stringify(s)), NOMINAL_STATE);
  await expect(page.locator('ph-shield-facings .arc-path')).toHaveCount(4);
});

test('shields console: variable arc count renders correctly', async ({ page }) => {
  await page.goto(CONSOLE_URL);
  await page.evaluate((s) => (window as any).__updateConsole('shields', JSON.stringify(s)), {
    facings: [{ arc_id: 'all', label: 'All', hp: 15, max_hp: 15, online: true, offline_remaining: 0, is_focused: false, center_deg: 0, width_deg: 360 }],
    hull_integrity_pct: 100,
    focused_facing: null,
    threat_bearing: null,
    grid_status: 'GRID NOMINAL',
  });
  await expect(page.locator('ph-shield-facings .arc-path')).toHaveCount(1);
  await expect(page.locator('ph-shield-facings .facing-label')).toHaveText(['ALL']);
});

test('shields console: facing values render from state', async ({ page }) => {
  await page.goto(CONSOLE_URL);
  await page.evaluate((s) => (window as any).__updateConsole('shields', JSON.stringify(s)), NOMINAL_STATE);
  const labels = page.locator('ph-shield-facings .facing-label');
  await expect(labels.nth(0)).toHaveText('FORE');
  await expect(labels.nth(2)).toHaveText('AFT');
});

test('shields console: focused facing gets focused class', async ({ page }) => {
  await page.goto(CONSOLE_URL);
  await page.evaluate((s) => (window as any).__updateConsole('shields', JSON.stringify(s)), NOMINAL_STATE);
  await expect(page.locator('ph-shield-facings .arc-path.focused')).toHaveCount(1);
  await expect(page.locator('ph-shield-facings .facing-label.focused-label')).toHaveText('FORE');
});

test('shields console: hull bar reflects hull_integrity_pct', async ({ page }) => {
  await page.goto(CONSOLE_URL);
  await page.evaluate((s) => (window as any).__updateConsole('shields', JSON.stringify(s)), NOMINAL_STATE);
  await expect(page.locator('#hull-val')).toHaveText('78%');
  await expect(page.locator('#hull-tag')).toHaveText('78%');
});

test('shields console: threat indicator visible when threat_bearing set', async ({ page }) => {
  await page.goto(CONSOLE_URL);
  await page.evaluate((s) => (window as any).__updateConsole('shields', JSON.stringify(s)), NOMINAL_STATE);
  await expect(page.locator('#threat-row')).toHaveClass(/active/);
  await expect(page.locator('#threat-bearing')).toHaveText(/272.*M/);
});

test('shields console: threat indicator hidden when no target', async ({ page }) => {
  await page.goto(CONSOLE_URL);
  await page.evaluate((s) => (window as any).__updateConsole('shields', JSON.stringify(s)), { ...NOMINAL_STATE, threat_bearing: null });
  await expect(page.locator('#threat-row')).not.toHaveClass(/active/);
});

test('shields console: shield segment click sends set_shield_focus with arc_id', async ({ page }) => {
  await page.goto(CONSOLE_URL);
  await page.evaluate((s) => (window as any).__updateConsole('shields', JSON.stringify(s)), NOMINAL_STATE);
  await page.evaluate(() => {
    (window as any).__sent = [];
    (window as any).__sendAction = (json: string) => (window as any).__sent.push(json);
  });
  await page.locator('ph-shield-facings .arc-path[data-facing-id="port"]').click();
  const sent = await page.evaluate(() => (window as any).__sent);
  expect(sent).toHaveLength(1);
  expect(JSON.parse(sent[0])).toEqual({ action: 'set_shield_focus', console: 'shields', arc_id: 'port', focused: true });
});

test('shields console: clicking focused facing clears focus via focused=false', async ({ page }) => {
  await page.goto(CONSOLE_URL);
  await page.evaluate((s) => (window as any).__updateConsole('shields', JSON.stringify(s)), NOMINAL_STATE);
  await page.evaluate(() => {
    (window as any).__sent = [];
    (window as any).__sendAction = (json: string) => (window as any).__sent.push(json);
  });
  await page.locator('ph-shield-facings .arc-path[data-facing-id="fore"]').click();
  const sent = await page.evaluate(() => (window as any).__sent);
  expect(sent).toHaveLength(1);
  expect(JSON.parse(sent[0])).toEqual({ action: 'set_shield_focus', console: 'shields', arc_id: 'fore', focused: false });
});

test('shields console: header shows focused facing display', async ({ page }) => {
  await page.goto(CONSOLE_URL);
  await page.evaluate((s) => (window as any).__updateConsole('shields', JSON.stringify(s)), { ...NOMINAL_STATE, focused_facing: 'Fore' });
  await expect(page.locator('#focus-display')).toHaveText('Fore');
  await expect(page.locator('#footer-focus')).toHaveText('Fore');
});

test('shields console: grid status shown in footer', async ({ page }) => {
  await page.goto(CONSOLE_URL);
  await page.evaluate((s) => (window as any).__updateConsole('shields', JSON.stringify(s)), NOMINAL_STATE);
  await expect(page.locator('#footer-grid')).toHaveText(ts('component.shield_panel.grid_nominal'));
});
