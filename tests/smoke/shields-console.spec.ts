// Issue #424 — Smoke test: HTML shields console tracer bullet.
//
// The shields console page (gui/shield-console.html) is a static HTML file
// copied into dist/gui/ by Trunk. It exercises the ADR-0001 bridge contract:
//   - window.__updateConsole(name, stateJson) renders ShieldsConsoleState into DOM.
//   - Focus buttons call window.__sendAction(...) with the action envelope.

import { test, expect } from './fixtures';

const CONSOLE_URL = '/gui/shield-console.html';

const NOMINAL_STATE = {
  facings: [
    { label: 'Fore', hp: 100, max_hp: 100, online: true, offline_remaining: 0, is_focused: true },
    { label: 'Port', hp: 72, max_hp: 100, online: true, offline_remaining: 0, is_focused: false },
    { label: 'Aft', hp: 0, max_hp: 100, online: false, offline_remaining: 8, is_focused: false },
    { label: 'Starboard', hp: 88, max_hp: 100, online: true, offline_remaining: 0, is_focused: false },
  ],
  hull_integrity_pct: 78,
  focused_facing: 'Fore',
  target_bearing: 272,
  grid_status: 'GRID NOMINAL',
};

test('shields console: __updateConsole renders four quadrant cards', async ({ page }) => {
  await page.goto(CONSOLE_URL);

  await page.evaluate((s) => (window as any).__updateConsole('Shields', JSON.stringify(s)), NOMINAL_STATE);

  await expect(page.locator('.quad-card')).toHaveCount(4);
});

test('shields console: facing values render from state', async ({ page }) => {
  await page.goto(CONSOLE_URL);

  await page.evaluate((s) => (window as any).__updateConsole('Shields', JSON.stringify(s)), NOMINAL_STATE);

  // FORE = 100%
  const foreCard = page.locator('.quad-card').nth(0);
  await expect(foreCard.locator('.qc-name')).toHaveText('FORE');

  // AFT = 0% (down)
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

test('shields console: shield segment click sends set_shield_focus', async ({ page }) => {
  await page.goto(CONSOLE_URL);

  await page.evaluate(() => {
    (window as any).__sent = [];
    (window as any).__sendAction = (json: string) => (window as any).__sent.push(json);
  });

  // Click Port segment (non-focused in demo state) to set focus to Port.
  await page.locator('.shield-segment[data-facing="Port"]').click();

  const sent: string[] = await page.evaluate(() => (window as any).__sent);
  expect(sent).toHaveLength(1);
  expect(JSON.parse(sent[0])).toEqual({
    action: 'set_shield_focus',
    console: 'Shields',
    facing: 'Port',
  });
});

test('shields console: clicking focused facing clears focus via null', async ({ page }) => {
  await page.goto(CONSOLE_URL);

  await page.evaluate(() => {
    (window as any).__sent = [];
    (window as any).__sendAction = (json: string) => (window as any).__sent.push(json);
  });

  // Click Fore segment (focused in demo state) — toggles to null.
  await page.locator('.shield-segment[data-facing="Fore"]').click();

  const sent: string[] = await page.evaluate(() => (window as any).__sent);
  expect(sent).toHaveLength(1);
  expect(JSON.parse(sent[0])).toEqual({
    action: 'set_shield_focus',
    console: 'Shields',
    facing: null,
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
