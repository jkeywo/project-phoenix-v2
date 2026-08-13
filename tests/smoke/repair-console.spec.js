import { test, expect } from './fixtures';

const CONSOLE_URL = '/gui/battleship/repair.html';

function repairState(overrides = {}) {
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
      // Issue #1015: the tap-to-prioritise list. Worst-first, as the host folds
      // it; `prioritised` is the host's own resolved pin.
      damaged_systems: [
        { system_id: 'aux-sensor', display_name: 'Auxiliary Sensor', tier: 'Destroyed', current: 0, max_hp: 10, damage_pct: 1.0, prioritised: false, in_progress: false },
        { system_id: 'hull-plating', display_name: 'Hull Plating', tier: 'Disabled', current: 2, max_hp: 20, damage_pct: 0.9, prioritised: false, in_progress: true },
        { system_id: 'core', display_name: 'Core', tier: 'Damaged', current: 12, max_hp: 20, damage_pct: 0.4, prioritised: true, in_progress: false },
      ],
    },
    overrides,
  );
}

test('repair console: renders overall hull and dispatch targets', async ({ page }) => {
  await page.goto(CONSOLE_URL);
  await page.evaluate((s) => window.__updateConsole('repair', JSON.stringify(s)), repairState());

  const fill = page.locator('ph-hull-integrity ph-damage-bar').locator('#bar-fill');
  const width = await fill.evaluate((el) => el.style.width);
  expect(width).toBe('80%');
  await expect(page.locator('ph-repair-teams .card')).toHaveCount(2);
  await expect(page.locator('ph-repair-teams .card[data-team-id="0"] .btn')).toHaveCount(3);
});

test('repair console: Core bar hides when there are no damageable core systems', async ({ page }) => {
  await page.goto(CONSOLE_URL);
  await page.evaluate((s) => window.__updateConsole('repair', JSON.stringify(s)), repairState({ core_systems: [] }));
  await expect(page.locator('#core-damage')).toBeHidden();
});

test('repair console: Core bar shows and pops up damaged core systems when clicked', async ({ page }) => {
  await page.goto(CONSOLE_URL);
  await page.evaluate((s) => window.__updateConsole('repair', JSON.stringify(s)), repairState());
  const coreBar = page.locator('#core-damage');
  await expect(coreBar).toBeVisible();
  await coreBar.locator('.bar').click();
  await expect(coreBar.locator('.popup')).toHaveClass(/open/);
  await expect(coreBar.locator('ph-damage-detail .row')).toHaveCount(1);
});

test('repair console: dispatch buttons call __sendAction with correct envelope', async ({ page }) => {
  await page.goto(CONSOLE_URL);
  await page.evaluate(() => {
    window.__sent = [];
    window.__sendAction = (json) => window.__sent.push(json);
  });
  await page.evaluate((s) => window.__updateConsole('repair', JSON.stringify(s)), repairState());
  await page.locator('ph-repair-teams .card[data-team-id="0"] .btn[data-target="helm"]').click();
  await page.locator('ph-repair-teams .card[data-team-id="1"] .btn[data-target="tactical"]').click();

  const sent = await page.evaluate(() => window.__sent);
  expect(sent).toHaveLength(2);
  expect(JSON.parse(sent[0])).toEqual({ action: 'dispatch_repair_team', console: 'repair', team_idx: 0, target: 'helm' });
  expect(JSON.parse(sent[1])).toEqual({ action: 'dispatch_repair_team', console: 'repair', team_idx: 1, target: 'tactical' });
});

test('repair console: damaged-systems list renders worst-first and highlights the host pin', async ({ page }) => {
  await page.goto(CONSOLE_URL);
  await page.evaluate((s) => window.__updateConsole('repair', JSON.stringify(s)), repairState());

  const rows = page.locator('ph-repair-teams .dmg-row');
  await expect(rows).toHaveCount(3);
  await expect(rows.nth(0)).toHaveAttribute('data-system-id', 'aux-sensor');
  await expect(rows.nth(2)).toHaveAttribute('data-system-id', 'core');
  // Exactly one highlight, and it is the row the HOST pinned — not the worst
  // row, which is how you can tell nothing here re-derived the choice.
  await expect(page.locator('ph-repair-teams .dmg-row.prioritised')).toHaveCount(1);
  await expect(page.locator('ph-repair-teams .dmg-row.prioritised')).toHaveAttribute('data-system-id', 'core');
  // The per-team 1/2/3 ordinal buttons this list replaced are gone.
  await expect(page.locator('ph-repair-teams .priority-btn')).toHaveCount(0);
});

test('repair console: tapping a damaged system sends set_repair_target_priority', async ({ page }) => {
  await page.goto(CONSOLE_URL);
  await page.evaluate(() => {
    window.__sent = [];
    window.__sendAction = (json) => window.__sent.push(json);
  });
  await page.evaluate((s) => window.__updateConsole('repair', JSON.stringify(s)), repairState());
  await page.locator('ph-repair-teams .dmg-row[data-system-id="aux-sensor"]').click();

  const sent = await page.evaluate(() => window.__sent);
  // No team index and no ordinal: the host resolves which team and pins the
  // system; the ordinal is untouched.
  expect(sent).toHaveLength(1);
  expect(JSON.parse(sent[0])).toEqual({
    action: 'set_repair_target_priority', console: 'repair', system_id: 'aux-sensor',
  });
});

test('repair console: a row a team is already on is shown but not offered', async ({ page }) => {
  await page.goto(CONSOLE_URL);
  await page.evaluate((s) => window.__updateConsole('repair', JSON.stringify(s)), repairState());

  // `hull-plating` is the in_progress row. A tap on it is structurally a no-op —
  // the host's sweep never offers the system a team is standing on as a
  // candidate — so it is rendered (the damage is real) but not offered as a
  // control. `core`, which the host pinned, stays live.
  await expect(page.locator('ph-repair-teams .dmg-row[data-system-id="hull-plating"]'))
    .toBeDisabled();
  await expect(page.locator('ph-repair-teams .dmg-row[data-system-id="core"]'))
    .toBeEnabled();
});

test('repair console: the damaged-systems section hides on an intact ship', async ({ page }) => {
  await page.goto(CONSOLE_URL);
  await page.evaluate((s) => window.__updateConsole('repair', JSON.stringify(s)), repairState({ damaged_systems: [] }));
  await expect(page.locator('ph-repair-teams .dmg-row')).toHaveCount(0);
  // Zero rows is not the same claim as "the section went away": the heading and
  // its container are what a player sees on an intact ship, and without this the
  // test passes just as happily against a visible, empty [DAMAGED SYSTEMS] box.
  await expect(page.locator('ph-repair-teams #damaged')).toBeHidden();
});
