// Issue #425 — Smoke test: HTML repair console tracer bullet.
//
// The repair console page (gui/repair-console.html) is a static HTML file
// copied into dist/gui/ by Trunk. It exercises the ADR-0001 bridge contract:
//   - window.__updateConsole(name, stateJson) renders RepairConsoleState into DOM.
//   - Dispatch buttons (inside the <ph-repair-teams> component's shadow DOM)
//     call window.__sendAction(...) with the action envelope.
//
// The console now renders via the shared component pattern (ph-hull-integrity
// for the ship-wide hull bar, ph-station-damage for the click-to-expand
// "Core" bar, ph-repair-teams for per-team dispatch) instead of bespoke
// inline DOM, matching the Cruiser/Destroyer engineering consoles.

import { test, expect } from './fixtures';

const CONSOLE_URL = '/gui/repair-console.html';

function repairState(overrides: Record<string, unknown> = {}) {
  return Object.assign(
    {
      teams: [
        { id: 0, label: 'Team 1', status: 'idle', target: '', progress_pct: 0 },
        { id: 1, label: 'Team 2', status: 'idle', target: '', progress_pct: 0 },
      ],
      dispatch_targets: [
        { id: 'helm', label: 'Helm', damage_pct: 0.2 },
        { id: 'tactical', label: 'Tactical', damage_pct: 0.5 },
        { id: 'core', label: 'Core', damage_pct: 0.1 },
      ],
      core_systems: [
        { system_id: 'core', display_name: 'Core', current: 18.0, max_hp: 20.0 },
      ],
      overall_hull: { current: 80, max: 100, pct: 0.8 },
      travel_duration_secs: 5.0,
      repair_auto: false,
    },
    overrides,
  );
}

test('repair console: renders overall hull and dispatch targets', async ({ page }) => {
  await page.goto(CONSOLE_URL);

  await page.evaluate((s) => (window as any).__updateConsole('repair', JSON.stringify(s)), repairState());

  // Ship-wide hull bar reflects overall_hull.pct (80%).
  const fill = page.locator('ph-hull-integrity ph-damage-bar').locator('#bar-fill');
  const width = await fill.evaluate((el) => (el as HTMLElement).style.width);
  expect(width).toBe('80%');

  // Two idle team cards, each offering the three dispatch targets.
  await expect(page.locator('ph-repair-teams .card')).toHaveCount(2);
  await expect(page.locator('ph-repair-teams .card[data-team-id="0"] .btn')).toHaveCount(3);
});

test('repair console: Core bar hides when there are no damageable core systems', async ({ page }) => {
  await page.goto(CONSOLE_URL);

  await page.evaluate((s) => (window as any).__updateConsole('repair', JSON.stringify(s)), repairState({ core_systems: [] }));

  await expect(page.locator('#core-damage')).toBeHidden();
});

test('repair console: Core bar shows and pops up damaged core systems when clicked', async ({ page }) => {
  await page.goto(CONSOLE_URL);

  await page.evaluate((s) => (window as any).__updateConsole('repair', JSON.stringify(s)), repairState());

  const coreBar = page.locator('#core-damage');
  await expect(coreBar).toBeVisible();

  await coreBar.locator('.bar').click();
  await expect(coreBar.locator('.popup')).toHaveClass(/open/);
  await expect(coreBar.locator('ph-damage-detail .row')).toHaveCount(1);
});

test('repair console: dispatch buttons call __sendAction with correct envelope', async ({ page }) => {
  await page.goto(CONSOLE_URL);

  // Stub __sendAction to capture calls.
  await page.evaluate(() => {
    (window as any).__sent = [];
    (window as any).__sendAction = (json: string) => (window as any).__sent.push(json);
  });

  await page.evaluate((s) => (window as any).__updateConsole('repair', JSON.stringify(s)), repairState());

  // Dispatch team 0 → helm.
  await page.locator('ph-repair-teams .card[data-team-id="0"] .btn[data-target="helm"]').click();

  // Dispatch team 1 → tactical.
  await page.locator('ph-repair-teams .card[data-team-id="1"] .btn[data-target="tactical"]').click();

  const sent: string[] = await page.evaluate(() => (window as any).__sent);
  expect(sent).toHaveLength(2);

  expect(JSON.parse(sent[0])).toEqual({
    action: 'dispatch_repair_team',
    console: 'repair',
    team_idx: 0,
    target: 'helm',
  });
  expect(JSON.parse(sent[1])).toEqual({
    action: 'dispatch_repair_team',
    console: 'repair',
    team_idx: 1,
    target: 'tactical',
  });
});
