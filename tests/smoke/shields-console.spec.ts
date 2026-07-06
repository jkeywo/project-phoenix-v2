// Issue #424 — Smoke test: HTML shields console tracer bullet.
//
// The shields console page (gui/shield-console.html) is a static HTML file
// copied into dist/gui/ by Trunk. It exercises the ADR-0001 bridge contract:
//   - window.__updateConsole(name, stateJson) renders ShieldsConsoleState into DOM.
//   - Focus buttons call window.__sendAction(...) with the action envelope.
//
// Issue #514 — Shields decomposed into per-arc fine systems. Facings now
// carry `arc_id`, `center_deg`, `width_deg`; the panel renders arcs
// dynamically from the facings list; focus button clicks send
// `set_shield_focus` with the `arc_id` string.

import { test, expect } from './fixtures';

const CONSOLE_URL = '/gui/shields-console.html';

const NOMINAL_STATE = {
  facings: [
    { arc_id: 'fore', label: 'Fore', hp: 100, max_hp: 100, online: true, offline_remaining: 0, is_focused: true, center_deg: 0, width_deg: 90 },
    { arc_id: 'port', label: 'Port', hp: 72, max_hp: 100, online: true, offline_remaining: 0, is_focused: false, center_deg: 270, width_deg: 90 },
    { arc_id: 'aft', label: 'Aft', hp: 0, max_hp: 100, online: false, offline_remaining: 8, is_focused: false, center_deg: 180, width_deg: 90 },
    { arc_id: 'starboard', label: 'Starboard', hp: 88, max_hp: 100, online: true, offline_remaining: 0, is_focused: false, center_deg: 90, width_deg: 90 },
  ],
  hull_integrity_pct: 78,
  focused_facing: 'Fore',
  target_bearing: 272,
  grid_status: 'GRID NOMINAL',
};

test('shields console: __updateConsole renders one card per facing', async ({ page }) => {
  await page.goto(CONSOLE_URL);

  await page.evaluate((s) => (window as any).__updateConsole('Shields', JSON.stringify(s)), NOMINAL_STATE);

  await expect(page.locator('.quad-card')).toHaveCount(4);
});

test('shields console: variable arc count renders correctly', async ({ page }) => {
  // Single omni arc (NPC-style ship shape).
  const omniState = {
    facings: [
      { arc_id: 'all', label: 'All', hp: 15, max_hp: 15, online: true, offline_remaining: 0, is_focused: false, center_deg: 0, width_deg: 360 },
    ],
    hull_integrity_pct: 100,
    focused_facing: null,
    target_bearing: null,
    grid_status: 'GRID NOMINAL',
  };
  await page.goto(CONSOLE_URL);
  await page.evaluate((s) => (window as any).__updateConsole('Shields', JSON.stringify(s)), omniState);
  await expect(page.locator('.quad-card')).toHaveCount(1);
  await expect(page.locator('.quad-card .qc-name')).toHaveText('ALL');
});

test('shields console: facing values render from state', async ({ page }) => {
  await page.goto(CONSOLE_URL);

  await page.evaluate((s) => (window as any).__updateConsole('Shields', JSON.stringify(s)), NOMINAL_STATE);

  // FORE = 100%
  const foreCard = page.locator('.quad-card').nth(0);
  await expect(foreCard.locator('.qc-name')).toHaveText('FORE');

  // AFT (third in the facings order) = 0% (down)
  const aftCard = page.locator('.quad-card').nth(2);
  await expect(aftCard.locator('.qc-name')).toHaveText('AFT');
});

test('shields console: focused quad-card gets focused class', async ({ page }) => {
  await page.goto(CONSOLE_URL);

  await page.evaluate((s) => (window as any).__updateConsole('Shields', JSON.stringify(s)), NOMINAL_STATE);

  // Fore is focused → its card should have the focused class.
  await expect(page.locator('.quad-card.focused')).toHaveCount(1);
  await expect(page.locator('.quad-card.focused .qc-name')).toHaveText('FORE');
});

test('shields console: hull bar reflects hull_integrity_pct', async ({ page }) => {
  await page.goto(CONSOLE_URL);

  await page.evaluate((s) => (window as any).__updateConsole('Shields', JSON.stringify(s)), NOMINAL_STATE);

  await expect(page.locator('#hull-val')).toHaveText('78%');
  await expect(page.locator('#hull-tag')).toHaveText('78%');
});

test('shields console: threat indicator visible when target_bearing set', async ({ page }) => {
  await page.goto(CONSOLE_URL);

  await page.evaluate((s) => (window as any).__updateConsole('Shields', JSON.stringify(s)), NOMINAL_STATE);

  await expect(page.locator('#threat-row')).toHaveClass(/active/);
  await expect(page.locator('#threat-bearing')).toHaveText('272°M');
});

test('shields console: threat indicator hidden when no target', async ({ page }) => {
  await page.goto(CONSOLE_URL);

  const noTargetState = { ...NOMINAL_STATE, target_bearing: null };
  await page.evaluate((s) => (window as any).__updateConsole('Shields', JSON.stringify(s)), noTargetState);

  await expect(page.locator('#threat-row')).not.toHaveClass(/active/);
});

test('shields console: shield segment click sends set_shield_focus with arc_id', async ({ page }) => {
  await page.goto(CONSOLE_URL);

  await page.evaluate((s) => (window as any).__updateConsole('Shields', JSON.stringify(s)), NOMINAL_STATE);
  await page.evaluate(() => {
    (window as any).__sent = [];
    (window as any).__sendAction = (json: string) => (window as any).__sent.push(json);
  });

  // Click Port segment (non-focused in nominal state) to set focus to Port.
  await page.locator('.shield-segment[data-arc-id="port"]').click();

  const sent: string[] = await page.evaluate(() => (window as any).__sent);
  expect(sent).toHaveLength(1);
  expect(JSON.parse(sent[0])).toEqual({
    action: 'set_shield_focus',
    console: 'shields',
    arc_id: 'port',
    focused: true,
  });
});

test('shields console: clicking focused facing clears focus via focused=false', async ({ page }) => {
  await page.goto(CONSOLE_URL);

  await page.evaluate((s) => (window as any).__updateConsole('Shields', JSON.stringify(s)), NOMINAL_STATE);
  await page.evaluate(() => {
    (window as any).__sent = [];
    (window as any).__sendAction = (json: string) => (window as any).__sent.push(json);
  });

  // Click Fore segment (focused in nominal state) — toggles focus off.
  await page.locator('.shield-segment[data-arc-id="fore"]').click();

  const sent: string[] = await page.evaluate(() => (window as any).__sent);
  expect(sent).toHaveLength(1);
  expect(JSON.parse(sent[0])).toEqual({
    action: 'set_shield_focus',
    console: 'shields',
    arc_id: 'fore',
    focused: false,
  });
});

test('shields console: header shows focused facing display', async ({ page }) => {
  await page.goto(CONSOLE_URL);

  const foreState = { ...NOMINAL_STATE, focused_facing: 'Fore' };
  await page.evaluate((s) => (window as any).__updateConsole('Shields', JSON.stringify(s)), foreState);

  await expect(page.locator('#focus-display')).toHaveText('Fore');
  await expect(page.locator('#footer-focus')).toHaveText('Fore');
});

test('shields console: grid status shown in footer', async ({ page }) => {
  await page.goto(CONSOLE_URL);

  await page.evaluate((s) => (window as any).__updateConsole('Shields', JSON.stringify(s)), NOMINAL_STATE);

  await expect(page.locator('#footer-grid')).toHaveText('GRID NOMINAL');
});
