// Issue #425 — Smoke test: HTML repair console tracer bullet.
//
// The repair console page (gui/repair-console.html) is a static HTML file
// copied into dist/gui/ by Trunk. It exercises the ADR-0001 bridge contract:
//   - window.__updateConsole(name, stateJson) renders RepairConsoleState into DOM.
//   - Dispatch buttons call window.__sendAction(...) with the action envelope.

import { test, expect } from './fixtures';

const CONSOLE_URL = '/gui/repair-console.html';

test('repair console: __updateConsole renders team slots with correct states', async ({ page }) => {
  await page.goto(CONSOLE_URL);

  const state = {
    teams: [
      'Idle',
      { Travelling: { console: 'Helm', elapsed: 1.0 } },
      { Repairing:  { console: 'Tactical' } },
    ],
    damageable_consoles: ['Helm', 'Tactical', 'Sensors', 'Shields'],
    console_hull: [
      { console: 'Helm',     current: 80.0, max_hp: 100.0 },
      { console: 'Tactical', current: 50.0, max_hp: 100.0 },
      { console: 'Sensors',  current: 100.0, max_hp: 100.0 },
      { console: 'Shields',  current: 90.0, max_hp: 100.0 },
    ],
    travel_duration_secs: 5.0,
  };

  await page.evaluate((s) => (window as any).__updateConsole('Repair', JSON.stringify(s)), state);

  // Three team-slot entries rendered in #teams-data.
  await expect(page.locator('#teams-data .team-slot')).toHaveCount(3);

  // Slot 0: Idle.
  const slot0 = page.locator('.team-slot[data-idx="0"]');
  await expect(slot0).toHaveAttribute('data-state', 'Idle');

  // Slot 1: Travelling to Helm.
  const slot1 = page.locator('.team-slot[data-idx="1"]');
  await expect(slot1).toHaveAttribute('data-state', 'Travelling');
  await expect(slot1).toHaveAttribute('data-console', 'Helm');

  // Slot 2: Repairing Tactical.
  const slot2 = page.locator('.team-slot[data-idx="2"]');
  await expect(slot2).toHaveAttribute('data-state', 'Repairing');
  await expect(slot2).toHaveAttribute('data-console', 'Tactical');
});

test('repair console: __updateConsole renders Returning slot with queued console', async ({ page }) => {
  await page.goto(CONSOLE_URL);

  const state = {
    teams: [
      { Returning: { remaining: 3.0, queued: 'Sensors' } },
    ],
    damageable_consoles: ['Helm', 'Tactical', 'Sensors'],
    console_hull: [
      { console: 'Helm',     current: 100.0, max_hp: 100.0 },
      { console: 'Tactical', current: 100.0, max_hp: 100.0 },
      { console: 'Sensors',  current: 60.0,  max_hp: 100.0 },
    ],
    travel_duration_secs: 5.0,
  };

  await page.evaluate((s) => (window as any).__updateConsole('Repair', JSON.stringify(s)), state);

  const slot0 = page.locator('.team-slot[data-idx="0"]');
  await expect(slot0).toHaveAttribute('data-state', 'Returning');
  await expect(slot0).toHaveAttribute('data-queued', 'Sensors');
});

test('repair console: dispatch buttons call __sendAction with correct envelope', async ({ page }) => {
  await page.goto(CONSOLE_URL);

  // Stub __sendAction to capture calls.
  await page.evaluate(() => {
    (window as any).__sent = [];
    (window as any).__sendAction = (json: string) => (window as any).__sent.push(json);
  });

  const state = {
    teams: ['Idle', 'Idle'],
    damageable_consoles: ['Helm', 'Tactical', 'Sensors'],
    console_hull: [
      { console: 'Helm',     current: 80.0, max_hp: 100.0 },
      { console: 'Tactical', current: 50.0, max_hp: 100.0 },
      { console: 'Sensors',  current: 70.0, max_hp: 100.0 },
    ],
    travel_duration_secs: 5.0,
  };

  await page.evaluate((s) => (window as any).__updateConsole('Repair', JSON.stringify(s)), state);

  // Dispatch team 0 → Helm.
  await page.locator('.dispatch-btn[data-console="Helm"][data-team="0"]').click();

  // Dispatch team 1 → Tactical.
  await page.locator('.dispatch-btn[data-console="Tactical"][data-team="1"]').click();

  const sent: string[] = await page.evaluate(() => (window as any).__sent);
  expect(sent).toHaveLength(2);

  expect(JSON.parse(sent[0])).toEqual({
    action: 'dispatch_repair_team',
    console: 'Repair',
    team_idx: 0,
    target: 'Helm',
  });
  expect(JSON.parse(sent[1])).toEqual({
    action: 'dispatch_repair_team',
    console: 'Repair',
    team_idx: 1,
    target: 'Tactical',
  });
});
