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

  // Post issue #619: `TeamSlot` variants carry SystemId-keyed fields
  // (`system_id`, `queued_system_id`) plus their display-name mirrors.
  // The repair console HTML populates `data-console` / `data-queued` from
  // the `system_id` (lowercase station id), not the display name.
  const state = {
    teams: [
      'Idle',
      { Travelling: { system_id: 'helm', display_name: 'Helm', elapsed: 1.0 } },
      { Repairing:  { system_id: 'tactical', display_name: 'Tactical' } },
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

  // Slot 1: Travelling to helm (data-console is the lowercase system_id).
  const slot1 = page.locator('.team-slot[data-idx="1"]');
  await expect(slot1).toHaveAttribute('data-state', 'Travelling');
  await expect(slot1).toHaveAttribute('data-console', 'helm');

  // Slot 2: Repairing tactical (data-console is the lowercase system_id).
  const slot2 = page.locator('.team-slot[data-idx="2"]');
  await expect(slot2).toHaveAttribute('data-state', 'Repairing');
  await expect(slot2).toHaveAttribute('data-console', 'tactical');
});

test('repair console: __updateConsole renders Returning slot with queued console', async ({ page }) => {
  await page.goto(CONSOLE_URL);

  const state = {
    teams: [
      { Returning: {
        remaining: 3.0,
        system_id: null,
        display_name: null,
        queued_system_id: 'sensors',
        queued_display_name: 'Sensors',
      } },
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
  // data-queued is populated from the lowercase queued_system_id.
  await expect(slot0).toHaveAttribute('data-queued', 'sensors');
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
