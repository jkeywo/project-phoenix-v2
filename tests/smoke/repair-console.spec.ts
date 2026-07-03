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

  // Post issue #618: RepairBlackboard emits SystemId-keyed fields
  // (`system_hull` + `damageable_systems`) with lowercase station ids.
  // The `TeamSlot::Travelling.console` field is a display name (populated
  // via EntitySystemHull.display_name) — the smoke test keeps display
  // names as authored (title-cased) since that's what the wire produces.
  const state = {
    teams: [
      'Idle',
      { Travelling: { console: 'Helm', elapsed: 1.0 } },
      { Repairing:  { console: 'Tactical' } },
    ],
    damageable_systems: ['helm', 'tactical', 'sensors', 'shields'],
    system_hull: [
      { system_id: 'helm',     display_name: 'Helm',     current: 80.0,  max_hp: 100.0 },
      { system_id: 'tactical', display_name: 'Tactical', current: 50.0,  max_hp: 100.0 },
      { system_id: 'sensors',  display_name: 'Sensors',  current: 100.0, max_hp: 100.0 },
      { system_id: 'shields',  display_name: 'Shields',  current: 90.0,  max_hp: 100.0 },
    ],
    travel_duration_secs: 5.0,
  };

  await page.evaluate((s) => (window as any).__updateConsole('repair', JSON.stringify(s)), state);

  // Three team-slot entries rendered in #teams-data.
  await expect(page.locator('#teams-data .team-slot')).toHaveCount(3);

  // Slot 0: Idle.
  const slot0 = page.locator('.team-slot[data-idx="0"]');
  await expect(slot0).toHaveAttribute('data-state', 'Idle');

  // Slot 1: Travelling to Helm (display name, wire field is a string).
  const slot1 = page.locator('.team-slot[data-idx="1"]');
  await expect(slot1).toHaveAttribute('data-state', 'Travelling');
  await expect(slot1).toHaveAttribute('data-console', 'Helm');

  // Slot 2: Repairing Tactical (display name).
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
    damageable_systems: ['helm', 'tactical', 'sensors'],
    system_hull: [
      { system_id: 'helm',     display_name: 'Helm',     current: 100.0, max_hp: 100.0 },
      { system_id: 'tactical', display_name: 'Tactical', current: 100.0, max_hp: 100.0 },
      { system_id: 'sensors',  display_name: 'Sensors',  current: 60.0,  max_hp: 100.0 },
    ],
    travel_duration_secs: 5.0,
  };

  await page.evaluate((s) => (window as any).__updateConsole('repair', JSON.stringify(s)), state);

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
    damageable_systems: ['helm', 'tactical', 'sensors'],
    system_hull: [
      { system_id: 'helm',     display_name: 'Helm',     current: 80.0, max_hp: 100.0 },
      { system_id: 'tactical', display_name: 'Tactical', current: 50.0, max_hp: 100.0 },
      { system_id: 'sensors',  display_name: 'Sensors',  current: 70.0, max_hp: 100.0 },
    ],
    travel_duration_secs: 5.0,
  };

  await page.evaluate((s) => (window as any).__updateConsole('repair', JSON.stringify(s)), state);

  // Dispatch team 0 → helm (button data-console is the lowercase station id).
  await page.locator('.dispatch-btn[data-console="helm"][data-team="0"]').click();

  // Dispatch team 1 → tactical.
  await page.locator('.dispatch-btn[data-console="tactical"][data-team="1"]').click();

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
